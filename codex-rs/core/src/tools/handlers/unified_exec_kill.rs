use async_trait::async_trait;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct UnifiedExecKillHandler;

#[derive(Deserialize)]
struct UnifiedExecKillArgs {
    session_id: String,
}

#[async_trait]
impl ToolHandler for UnifiedExecKillHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::UnifiedExec
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(
            payload,
            ToolPayload::UnifiedExec { .. } | ToolPayload::Function { .. }
        )
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;

        let args = match payload {
            ToolPayload::UnifiedExec { arguments } | ToolPayload::Function { arguments } => {
                serde_json::from_str::<UnifiedExecKillArgs>(&arguments).map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse function arguments: {err:?}"
                    ))
                })?
            }
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "unified_exec_kill handler received unsupported payload".to_string(),
                ));
            }
        };

        let id: i32 = args.session_id.parse().map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "invalid session_id: {} due to error {:?}",
                args.session_id, e
            ))
        })?;

        session
            .terminate_unified_exec_session(id)
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("unified exec kill failed: {err:?}"))
            })?;

        Ok(ToolOutput::Function {
            content: "{\"ok\":true}".to_string(),
            success: Some(true),
        })
    }
}
