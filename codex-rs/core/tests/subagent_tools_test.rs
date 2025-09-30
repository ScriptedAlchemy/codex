//! Tests for subagent tool definitions

use codex_core::subagent_tools::check_inbox_tool;
use codex_core::subagent_tools::create_subagent_tool;
use codex_core::subagent_tools::end_subagent_tool;
use codex_core::subagent_tools::list_subagents_tool;
use codex_core::subagent_tools::reply_to_subagent_tool;

#[test]
fn test_create_subagent_tool_definition() {
    let tool = create_subagent_tool();

    // Verify the tool has the correct structure
    let json = serde_json::to_value(&tool).unwrap();
    assert!(json.is_object());
    assert_eq!(json["name"], "CreateSubagent");
    assert!(json["description"].is_string());
    assert!(json["description"].as_str().unwrap().len() > 20);
}

#[test]
fn test_list_subagents_tool_definition() {
    let tool = list_subagents_tool();

    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["name"], "ListSubagents");
    assert!(json["description"].is_string());
}

#[test]
fn test_check_inbox_tool_definition() {
    let tool = check_inbox_tool();

    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["name"], "CheckInbox");
    assert!(json["description"].is_string());

    // Should have optional parameters
    assert!(json["parameters"].is_object());
}

#[test]
fn test_reply_to_subagent_tool_definition() {
    let tool = reply_to_subagent_tool();

    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["name"], "ReplyToSubagent");
    assert!(json["description"].is_string());

    // Should have required parameters
    assert!(json["parameters"].is_object());
}

#[test]
fn test_end_subagent_tool_definition() {
    let tool = end_subagent_tool();

    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["name"], "EndSubagent");
    assert!(json["description"].is_string());
}

#[test]
fn test_all_tools_unique_names() {
    let tools = vec![
        create_subagent_tool(),
        list_subagents_tool(),
        check_inbox_tool(),
        reply_to_subagent_tool(),
        end_subagent_tool(),
    ];

    let mut names = std::collections::HashSet::new();
    for tool in &tools {
        let json = serde_json::to_value(tool).unwrap();
        let name = json["name"].as_str().unwrap();
        assert!(names.insert(name.to_string()), "Duplicate tool name found");
    }

    assert_eq!(names.len(), 5);
}

#[test]
fn test_tool_descriptions_not_empty() {
    let tools = vec![
        create_subagent_tool(),
        list_subagents_tool(),
        check_inbox_tool(),
        reply_to_subagent_tool(),
        end_subagent_tool(),
    ];

    for tool in &tools {
        let json = serde_json::to_value(tool).unwrap();
        let name = json["name"].as_str().unwrap();
        let desc = json["description"].as_str().unwrap();

        assert!(!desc.is_empty(), "Tool {} has empty description", name);
        assert!(desc.len() > 20, "Tool {} description is too short", name);
    }
}

#[test]
fn test_tools_serialize_correctly() {
    let tools = vec![
        create_subagent_tool(),
        list_subagents_tool(),
        check_inbox_tool(),
        reply_to_subagent_tool(),
        end_subagent_tool(),
    ];

    for tool in &tools {
        // Should serialize to JSON without errors
        let json = serde_json::to_string(tool);
        let json_value = serde_json::to_value(tool).unwrap();
        let name = json_value["name"].as_str().unwrap();

        assert!(json.is_ok(), "Tool {} failed to serialize", name);

        // Verify required fields are present
        assert!(json_value["name"].is_string());
        assert!(json_value["description"].is_string());
        assert!(json_value["parameters"].is_object());
    }
}

#[test]
fn test_create_subagent_tool_has_task_parameter() {
    let tool = create_subagent_tool();
    let json = serde_json::to_value(&tool).unwrap();

    let params = &json["parameters"];
    assert!(params["properties"].is_object());

    // Should have "task" property
    let props = params["properties"].as_object().unwrap();
    assert!(props.contains_key("task"));
}

#[test]
fn test_reply_tool_has_required_parameters() {
    let tool = reply_to_subagent_tool();
    let json = serde_json::to_value(&tool).unwrap();

    let params = &json["parameters"];
    let props = params["properties"].as_object().unwrap();

    // Should have both subagent_id and message
    assert!(props.contains_key("subagent_id"));
    assert!(props.contains_key("message"));
}

#[test]
fn test_end_subagent_tool_has_subagent_id() {
    let tool = end_subagent_tool();
    let json = serde_json::to_value(&tool).unwrap();

    let params = &json["parameters"];
    let props = params["properties"].as_object().unwrap();

    assert!(props.contains_key("subagent_id"));
}
