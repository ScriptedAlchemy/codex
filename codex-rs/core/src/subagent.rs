use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
// (No direct atomic imports; use fully-qualified types inline.)
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use crate::AuthManager;
use crate::ConversationManager;
use crate::codex::TurnContext;
use crate::codex_conversation::CodexConversation;
use crate::conversation_manager::NewConversation;
use crate::protocol::EventMsg;
use codex_protocol::mcp_protocol::ConversationId;
use codex_protocol::protocol::TokenUsage;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tokio::sync::TryAcquireError;

// Subagent role guidance appended to child subagent sessions via user_instructions.
pub const SUBAGENT_USER_GUIDE: &str = include_str!("../subagent_prompt.md");

pub(crate) const DEFAULT_MAX_SUBAGENT_DEPTH: u8 = 1;
pub(crate) const DEFAULT_MAX_SUBAGENT_CONCURRENT: usize = 2;
pub(crate) const SUBAGENT_MAIL_SUBJECT_MAX_LEN: usize = 80;

#[derive(Default)]
pub(crate) struct Mailbox {
    next_id: u64,
    order: VecDeque<String>,
    items: HashMap<String, MailItem>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct MailItem {
    id: String,
    subagent_id: String,
    subject: String,
    body: String,
    token_usage: Option<TokenUsage>,
    timestamp: SystemTime,
    unread: bool,
    turn_index: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct SubagentSettings {
    pub max_depth: u8,
    // Currently enforced via `subagent_slots`; keep config value for future tweaks.
    pub(crate) _max_concurrent: usize,
}

pub(crate) struct SubagentState {
    pub conversation: Arc<CodexConversation>,
    pub conversation_id: ConversationId,
    pub rollout_path: PathBuf,
    pub description: String,
    // Keep for future UI/debugging; not read directly.
    _created_at: Instant,
    pub last_active: Instant,
    pub turns_completed: usize,
    pub running: bool,
    pub max_turns: Option<usize>,
    pub max_runtime: Option<Duration>,
    // Hold semaphore permit to enforce concurrency until drop.
    _permit: OwnedSemaphorePermit,
}

#[derive(Deserialize)]
pub(crate) struct SubagentOpenArgs {
    pub goal: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub approval_policy: Option<crate::protocol::AskForApproval>,
    #[serde(default)]
    pub sandbox_mode: Option<crate::protocol_config_types::SandboxMode>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub max_turns: Option<usize>,
    #[serde(default)]
    pub max_runtime_ms: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct SubagentOpenResult {
    pub subagent_id: String,
    pub conversation_id: ConversationId,
    pub rollout_path: PathBuf,
    pub description: String,
}

#[derive(Deserialize)]
pub(crate) struct SubagentReplyArgs {
    pub subagent_id: String,
    pub message: String,
    #[serde(default)]
    pub images: Option<Vec<String>>,
    #[serde(default)]
    pub mode: Option<String>, // "blocking" | "nonblocking" (default blocking)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct SubagentMailboxArgs {
    #[serde(default)]
    pub subagent_id: Option<String>,
    #[serde(default)]
    pub only_unread: Option<bool>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct SubagentReadArgs {
    pub mail_id: String,
    #[serde(default)]
    pub peek: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct SubagentEndArgs {
    pub subagent_id: String,
    #[serde(default)]
    pub persist: Option<bool>,
    #[serde(default)]
    pub archive_to: Option<String>,
}

pub(crate) fn summarize_goal(goal: &str) -> String {
    let trimmed = goal.trim();
    if trimmed.is_empty() {
        return "subagent task".to_string();
    }

    let mut summary: String = trimmed
        .chars()
        .take(SUBAGENT_MAIL_SUBJECT_MAX_LEN)
        .collect();
    if trimmed.chars().count() > SUBAGENT_MAIL_SUBJECT_MAX_LEN {
        summary.push('…');
    }
    summary
}

pub(crate) async fn open_subagent(
    auth_manager: Arc<AuthManager>,
    next_internal_sub_id: &std::sync::atomic::AtomicU64,
    subagent_slots: Arc<Semaphore>,
    subagent_settings: SubagentSettings,
    subagents: &Mutex<HashMap<String, SubagentState>>,
    turn_context: &TurnContext,
    args: SubagentOpenArgs,
) -> Result<SubagentOpenResult, String> {
    // Interpret `max_depth` as the maximum nesting depth, not the number of
    // sibling subagents. Because child subagents do not expose the subagent
    // tools (see `include_subagent_tool = false` below), calls here originate
    // from the root session, which is depth 0. Therefore, if `max_depth` is 0
    // we disallow opening any subagents; otherwise we allow any number of
    // siblings subject to the concurrency semaphore.
    if subagent_settings.max_depth == 0 {
        return Err("subagent depth limit reached".to_string());
    }

    let permit = match subagent_slots.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(TryAcquireError::Closed) => {
            return Err("subagent scheduler unavailable".to_string());
        }
        Err(TryAcquireError::NoPermits) => {
            return Err("maximum concurrent subagents reached".to_string());
        }
    };

    let SubagentOpenArgs {
        goal,
        system_prompt,
        model,
        approval_policy,
        sandbox_mode,
        cwd,
        max_turns,
        max_runtime_ms,
    } = args;

    if let Some(0) = max_turns {
        return Err("max_turns must be greater than zero".to_string());
    }
    if let Some(0) = max_runtime_ms {
        return Err("max_runtime_ms must be greater than zero".to_string());
    }

    let subagent_id = format!(
        "subagent-{}",
        next_internal_sub_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let description = summarize_goal(&goal);

    // Clone the underlying `Config` value (not the `Arc`).
    let mut child_config = (**turn_context.client.config()).clone();
    // Nested subagents are disabled for now. If enabling them in the future,
    // ensure the parent's current depth would be propagated so the child starts
    // at parent_depth + 1 and respects the global max depth.
    child_config.include_subagent_tool = false;
    // Ensure subagents always have access to the plan tool so they can
    // externalize a brief plan before acting.
    child_config.include_plan_tool = true;
    child_config.base_instructions =
        system_prompt.or_else(|| turn_context.base_instructions.clone());
    child_config.approval_policy = approval_policy.unwrap_or(turn_context.approval_policy);
    // Honor explicit sandbox_mode requests for the subagent. In particular,
    // map WorkspaceWrite to a concrete workspace‑write policy rather than
    // inheriting the parent's policy. This ensures callers can request
    // write access for the child even when the parent is currently running
    // in read‑only mode.
    child_config.sandbox_policy = match sandbox_mode {
        Some(crate::protocol_config_types::SandboxMode::DangerFullAccess) => {
            crate::protocol::SandboxPolicy::DangerFullAccess
        }
        Some(crate::protocol_config_types::SandboxMode::ReadOnly) => {
            crate::protocol::SandboxPolicy::new_read_only_policy()
        }
        Some(crate::protocol_config_types::SandboxMode::WorkspaceWrite) => {
            crate::protocol::SandboxPolicy::new_workspace_write_policy()
        }
        None => turn_context.sandbox_policy.clone(),
    };

    // Default subagents to the Codex (swiftfox) model line unless explicitly overridden.
    // This keeps child conversations optimized for coding tasks even when the
    // parent session is using a different model.
    if let Some(model) = model {
        child_config.model = model.clone();
        if let Some(model_family) = crate::model_family::find_family_for_model(&model) {
            child_config.model_family = model_family.clone();
            if let Some(info) = crate::openai_model_info::get_model_info(&model_family) {
                child_config.model_context_window = Some(info.context_window);
                child_config.model_max_output_tokens = Some(info.max_output_tokens);
                child_config.model_auto_compact_token_limit = info.auto_compact_token_limit;
            }
        }
    } else {
        use crate::config::GPT_5_CODEX_MEDIUM_MODEL;
        child_config.model = GPT_5_CODEX_MEDIUM_MODEL.to_string();
        if let Some(model_family) = crate::model_family::find_family_for_model(&child_config.model)
        {
            child_config.model_family = model_family.clone();
            if let Some(info) = crate::openai_model_info::get_model_info(&model_family) {
                child_config.model_context_window = Some(info.context_window);
                child_config.model_max_output_tokens = Some(info.max_output_tokens);
                child_config.model_auto_compact_token_limit = info.auto_compact_token_limit;
            }
        }
    }

    // Ensure child subagents explicitly receive their role guidance.
    // Deliver via user_instructions (not base instructions) to keep
    // model `instructions` stable as tested elsewhere.
    child_config.user_instructions = Some(match child_config.user_instructions.take() {
        Some(existing) => {
            format!("{existing}\n\n--- subagent-guide ---\n\n{SUBAGENT_USER_GUIDE}")
        }
        None => SUBAGENT_USER_GUIDE.to_string(),
    });

    let resolved_cwd = match cwd {
        Some(path) => {
            let candidate = PathBuf::from(path);
            if candidate.is_absolute() {
                candidate
            } else {
                turn_context.cwd.join(candidate)
            }
        }
        None => turn_context.cwd.clone(),
    };
    child_config.cwd = resolved_cwd;

    // Note: In v1 we interpret `max_runtime_ms` as a maximum IDLE window
    // (time since last activity) rather than absolute wall‑clock runtime.
    // The idle timer will be refreshed by the reply runner whenever the
    // child produces output (AgentMessageDelta / AgentReasoningDelta). The
    // enforcement lives in the reply/nonblocking task runner.
    let max_runtime = max_runtime_ms.map(Duration::from_millis);

    let manager = ConversationManager::new(Arc::clone(&auth_manager));
    let new_conversation = manager
        .new_conversation(child_config)
        .await
        .map_err(|e| format!("failed to start subagent: {e}"))?;

    let NewConversation {
        conversation_id,
        conversation,
        session_configured,
    } = new_conversation;

    let rollout_path = session_configured.rollout_path.clone();

    // Register the new subagent in the session map and keep the semaphore permit
    // to enforce concurrency until the subagent is ended.
    {
        let mut subs = subagents.lock().await;
        subs.insert(
            subagent_id.clone(),
            SubagentState {
                conversation: Arc::clone(&conversation),
                conversation_id,
                rollout_path: rollout_path.clone(),
                description: description.clone(),
                _created_at: Instant::now(),
                last_active: Instant::now(),
                turns_completed: 0,
                running: false,
                max_turns,
                max_runtime,
                _permit: permit,
            },
        );
    }

    // Depth is not incremented for sibling subagents; children cannot spawn
    // subagents because we disable the tool set for them below.

    Ok(SubagentOpenResult {
        subagent_id,
        conversation_id,
        rollout_path,
        description,
    })
}

pub(crate) async fn subagent_reply_blocking(
    subagents: &Mutex<HashMap<String, SubagentState>>,
    mailbox: &Mutex<Mailbox>,
    args: &SubagentReplyArgs,
) -> Result<serde_json::Value, String> {
    let subagent_id = &args.subagent_id;
    let message = &args.message;
    let images = args.images.clone().unwrap_or_default();

    // Fetch subagent state and basic guards.
    let mut guard = subagents.lock().await;
    let state = match guard.get_mut(subagent_id) {
        Some(s) => s,
        None => return Err(format!("unknown subagent_id: {subagent_id}")),
    };
    if state.running {
        return Err("subagent is already running".to_string());
    }
    if let Some(max_turns) = state.max_turns
        && state.turns_completed >= max_turns
    {
        return Err("subagent turn limit reached".to_string());
    }
    state.running = true; // mark busy
    let description = state.description.clone();
    let conversation = state.conversation.clone();
    let max_idle = state.max_runtime; // interpreted as idle window
    drop(guard);

    // Build child input items
    let mut items: Vec<crate::protocol::InputItem> = Vec::new();
    items.push(crate::protocol::InputItem::Text {
        text: message.clone(),
    });
    for img in images.into_iter() {
        items.push(crate::protocol::InputItem::LocalImage {
            path: PathBuf::from(img),
        });
    }

    // Submit user input to child
    let submit_result = conversation
        .submit(crate::protocol::Op::UserInput { items })
        .await;
    let _child_submit_id = match submit_result {
        Ok(id) => id,
        Err(e) => {
            let mut guard = subagents.lock().await;
            if let Some(st) = guard.get_mut(subagent_id) {
                st.running = false;
            }
            return Err(format!("failed to submit to subagent: {e}"));
        }
    };

    // Event loop: collect reply, token usage; enforce idle timeout
    let mut reply_text: String = String::new();
    let mut last_usage: Option<TokenUsage> = None;

    // Optional hard deadline for the overall wait window.
    let hard_deadline = args
        .timeout_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));

    // Compute dynamic idle budget per next_event call.
    loop {
        let next_event_fut = conversation.next_event();
        // Compute time caps: idle cap (max_idle) and hard deadline cap (timeout_ms)
        let idle_remaining = if let Some(max_idle) = max_idle {
            let (remaining, timed_out) = {
                let mut sub_map = subagents.lock().await;
                if let Some(st) = sub_map.get_mut(subagent_id) {
                    let since = st.last_active.elapsed();
                    if since >= max_idle {
                        (Duration::from_millis(0), true)
                    } else {
                        (max_idle - since, false)
                    }
                } else {
                    (Duration::from_millis(0), true)
                }
            };
            if timed_out {
                Some(Duration::from_millis(0))
            } else {
                Some(remaining)
            }
        } else {
            None
        };
        let hard_remaining = hard_deadline.map(|dl| dl.saturating_duration_since(Instant::now()));
        // Choose the most restrictive remaining duration, if any
        let (wait_dur_opt, hard_is_min) = match (idle_remaining, hard_remaining) {
            (Some(a), Some(b)) => (Some(std::cmp::min(a, b)), b <= a),
            (Some(a), None) => (Some(a), false),
            (None, Some(b)) => (Some(b), true),
            (None, None) => (None, false),
        };

        let evt_res = if let Some(wait_dur) = wait_dur_opt {
            if wait_dur.is_zero() {
                // synthetic timeout (cap already exhausted)
                Err(())
            } else {
                match tokio::time::timeout(wait_dur, next_event_fut).await {
                    Ok(res) => Ok(res),
                    Err(_) => Err(()),
                }
            }
        } else {
            // No caps; wait normally
            Ok(next_event_fut.await)
        };

        match evt_res {
            Ok(Ok(crate::protocol::Event { msg, .. })) => {
                // Any child activity refreshes last_active
                {
                    let mut sub_map = subagents.lock().await;
                    if let Some(st) = sub_map.get_mut(subagent_id) {
                        st.last_active = Instant::now();
                    }
                }

                match msg {
                    EventMsg::AgentMessageDelta(delta) => {
                        reply_text.push_str(&delta.delta);
                    }
                    EventMsg::AgentMessage(ev) => {
                        reply_text.push_str(&ev.message);
                    }
                    EventMsg::TokenCount(crate::protocol::TokenCountEvent {
                        info: Some(info),
                        ..
                    }) => {
                        last_usage = Some(info.last_token_usage);
                    }
                    EventMsg::TokenCount(crate::protocol::TokenCountEvent {
                        info: None, ..
                    }) => {}
                    EventMsg::TaskComplete(crate::protocol::TaskCompleteEvent {
                        last_agent_message,
                    }) => {
                        if let Some(full) = last_agent_message
                            && !full.is_empty()
                        {
                            reply_text = full;
                        }
                        // Turn finished normally
                        break;
                    }
                    // Ignore other events for the mailbox v1 path
                    _ => {}
                }
            }
            Ok(Err(e)) => {
                reply_text = format!("subagent error: {e}");
                break;
            }
            Err(()) => {
                // Timeout: best‑effort cause attribution based on which cap we enforced this loop.
                let hard_took_precedence = hard_is_min;
                let _ = conversation.submit(crate::protocol::Op::Interrupt).await;
                reply_text = if hard_took_precedence {
                    "Subagent reply timed out".to_string()
                } else {
                    "Subagent timed out due to inactivity".to_string()
                };
                break;
            }
        }
    }

    // Update state and enqueue mailbox entry
    let mut sub_map = subagents.lock().await;
    if let Some(st) = sub_map.get_mut(subagent_id) {
        st.turns_completed += 1;
        st.running = false;
    }
    drop(sub_map);

    let mail_id = enqueue_mail(
        subagents,
        mailbox,
        subagent_id,
        &description,
        &reply_text,
        last_usage.clone(),
    )
    .await;

    Ok(serde_json::json!({
        "reply": reply_text,
        "token_usage": last_usage,
        "done": true,
        "mail_id": mail_id,
    }))
}

pub(crate) async fn enqueue_mail(
    subagents: &Mutex<HashMap<String, SubagentState>>,
    mailbox: &Mutex<Mailbox>,
    subagent_id: &str,
    subject: &str,
    body: &str,
    token_usage: Option<TokenUsage>,
) -> String {
    let mut box_guard = mailbox.lock().await;
    let id_num = box_guard.next_id;
    box_guard.next_id += 1;
    let mail_id = format!("mail-{id_num}");
    let turn_idx = {
        let sub_guard = subagents.lock().await;
        sub_guard
            .get(subagent_id)
            .map(|s| s.turns_completed)
            .unwrap_or(0)
    };
    let item = MailItem {
        id: mail_id.clone(),
        subagent_id: subagent_id.to_string(),
        subject: subject.to_string(),
        body: body.to_string(),
        token_usage,
        timestamp: SystemTime::now(),
        unread: true,
        turn_index: turn_idx,
    };
    box_guard.order.push_front(mail_id.clone());
    box_guard.items.insert(mail_id.clone(), item);
    mail_id
}

pub(crate) async fn list_mailbox(
    mailbox: &Mutex<Mailbox>,
    filter: SubagentMailboxArgs,
) -> serde_json::Value {
    let SubagentMailboxArgs {
        subagent_id,
        only_unread,
        limit,
    } = filter;
    let only_unread = only_unread.unwrap_or(false);
    let limit = limit.unwrap_or(100);
    let guard = mailbox.lock().await;
    let mut items = Vec::new();
    for id in guard.order.iter() {
        if let Some(mi) = guard.items.get(id) {
            if only_unread && !mi.unread {
                continue;
            }
            if let Some(ref sid) = subagent_id
                && &mi.subagent_id != sid
            {
                continue;
            }
            let at: chrono::DateTime<chrono::Utc> = mi.timestamp.into();
            items.push(serde_json::json!({
                "mail_id": mi.id,
                "subagent_id": mi.subagent_id,
                "subject": mi.subject,
                "at": at.to_rfc3339(),
                "unread": mi.unread,
                "turns_completed": mi.turn_index,
            }));
            if items.len() >= limit {
                break;
            }
        }
    }
    serde_json::json!({"items": items})
}

pub(crate) async fn read_mail(
    mailbox: &Mutex<Mailbox>,
    args: SubagentReadArgs,
) -> Result<serde_json::Value, String> {
    let SubagentReadArgs { mail_id, peek } = args;
    let mut guard = mailbox.lock().await;
    let item = guard
        .items
        .get_mut(&mail_id)
        .cloned()
        .ok_or_else(|| format!("unknown mail_id: {mail_id}"))?;
    if !peek.unwrap_or(false)
        && let Some(mi) = guard.items.get_mut(&mail_id)
    {
        mi.unread = false;
    }
    let at: chrono::DateTime<chrono::Utc> = item.timestamp.into();
    Ok(serde_json::json!({
        "subagent_id": item.subagent_id,
        "subject": item.subject,
        "body": item.body,
        "token_usage": item.token_usage,
        "at": at.to_rfc3339(),
    }))
}

pub(crate) async fn end_subagent(
    subagents: &Mutex<HashMap<String, SubagentState>>,
    args: SubagentEndArgs,
) -> Result<serde_json::Value, String> {
    let SubagentEndArgs {
        subagent_id,
        persist,
        archive_to,
    } = args;

    // Take ownership of the state
    let (conversation, conversation_id, rollout_path) = {
        let mut map = subagents.lock().await;
        let Some(state) = map.remove(&subagent_id) else {
            return Err(format!("unknown subagent_id: {subagent_id}"));
        };
        (
            state.conversation,
            state.conversation_id,
            state.rollout_path,
        )
    };

    // Gracefully shutdown child conversation
    let _ = conversation.submit(crate::protocol::Op::Shutdown).await;
    // Drain one event (best effort) to allow graceful shutdown
    let _ = tokio::time::timeout(Duration::from_secs(2), conversation.next_event()).await;

    // Handle persistence/archival
    let mut archived_path: Option<String> = None;
    match (persist.unwrap_or(true), archive_to) {
        (false, _) => {
            let _ = std::fs::remove_file(&rollout_path);
        }
        (true, Some(dir)) => {
            let to_dir = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&to_dir);
            if let Some(name) = rollout_path.file_name() {
                let dest = to_dir.join(name);
                if std::fs::rename(&rollout_path, &dest).is_ok() {
                    archived_path = Some(dest.to_string_lossy().to_string());
                }
            }
        }
        _ => {}
    }

    Ok(serde_json::json!({
        "conversation_id": conversation_id,
        "archived_path": archived_path,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::test_helpers::dead_submit_conversation;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[tokio::test]
    async fn running_flag_clears_when_submit_fails() {
        let conversation = dead_submit_conversation();
        let semaphore = Arc::new(Semaphore::new(1));
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("acquire permit for subagent state");

        let subagent_id = "test-subagent".to_string();

        let state = SubagentState {
            conversation,
            conversation_id: ConversationId::new(),
            rollout_path: PathBuf::new(),
            description: "test".to_string(),
            _created_at: Instant::now(),
            last_active: Instant::now(),
            turns_completed: 0,
            running: false,
            max_turns: None,
            max_runtime: None,
            _permit: permit,
        };

        let subagents = Mutex::new(HashMap::from([(subagent_id.clone(), state)]));
        let mailbox = Mutex::new(Mailbox::default());

        let args = SubagentReplyArgs {
            subagent_id: subagent_id.clone(),
            message: "hello".to_string(),
            images: None,
            mode: None,
            timeout_ms: None,
        };

        let err = subagent_reply_blocking(&subagents, &mailbox, &args)
            .await
            .expect_err("failing submit must propagate error");
        assert!(
            err.contains("failed to submit to subagent"),
            "unexpected error message: {err}"
        );

        let guard = subagents.lock().await;
        let state = guard
            .get(&subagent_id)
            .expect("subagent should remain registered");
        assert_eq!(
            false, state.running,
            "running flag should reset even when submit fails"
        );
    }
}
