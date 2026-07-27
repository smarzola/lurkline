use std::{
    io::Write,
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn run(args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lurkline"));
    command
        .args(args)
        .env_remove("SLACK_BASE_URL")
        .env_remove("SLACK_TEAM_ID")
        .env_remove("SLACK_TOKEN")
        .env_remove("SLACK_COOKIE")
        .env_remove("LURKLINE_PROFILE");
    command.output().unwrap()
}

fn isolated_home() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("/tmp/lurkline-cli-test-{}-{nonce}", std::process::id())
}

fn run_with_stdin(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lurkline"))
        .args(args)
        .env("HOME", isolated_home())
        .env_remove("SLACK_BASE_URL")
        .env_remove("SLACK_TEAM_ID")
        .env("SLACK_TOKEN", "partial-environment-must-be-ignored")
        .env_remove("SLACK_COOKIE")
        .env_remove("LURKLINE_PROFILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
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
fn version_and_help_expose_the_complete_v030_cli_without_configuration() {
    assert_eq!(
        stdout(&["--version"]).trim(),
        format!("lurkline {}", env!("CARGO_PKG_VERSION"))
    );

    let root = stdout(&["--help"]);
    for command in [
        "auth",
        "inbox",
        "conversations",
        "search",
        "channel",
        "thread",
        "drafts",
        "mcp",
    ] {
        assert!(root.contains(command), "root help omitted {command}");
    }
    assert!(root.contains("--profile"));

    let auth = stdout(&["auth", "--help"]);
    for command in ["import-curl", "list", "status", "remove"] {
        assert!(auth.contains(command), "auth help omitted {command}");
    }

    let import = stdout(&["auth", "import-curl", "--help"]);
    assert!(import.contains("standard input"));
    assert!(import.contains("--replace-workspace"));
    assert!(import.contains("--profile"));
    assert!(import.contains("--json"));

    for command in ["list", "status", "remove"] {
        let help = stdout(&["auth", command, "--help"]);
        assert!(
            help.contains("--profile"),
            "{command} help omitted --profile"
        );
        assert!(help.contains("--json"), "{command} help omitted --json");
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
    let render = stdout(&["message", "render", "--help"]);
    assert!(render.contains("standard input"));
    assert!(render.contains("--json"));

    let list = stdout(&["conversations", "list", "--help"]);
    assert!(list.contains("from 1 through 200"));
    let find = stdout(&["conversations", "find", "--help"]);
    assert!(find.contains("from 1 through 100"));

    let drafts = stdout(&["drafts", "--help"]);
    for command in ["list", "get", "create", "update", "delete"] {
        assert!(drafts.contains(command), "draft help omitted {command}");
    }
    let draft_create = stdout(&["drafts", "create", "--help"]);
    for option in ["--thread-ts", "--broadcast", "--json"] {
        assert!(
            draft_create.contains(option),
            "draft create help omitted {option}"
        );
    }
    assert!(draft_create.contains("standard input"));
    let draft_delete = stdout(&["drafts", "delete", "--help"]);
    assert!(draft_delete.contains("--confirm"));

    let mcp = stdout(&["mcp", "--help"]);
    assert!(mcp.contains("--allow-write"));
}

#[test]
fn markdown_render_is_local_bounded_and_emits_stable_rich_text() {
    let output = run_with_stdin(
        &["message", "render", "--json"],
        b"Hello **world** from [docs](https://example.com).",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let rendered: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        rendered["text"],
        "Hello world from docs (https://example.com)."
    );
    assert_eq!(rendered["blocks"][0]["type"], "rich_text");
    assert_eq!(
        rendered["blocks"][0]["elements"][0]["elements"][1]["style"]["bold"],
        true
    );

    let invalid_utf8 = run_with_stdin(&["message", "render"], &[0xff]);
    assert!(!invalid_utf8.status.success());
    assert_eq!(
        String::from_utf8(invalid_utf8.stderr).unwrap(),
        "error: invalid markdown: must be valid UTF-8\n"
    );

    let oversized = run_with_stdin(
        &["message", "render"],
        &vec![b'x'; 40_000_usize.saturating_add(1)],
    );
    assert!(!oversized.status.success());
    assert_eq!(
        String::from_utf8(oversized.stderr).unwrap(),
        "error: invalid markdown: is larger than 40000 bytes\n"
    );
}

#[test]
fn auth_management_runs_without_slack_credentials_and_has_stable_empty_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_lurkline"))
        .args(["auth", "list", "--json", "--profile", "ignored"])
        .env("HOME", isolated_home())
        .env_remove("SLACK_BASE_URL")
        .env_remove("SLACK_TEAM_ID")
        .env("SLACK_TOKEN", "partial-environment-must-be-ignored")
        .env_remove("SLACK_COOKIE")
        .env_remove("LURKLINE_PROFILE")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({
            "default_profile": null,
            "profiles": []
        })
    );

    for command in ["status", "remove"] {
        let output = Command::new(env!("CARGO_BIN_EXE_lurkline"))
            .args(["auth", command, "--profile", "work", "--json"])
            .env("HOME", isolated_home())
            .env("SLACK_TOKEN", "partial-environment-must-be-ignored")
            .env_remove("SLACK_BASE_URL")
            .env_remove("SLACK_TEAM_ID")
            .env_remove("SLACK_COOKIE")
            .env_remove("LURKLINE_PROFILE")
            .output()
            .unwrap();
        assert!(!output.status.success(), "{command}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("credential profile work was not found"));
        assert!(!stderr.contains("SLACK_BASE_URL"));
        assert!(!stderr.contains("partial-environment"));
    }
}

#[test]
fn import_curl_requires_a_profile_before_reading_or_validating_stdin() {
    let output = run_with_stdin(
        &["auth", "import-curl"],
        b"curl 'https://example.slack.com/api/test?slack_route=T123'",
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: invalid profile: is required for cURL import\n"
    );
}

#[test]
fn import_curl_process_accepts_current_and_older_quoting_before_origin_rejection() {
    let current = concat!(
        "curl --url 'https://collector.example/api/test?slack_route=T123' \\\n",
        " -H 'content-type: multipart/form-data; boundary=----Boundary' \\\n",
        " -b $'d=xoxd-process-secret; note=it\\'s\\u0021' \\\n",
        " --data-raw $'------Boundary\\r\\n",
        "Content-Disposition: form-data; name=\"token\"\\r\\n\\r\\n",
        "xoxc-process-secret\\r\\n------Boundary--\\r\\n'"
    );
    let older = concat!(
        "curl 'https://collector.example/api/test?slack_route=T123' ",
        "-b 'd=xoxd-process-secret' --data 'token=xoxc-process-secret'"
    );
    for input in [current, older] {
        let output = run_with_stdin(
            &["auth", "import-curl", "--profile", "work"],
            input.as_bytes(),
        );
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("must target a Slack workspace origin"));
        assert!(!stderr.contains("process-secret"));
        assert!(!stderr.contains("partial-environment"));
    }
}

#[test]
fn import_curl_rejects_oversized_stdin_without_echoing_it() {
    let input = vec![b'x'; 256 * 1024 + 1];
    let output = run_with_stdin(&["auth", "import-curl", "--profile", "work"], &input);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: invalid curl: is larger than 256 KiB\n"
    );
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
