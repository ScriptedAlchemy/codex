use super::*;
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
