use super::tests::{drain_insert_history, make_chatwidget_manual};
use codex_core::protocol::{BackgroundEventEvent, Event, EventMsg, Op, TaskStartedEvent};
use pretty_assertions::assert_eq;
use std::path::PathBuf;

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

#[test]
fn parent_input_should_continue_during_subagent_work() {
    let (mut chat, mut rx, mut op_rx) = make_chatwidget_manual();

    // Announce a subagent so the widget tracks it.
    chat.handle_codex_event(Event {
        id: "root".into(),
        msg: EventMsg::BackgroundEvent(BackgroundEventEvent {
            message: "Subagent subagent-plan opened: draft plan".to_string(),
        }),
    });
    drain_insert_history(&mut rx);

    // Backend signals a running task (e.g., subagent turn still executing).
    chat.handle_codex_event(Event {
        id: "subagent-plan".into(),
        msg: EventMsg::TaskStarted(TaskStartedEvent {
            model_context_window: None,
        }),
    });
    drain_insert_history(&mut rx);
    assert!(!chat.bottom_pane.is_task_running());

    // User attempts to keep chatting with the parent while subagent work continues.
    chat.set_composer_text("Can you also summarize the README?".into());
    chat.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    // Expectation: parent turn stays interactive and dispatches immediately.
    let mut op = op_rx
        .try_recv()
        .expect("expected parent message to dispatch immediately");
    while matches!(op, Op::AddToHistory { .. }) {
        op = op_rx
            .try_recv()
            .expect("expected UserInput after AddToHistory");
    }
    match op {
        Op::UserInput { .. } => {}
        other => panic!("unexpected op while parent should remain interactive: {other:?}"),
    }
}

#[test]
fn subagent_progress_should_update_status_banner() {
    let (mut chat, mut rx, _) = make_chatwidget_manual();

    chat.handle_codex_event(Event {
        id: "root".into(),
        msg: EventMsg::BackgroundEvent(BackgroundEventEvent {
            message: "Subagent subagent-plan opened: draft plan".to_string(),
        }),
    });
    drain_insert_history(&mut rx);

    // Start a task so the status indicator becomes visible.
    chat.handle_codex_event(Event {
        id: "subagent-plan".into(),
        msg: EventMsg::TaskStarted(TaskStartedEvent {
            model_context_window: None,
        }),
    });
    drain_insert_history(&mut rx);

    let header_before = chat
        .bottom_pane
        .status_header()
        .expect("status indicator should be visible")
        .to_string();
    assert!(
        header_before.contains("Subagent"),
        "expected initial header to reflect subagent activity, got: {header_before}"
    );

    // Subagent emits a background progress update.
    chat.handle_codex_event(Event {
        id: "root".into(),
        msg: EventMsg::BackgroundEvent(BackgroundEventEvent {
            message: "Subagent subagent-plan progress: enumerating repository".to_string(),
        }),
    });
    drain_insert_history(&mut rx);

    let header_after = chat
        .bottom_pane
        .status_header()
        .expect("status indicator should remain visible")
        .to_string();

    assert!(
        header_after.contains("Subagent"),
        "expected status banner to include subagent progress, got: {header_after}"
    );
}

#[test]
fn esc_then_parent_message_should_dispatch_even_with_subagent_running() {
    let (mut chat, mut rx, mut op_rx) = make_chatwidget_manual();

    // Register a subagent and mark the turn as started.
    chat.handle_codex_event(Event {
        id: "root".into(),
        msg: EventMsg::BackgroundEvent(BackgroundEventEvent {
            message: "Subagent subagent-async opened: async work".to_string(),
        }),
    });
    drain_insert_history(&mut rx);
    chat.handle_codex_event(Event {
        id: "subagent-async".into(),
        msg: EventMsg::TaskStarted(TaskStartedEvent {
            model_context_window: None,
        }),
    });
    drain_insert_history(&mut rx);

    // User presses Esc, interrupting the parent turn.
    chat.handle_codex_event(Event {
        id: "root".into(),
        msg: EventMsg::TurnAborted(codex_core::protocol::TurnAbortedEvent {
            reason: codex_core::protocol::TurnAbortReason::Interrupted,
        }),
    });
    drain_insert_history(&mut rx);

    // Subagent continues emitting progress, triggering another TaskStarted.
    chat.handle_codex_event(Event {
        id: "subagent-async".into(),
        msg: EventMsg::TaskStarted(TaskStartedEvent {
            model_context_window: None,
        }),
    });
    drain_insert_history(&mut rx);

    // User attempts to send a fresh parent message.
    chat.set_composer_text("resuming conversation".into());
    chat.handle_key_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    // Expect dispatch; current behaviour leaves the queue empty.
    let mut op = op_rx
        .try_recv()
        .expect("expected parent message to dispatch immediately after Esc");
    while matches!(op, Op::AddToHistory { .. }) {
        op = op_rx
            .try_recv()
            .expect("expected UserInput after AddToHistory");
    }
    match op {
        Op::UserInput { .. } => {}
        other => {
            panic!("unexpected op while parent should remain interactive after Esc: {other:?}")
        }
    }
}
