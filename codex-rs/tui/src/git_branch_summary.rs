//! Lightweight summary helpers for `/review-branch`.

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

/// Parse a `git diff --shortstat` line like
/// "12 files changed, 345 insertions(+), 67 deletions(-)"
/// into (files, insertions, deletions). Missing parts are treated as 0.
pub(crate) fn parse_shortstat_line(line: &str) -> Option<(usize, usize, usize)> {
    if line.trim().is_empty() {
        return None;
    }
    let mut files: Option<usize> = None;
    let mut insertions: usize = 0;
    let mut deletions: usize = 0;

    // Split by ',' and inspect fragments.
    for frag in line.split(',') {
        let frag = frag.trim();
        if frag.is_empty() {
            continue;
        }
        // Extract first integer in the fragment
        let mut num: Option<usize> = None;
        let mut acc = String::new();
        for ch in frag.chars() {
            if ch.is_ascii_digit() {
                acc.push(ch);
            } else if !acc.is_empty() {
                break;
            }
        }
        if !acc.is_empty()
            && let Ok(n) = acc.parse::<usize>()
        {
            num = Some(n);
        }
        let lower = frag.to_ascii_lowercase();
        if lower.contains("file changed") || lower.contains("files changed") {
            files = num;
        } else if lower.contains("insertion") {
            insertions = num.unwrap_or(0);
        } else if lower.contains("deletion") {
            deletions = num.unwrap_or(0);
        }
    }

    files.map(|f| (f, insertions, deletions))
}
