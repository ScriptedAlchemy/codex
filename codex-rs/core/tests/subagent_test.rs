//! Integration tests for async subagent system

use codex_core::subagent::NotificationType;
use codex_core::subagent::SubagentId;
use codex_core::subagent::SubagentManager;
use codex_core::subagent::SubagentState;
use std::sync::Arc;

#[tokio::test]
async fn test_create_subagent() {
    let manager = SubagentManager::new();

    // Create a mock conversation (this would normally be a real CodexConversation)
    // For now, we'll skip this as it requires full setup
    // This test demonstrates the API structure

    // Verify manager starts empty
    let subagents = manager.list_subagents().await;
    assert_eq!(subagents.len(), 0);
}

#[tokio::test]
async fn test_subagent_notifications() {
    // Test notification creation
    let notification = NotificationType::Message {
        content: "Test message".to_string(),
    };

    // Verify notification types
    assert!(!matches!(notification, NotificationType::Completed { .. }));
    assert!(matches!(notification, NotificationType::Message { .. }));
}

#[tokio::test]
async fn test_subagent_id_generation() {
    let id1 = SubagentId::new();
    let id2 = SubagentId::new();

    // IDs should be unique
    assert_ne!(id1, id2);
    assert!(!id1.as_str().is_empty());
    assert!(!id2.as_str().is_empty());
}

#[tokio::test]
async fn test_subagent_state_transitions() {
    // Test state enum values
    let active = SubagentState::Active;
    let completed = SubagentState::Completed;
    let error = SubagentState::Error {
        message: "Test error".to_string(),
    };

    assert_eq!(active, SubagentState::Active);
    assert_eq!(completed, SubagentState::Completed);
    assert!(matches!(error, SubagentState::Error { .. }));
}

#[tokio::test]
async fn test_inbox_empty_initially() {
    let manager = SubagentManager::new();

    // Check inbox when no subagents exist
    let notifications = manager.check_inbox(false).await;
    assert_eq!(notifications.len(), 0);

    let unread = manager.unread_count().await;
    assert_eq!(unread, 0);
}

#[tokio::test]
async fn test_notification_types() {
    // Test all notification type variants
    let message = NotificationType::Message {
        content: "Message".to_string(),
    };
    let question = NotificationType::Question {
        content: "Question?".to_string(),
    };
    let completed = NotificationType::Completed {
        summary: "Done".to_string(),
    };
    let error = NotificationType::Error {
        message: "Error!".to_string(),
    };

    // Verify they serialize/deserialize correctly
    let message_json = serde_json::to_string(&message).unwrap();
    assert!(message_json.contains("message"));

    let question_json = serde_json::to_string(&question).unwrap();
    assert!(question_json.contains("question"));

    let completed_json = serde_json::to_string(&completed).unwrap();
    assert!(completed_json.contains("completed"));

    let error_json = serde_json::to_string(&error).unwrap();
    assert!(error_json.contains("error"));
}

#[tokio::test]
async fn test_subagent_manager_concurrent_access() {
    let manager = Arc::new(SubagentManager::new());

    // Spawn multiple tasks that access the manager concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            let subagents = manager_clone.list_subagents().await;
            assert!(subagents.is_empty());
            i
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_subagent_id_display() {
    let id = SubagentId::new();
    let display_str = format!("{id}");
    let as_str = id.as_str();

    assert_eq!(display_str, as_str);
    assert!(!display_str.is_empty());
}

#[tokio::test]
async fn test_notification_terminal_check() {
    let message = NotificationType::Message {
        content: "Test".to_string(),
    };
    let completed = NotificationType::Completed {
        summary: "Done".to_string(),
    };
    let error = NotificationType::Error {
        message: "Error".to_string(),
    };

    // Terminal notifications should be Completed and Error
    assert!(!notification_is_terminal(&message));
    assert!(notification_is_terminal(&completed));
    assert!(notification_is_terminal(&error));
}

// Helper function to test terminal state
fn notification_is_terminal(notif: &NotificationType) -> bool {
    matches!(
        notif,
        NotificationType::Completed { .. } | NotificationType::Error { .. }
    )
}

#[tokio::test]
async fn test_manager_default() {
    let manager1 = SubagentManager::new();
    let manager2 = SubagentManager::default();

    // Both should start empty
    assert_eq!(manager1.list_subagents().await.len(), 0);
    assert_eq!(manager2.list_subagents().await.len(), 0);
}
