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

fn run_with_credentials(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lurkline"))
        .args(args)
        .env("HOME", isolated_home())
        .env("SLACK_BASE_URL", "https://example.slack.com")
        .env("SLACK_TEAM_ID", "T000TEST")
        .env("SLACK_TOKEN", "xoxc-cli-test-secret")
        .env("SLACK_COOKIE", "d=xoxd-cli-test-secret")
        .env("LURKLINE_TIMEOUT_MS", "500")
        .env_remove("LURKLINE_PROFILE")
        .output()
        .unwrap()
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
fn version_and_help_expose_the_complete_cli_without_configuration() {
    assert_eq!(
        stdout(&["--version"]).trim(),
        format!("lurkline {}", env!("CARGO_PKG_VERSION"))
    );

    let root = stdout(&["--help"]);
    assert!(root.contains("guarded rich-text authoring"));
    for command in [
        "auth",
        "activity",
        "inbox",
        "conversations",
        "search",
        "channel",
        "thread",
        "drafts",
        "files",
        "emoji",
        "reactions",
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
    assert!(inbox.contains("resolving message authors once per snapshot"));

    let activity = stdout(&["activity", "--help"]);
    for option in [
        "--since",
        "--after",
        "--before",
        "--include",
        "--exclude",
        "--oldest-first",
        "--conversations",
        "--per-conversation",
        "--limit",
        "--cursor",
        "--json",
    ] {
        assert!(activity.contains(option), "activity help omitted {option}");
    }
    assert!(activity.contains("from 1 through 50"));
    assert!(activity.contains("from 1 through 200"));
    assert!(activity.contains("from 1 through 100"));

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
    let message_group = stdout(&["message", "--help"]);
    for command in ["get", "render", "send"] {
        assert!(
            message_group.contains(command),
            "message help omitted {command}"
        );
    }
    let thread_group = stdout(&["thread", "--help"]);
    for command in ["read", "reply"] {
        assert!(
            thread_group.contains(command),
            "thread help omitted {command}"
        );
    }

    let list = stdout(&["conversations", "list", "--help"]);
    assert!(list.contains("from 1 through 200"));
    let find = stdout(&["conversations", "find", "--help"]);
    assert!(find.contains("from 1 through 100"));

    let drafts = stdout(&["drafts", "--help"]);
    for command in [
        "list",
        "get",
        "create",
        "create-file",
        "update",
        "delete",
        "send",
    ] {
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
    let draft_create_file = stdout(&["drafts", "create-file", "--help"]);
    for option in [
        "--path",
        "--thread-ts",
        "--broadcast",
        "--title",
        "--alt-text",
        "--max-bytes",
        "--confirm",
        "--json",
    ] {
        assert!(
            draft_create_file.contains(option),
            "draft create-file help omitted {option}"
        );
    }
    assert!(draft_create_file.contains("standard input"));
    let draft_delete = stdout(&["drafts", "delete", "--help"]);
    assert!(draft_delete.contains("--confirm"));

    let files = stdout(&["files", "--help"]);
    for command in ["info", "download", "upload"] {
        assert!(files.contains(command), "files help omitted {command}");
    }
    let download = stdout(&["files", "download", "--help"]);
    for option in ["--output", "--max-bytes", "--json"] {
        assert!(download.contains(option), "download help omitted {option}");
    }
    let upload = stdout(&["files", "upload", "--help"]);
    for option in [
        "--path",
        "--thread-ts",
        "--title",
        "--alt-text",
        "--max-bytes",
        "--confirm",
        "--json",
    ] {
        assert!(upload.contains(option), "upload help omitted {option}");
    }
    let reactions = stdout(&["reactions", "--help"]);
    for command in ["add", "remove"] {
        assert!(
            reactions.contains(command),
            "reaction help omitted {command}"
        );
        assert!(stdout(&["reactions", command, "--help"]).contains("--confirm"));
    }
    assert!(stdout(&["emoji", "list", "--help"]).contains("--json"));

    let mcp = stdout(&["mcp", "--help"]);
    assert!(mcp.contains("--allow-write"));
    assert!(mcp.contains("--file-root"));
}

#[test]
fn authoring_help_exposes_confirmed_root_reply_and_draft_publication() {
    let message = stdout(&["message", "send", "--help"]);
    assert!(message.contains("standard input"));
    assert!(message.contains("--confirm"));
    assert!(message.contains("--json"));

    let reply = stdout(&["thread", "reply", "--help"]);
    assert!(reply.contains("standard input"));
    assert!(reply.contains("--broadcast"));
    assert!(reply.contains("--confirm"));
    assert!(reply.contains("--json"));

    let draft = stdout(&["drafts", "send", "--help"]);
    assert!(draft.contains("--confirm"));
    assert!(draft.contains("--json"));
}

#[test]
fn file_draft_creation_requires_confirmation_before_stdin_or_file_io() {
    let output = run_with_credentials(&[
        "drafts",
        "create-file",
        "C123",
        "--path",
        "/definitely/missing/lurkline-file-draft.txt",
    ]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: confirmation is required for file draft creation\n"
    );
}

#[test]
fn file_draft_creation_rejects_root_broadcast_before_stdin_or_file_io() {
    let output = run_with_credentials(&[
        "drafts",
        "create-file",
        "C123",
        "--path",
        "/definitely/missing/lurkline-file-draft.txt",
        "--broadcast",
        "--confirm",
    ]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: invalid broadcast: is valid only for a thread reply\n"
    );
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

    let deeply_nested_quote = format!("{}visible\n", "> ".repeat(256));
    let nested = run_with_stdin(&["message", "render"], deeply_nested_quote.as_bytes());
    assert_eq!(nested.status.code(), Some(1), "renderer process aborted");
    assert_eq!(
        String::from_utf8(nested.stderr).unwrap(),
        "error: invalid markdown: nesting exceeds 64 levels\n"
    );

    let slack_link = run_with_stdin(
        &["message", "render"],
        b"Use <https://example.com/runbook|the runbook>.",
    );
    assert_eq!(slack_link.status.code(), Some(1));
    assert!(slack_link.stdout.is_empty());
    assert_eq!(
        String::from_utf8(slack_link.stderr).unwrap(),
        concat!(
            "error: invalid markdown: Slack-native <URL|label> link syntax is unsupported; ",
            "use standard Markdown: [label](URL)\n"
        )
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

#[test]
fn file_and_reaction_process_guards_fail_before_network_access() {
    for action in ["add", "remove"] {
        let output = run_with_credentials(&["reactions", action, "C123", "100.000001", "eyes"]);
        assert!(!output.status.success(), "{action}");
        assert!(output.stdout.is_empty(), "{action}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "error: confirmation is required for reaction mutation\n",
            "{action}"
        );
    }

    let upload = run_with_credentials(&[
        "files",
        "upload",
        "C123",
        "--path",
        "/path/that/must/not/be/opened",
    ]);
    assert!(!upload.status.success());
    assert!(upload.stdout.is_empty());
    assert_eq!(
        String::from_utf8(upload.stderr).unwrap(),
        "error: confirmation is required for file upload\n"
    );

    for (args, expected) in [
        (
            vec![
                "files",
                "upload",
                "C123",
                "--path",
                "/path/that/must/not/be/opened",
                "--thread-ts",
                "invalid",
                "--confirm",
            ],
            "error: invalid thread_ts: must be a Slack message timestamp\n",
        ),
        (
            vec![
                "files",
                "upload",
                "",
                "--path",
                "/path/that/must/not/be/opened",
                "--confirm",
            ],
            "error: invalid conversation: must be a Slack conversation ID or a 1 to 128 character name\n",
        ),
        (
            vec![
                "files",
                "upload",
                "C123",
                "--path",
                "/path/that/must/not/be/opened",
                "--title",
                "bad\ntitle",
                "--confirm",
            ],
            "error: invalid title: must contain bounded non-control text\n",
        ),
    ] {
        let output = run_with_credentials(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
    }

    let oversized_upload_path = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "lurkline-upload-limit-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&oversized_upload_path, b"12345").unwrap();
    let oversized_upload = run_with_credentials(&[
        "files",
        "upload",
        "C123",
        "--path",
        oversized_upload_path.to_str().unwrap(),
        "--max-bytes",
        "4",
        "--confirm",
    ]);
    std::fs::remove_file(&oversized_upload_path).unwrap();
    assert!(!oversized_upload.status.success());
    assert!(oversized_upload.stdout.is_empty());
    assert_eq!(
        String::from_utf8(oversized_upload.stderr).unwrap(),
        "error: local file operation failed: file exceeds the configured 4-byte limit\n"
    );

    let invalid_file = run_with_credentials(&["files", "info", "not-a-file"]);
    assert!(!invalid_file.status.success());
    assert!(invalid_file.stdout.is_empty());
    assert_eq!(
        String::from_utf8(invalid_file.stderr).unwrap(),
        "error: invalid file_id: must be a Slack file identifier\n"
    );

    let invalid_path = run_with_credentials(&[
        "files",
        "download",
        "F123",
        "--output",
        "../must-not-escape",
    ]);
    assert!(!invalid_path.status.success());
    assert!(invalid_path.stdout.is_empty());
    assert_eq!(
        String::from_utf8(invalid_path.stderr).unwrap(),
        "error: local file operation failed: invalid local path: parent path components are not allowed\n"
    );

    let oversized_leaf = "x".repeat(256);
    let oversized_path =
        run_with_credentials(&["files", "download", "F123", "--output", &oversized_leaf]);
    assert!(!oversized_path.status.success());
    assert!(oversized_path.stdout.is_empty());
    assert_eq!(
        String::from_utf8(oversized_path.stderr).unwrap(),
        "error: local file operation failed: invalid local path: path component exceeds the 255-byte limit\n"
    );

    let output_path = format!(
        "/tmp/lurkline-must-not-create-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let invalid_bound = run_with_credentials(&[
        "files",
        "download",
        "F123",
        "--output",
        &output_path,
        "--max-bytes",
        "0",
    ]);
    assert_eq!(invalid_bound.status.code(), Some(2));
    assert!(invalid_bound.stdout.is_empty());
    assert!(
        String::from_utf8(invalid_bound.stderr)
            .unwrap()
            .contains("invalid value '0' for '--max-bytes")
    );
    assert!(!std::path::Path::new(&output_path).exists());
}
