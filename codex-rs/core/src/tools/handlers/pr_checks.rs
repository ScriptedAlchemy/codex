use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::process::Command;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct PrChecksHandler;

#[async_trait]
impl ToolHandler for PrChecksHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { payload, turn, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "run_pr_checks handler received unsupported payload".to_string(),
                ));
            }
        };

        validate_arguments(&arguments)?;

        let cwd = turn.cwd.clone();
        let output = run_pr_checks_command(cwd).await?;

        let formatted = format_pr_checks_output(&output);

        Ok(ToolOutput::Function {
            content: formatted,
            success: Some(output.success),
        })
    }
}

fn validate_arguments(arguments: &str) -> Result<(), FunctionCallError> {
    let value: JsonValue = serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse run_pr_checks arguments: {err}"))
    })?;

    match value {
        JsonValue::Null => Ok(()),
        JsonValue::Object(ref map) if map.is_empty() => Ok(()),
        _ => Err(FunctionCallError::RespondToModel(
            "run_pr_checks does not accept parameters".to_string(),
        )),
    }
}

struct CommandOutputBundle {
    stdout: String,
    stderr: String,
    success: bool,
    exit_code: Option<i32>,
}

async fn run_pr_checks_command(cwd: PathBuf) -> Result<CommandOutputBundle, FunctionCallError> {
    let mut command = Command::new("gh");
    command.args(["pr", "checks", "--watch"]).current_dir(cwd);

    match command.output().await {
        Ok(output) => Ok(CommandOutputBundle {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            success: output.status.success(),
            exit_code: output.status.code(),
        }),
        Err(err) => Err(FunctionCallError::RespondToModel(format!(
            "failed to execute `gh pr checks --watch`: {err}"
        ))),
    }
}

fn format_pr_checks_output(output: &CommandOutputBundle) -> String {
    let status_line = match output.exit_code {
        Some(code) => format!("success: {}\nexit_code: {code}", output.success),
        None => format!("success: {}\nexit_code: <signal>", output.success),
    };

    let mut formatted = format!("{status_line}\nstdout:\n{}\n", output.stdout);

    if !output.stderr.is_empty() {
        formatted.push_str("stderr:\n");
        formatted.push_str(&output.stderr);
        formatted.push('\n');
    }

    formatted
}
