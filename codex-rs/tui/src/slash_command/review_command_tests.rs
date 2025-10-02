use super::*;

#[test]
fn review_command_is_registered() {
    let mut names: Vec<&str> = built_in_slash_commands()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.sort();

    assert!(
        names.contains(&"review"),
        "expected /review to remain registered"
    );
}

#[test]
fn review_command_metadata_is_stable() {
    let cmd = SlashCommand::Review;
    assert_eq!(cmd.command(), "review");
    assert_eq!(
        cmd.description(),
        "review my current changes and find issues"
    );
    assert!(
        !cmd.available_during_task(),
        "/review should remain disabled during tasks"
    );
}
