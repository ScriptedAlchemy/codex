//! Lightweight summary helpers for branch reviews.

use std::io;
use std::process::Stdio;
use tokio::process::Command;

/// Return a concise shortstat for `<base>...HEAD` if there are changes, e.g.:
/// "12 files changed, 345 insertions(+), 67 deletions(-)".
/// Returns Ok(None) if the diff is empty or cannot be computed.
pub(crate) async fn branch_shortstat(base: &str) -> io::Result<Option<String>> {
    let args = ["diff", "--shortstat", &format!("{base}...HEAD")];
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await?;
    if !(output.status.success() || output.status.code() == Some(1)) {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}
