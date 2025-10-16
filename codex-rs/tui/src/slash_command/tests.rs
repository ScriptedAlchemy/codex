use super::*;
use pretty_assertions::assert_eq;
use std::str::FromStr;

#[test]
fn pr_checks_command_is_registered() {
    let command =
        SlashCommand::from_str("pr-checks").expect("/pr-checks should parse into a slash command");
    assert_eq!(command, SlashCommand::PrChecks);

    assert!(
        built_in_slash_commands()
            .into_iter()
            .map(|(name, _)| name)
            .any(|name| name == "pr-checks")
    );
}

#[test]
fn staged_compact_command_is_registered() {
    let command = SlashCommand::from_str("staged-compact")
        .expect("/staged-compact should parse into a slash command");
    assert_eq!(command, SlashCommand::StagedCompact);

    assert!(
        built_in_slash_commands()
            .into_iter()
            .map(|(name, _)| name)
            .any(|name| name == "staged-compact"),
        "expected /staged-compact to be in the built-in command list"
    );
}

#[test]
fn staged_compact_command_metadata_is_stable() {
    let cmd = SlashCommand::StagedCompact;
    assert_eq!(cmd.command(), "staged-compact");
    assert_eq!(
        cmd.description(),
        "hierarchically summarize older history while keeping the latest details"
    );
    assert!(
        !cmd.available_during_task(),
        "/staged-compact should be disabled while tasks are running"
    );
}
