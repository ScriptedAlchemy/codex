#![allow(clippy::unwrap_used)]

use core_test_support::load_default_config_for_test;
use core_test_support::skip_if_no_network;
use tempfile::TempDir;

use codex_core::CodexAuth;
use codex_core::auth::read_openai_api_key_from_env;
use codex_core::auth::OPENAI_API_KEY_ENV_VAR;
use codex_core::ConversationManager;
use codex_core::protocol::EventMsg;
use codex_core::protocol::Op;
use codex_core::protocol::ReviewRequest;

/// Integration test: exercise the review flow against the live API using a real API key.
///
/// Skips when network is disabled or when `OPENAI_API_KEY` is not set.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_slash_command_flow_no_400_errors() {
    skip_if_no_network!();

    // Prefer ChatGPT OAuth token from the user's ~/.codex/auth.json (codex login flow).
    // Fall back to OPENAI_API_KEY from the environment. If neither is available,
    // skip to keep CI green when auth is not configured.
    let auth = if let Some(a) = dirs::home_dir()
        .map(|mut h| {
            h.push(".codex");
            h
        })
        .and_then(|p| CodexAuth::from_codex_home(&p).ok().flatten())
    {
        a
    } else if let Some(api_key) = read_openai_api_key_from_env() {
        CodexAuth::from_api_key(&api_key)
    } else {
        eprintln!(
            "Skipping review_slash_command_flow_no_400_errors: no auth found in ~/.codex/auth.json and {OPENAI_API_KEY_ENV_VAR} not set"
        );
        return;
    };

    // Hermetic codex home and cwd for the test.
    let codex_home = TempDir::new().expect("create temp codex_home");
    let mut config = load_default_config_for_test(&codex_home);
    // Keep a tiny prompt to avoid any instruction size issues; the review
    // thread already prepends the standard review instructions.
    config.user_instructions = Some("integration test".to_string());

    // Build conversation manager with real ChatGPT auth.
    let manager = ConversationManager::with_auth(auth);
    let conversation = manager
        .new_conversation(config)
        .await
        .expect("create conversation")
        .conversation;

    // Kick off review (equivalent to the TUI's `/review` preset for current changes).
    conversation
        .submit(Op::Review {
            review_request: ReviewRequest {
                prompt: "Please respond with a short acknowledgment for the integration test.".to_string(),
                user_facing_hint: "integration test".to_string(),
            },
        })
        .await
        .expect("submit review op");

    // Consume events until we exit review mode. Fail fast on 400s from the API.
    use tokio::time::{timeout, Duration};
    let overall_deadline = Duration::from_secs(120);
    let started_at = std::time::Instant::now();
    let mut saw_entered = false;
    let mut saw_exited = false;
    while started_at.elapsed() < overall_deadline && !saw_exited {
        let next = timeout(Duration::from_secs(30), conversation.next_event())
            .await
            .expect("timeout waiting for review events")
            .expect("stream ended unexpectedly");
        match next.msg {
            EventMsg::EnteredReviewMode(_) => {
                saw_entered = true;
            }
            EventMsg::ExitedReviewMode(_ev) => {
                saw_exited = true;
            }
            EventMsg::StreamError(err) => {
                // Ensure we did not hit schema/formatting issues resulting in 400s.
                let msg = err.message.to_lowercase();
                assert!(
                    !msg.contains("400"),
                    "Review flow received a 400 error from the API: {msg}"
                );
            }
            _ => {}
        }
    }

    assert!(saw_entered, "Expected to enter review mode");
    assert!(saw_exited, "Expected to exit review mode within the deadline");
}
