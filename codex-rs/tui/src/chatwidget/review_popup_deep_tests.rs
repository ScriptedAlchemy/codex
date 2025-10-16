use super::ReviewBranchMode;
use super::tests::make_chatwidget_manual;
use crate::app_event::AppEvent;
use crate::chatwidget::show_review_branch_picker_with_entries;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;

#[test]
fn review_popup_deep_option_sends_event() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual();

    chat.open_review_popup();
    // Move from the first item to the deep review option and activate it.
    chat.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    chat.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let mut found = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, AppEvent::OpenDeepReviewBranchPicker(_)) {
            found = true;
            break;
        }
    }

    assert!(found, "expected deep review picker event to be sent");
}

#[test]
fn deep_review_branch_picker_dispatches_start_event() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual();

    show_review_branch_picker_with_entries(
        &mut chat,
        ReviewBranchMode::Deep,
        "feature/awesome",
        vec!["origin/main".to_string(), "origin/develop".to_string()],
    );

    // Activate the first entry (origin/main).
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let mut received_base = None;
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::StartDeepReviewAgainstBase { base, .. } = event {
            received_base = Some(base);
            break;
        }
    }

    assert_eq!(
        received_base.as_deref(),
        Some("origin/main"),
        "expected deep review flow to request origin/main",
    );
}
