use super::*;
use crate::codex_conversation::CodexConversation;
use std::sync::Arc;

pub(crate) fn dead_submit_conversation() -> Arc<CodexConversation> {
    let (tx_sub, rx_sub) = async_channel::bounded(SUBMISSION_CHANNEL_CAPACITY);
    drop(rx_sub);
    let (_tx_event, rx_event) = async_channel::unbounded();
    let codex = Codex {
        next_id: std::sync::atomic::AtomicU64::new(0),
        tx_sub,
        rx_event,
    };
    Arc::new(CodexConversation::new(codex))
}
