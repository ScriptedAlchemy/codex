use std::sync::Arc;

use super::Session;
use super::TurnContext;
use super::get_last_assistant_message_from_turn;
use crate::Prompt;
use crate::client_common::ResponseEvent;
use crate::error::CodexErr;
use crate::error::Result as CodexResult;
use crate::protocol::AgentMessageEvent;
use crate::protocol::CompactedItem;
use crate::protocol::ErrorEvent;
use crate::protocol::Event;
use crate::protocol::EventMsg;
use crate::protocol::InputItem;
use crate::protocol::InputMessageKind;
use crate::protocol::TaskStartedEvent;
use crate::protocol::TurnContextItem;
use crate::truncate::truncate_middle;
use crate::util::backoff;
use askama::Template;
use codex_protocol::models::ContentItem;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use futures::prelude::*;

pub const SUMMARIZATION_PROMPT: &str = include_str!("../../templates/compact/prompt.md");
const COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000;
const STAGED_COMPACT_RECENT_FRACTION: f32 = 0.30;
const STAGED_COMPACT_SEGMENT_ITEMS: usize = 12;
const STAGED_COMPACT_SEGMENT_MAX_CHARS: usize = 8_000;

#[derive(Template)]
#[template(path = "compact/history_bridge.md", escape = "none")]
struct HistoryBridgeTemplate<'a> {
    user_messages_text: &'a str,
    summary_text: &'a str,
}

pub(crate) async fn run_inline_auto_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
) {
    let sub_id = sess.next_internal_sub_id();
    let input = vec![InputItem::Text {
        text: SUMMARIZATION_PROMPT.to_string(),
    }];
    run_compact_task_inner(sess, turn_context, sub_id, input).await;
}

pub(crate) async fn run_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    sub_id: String,
    input: Vec<InputItem>,
) -> Option<String> {
    let start_event = Event {
        id: sub_id.clone(),
        msg: EventMsg::TaskStarted(TaskStartedEvent {
            model_context_window: turn_context.client.get_model_context_window(),
        }),
    };
    sess.send_event(start_event).await;
    run_compact_task_inner(sess.clone(), turn_context, sub_id.clone(), input).await;
    None
}

pub(crate) async fn run_staged_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    sub_id: String,
    input: Vec<InputItem>,
) -> Option<String> {
    let _ = input;
    let start_event = Event {
        id: sub_id.clone(),
        msg: EventMsg::TaskStarted(TaskStartedEvent {
            model_context_window: turn_context.client.get_model_context_window(),
        }),
    };
    sess.send_event(start_event).await;

    if let Err(err) =
        run_staged_compact_task_inner(sess.clone(), turn_context.clone(), &sub_id).await
    {
        let event = Event {
            id: sub_id.clone(),
            msg: EventMsg::Error(ErrorEvent {
                message: err.to_string(),
            }),
        };
        sess.send_event(event).await;
    }

    None
}

async fn run_staged_compact_task_inner(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    sub_id: &str,
) -> CodexResult<()> {
    let history_snapshot = sess.history_snapshot().await;
    if history_snapshot.is_empty() {
        let event = Event {
            id: sub_id.to_string(),
            msg: EventMsg::AgentMessage(AgentMessageEvent {
                message: "Staged compact skipped because there is no conversation history."
                    .to_string(),
            }),
        };
        sess.send_event(event).await;
        return Ok(());
    }

    let rollout_item = RolloutItem::TurnContext(TurnContextItem {
        cwd: turn_context.cwd.clone(),
        approval_policy: turn_context.approval_policy,
        sandbox_policy: turn_context.sandbox_policy.clone(),
        model: turn_context.client.get_model(),
        effort: turn_context.client.get_reasoning_effort(),
        summary: turn_context.client.get_reasoning_summary(),
    });
    sess.persist_rollout_items(&[rollout_item]).await;

    let initial_context = sess.build_initial_context(turn_context.as_ref());
    let initial_len = initial_context.len().min(history_snapshot.len());
    let mut working_items = history_snapshot[initial_len..].to_vec();
    if working_items.is_empty() {
        let event = Event {
            id: sub_id.to_string(),
            msg: EventMsg::AgentMessage(AgentMessageEvent {
                message: "Staged compact skipped because only initial context is present."
                    .to_string(),
            }),
        };
        sess.send_event(event).await;
        return Ok(());
    }

    let suffix_len = staged_compact_suffix_len(working_items.len());
    let prefix_len = working_items.len().saturating_sub(suffix_len);
    if prefix_len == 0 {
        let event = Event {
            id: sub_id.to_string(),
            msg: EventMsg::AgentMessage(AgentMessageEvent {
                message: "Staged compact skipped because the transcript is already within the recent window.".to_string(),
            }),
        };
        sess.send_event(event).await;
        return Ok(());
    }

    let suffix = working_items.split_off(prefix_len);
    let prefix = working_items;

    let segments: Vec<&[ResponseItem]> = if prefix.len() <= STAGED_COMPACT_SEGMENT_ITEMS {
        vec![prefix.as_slice()]
    } else {
        prefix
            .chunks(STAGED_COMPACT_SEGMENT_ITEMS)
            .collect::<Vec<_>>()
    };

    let total_segments = segments.len();
    let mut segment_summaries = Vec::with_capacity(total_segments);
    for (index, segment) in segments.iter().enumerate() {
        let display_index = index + 1;
        let notice =
            format!("Summarizing segment {display_index}/{total_segments} for staged compact…");
        sess.notify_background_event(sub_id, notice).await;

        let segment_text = response_items_to_text(segment);
        let prompt_text = build_segment_prompt(display_index, total_segments, &segment_text);
        let segment_sub_id = format!("{sub_id}-segment-{display_index}");
        let summary =
            summarize_prompt(&sess, turn_context.as_ref(), &segment_sub_id, &prompt_text).await?;
        segment_summaries.push(summary);
    }

    let consolidated_summary = if segment_summaries.len() == 1 {
        segment_summaries[0].clone()
    } else {
        let prompt_text = build_consolidated_prompt(&segment_summaries);
        summarize_prompt(&sess, turn_context.as_ref(), sub_id, &prompt_text).await?
    };

    let summary_payload = assemble_staged_summary(&consolidated_summary, &segment_summaries);
    let user_messages = collect_user_messages(&prefix);
    let mut new_history =
        build_compacted_history(initial_context, &user_messages, &summary_payload);
    new_history.extend_from_slice(&suffix);
    sess.replace_history(new_history).await;

    sess.persist_rollout_items(&[RolloutItem::Compacted(CompactedItem {
        message: summary_payload.clone(),
    })])
    .await;
    if !suffix.is_empty() {
        sess.persist_rollout_response_items(&suffix).await;
    }

    let kept = suffix.len();
    let message = format!("Staged compact completed — kept {kept} recent item(s) verbatim.");
    let event = Event {
        id: sub_id.to_string(),
        msg: EventMsg::AgentMessage(AgentMessageEvent { message }),
    };
    sess.send_event(event).await;

    Ok(())
}

async fn run_compact_task_inner(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    sub_id: String,
    input: Vec<InputItem>,
) {
    let initial_input_for_turn: ResponseInputItem = ResponseInputItem::from(input);
    let mut turn_input = sess
        .turn_input_with_history(vec![initial_input_for_turn.clone().into()])
        .await;
    let mut truncated_count = 0usize;

    let max_retries = turn_context.client.get_provider().stream_max_retries();
    let mut retries = 0;

    let rollout_item = RolloutItem::TurnContext(TurnContextItem {
        cwd: turn_context.cwd.clone(),
        approval_policy: turn_context.approval_policy,
        sandbox_policy: turn_context.sandbox_policy.clone(),
        model: turn_context.client.get_model(),
        effort: turn_context.client.get_reasoning_effort(),
        summary: turn_context.client.get_reasoning_summary(),
    });
    sess.persist_rollout_items(&[rollout_item]).await;

    loop {
        let prompt = Prompt {
            input: turn_input.clone(),
            ..Default::default()
        };
        let attempt_result =
            drain_to_completed(&sess, turn_context.as_ref(), &sub_id, &prompt).await;

        match attempt_result {
            Ok(()) => {
                if truncated_count > 0 {
                    sess.notify_background_event(
                        &sub_id,
                        format!(
                            "Trimmed {truncated_count} older conversation item(s) before compacting so the prompt fits the model context window."
                        ),
                    )
                    .await;
                }
                break;
            }
            Err(CodexErr::Interrupted) => {
                return;
            }
            Err(e @ CodexErr::ContextWindowExceeded) => {
                if turn_input.len() > 1 {
                    turn_input.remove(0);
                    truncated_count += 1;
                    retries = 0;
                    continue;
                }
                sess.set_total_tokens_full(&sub_id, turn_context.as_ref())
                    .await;
                let event = Event {
                    id: sub_id.clone(),
                    msg: EventMsg::Error(ErrorEvent {
                        message: e.to_string(),
                    }),
                };
                sess.send_event(event).await;
                return;
            }
            Err(e) => {
                if retries < max_retries {
                    retries += 1;
                    let delay = backoff(retries);
                    sess.notify_stream_error(
                        &sub_id,
                        format!(
                            "stream error: {e}; retrying {retries}/{max_retries} in {delay:?}…"
                        ),
                    )
                    .await;
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    let event = Event {
                        id: sub_id.clone(),
                        msg: EventMsg::Error(ErrorEvent {
                            message: e.to_string(),
                        }),
                    };
                    sess.send_event(event).await;
                    return;
                }
            }
        }
    }

    let history_snapshot = sess.history_snapshot().await;
    let summary_text = get_last_assistant_message_from_turn(&history_snapshot).unwrap_or_default();
    let user_messages = collect_user_messages(&history_snapshot);
    let initial_context = sess.build_initial_context(turn_context.as_ref());
    let new_history = build_compacted_history(initial_context, &user_messages, &summary_text);
    sess.replace_history(new_history).await;

    let rollout_item = RolloutItem::Compacted(CompactedItem {
        message: summary_text.clone(),
    });
    sess.persist_rollout_items(&[rollout_item]).await;

    let event = Event {
        id: sub_id.clone(),
        msg: EventMsg::AgentMessage(AgentMessageEvent {
            message: "Compact task completed".to_string(),
        }),
    };
    sess.send_event(event).await;
}

fn staged_compact_suffix_len(len: usize) -> usize {
    if len == 0 {
        0
    } else {
        let desired = (len as f32 * STAGED_COMPACT_RECENT_FRACTION).ceil() as usize;
        desired.min(len)
    }
}

fn limit_for_prompt(text: &str) -> String {
    if text.len() > STAGED_COMPACT_SEGMENT_MAX_CHARS {
        truncate_middle(text, STAGED_COMPACT_SEGMENT_MAX_CHARS).0
    } else {
        text.to_string()
    }
}

fn build_segment_prompt(index: usize, total: usize, segment_text: &str) -> String {
    let content = if segment_text.trim().is_empty() {
        "(no textual content in this segment)".to_string()
    } else {
        limit_for_prompt(segment_text)
    };
    format!(
        "You are compacting a conversation transcript. Produce a crisp summary for segment {index}/{total} highlighting key actions, decisions, open questions, and TODOs. Prefer bullet points when appropriate.\n\nSegment transcript:\n{content}"
    )
}

fn build_consolidated_prompt(segment_summaries: &[String]) -> String {
    let mut body = String::new();
    for (index, summary) in segment_summaries.iter().enumerate() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        let trimmed = summary.trim();
        let entry = if trimmed.is_empty() {
            "(segment produced an empty summary)"
        } else {
            trimmed
        };
        body.push_str(&format!("Segment {}:\n{}", index + 1, entry));
    }
    let content = limit_for_prompt(&body);
    format!(
        "Combine the following segment summaries into a cohesive narrative that preserves chronology, critical decisions, outstanding work, and risks. If information is already concise, keep it; otherwise merge overlapping points.\n\nSegment summaries:\n{content}"
    )
}

fn assemble_staged_summary(consolidated: &str, segments: &[String]) -> String {
    let mut sections = Vec::new();
    let consolidated = consolidated.trim();
    if !consolidated.is_empty() {
        sections.push(format!("High-level summary:\n{consolidated}"));
    }
    if !segments.is_empty() {
        let mut breakdown = String::from("Segment breakdown:");
        for (index, summary) in segments.iter().enumerate() {
            let trimmed = summary.trim();
            let entry = if trimmed.is_empty() {
                "(empty)"
            } else {
                trimmed
            };
            breakdown.push_str(&format!("\n{}. {entry}", index + 1));
        }
        sections.push(breakdown);
    }
    sections.join("\n\n")
}

fn response_items_to_text(items: &[ResponseItem]) -> String {
    use codex_protocol::models::LocalShellStatus;

    let mut lines = Vec::new();
    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => {
                if let Some(text) = content_items_to_text(content) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        lines.push(format!("{role}: {trimmed}"));
                    }
                }
            }
            ResponseItem::Reasoning { summary, .. } => {
                let mut pieces = Vec::new();
                for entry in summary {
                    match entry {
                        ReasoningItemReasoningSummary::SummaryText { text } => {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                pieces.push(trimmed.to_string());
                            }
                        }
                    }
                }
                if !pieces.is_empty() {
                    lines.push(format!("assistant.reasoning: {}", pieces.join(" | ")));
                }
            }
            ResponseItem::FunctionCall {
                name, arguments, ..
            } => {
                let truncated = limit_for_prompt(arguments);
                lines.push(format!("assistant.function_call[{name}]: {truncated}"));
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                let truncated = limit_for_prompt(&output.content);
                lines.push(format!("tool_output[{call_id}]: {truncated}"));
            }
            ResponseItem::CustomToolCall { name, input, .. } => {
                let truncated = limit_for_prompt(input);
                lines.push(format!("assistant.custom_tool[{name}]: {truncated}"));
            }
            ResponseItem::CustomToolCallOutput { call_id, output } => {
                let truncated = limit_for_prompt(output);
                lines.push(format!("custom_tool_output[{call_id}]: {truncated}"));
            }
            ResponseItem::LocalShellCall { status, action, .. } => {
                match action {
                    LocalShellAction::Exec(exec) => {
                        let command = exec.command.join(" ");
                        lines.push(format!("exec[{status:?}]: {}", limit_for_prompt(&command)));
                    }
                }
                if *status == LocalShellStatus::Incomplete {
                    lines.push("exec result: incomplete".to_string());
                }
            }
            ResponseItem::WebSearchCall { action, .. } => match action {
                codex_protocol::models::WebSearchAction::Search { query } => {
                    lines.push(format!("web_search: {query}"));
                }
                codex_protocol::models::WebSearchAction::Other => {
                    lines.push("web_search: other".to_string());
                }
            },
            ResponseItem::Other => {}
        }
    }

    let joined = lines.join("\n");
    limit_for_prompt(&joined)
}

async fn summarize_prompt(
    sess: &Session,
    turn_context: &TurnContext,
    sub_id: &str,
    prompt_text: &str,
) -> CodexResult<String> {
    let prompt_message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: limit_for_prompt(prompt_text),
        }],
    };
    let prompt = Prompt {
        input: vec![prompt_message],
        ..Default::default()
    };

    let max_retries = turn_context.client.get_provider().stream_max_retries();
    let mut retries = 0;

    loop {
        let mut stream = turn_context.client.clone().stream(&prompt).await?;
        let mut responses = Vec::new();

        loop {
            let maybe_event = stream.next().await;
            let Some(event) = maybe_event else {
                return Err(CodexErr::Stream(
                    "stream closed before response.completed".to_string(),
                    None,
                ));
            };
            match event {
                Ok(ResponseEvent::OutputItemDone(item)) => {
                    responses.push(item);
                }
                Ok(ResponseEvent::RateLimits(snapshot)) => {
                    sess.update_rate_limits(sub_id, snapshot).await;
                }
                Ok(ResponseEvent::Completed { token_usage, .. }) => {
                    sess.update_token_usage_info(sub_id, turn_context, token_usage.as_ref())
                        .await;
                    let summary =
                        get_last_assistant_message_from_turn(&responses).unwrap_or_default();
                    return Ok(summary);
                }
                Ok(_) => continue,
                Err(err) => {
                    if retries < max_retries {
                        retries += 1;
                        let delay = backoff(retries);
                        sess.notify_stream_error(
                            sub_id,
                            format!(
                                "stream error: {err}; retrying {retries}/{max_retries} in {delay:?}…"
                            ),
                        )
                        .await;
                        tokio::time::sleep(delay).await;
                        break;
                    } else {
                        return Err(err);
                    }
                }
            }
        }
    }
}

pub fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let mut pieces = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

pub(crate) fn collect_user_messages(items: &[ResponseItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                content_items_to_text(content)
            }
            _ => None,
        })
        .filter(|text| !is_session_prefix_message(text))
        .collect()
}

pub fn is_session_prefix_message(text: &str) -> bool {
    matches!(
        InputMessageKind::from(("user", text)),
        InputMessageKind::UserInstructions | InputMessageKind::EnvironmentContext
    )
}

pub(crate) fn build_compacted_history(
    initial_context: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
) -> Vec<ResponseItem> {
    let mut history = initial_context;
    let mut user_messages_text = if user_messages.is_empty() {
        "(none)".to_string()
    } else {
        user_messages.join("\n\n")
    };
    // Truncate the concatenated prior user messages so the bridge message
    // stays well under the context window (approx. 4 bytes/token).
    let max_bytes = COMPACT_USER_MESSAGE_MAX_TOKENS * 4;
    if user_messages_text.len() > max_bytes {
        user_messages_text = truncate_middle(&user_messages_text, max_bytes).0;
    }
    let summary_text = if summary_text.is_empty() {
        "(no summary available)".to_string()
    } else {
        summary_text.to_string()
    };
    let Ok(bridge) = HistoryBridgeTemplate {
        user_messages_text: &user_messages_text,
        summary_text: &summary_text,
    }
    .render() else {
        return vec![];
    };
    history.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text: bridge }],
    });
    history
}

async fn drain_to_completed(
    sess: &Session,
    turn_context: &TurnContext,
    sub_id: &str,
    prompt: &Prompt,
) -> CodexResult<()> {
    let mut stream = turn_context.client.clone().stream(prompt).await?;
    loop {
        let maybe_event = stream.next().await;
        let Some(event) = maybe_event else {
            return Err(CodexErr::Stream(
                "stream closed before response.completed".into(),
                None,
            ));
        };
        match event {
            Ok(ResponseEvent::OutputItemDone(item)) => {
                sess.record_into_history(std::slice::from_ref(&item)).await;
            }
            Ok(ResponseEvent::RateLimits(snapshot)) => {
                sess.update_rate_limits(sub_id, snapshot).await;
            }
            Ok(ResponseEvent::Completed { token_usage, .. }) => {
                sess.update_token_usage_info(sub_id, turn_context, token_usage.as_ref())
                    .await;
                return Ok(());
            }
            Ok(_) => continue,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn content_items_to_text_joins_non_empty_segments() {
        let items = vec![
            ContentItem::InputText {
                text: "hello".to_string(),
            },
            ContentItem::OutputText {
                text: String::new(),
            },
            ContentItem::OutputText {
                text: "world".to_string(),
            },
        ];

        let joined = content_items_to_text(&items);

        assert_eq!(Some("hello\nworld".to_string()), joined);
    }

    #[test]
    fn content_items_to_text_ignores_image_only_content() {
        let items = vec![ContentItem::InputImage {
            image_url: "file://image.png".to_string(),
        }];

        let joined = content_items_to_text(&items);

        assert_eq!(None, joined);
    }

    #[test]
    fn collect_user_messages_extracts_user_text_only() {
        let items = vec![
            ResponseItem::Message {
                id: Some("assistant".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "ignored".to_string(),
                }],
            },
            ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![
                    ContentItem::InputText {
                        text: "first".to_string(),
                    },
                    ContentItem::OutputText {
                        text: "second".to_string(),
                    },
                ],
            },
            ResponseItem::Other,
        ];

        let collected = collect_user_messages(&items);

        assert_eq!(vec!["first\nsecond".to_string()], collected);
    }

    #[test]
    fn collect_user_messages_filters_session_prefix_entries() {
        let items = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<user_instructions>do things</user_instructions>".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<ENVIRONMENT_CONTEXT>cwd=/tmp</ENVIRONMENT_CONTEXT>".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "real user message".to_string(),
                }],
            },
        ];

        let collected = collect_user_messages(&items);

        assert_eq!(vec!["real user message".to_string()], collected);
    }

    #[test]
    fn build_compacted_history_truncates_overlong_user_messages() {
        // Prepare a very large prior user message so the aggregated
        // `user_messages_text` exceeds the truncation threshold used by
        // `build_compacted_history` (80k bytes).
        let big = "X".repeat(200_000);
        let history = build_compacted_history(Vec::new(), std::slice::from_ref(&big), "SUMMARY");

        // Expect exactly one bridge message added to history (plus any initial context we provided, which is none).
        assert_eq!(history.len(), 1);

        // Extract the text content of the bridge message.
        let bridge_text = match &history[0] {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                content_items_to_text(content).unwrap_or_default()
            }
            other => panic!("unexpected item in history: {other:?}"),
        };

        // The bridge should contain the truncation marker and not the full original payload.
        assert!(
            bridge_text.contains("tokens truncated"),
            "expected truncation marker in bridge message"
        );
        assert!(
            !bridge_text.contains(&big),
            "bridge should not include the full oversized user text"
        );
        assert!(
            bridge_text.contains("SUMMARY"),
            "bridge should include the provided summary text"
        );
    }

    #[test]
    fn staged_compact_suffix_len_respects_fraction() {
        assert_eq!(staged_compact_suffix_len(0), 0);
        assert_eq!(staged_compact_suffix_len(1), 1);
        assert_eq!(staged_compact_suffix_len(3), 1);
        assert_eq!(staged_compact_suffix_len(10), 3);
    }

    #[test]
    fn assemble_staged_summary_formats_sections() {
        let consolidated = "Overall summary";
        let segments = vec!["first chunk".to_string(), "second chunk".to_string()];
        let formatted = assemble_staged_summary(consolidated, &segments);

        assert!(formatted.contains("High-level summary"));
        assert!(formatted.contains("Segment breakdown"));
        assert!(formatted.contains("1. first chunk"));
        assert!(formatted.contains("2. second chunk"));
    }
}
