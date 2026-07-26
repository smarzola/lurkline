use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lurkline"));
    command
        .args(args)
        .env_remove("SLACK_BASE_URL")
        .env_remove("SLACK_TEAM_ID")
        .env_remove("SLACK_TOKEN")
        .env_remove("SLACK_COOKIE");
    command.output().unwrap()
}

fn stdout(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn version_and_help_expose_the_complete_v020_cli_without_configuration() {
    assert_eq!(
        stdout(&["--version"]).trim(),
        format!("lurkline {}", env!("CARGO_PKG_VERSION"))
    );

    let root = stdout(&["--help"]);
    for command in [
        "inbox",
        "conversations",
        "search",
        "channel",
        "thread",
        "mcp",
    ] {
        assert!(root.contains(command), "root help omitted {command}");
    }

    let inbox = stdout(&["inbox", "--help"]);
    assert!(inbox.contains("--conversations"));
    assert!(inbox.contains("--messages"));
    assert!(inbox.contains("--json"));
    assert!(inbox.contains("from 1 through 50"));
    assert!(inbox.contains("from 1 through 200"));

    let search = stdout(&["search", "messages", "--help"]);
    for option in [
        "--in", "--after", "--before", "--cursor", "--limit", "--json",
    ] {
        assert!(search.contains(option), "search help omitted {option}");
    }
    assert!(search.contains("from 1 through 100"));
    assert!(search.contains("use # or @ to force a colliding name"));

    let channel = stdout(&["channel", "read", "--help"]);
    assert!(channel.contains("--cursor"));
    assert!(channel.contains("use # or @ to force a colliding name"));
    assert!(channel.contains("from 1 through 200"));
    let thread = stdout(&["thread", "read", "--help"]);
    assert!(thread.contains("--cursor"));
    assert!(thread.contains("use # or @ to force a colliding name"));
    assert!(thread.contains("from 1 through 200"));

    let message = stdout(&["message", "get", "--help"]);
    assert!(message.contains("conversation ID or exact name"));
    assert!(message.contains("use # or @ to force a colliding name"));

    let list = stdout(&["conversations", "list", "--help"]);
    assert!(list.contains("from 1 through 200"));
    let find = stdout(&["conversations", "find", "--help"]);
    assert!(find.contains("from 1 through 100"));
}

#[test]
fn invalid_inbox_bounds_fail_before_a_slack_request() {
    let output = Command::new(env!("CARGO_BIN_EXE_lurkline"))
        .args(["inbox", "--conversations", "0"])
        .env("SLACK_BASE_URL", "https://example.slack.com")
        .env("SLACK_TEAM_ID", "T000TEST")
        .env("SLACK_TOKEN", "xoxc-cli-test-secret")
        .env("SLACK_COOKIE", "d=xoxd-cli-test-secret")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: invalid conversation_limit: is outside the allowed range\n"
    );
}

#[test]
fn invalid_search_query_fails_before_a_slack_request() {
    let output = Command::new(env!("CARGO_BIN_EXE_lurkline"))
        .args(["search", "messages", ""])
        .env("SLACK_BASE_URL", "https://example.slack.com")
        .env("SLACK_TEAM_ID", "T000TEST")
        .env("SLACK_TOKEN", "xoxc-cli-test-secret")
        .env("SLACK_COOKIE", "d=xoxd-cli-test-secret")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: invalid query: must contain 1 to 512 non-control characters\n"
    );
}
