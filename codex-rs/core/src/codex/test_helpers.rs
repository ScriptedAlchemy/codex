use super::Codex;
use crate::CodexConversation;
use crate::protocol::Event;
use crate::protocol::Submission;
use async_channel;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

/// Returns a CodexConversation whose submit() call will fail immediately.
///
/// This is accomplished by constructing a Codex with a submission channel
/// whose receiver has been dropped, causing any send to return an error.
/// The event channel sender is also dropped so `next_event()` returns an error
/// promptly when awaited in tests.
pub(crate) fn dead_submit_conversation() -> Arc<CodexConversation> {
    // Create a bounded submission channel and drop the receiver so `send()` fails.
    let (tx_sub, rx_sub) = async_channel::bounded::<Submission>(1);
    drop(rx_sub);

    // Create an unbounded event channel and drop the sender so `recv()` fails.
    let (tx_event, rx_event) = async_channel::unbounded::<Event>();
    drop(tx_event);

    // Construct a Codex with dead channels. This module is a child of `codex`,
    // so it may access private fields of `Codex`.
    let codex = Codex {
        next_id: AtomicU64::new(0),
        tx_sub,
        rx_event,
    };
    Arc::new(CodexConversation::new(codex))
}
