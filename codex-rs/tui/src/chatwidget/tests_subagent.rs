use super::*;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::BottomPane;
use crate::bottom_pane::BottomPaneParams;
use crate::tui::FrameRequester;
use codex_core::AuthManager;
use codex_core::CodexAuth;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::config::ConfigToml;
use codex_core::protocol::BackgroundEventEvent;
use codex_core::protocol::Event;
use codex_core::protocol::EventMsg;
use codex_core::protocol::Op;
use pretty_assertions::assert_eq;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc::unbounded_channel;

// Keep this test file focused on subagent-specific flows so the main tests.rs
// file doesn’t grow unmanageably large.

#[test]
fn subagent_direct_message_includes_images() {
    let (mut chat, mut rx, mut op_rx) = make_chatwidget_manual();

    // Announce a subagent and focus it via Ctrl+O.
    chat.handle_codex_event(Event {
        id: "s-img".into(),
        msg: EventMsg::BackgroundEvent(BackgroundEventEvent {
            message: "Subagent subagent-img opened: process images".to_string(),
        }),
    });
    drain_insert_history(&mut rx);
    chat.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('o'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    drain_insert_history(&mut rx); // focus banner

    // Prepare text then attach an image (so placeholder stays in the text).
    let img_path = PathBuf::from("/tmp/test-image-a.png");
    chat.set_composer_text("see this image".into());
    chat.attach_image(img_path.clone(), 32, 16, "PNG");
    chat.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    // Drain history inserts (banner + "To subagent …" line) before reading the op.
    drain_insert_history(&mut rx);

    // Find the SubagentDirectMessage op and assert images are propagated.
    let mut op = op_rx.try_recv().expect("expected an op");
    while matches!(op, Op::AddToHistory { .. }) {
        op = op_rx.try_recv().expect("expected SubagentDirectMessage op");
    }
    match op {
        Op::SubagentDirectMessage {
            subagent_id,
            message,
            images,
            ..
        } => {
            assert_eq!(subagent_id, "subagent-img");
            assert!(message.contains("see this image"));
            assert!(message.contains("image 32x16 PNG"));
            let imgs = images.expect("images should be present");
            assert_eq!(imgs, vec![img_path.display().to_string()]);
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

// --- Minimal helpers (duplicated to keep this file self-contained) ---
fn make_chatwidget_manual() -> (
    ChatWidget,
    tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    tokio::sync::mpsc::UnboundedReceiver<Op>,
) {
    let (tx_raw, rx) = unbounded_channel::<AppEvent>();
    let app_event_tx = AppEventSender::new(tx_raw);
    let (op_tx, op_rx) = unbounded_channel::<Op>();
    let cfg = test_config();
    let bottom = BottomPane::new(BottomPaneParams {
        app_event_tx: app_event_tx.clone(),
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Codex to do anything".to_string(),
        disable_paste_burst: false,
    });
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("test"));
    let widget = ChatWidget {
        app_event_tx,
        codex_op_tx: op_tx,
        bottom_pane: bottom,
        active_exec_cell: None,
        config: cfg.clone(),
        auth_manager,
        session_header: SessionHeader::new(cfg.model.clone()),
        initial_user_message: None,
        token_info: None,
        rate_limit_snapshot: None,
        rate_limit_warnings: RateLimitWarningState::default(),
        stream: StreamController::new(cfg),
        running_commands: HashMap::new(),
        task_complete_pending: false,
        interrupts: InterruptManager::new(),
        reasoning_buffer: String::new(),
        full_reasoning_buffer: String::new(),
        conversation_id: None,
        frame_requester: FrameRequester::test_dummy(),
        show_welcome_banner: true,
        queued_user_messages: std::collections::VecDeque::new(),
        suppress_session_configured_redraw: false,
        pending_notification: None,
        is_review_mode: false,
        suppress_next_review_render: false,
        review_orchestrator: None,
        tui_notifications: codex_core::config_types::Notifications::Enabled(false),
        focused_subagent: None,
        subagents: std::collections::BTreeMap::new(),
    };
    (widget, rx, op_rx)
}

fn drain_insert_history(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
) -> Vec<Vec<ratatui::text::Line<'static>>> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let AppEvent::InsertHistoryCell(cell) = ev {
            let mut lines = cell.display_lines(80);
            if !cell.is_stream_continuation() && !out.is_empty() && !lines.is_empty() {
                lines.insert(0, "".into());
            }
            out.push(lines)
        }
    }
    out
}

fn test_config() -> Config {
    Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        std::env::temp_dir(),
    )
    .expect("config")
}
