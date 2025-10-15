use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::openai_tools::JsonSchema;
use crate::openai_tools::OpenAiTool;
use crate::openai_tools::ResponsesApiTool;

pub(crate) static PR_CHECKS_TOOL: LazyLock<OpenAiTool> = LazyLock::new(|| {
    OpenAiTool::Function(ResponsesApiTool {
        name: "run_pr_checks".to_string(),
        description:
            "Runs `gh pr checks --watch` in the current workspace and returns the command output."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: Some(false),
        },
    })
});
