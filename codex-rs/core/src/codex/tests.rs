use super::*;
use crate::CodexConversation;
use crate::config::ConfigOverrides;
use crate::config::ConfigToml;
use crate::protocol::CompactedItem;
use crate::protocol::InitialHistory;
use crate::protocol::ResumedHistory;
use codex_protocol::models::ContentItem;
use mcp_types::ContentBlock;
use mcp_types::TextContent;
use pretty_assertions::assert_eq;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use std::time::Instant;

fn build_session_for_subagent_tests(
) -> (Arc<Session>, TurnContext, async_channel::Receiver<Event>) {
    let (tx_event, rx_event) = async_channel::unbounded();

    let codex_home = tempfile::tempdir().expect("tempdir");
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        codex_home.path().to_path_buf(),
    )
    .expect("load default config");
    let config = Arc::new(config);

    let conversation_id = ConversationId::default();
    let client = ModelClient::new(
        Arc::clone(&config),
        None,
        config.model_provider.clone(),
        config.model_reasoning_effort,
        config.model_reasoning_summary,
        conversation_id,
    );
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_family: &config.model_family,
            include_plan_tool: config.include_plan_tool,
            include_apply_patch_tool: config.include_apply_patch_tool,
            include_web_search_request: config.tools_web_search_request,
            use_streamable_shell_tool: config.use_experimental_streamable_shell_tool,
            include_view_image_tool: config.include_view_image_tool,
            include_subagent_tool: config.include_subagent_tool,
            experimental_unified_exec_tool: config.use_experimental_unified_exec_tool,
        });

    let turn_context = TurnContext {
        client,
        cwd: config.cwd.clone(),
        base_instructions: config.base_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        approval_policy: config.approval_policy,
        sandbox_policy: config.sandbox_policy.clone(),
        shell_environment_policy: config.shell_environment_policy.clone(),
        tools_config,
        is_review_mode: false,
    };

    let session = Arc::new(Session {
        conversation_id,
        tx_event,
        mcp_connection_manager: McpConnectionManager::default(),
        session_manager: ExecSessionManager::default(),
        unified_exec_manager: UnifiedExecSessionManager::default(),
        auth_manager: crate::AuthManager::from_auth_for_testing(
            crate::CodexAuth::from_api_key("dummy"),
        ),
        notify: None,
        rollout: Mutex::new(None),
        state: Mutex::new(State {
            history: ConversationHistory::new(),
            ..Default::default()
        }),
        codex_linux_sandbox_exe: None,
        user_shell: shell::Shell::Unknown,
        show_raw_agent_reasoning: config.show_raw_agent_reasoning,
        next_internal_sub_id: AtomicU64::new(0),
        subagents: Mutex::new(HashMap::new()),
        mailbox: Mutex::new(Mailbox::default()),
        subagent_slots: Arc::new(Semaphore::new(DEFAULT_MAX_SUBAGENT_CONCURRENT)),
        subagent_settings: SubagentSettings {
            max_depth: DEFAULT_MAX_SUBAGENT_DEPTH,
            _max_concurrent: DEFAULT_MAX_SUBAGENT_CONCURRENT,
        },
    });

    (session, turn_context, rx_event)
}

#[test]
fn subagent_tool_calls_emit_background_events() {
    let (sess, turn_context, rx_event) = build_session_for_subagent_tests();

    // Open a subagent through the tool-call path to trigger a BackgroundEvent.
    let mut tracker = crate::turn_diff_tracker::TurnDiffTracker::new();
    let open_args = json!({ "goal": "test open" });
    let res = tokio_test::block_on(handle_custom_tool_call(
        sess.clone(),
        &turn_context,
        &mut tracker,
        "p1".to_string(),
        "subagent_open".to_string(),
        open_args.to_string(),
        "c-open".to_string(),
    ));

    // Expect a background event announcing the subagent
    let ev = tokio_test::block_on(rx_event.recv());
    match ev {
        Ok(Event {
            msg: EventMsg::BackgroundEvent(be),
            ..
        }) => {
            assert!(be.message.starts_with("Subagent "));
            assert!(be.message.contains("opened"));
        }
        other => panic!("expected BackgroundEvent, got {other:?}"),
    }

    // Parse subagent_id from the tool-call output
    let subagent_id = match res {
        ResponseInputItem::FunctionCallOutput { output, .. } => {
            let v: serde_json::Value = serde_json::from_str(&output.content).expect("json");
            v.get("subagent_id")
                .and_then(|s| s.as_str())
                .expect("subagent_id")
                .to_string()
        }
        other => panic!("unexpected output variant: {other:?}"),
    };

    // End the subagent through the tool-call path and expect a second BackgroundEvent.
    let end_args = json!({ "subagent_id": subagent_id, "persist": false });
    let _ = tokio_test::block_on(handle_custom_tool_call(
        sess,
        &turn_context,
        &mut tracker,
        "p1".to_string(),
        "subagent_end".to_string(),
        end_args.to_string(),
        "c-end".to_string(),
    ));

    let ev2 = tokio_test::block_on(rx_event.recv());
    match ev2 {
        Ok(Event {
            msg: EventMsg::BackgroundEvent(be),
            ..
        }) => {
            assert!(be.message.contains("ended"), "{be:?}");
        }
        other => panic!("expected BackgroundEvent end, got {other:?}"),
    }
}

#[tokio::test]
async fn subagent_reply_emits_background_event() {
    let (session, turn_context, rx_event) = build_session_for_subagent_tests();

    // Open a subagent with a small idle timeout so the reply finishes quickly via timeout.
    let open = session
        .open_subagent(
            &turn_context,
            SubagentOpenArgs {
                goal: "reply-notify".to_string(),
                system_prompt: None,
                model: None,
                approval_policy: None,
                sandbox_mode: None,
                cwd: None,
                max_turns: None,
                max_runtime_ms: Some(25),
            },
        )
        .await
        .expect("open_subagent");

    // Swap in a dummy conversation so `submit` succeeds and `next_event` blocks
    // until the idle timeout, independent of external services.
    let (tx_sub, _rx_sub) = async_channel::bounded(SUBMISSION_CHANNEL_CAPACITY);
    let (_tx_ev_unused, rx_ev_unused) = async_channel::unbounded();
    let dummy_codex = Codex {
        next_id: AtomicU64::new(0),
        tx_sub,
        rx_event: rx_ev_unused,
    };
    let dummy_conversation = Arc::new(CodexConversation::new(dummy_codex));
    {
        let mut subs = session.subagents.lock().await;
        let st = subs.get_mut(&open.subagent_id).expect("state");
        st.conversation = dummy_conversation;
    }

    // Trigger a reply (which will time out quickly). The wrapper should emit a BackgroundEvent.
    let _ = session
        .subagent_reply_blocking(&SubagentReplyArgs {
            subagent_id: open.subagent_id.clone(),
            message: "ping".to_string(),
            images: None,
            mode: None,
            timeout_ms: None,
        })
        .await
        .expect("subagent_reply_blocking");

    // Receive the background event with a timeout so the test fails fast on regression.
    let ev = tokio::time::timeout(StdDuration::from_secs(2), rx_event.recv()).await;

    match ev {
        Ok(Ok(Event {
            msg: EventMsg::BackgroundEvent(be),
            ..
        })) => {
            assert!(be.message.contains("Subagent "));
            assert!(be.message.contains("replied"), "{be:?}");
        }
        other => panic!("expected BackgroundEvent reply, got {other:?}"),
    }
}

#[tokio::test]
async fn subagent_reply_nonblocking_is_async() {
    use tokio::time::{sleep, timeout};

    let (session, turn_context, rx_event) = build_session_for_subagent_tests();

    let open = session
        .open_subagent(
            &turn_context,
            SubagentOpenArgs {
                goal: "nonblocking reply".to_string(),
                system_prompt: None,
                model: None,
                approval_policy: None,
                sandbox_mode: None,
                cwd: None,
                max_turns: None,
                max_runtime_ms: None,
            },
        )
        .await
        .expect("open subagent");

    let subagent_id = open.subagent_id.clone();
    let description = open.description.clone();

    let (child_tx_sub, child_rx_sub) = async_channel::bounded(SUBMISSION_CHANNEL_CAPACITY);
    let (child_tx_event, child_rx_event) = async_channel::unbounded();
    let dummy_codex = Codex {
        next_id: AtomicU64::new(0),
        tx_sub: child_tx_sub,
        rx_event: child_rx_event,
    };
    let dummy_conversation = Arc::new(CodexConversation::new(dummy_codex));

    {
        let mut subs = session.subagents.lock().await;
        let state = subs
            .get_mut(&subagent_id)
            .expect("subagent state present");
        state.conversation = dummy_conversation;
    }

    while rx_event.try_recv().is_ok() {}

    let completion_delay = StdDuration::from_millis(200);
    let event_task = tokio::spawn({
        let child_tx_event = child_tx_event.clone();
        async move {
            let submission = child_rx_sub.recv().await.expect("submission");
            sleep(completion_delay).await;
            child_tx_event
                .send(Event {
                    id: submission.id,
                    msg: EventMsg::TaskComplete(TaskCompleteEvent {
                        last_agent_message: Some("async work complete".to_string()),
                    }),
                })
                .await
                .expect("send task complete");
        }
    });

    let mut tracker = crate::turn_diff_tracker::TurnDiffTracker::new();
    let args = json!({
        "subagent_id": subagent_id.clone(),
        "message": "do work",
        "mode": "nonblocking",
    });

    let call_start = Instant::now();
    let response = handle_custom_tool_call(
        session.clone(),
        &turn_context,
        &mut tracker,
        "parent".to_string(),
        "subagent_reply".to_string(),
        args.to_string(),
        "call".to_string(),
    )
    .await;

    let accepted = match response {
        ResponseInputItem::FunctionCallOutput { output, .. } => {
            let value: serde_json::Value =
                serde_json::from_str(&output.content).expect("valid json output");
            value
                .get("accepted")
                .and_then(|v| v.as_bool())
                .expect("accepted flag present")
        }
        other => panic!("unexpected response variant: {other:?}"),
    };

    assert!(accepted, "nonblocking reply should be accepted immediately");
    assert!(
        call_start.elapsed() < StdDuration::from_millis(80),
        "nonblocking call returned too slowly"
    );

    let event = timeout(StdDuration::from_secs(2), rx_event.recv())
        .await
        .expect("background event timed out")
        .expect("event channel closed");

    assert!(
        call_start.elapsed() >= completion_delay,
        "background event arrived before subagent finished"
    );

    match event {
        Event {
            msg: EventMsg::BackgroundEvent(be),
            ..
        } => {
            assert!(be.message.contains("replied"), "{be:?}");
            assert!(be.message.contains(&description), "{be:?}");
        }
        other => panic!("expected background reply event, got {other:?}"),
    }

    {
        let subs = session.subagents.lock().await;
        let state = subs.get(&subagent_id).expect("subagent state");
        assert!(!state.running, "subagent should be idle after completion");
    }

    event_task.await.expect("background task completed");
}

#[test]
fn subagent_depth_one_allows_multiple_siblings_until_concurrency() {
    let (mut session, turn_context) = make_session_and_context();

    // depth=1 means root may spawn children; children may not spawn more.
    session.subagent_settings.max_depth = 1;

    let mk = |goal: &str| SubagentOpenArgs {
        goal: goal.to_string(),
        system_prompt: None,
        model: None,
        approval_policy: None,
        sandbox_mode: None,
        cwd: None,
        max_turns: None,
        max_runtime_ms: None,
    };

    let _one = tokio_test::block_on(session.open_subagent(&turn_context, mk("one")))
        .expect("first subagent should open");
    let _two = tokio_test::block_on(session.open_subagent(&turn_context, mk("two")))
        .expect("second subagent should open (sibling)");

    // Default concurrency is 2; third should fail with concurrency, not depth.
    let err = tokio_test::block_on(session.open_subagent(&turn_context, mk("three")))
        .expect_err("third subagent should be limited by concurrency");
    assert!(err.contains("maximum concurrent subagents"), "{err}");
}

#[test]
fn subagent_depth_limit_zero_disallows_open() {
    let (mut session, turn_context) = make_session_and_context();
    // Disallow any subagent
    session.subagent_settings.max_depth = 0;

    let args = SubagentOpenArgs {
        goal: "nope".to_string(),
        system_prompt: None,
        model: None,
        approval_policy: None,
        sandbox_mode: None,
        cwd: None,
        max_turns: None,
        max_runtime_ms: None,
    };
    let err = tokio_test::block_on(session.open_subagent(&turn_context, args))
        .expect_err("should fail when max_depth=0");
    assert!(err.contains("subagent depth limit"), "{err}");
}

#[test]
fn reconstruct_history_matches_live_compactions() {
    let (session, turn_context) = make_session_and_context();
    let (rollout_items, expected) = sample_rollout(&session, &turn_context);

    let reconstructed = session.reconstruct_history_from_rollout(&turn_context, &rollout_items);

    assert_eq!(expected, reconstructed);
}

#[test]
fn record_initial_history_reconstructs_resumed_transcript() {
    let (session, turn_context) = make_session_and_context();
    let (rollout_items, expected) = sample_rollout(&session, &turn_context);

    tokio_test::block_on(session.record_initial_history(
        &turn_context,
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: ConversationId::default(),
            history: rollout_items,
            rollout_path: PathBuf::from("/tmp/resume.jsonl"),
        }),
    ));

    let actual = tokio_test::block_on(async { session.state.lock().await.history.contents() });
    assert_eq!(expected, actual);
}

#[test]
fn record_initial_history_reconstructs_forked_transcript() {
    let (session, turn_context) = make_session_and_context();
    let (rollout_items, expected) = sample_rollout(&session, &turn_context);

    tokio_test::block_on(
        session.record_initial_history(&turn_context, InitialHistory::Forked(rollout_items)),
    );

    let actual = tokio_test::block_on(async { session.state.lock().await.history.contents() });
    assert_eq!(expected, actual);
}

#[test]
fn prefers_structured_content_when_present() {
    let ctr = CallToolResult {
        // Content present but should be ignored because structured_content is set.
        content: vec![text_block("ignored")],
        is_error: None,
        structured_content: Some(json!({
            "ok": true,
            "value": 42
        })),
    };

    let got = convert_call_tool_result_to_function_call_output_payload(&ctr);
    let expected = FunctionCallOutputPayload {
        content: serde_json::to_string(&json!({
            "ok": true,
            "value": 42
        }))
        .unwrap(),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

#[test]
fn model_truncation_head_tail_by_lines() {
    // Build 400 short lines so line-count limit, not byte budget, triggers truncation
    let lines: Vec<String> = (1..=400).map(|i| format!("line{i}")).collect();
    let full = lines.join("\n");

    let exec = ExecToolCallOutput {
        exit_code: 0,
        stdout: StreamOutput::new(String::new()),
        stderr: StreamOutput::new(String::new()),
        aggregated_output: StreamOutput::new(full),
        duration: StdDuration::from_secs(1),
        timed_out: false,
        was_cancelled: false,
    };

    let out = format_exec_output_str(&exec);

    // Expect elision marker with correct counts
    let omitted = 400 - MODEL_FORMAT_MAX_LINES; // 144
    let marker = format!("\n[... omitted {omitted} of 400 lines ...]\n\n");
    assert!(out.contains(&marker), "missing marker: {out}");

    // Validate head and tail
    let parts: Vec<&str> = out.split(&marker).collect();
    assert_eq!(parts.len(), 2, "expected one marker split");
    let head = parts[0];
    let tail = parts[1];

    let expected_head: String = (1..=MODEL_FORMAT_HEAD_LINES)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(head.starts_with(&expected_head), "head mismatch");

    let expected_tail: String = ((400 - MODEL_FORMAT_TAIL_LINES + 1)..=400)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(tail.ends_with(&expected_tail), "tail mismatch");
}

#[test]
fn model_truncation_respects_byte_budget() {
    // Construct a large output (about 100kB) so byte budget dominates
    let big_line = "x".repeat(100);
    let full = std::iter::repeat_n(big_line, 1000)
        .collect::<Vec<_>>()
        .join("\n");

    let exec = ExecToolCallOutput {
        exit_code: 0,
        stdout: StreamOutput::new(String::new()),
        stderr: StreamOutput::new(String::new()),
        aggregated_output: StreamOutput::new(full.clone()),
        duration: StdDuration::from_secs(1),
        timed_out: false,
        was_cancelled: false,
    };

    let out = format_exec_output_str(&exec);
    assert!(out.len() <= MODEL_FORMAT_MAX_BYTES, "exceeds byte budget");
    assert!(out.contains("omitted"), "should contain elision marker");

    // Ensure head and tail are drawn from the original
    assert!(full.starts_with(out.chars().take(8).collect::<String>().as_str()));
    assert!(
        full.ends_with(
            out.chars()
                .rev()
                .take(8)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
                .as_str()
        )
    );
}

#[test]
fn includes_timed_out_message() {
    let exec = ExecToolCallOutput {
        exit_code: 0,
        stdout: StreamOutput::new(String::new()),
        stderr: StreamOutput::new(String::new()),
        aggregated_output: StreamOutput::new("Command output".to_string()),
        duration: StdDuration::from_secs(1),
        timed_out: true,
        was_cancelled: false,
    };

    let out = format_exec_output_str(&exec);

    assert_eq!(
        out,
        "command timed out after 1000 milliseconds\nCommand output"
    );
}

#[test]
fn falls_back_to_content_when_structured_is_null() {
    let ctr = CallToolResult {
        content: vec![text_block("hello"), text_block("world")],
        is_error: None,
        structured_content: Some(serde_json::Value::Null),
    };

    let got = convert_call_tool_result_to_function_call_output_payload(&ctr);
    let expected = FunctionCallOutputPayload {
        content: serde_json::to_string(&vec![text_block("hello"), text_block("world")])
            .unwrap(),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

#[test]
fn success_flag_reflects_is_error_true() {
    let ctr = CallToolResult {
        content: vec![text_block("unused")],
        is_error: Some(true),
        structured_content: Some(json!({ "message": "bad" })),
    };

    let got = convert_call_tool_result_to_function_call_output_payload(&ctr);
    let expected = FunctionCallOutputPayload {
        content: serde_json::to_string(&json!({ "message": "bad" })).unwrap(),
        success: Some(false),
    };

    assert_eq!(expected, got);
}

#[test]
fn success_flag_true_with_no_error_and_content_used() {
    let ctr = CallToolResult {
        content: vec![text_block("alpha")],
        is_error: Some(false),
        structured_content: None,
    };

    let got = convert_call_tool_result_to_function_call_output_payload(&ctr);
    let expected = FunctionCallOutputPayload {
        content: serde_json::to_string(&vec![text_block("alpha")]).unwrap(),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

fn text_block(s: &str) -> ContentBlock {
    ContentBlock::TextContent(TextContent {
        annotations: None,
        text: s.to_string(),
        r#type: "text".to_string(),
    })
}

pub(crate) fn make_session_and_context() -> (Session, TurnContext) {
    let (tx_event, _rx_event) = async_channel::unbounded();
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        codex_home.path().to_path_buf(),
    )
    .expect("load default test config");
    let config = Arc::new(config);
    let conversation_id = ConversationId::default();
    let client = ModelClient::new(
        config.clone(),
        None,
        config.model_provider.clone(),
        config.model_reasoning_effort,
        config.model_reasoning_summary,
        conversation_id,
    );
        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_family: &config.model_family,
            include_plan_tool: config.include_plan_tool,
            include_apply_patch_tool: config.include_apply_patch_tool,
            include_web_search_request: config.tools_web_search_request,
            use_streamable_shell_tool: config.use_experimental_streamable_shell_tool,
            include_view_image_tool: config.include_view_image_tool,
            include_subagent_tool: config.include_subagent_tool,
            experimental_unified_exec_tool: config.use_experimental_unified_exec_tool,
        });
    let turn_context = TurnContext {
        client,
        cwd: config.cwd.clone(),
        base_instructions: config.base_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        approval_policy: config.approval_policy,
        sandbox_policy: config.sandbox_policy.clone(),
        shell_environment_policy: config.shell_environment_policy.clone(),
        tools_config,
        is_review_mode: false,
    };
    let session = Session {
        conversation_id,
        tx_event,
        mcp_connection_manager: McpConnectionManager::default(),
        session_manager: ExecSessionManager::default(),
        unified_exec_manager: UnifiedExecSessionManager::default(),
        auth_manager: crate::AuthManager::from_auth_for_testing(
            crate::CodexAuth::from_api_key("dummy"),
        ),
        notify: None,
        rollout: Mutex::new(None),
        state: Mutex::new(State {
            history: ConversationHistory::new(),
            ..Default::default()
        }),
        codex_linux_sandbox_exe: None,
        user_shell: shell::Shell::Unknown,
        show_raw_agent_reasoning: config.show_raw_agent_reasoning,
        next_internal_sub_id: AtomicU64::new(0),
        subagents: Mutex::new(HashMap::new()),
        mailbox: Mutex::new(Mailbox::default()),
        subagent_slots: Arc::new(Semaphore::new(DEFAULT_MAX_SUBAGENT_CONCURRENT)),
        subagent_settings: SubagentSettings {
            max_depth: DEFAULT_MAX_SUBAGENT_DEPTH,
            _max_concurrent: DEFAULT_MAX_SUBAGENT_CONCURRENT,
        },
    };
    (session, turn_context)
}

fn sample_rollout(
    session: &Session,
    turn_context: &TurnContext,
) -> (Vec<RolloutItem>, Vec<ResponseItem>) {
    let mut rollout_items = Vec::new();
    let mut live_history = ConversationHistory::new();

    let initial_context = session.build_initial_context(turn_context);
    for item in &initial_context {
        rollout_items.push(RolloutItem::ResponseItem(item.clone()));
    }
    live_history.record_items(initial_context.iter());

    let user1 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "first user".to_string(),
        }],
    };
    live_history.record_items(std::iter::once(&user1));
    rollout_items.push(RolloutItem::ResponseItem(user1.clone()));

    let assistant1 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply one".to_string(),
        }],
    };
    live_history.record_items(std::iter::once(&assistant1));
    rollout_items.push(RolloutItem::ResponseItem(assistant1.clone()));

    let summary1 = "summary one";
    let snapshot1 = live_history.contents();
    let user_messages1 = collect_user_messages(&snapshot1);
    let rebuilt1 = build_compacted_history(
        session.build_initial_context(turn_context),
        &user_messages1,
        summary1,
    );
    live_history.replace(rebuilt1);
    rollout_items.push(RolloutItem::Compacted(CompactedItem {
        message: summary1.to_string(),
    }));

    let user2 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "second user".to_string(),
        }],
    };
    live_history.record_items(std::iter::once(&user2));
    rollout_items.push(RolloutItem::ResponseItem(user2.clone()));

    let assistant2 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply two".to_string(),
        }],
    };
    live_history.record_items(std::iter::once(&assistant2));
    rollout_items.push(RolloutItem::ResponseItem(assistant2.clone()));

    let summary2 = "summary two";
    let snapshot2 = live_history.contents();
    let user_messages2 = collect_user_messages(&snapshot2);
    let rebuilt2 = build_compacted_history(
        session.build_initial_context(turn_context),
        &user_messages2,
        summary2,
    );
    live_history.replace(rebuilt2);
    rollout_items.push(RolloutItem::Compacted(CompactedItem {
        message: summary2.to_string(),
    }));

    let user3 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "third user".to_string(),
        }],
    };
    live_history.record_items(std::iter::once(&user3));
    rollout_items.push(RolloutItem::ResponseItem(user3.clone()));

    let assistant3 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply three".to_string(),
        }],
    };
    live_history.record_items(std::iter::once(&assistant3));
    rollout_items.push(RolloutItem::ResponseItem(assistant3.clone()));

    (rollout_items, live_history.contents())
}

#[tokio::test]
async fn rejects_escalated_permissions_when_policy_not_on_request() {
    use crate::exec::ExecParams;
    use crate::protocol::AskForApproval;
    use crate::protocol::SandboxPolicy;
    use crate::turn_diff_tracker::TurnDiffTracker;
    use std::collections::HashMap;

    let (session, mut turn_context) = make_session_and_context();
    // Ensure policy is NOT OnRequest so the early rejection path triggers
    turn_context.approval_policy = AskForApproval::OnFailure;

    let params = ExecParams {
        command: if cfg!(windows) {
            vec![
                "cmd.exe".to_string(),
                "/C".to_string(),
                "echo hi".to_string(),
            ]
        } else {
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo hi".to_string(),
            ]
        },
        cwd: turn_context.cwd.clone(),
        timeout_ms: Some(1000),
        env: HashMap::new(),
        with_escalated_permissions: Some(true),
        justification: Some("test".to_string()),
    };

    let params2 = ExecParams {
        with_escalated_permissions: Some(false),
        ..params.clone()
    };

    let mut turn_diff_tracker = TurnDiffTracker::new();

    let sub_id = "test-sub".to_string();
    let call_id = "test-call".to_string();

    let resp = handle_container_exec_with_params(
        params,
        &session,
        &turn_context,
        &mut turn_diff_tracker,
        sub_id,
        call_id,
    )
    .await;

    let ResponseInputItem::FunctionCallOutput { output, .. } = resp else {
        panic!("expected FunctionCallOutput");
    };

    let expected = format!(
        "approval policy is {policy:?}; reject command — you should not ask for escalated permissions if the approval policy is {policy:?}",
        policy = turn_context.approval_policy
    );

    pretty_assertions::assert_eq!(output.content, expected);

    // Now retry the same command WITHOUT escalated permissions; should succeed.
    // Force DangerFullAccess to avoid platform sandbox dependencies in tests.
    turn_context.sandbox_policy = SandboxPolicy::DangerFullAccess;

    let resp2 = handle_container_exec_with_params(
        params2,
        &session,
        &turn_context,
        &mut turn_diff_tracker,
        "test-sub".to_string(),
        "test-call-2".to_string(),
    )
    .await;

    let ResponseInputItem::FunctionCallOutput { output, .. } = resp2 else {
        panic!("expected FunctionCallOutput on retry");
    };

    #[derive(Deserialize, PartialEq, Eq, Debug)]
    struct ResponseExecMetadata {
        exit_code: i32,
    }

    #[derive(Deserialize)]
    struct ResponseExecOutput {
        output: String,
        metadata: ResponseExecMetadata,
    }

    let exec_output: ResponseExecOutput =
        serde_json::from_str(&output.content).expect("valid exec output json");

    pretty_assertions::assert_eq!(exec_output.metadata, ResponseExecMetadata { exit_code: 0 });
    assert!(exec_output.output.contains("hi"));
    pretty_assertions::assert_eq!(output.success, Some(true));
}
