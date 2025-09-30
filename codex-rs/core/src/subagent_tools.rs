//! Tool definitions for async subagent operations

use crate::openai_tools::{JsonSchema, ResponsesApiTool};
use std::collections::BTreeMap;

/// Tool for creating a new subagent
pub fn create_subagent_tool() -> ResponsesApiTool {
    let mut properties = BTreeMap::new();
    properties.insert(
        "task".to_string(),
        JsonSchema::String {
            description: Some("The task or prompt for the subagent to work on. Be specific and clear about what you want the subagent to accomplish.".to_string()),
        },
    );
    properties.insert(
        "config".to_string(),
        JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: Some(false),
        },
    );

    ResponsesApiTool {
        name: "CreateSubagent".to_string(),
        description: "Create a new async subagent to work on a task in the background without blocking the main conversation. The subagent will run independently and notify the parent agent when it has updates or needs input.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["task".to_string()]),
            additional_properties: Some(false),
        },
    }
}

/// Tool for listing all subagents
pub fn list_subagents_tool() -> ResponsesApiTool {
    ResponsesApiTool {
        name: "ListSubagents".to_string(),
        description: "List all active subagents with their current status, unread notification counts, and last activity. Use this to check on the progress of background tasks.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: Some(false),
        },
    }
}

/// Tool for checking the notification inbox
pub fn check_inbox_tool() -> ResponsesApiTool {
    let mut properties = BTreeMap::new();
    properties.insert(
        "subagent_id".to_string(),
        JsonSchema::String {
            description: Some("Optional: Only check notifications from this specific subagent. If not provided, returns notifications from all subagents.".to_string()),
        },
    );
    properties.insert(
        "mark_as_read".to_string(),
        JsonSchema::Boolean {
            description: Some("Whether to mark the retrieved notifications as read. Defaults to true.".to_string()),
        },
    );

    ResponsesApiTool {
        name: "CheckInbox".to_string(),
        description: "Check the inbox for notifications from all subagents. Returns messages, questions, completion notices, and errors from background tasks. You can optionally filter by subagent ID and mark notifications as read.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false),
        },
    }
}

/// Tool for replying to a subagent
pub fn reply_to_subagent_tool() -> ResponsesApiTool {
    let mut properties = BTreeMap::new();
    properties.insert(
        "subagent_id".to_string(),
        JsonSchema::String {
            description: Some("The ID of the subagent to send the message to.".to_string()),
        },
    );
    properties.insert(
        "message".to_string(),
        JsonSchema::String {
            description: Some("The message or response to send to the subagent.".to_string()),
        },
    );

    ResponsesApiTool {
        name: "ReplyToSubagent".to_string(),
        description: "Send a message or response to a specific subagent. Use this to answer questions, provide additional context, or give instructions to a running subagent.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["subagent_id".to_string(), "message".to_string()]),
            additional_properties: Some(false),
        },
    }
}

/// Tool for ending a subagent conversation
pub fn end_subagent_tool() -> ResponsesApiTool {
    let mut properties = BTreeMap::new();
    properties.insert(
        "subagent_id".to_string(),
        JsonSchema::String {
            description: Some("The ID of the subagent to end.".to_string()),
        },
    );

    ResponsesApiTool {
        name: "EndSubagent".to_string(),
        description: "End a subagent conversation and clean up its resources. Use this when a subagent has completed its task or is no longer needed. Returns a final summary of the subagent's status.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["subagent_id".to_string()]),
            additional_properties: Some(false),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_subagent_tool() {
        let tool = create_subagent_tool();
        assert_eq!(tool.name, "CreateSubagent");
        assert!(!tool.description.is_empty());
    }

    #[test]
    fn test_list_subagents_tool() {
        let tool = list_subagents_tool();
        assert_eq!(tool.name, "ListSubagents");
        assert!(!tool.description.is_empty());
    }

    #[test]
    fn test_check_inbox_tool() {
        let tool = check_inbox_tool();
        assert_eq!(tool.name, "CheckInbox");
        assert!(!tool.description.is_empty());
    }

    #[test]
    fn test_reply_to_subagent_tool() {
        let tool = reply_to_subagent_tool();
        assert_eq!(tool.name, "ReplyToSubagent");
        assert!(!tool.description.is_empty());
    }

    #[test]
    fn test_end_subagent_tool() {
        let tool = end_subagent_tool();
        assert_eq!(tool.name, "EndSubagent");
        assert!(!tool.description.is_empty());
    }
}