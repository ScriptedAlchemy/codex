use std::path::PathBuf;

use tokio::process::Command;

/// Result of executing `gh pr checks --watch`.
#[derive(Debug, Clone)]
pub(crate) struct PrChecksOutcome {
    /// Whether the command completed successfully (exit status 0).
    pub success: bool,
    /// Exit status reported by the command, if available.
    pub exit_status: Option<i32>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Error returned when spawning or awaiting the command, if any.
    pub spawn_error: Option<String>,
}

impl PrChecksOutcome {
    pub(crate) fn failure_with_error(err: String) -> Self {
        Self {
            success: false,
            exit_status: None,
            stdout: String::new(),
            stderr: String::new(),
            spawn_error: Some(err),
        }
    }
}

/// Execute `gh pr checks --watch` in the provided working directory.
pub(crate) async fn run_pr_checks(cwd: PathBuf) -> PrChecksOutcome {
    let mut command = Command::new("gh");
    command.args(["pr", "checks", "--watch"]).current_dir(cwd);

    match command.output().await {
        Ok(output) => PrChecksOutcome {
            success: output.status.success(),
            exit_status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            spawn_error: None,
        },
        Err(err) => PrChecksOutcome::failure_with_error(err.to_string()),
    }
}
