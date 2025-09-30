//! Async subagent system for spawning background conversations
//!
//! This module implements async subagents that allow a parent agent to spawn
//! child conversations that run in the background without blocking the main
//! chat flow. The parent agent can:
//! - Create subagents with a specific task
//! - Check an inbox for notifications from subagents
//! - Reply to subagent messages
//! - End subagent conversations
//! - List all active subagents

use crate::CodexConversation;
use crate::error::CodexErr;
use crate::error::Result as CodexResult;
use crate::protocol::Op;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Unique identifier for a subagent
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubagentId(String);

impl SubagentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SubagentId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SubagentId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl Default for SubagentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SubagentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// State of a subagent
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentState {
    /// Subagent is actively running
    Active,
    /// Subagent has completed its task
    Completed,
    /// Subagent encountered an error
    Error { message: String },
}

/// Type of notification from a subagent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationType {
    /// Subagent has a message for the parent
    Message { content: String },
    /// Subagent is asking for input/clarification
    Question { content: String },
    /// Subagent has completed its task
    Completed { summary: String },
    /// Subagent encountered an error
    Error { message: String },
}

impl NotificationType {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            NotificationType::Completed { .. } | NotificationType::Error { .. }
        )
    }
}

/// A notification from a subagent to its parent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentNotification {
    /// ID of the subagent that sent this notification
    pub subagent_id: SubagentId,
    /// When this notification was created
    pub timestamp: DateTime<Utc>,
    /// The type and content of the notification
    pub notification: NotificationType,
    /// Whether this notification has been read
    pub read: bool,
}

/// Information about a subagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: SubagentId,
    pub task: String,
    pub state: SubagentState,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub unread_count: usize,
}

/// Internal representation of a subagent
struct Subagent {
    id: SubagentId,
    task: String,
    state: SubagentState,
    conversation: Option<Arc<CodexConversation>>,
    created_at: DateTime<Utc>,
    last_activity: DateTime<Utc>,
    notifications: VecDeque<SubagentNotification>,
}

impl Subagent {
    fn info(&self) -> SubagentInfo {
        let unread_count = self.notifications.iter().filter(|n| !n.read).count();

        SubagentInfo {
            id: self.id.clone(),
            task: self.task.clone(),
            state: self.state.clone(),
            created_at: self.created_at,
            last_activity: self.last_activity,
            unread_count,
        }
    }

    fn add_notification(&mut self, notification: NotificationType) {
        self.notifications.push_back(SubagentNotification {
            subagent_id: self.id.clone(),
            timestamp: Utc::now(),
            notification,
            read: false,
        });
        self.last_activity = Utc::now();
    }
}

/// Manages subagents for a parent conversation
pub struct SubagentManager {
    subagents: Arc<RwLock<HashMap<SubagentId, Subagent>>>,
}

impl SubagentManager {
    pub fn new() -> Self {
        Self {
            subagents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new subagent with the given task
    pub async fn create_subagent(
        &self,
        task: String,
        conversation: Option<Arc<CodexConversation>>,
    ) -> CodexResult<SubagentId> {
        let id = SubagentId::new();
        let now = Utc::now();

        let subagent = Subagent {
            id: id.clone(),
            task: task.clone(),
            state: SubagentState::Active,
            conversation,
            created_at: now,
            last_activity: now,
            notifications: VecDeque::new(),
        };

        self.subagents.write().await.insert(id.clone(), subagent);

        Ok(id)
    }

    /// List all subagents
    pub async fn list_subagents(&self) -> Vec<SubagentInfo> {
        let subagents = self.subagents.read().await;
        let mut infos: Vec<_> = subagents.values().map(Subagent::info).collect();

        // Sort by last activity, most recent first
        infos.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));

        infos
    }

    /// Get information about a specific subagent
    pub async fn get_subagent_info(&self, id: &SubagentId) -> CodexResult<SubagentInfo> {
        let subagents = self.subagents.read().await;
        subagents
            .get(id)
            .map(Subagent::info)
            .ok_or_else(|| CodexErr::SubagentNotFound(id.clone()))
    }

    /// Check the inbox for notifications from all subagents
    pub async fn check_inbox(&self, mark_as_read: bool) -> Vec<SubagentNotification> {
        let mut subagents = self.subagents.write().await;
        let mut all_notifications = Vec::new();

        for subagent in subagents.values_mut() {
            if mark_as_read {
                for notif in &mut subagent.notifications {
                    notif.read = true;
                }
            }

            let notifications: Vec<_> = subagent.notifications.iter().cloned().collect();
            all_notifications.extend(notifications);
        }

        // Sort by timestamp, most recent first
        all_notifications.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        all_notifications
    }

    /// Check inbox for a specific subagent
    pub async fn check_subagent_inbox(
        &self,
        id: &SubagentId,
        mark_as_read: bool,
    ) -> CodexResult<Vec<SubagentNotification>> {
        let mut subagents = self.subagents.write().await;
        let subagent = subagents
            .get_mut(id)
            .ok_or_else(|| CodexErr::SubagentNotFound(id.clone()))?;

        if mark_as_read {
            for notif in &mut subagent.notifications {
                notif.read = true;
            }
        }

        Ok(subagent.notifications.iter().cloned().collect())
    }

    /// Send a message to a subagent
    pub async fn reply_to_subagent(&self, id: &SubagentId, message: String) -> CodexResult<()> {
        let subagents = self.subagents.read().await;
        let subagent = subagents
            .get(id)
            .ok_or_else(|| CodexErr::SubagentNotFound(id.clone()))?;

        // Submit the message to the subagent's conversation, if available.
        // If the conversation is not wired yet, accept the reply without error
        // so the tool/op remains usable. Delivery can be handled by a future
        // wiring or ignored depending on higher-level policy.
        if let Some(conv) = &subagent.conversation {
            conv.submit(Op::UserInput {
                items: vec![crate::protocol::InputItem::Text { text: message }],
            })
            .await?;
        }

        Ok(())
    }

    /// End a subagent conversation
    pub async fn end_subagent(&self, id: &SubagentId) -> CodexResult<SubagentInfo> {
        let mut subagents = self.subagents.write().await;
        let subagent = subagents
            .get_mut(id)
            .ok_or_else(|| CodexErr::SubagentNotFound(id.clone()))?;

        // Update state to completed
        subagent.state = SubagentState::Completed;
        subagent.last_activity = Utc::now();

        // Shut down the conversation if there is one
        if let Some(conv) = &subagent.conversation {
            conv.submit(Op::Shutdown).await?;
        }

        let info = subagent.info();

        // Remove from active subagents
        subagents.remove(id);

        Ok(info)
    }

    /// Add a notification to a subagent (used internally by event processing)
    pub async fn add_notification(
        &self,
        id: &SubagentId,
        notification: NotificationType,
    ) -> CodexResult<()> {
        let mut subagents = self.subagents.write().await;
        let subagent = subagents
            .get_mut(id)
            .ok_or_else(|| CodexErr::SubagentNotFound(id.clone()))?;

        // Update state based on notification type before adding
        if notification.is_terminal() {
            match &notification {
                NotificationType::Completed { .. } => {
                    subagent.state = SubagentState::Completed;
                }
                NotificationType::Error { message } => {
                    subagent.state = SubagentState::Error {
                        message: message.clone(),
                    };
                }
                _ => {}
            }
        }

        subagent.add_notification(notification);

        Ok(())
    }

    /// Get the conversation for a subagent (for event processing)
    pub async fn get_conversation(&self, id: &SubagentId) -> CodexResult<Arc<CodexConversation>> {
        let subagents = self.subagents.read().await;
        let sub = subagents
            .get(id)
            .ok_or_else(|| CodexErr::SubagentNotFound(id.clone()))?;
        match &sub.conversation {
            Some(conv) => Ok(conv.clone()),
            None => Err(CodexErr::UnsupportedOperation(
                "Subagent conversation is not available".to_string(),
            )),
        }
    }

    /// Get count of unread notifications across all subagents
    pub async fn unread_count(&self) -> usize {
        let subagents = self.subagents.read().await;
        subagents
            .values()
            .map(|s| s.notifications.iter().filter(|n| !n.read).count())
            .sum()
    }
}

impl Default for SubagentManager {
    fn default() -> Self {
        Self::new()
    }
}
