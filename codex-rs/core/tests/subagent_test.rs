//! Integration tests for the async subagent system.

use codex_core::error::CodexErr;
use codex_core::subagent::NotificationType;
use codex_core::subagent::SubagentId;
use codex_core::subagent::SubagentManager;
use codex_core::subagent::SubagentState;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn create_subagent_is_listed() {
    let manager = SubagentManager::new();

    let id = manager
        .create_subagent("track progress".to_string(), None)
        .await
        .unwrap();

    let subagents = manager.list_subagents().await;
    assert_eq!(1, subagents.len());
    let info = &subagents[0];
    assert_eq!(info.id.as_str(), id.as_str());
    assert_eq!(info.task, "track progress");
    assert_eq!(info.state, SubagentState::Active);
    assert_eq!(0, info.unread_count);
}

#[tokio::test]
async fn check_inbox_marks_read_and_clears() {
    let manager = SubagentManager::new();
    let subagent_id = manager
        .create_subagent("notify".to_string(), None)
        .await
        .unwrap();

    manager
        .add_notification(
            &subagent_id,
            NotificationType::Message {
                content: "Initial update".to_string(),
            },
        )
        .await
        .unwrap();

    let notifications = manager.check_inbox(false).await;
    assert_eq!(1, notifications.len());
    assert_eq!(notifications[0].subagent_id.as_str(), subagent_id.as_str());
    assert!(!notifications[0].read);
    match &notifications[0].notification {
        NotificationType::Message { content } => assert_eq!(content, "Initial update"),
        other => panic!("unexpected notification: {other:?}"),
    }

    let notifications = manager.check_inbox(true).await;
    assert_eq!(1, notifications.len());
    assert!(notifications[0].read);

    let info = manager.get_subagent_info(&subagent_id).await.unwrap();
    assert_eq!(0, info.unread_count);
    assert!(manager.check_inbox(false).await.is_empty());
}

#[tokio::test]
async fn check_subagent_inbox_only_clears_selected() {
    let manager = SubagentManager::new();
    let first = manager
        .create_subagent("first".to_string(), None)
        .await
        .unwrap();
    sleep(Duration::from_millis(5)).await;
    let second = manager
        .create_subagent("second".to_string(), None)
        .await
        .unwrap();

    manager
        .add_notification(
            &first,
            NotificationType::Question {
                content: "Need guidance?".to_string(),
            },
        )
        .await
        .unwrap();
    manager
        .add_notification(
            &second,
            NotificationType::Message {
                content: "Still running".to_string(),
            },
        )
        .await
        .unwrap();

    let notifications = manager.check_subagent_inbox(&first, false).await.unwrap();
    assert_eq!(1, notifications.len());
    assert!(!notifications[0].read);

    let notifications = manager.check_subagent_inbox(&first, true).await.unwrap();
    assert_eq!(1, notifications.len());
    assert!(notifications[0].read);
    assert!(
        manager
            .check_subagent_inbox(&first, false)
            .await
            .unwrap()
            .is_empty()
    );

    let info = manager.get_subagent_info(&second).await.unwrap();
    assert_eq!(1, info.unread_count);
}

#[tokio::test]
async fn completed_notification_updates_state() {
    let manager = SubagentManager::new();
    let subagent_id = manager
        .create_subagent("wrap up".to_string(), None)
        .await
        .unwrap();

    manager
        .add_notification(
            &subagent_id,
            NotificationType::Completed {
                summary: "All done".to_string(),
            },
        )
        .await
        .unwrap();

    let info = manager.get_subagent_info(&subagent_id).await.unwrap();
    assert_eq!(SubagentState::Completed, info.state);
}

#[tokio::test]
async fn error_notification_updates_state() {
    let manager = SubagentManager::new();
    let subagent_id = manager
        .create_subagent("might fail".to_string(), None)
        .await
        .unwrap();

    manager
        .add_notification(
            &subagent_id,
            NotificationType::Error {
                message: "Disk full".to_string(),
            },
        )
        .await
        .unwrap();

    let info = manager.get_subagent_info(&subagent_id).await.unwrap();
    assert_eq!(
        SubagentState::Error {
            message: "Disk full".to_string(),
        },
        info.state
    );
}

#[tokio::test]
async fn end_subagent_returns_final_state_and_removes() {
    let manager = SubagentManager::new();
    let subagent_id = manager
        .create_subagent("cleanup".to_string(), None)
        .await
        .unwrap();

    manager
        .add_notification(
            &subagent_id,
            NotificationType::Completed {
                summary: "Cleanup finished".to_string(),
            },
        )
        .await
        .unwrap();

    let final_state = manager.end_subagent(&subagent_id).await.unwrap();
    assert_eq!(SubagentState::Completed, final_state.state);
    assert!(manager.list_subagents().await.is_empty());
}

#[tokio::test]
async fn reply_without_conversation_is_noop() {
    let manager = SubagentManager::new();
    let subagent_id = manager
        .create_subagent("no wiring yet".to_string(), None)
        .await
        .unwrap();

    manager
        .reply_to_subagent(&subagent_id, "hello".to_string())
        .await
        .unwrap();
}

#[tokio::test]
async fn reply_to_missing_subagent_returns_error() {
    let manager = SubagentManager::new();
    let missing = SubagentId::new();
    let err = manager
        .reply_to_subagent(&missing, "data".to_string())
        .await
        .unwrap_err();

    assert!(matches!(err, CodexErr::SubagentNotFound(_)));
}

#[tokio::test]
async fn list_subagents_sorted_by_last_activity() {
    let manager = SubagentManager::new();
    let first = manager
        .create_subagent("first".to_string(), None)
        .await
        .unwrap();
    sleep(Duration::from_millis(5)).await;
    let second = manager
        .create_subagent("second".to_string(), None)
        .await
        .unwrap();

    manager
        .add_notification(
            &first,
            NotificationType::Message {
                content: "progress".to_string(),
            },
        )
        .await
        .unwrap();

    let list = manager.list_subagents().await;
    assert_eq!(2, list.len());
    assert_eq!(list[0].id.as_str(), first.as_str());
    assert_eq!(list[1].id.as_str(), second.as_str());
}

#[tokio::test]
async fn unread_count_across_multiple_subagents() {
    let manager = SubagentManager::new();
    let first = manager
        .create_subagent("first".to_string(), None)
        .await
        .unwrap();
    let second = manager
        .create_subagent("second".to_string(), None)
        .await
        .unwrap();

    manager
        .add_notification(
            &first,
            NotificationType::Message {
                content: "msg".to_string(),
            },
        )
        .await
        .unwrap();
    manager
        .add_notification(
            &second,
            NotificationType::Question {
                content: "status?".to_string(),
            },
        )
        .await
        .unwrap();

    assert_eq!(2, manager.unread_count().await);

    manager.check_subagent_inbox(&first, true).await.unwrap();
    assert_eq!(1, manager.unread_count().await);

    manager.check_inbox(true).await;
    assert_eq!(0, manager.unread_count().await);
}
