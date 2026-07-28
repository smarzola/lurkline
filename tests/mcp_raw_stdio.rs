use std::{
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    time::timeout,
};

async fn send(stdin: &mut ChildStdin, message: Value) {
    let mut encoded = serde_json::to_vec(&message).unwrap();
    encoded.push(b'\n');
    stdin.write_all(&encoded).await.unwrap();
    stdin.flush().await.unwrap();
}

async fn response_with_id(
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    expected_id: u64,
) -> Value {
    timeout(Duration::from_secs(5), async {
        loop {
            let mut line = String::new();
            let bytes = stdout.read_line(&mut line).await.unwrap();
            assert_ne!(bytes, 0, "MCP server closed before response {expected_id}");
            let value: Value = serde_json::from_str(&line).expect("stdout must be JSONL only");
            if value["id"] == expected_id {
                return value;
            }
        }
    })
    .await
    .expect("MCP response timeout")
}

async fn call_tool(
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    id: u64,
    name: &str,
    arguments: Value,
) -> Value {
    send(
        stdin,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        }),
    )
    .await;
    response_with_id(stdout, id).await
}

#[tokio::test]
async fn raw_json_rpc_initializes_lists_tools_and_returns_a_validation_error() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lurkline"))
        .arg("mcp")
        .env("SLACK_BASE_URL", "https://example.slack.com")
        .env("SLACK_TEAM_ID", "T000TEST")
        .env("SLACK_TOKEN", "xoxc-mcp-test-secret")
        .env("SLACK_COOKIE", "d=xoxd-mcp-test-secret; b=test")
        .env("LURKLINE_TIMEOUT_MS", "500")
        .env_remove("LURKLINE_MAX_RESPONSE_BYTES")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut stderr = child.stderr.take().unwrap();

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "raw-test", "version": "1.0"}
            }
        }),
    )
    .await;
    let initialized = response_with_id(&mut stdout, 1).await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "lurkline");
    assert_eq!(
        initialized["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )
    .await;
    send(
        &mut stdin,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    let tools = response_with_id(&mut stdout, 2).await;
    let names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "slack_add_reaction",
            "slack_doctor",
            "slack_create_draft",
            "slack_delete_draft",
            "slack_find_conversations",
            "slack_find_users",
            "slack_get_draft",
            "slack_get_file",
            "slack_get_message",
            "slack_list_conversations",
            "slack_list_custom_emoji",
            "slack_list_drafts",
            "slack_list_unreads",
            "slack_read_channel",
            "slack_read_inbox",
            "slack_read_thread",
            "slack_remove_reaction",
            "slack_render_markdown",
            "slack_search_messages",
            "slack_send_draft",
            "slack_send_message",
            "slack_update_draft",
        ])
    );
    for tool_name in [
        "slack_read_channel",
        "slack_read_thread",
        "slack_get_message",
    ] {
        let tool = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == tool_name)
            .unwrap();
        assert!(
            tool["inputSchema"]["properties"]["channel_id"]["description"]
                .as_str()
                .unwrap()
                .contains("force a colliding name"),
            "{tool_name} schema omits the ID/name precedence escape"
        );
    }
    let search_tool = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "slack_search_messages")
        .unwrap();
    assert!(
        search_tool["inputSchema"]["properties"]["conversation"]["description"]
            .as_str()
            .unwrap()
            .contains("force a colliding name")
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "slack_render_markdown",
                "arguments": {"markdown": "**hello**"}
            }
        }),
    )
    .await;
    let rendered = response_with_id(&mut stdout, 20).await;
    assert_eq!(rendered["result"]["isError"], false);
    assert_eq!(rendered["result"]["structuredContent"]["text"], "hello");
    assert_eq!(
        rendered["result"]["structuredContent"]["blocks"][0]["type"],
        "rich_text"
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "slack_render_markdown",
                "arguments": {
                    "markdown": format!("{}visible\n", "> ".repeat(256))
                }
            }
        }),
    )
    .await;
    let nested = response_with_id(&mut stdout, 23).await;
    assert_eq!(nested["result"]["isError"], true);
    assert_eq!(
        nested["result"]["structuredContent"],
        json!({
            "error": {
                "code": "invalid_input",
                "message": "invalid markdown: nesting exceeds 64 levels"
            }
        })
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 24,
            "method": "tools/call",
            "params": {
                "name": "slack_render_markdown",
                "arguments": {"markdown": "still alive"}
            }
        }),
    )
    .await;
    let after_nested = response_with_id(&mut stdout, 24).await;
    assert_eq!(after_nested["result"]["isError"], false);
    assert_eq!(
        after_nested["result"]["structuredContent"]["text"],
        "still alive"
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "slack_create_draft",
                "arguments": {
                    "conversation": "C123",
                    "markdown": "must not reach Slack"
                }
            }
        }),
    )
    .await;
    let write_disabled = response_with_id(&mut stdout, 21).await;
    assert_eq!(write_disabled["result"]["isError"], true);
    assert_eq!(
        write_disabled["result"]["structuredContent"],
        json!({
            "error": {
                "code": "write_not_allowed",
                "message": "Slack writes are disabled; start the MCP server with --allow-write"
            }
        })
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "slack_send_message",
                "arguments": {
                    "conversation": "C123",
                    "markdown": "must not reach Slack",
                    "confirm": true
                }
            }
        }),
    )
    .await;
    let send_disabled = response_with_id(&mut stdout, 22).await;
    assert_eq!(send_disabled["result"]["isError"], true);
    assert_eq!(
        send_disabled["result"]["structuredContent"],
        json!({
            "error": {
                "code": "write_not_allowed",
                "message": "Slack writes are disabled; start the MCP server with --allow-write"
            }
        })
    );

    for (id, name) in [(25, "slack_add_reaction"), (26, "slack_remove_reaction")] {
        let response = call_tool(
            &mut stdin,
            &mut stdout,
            id,
            name,
            json!({
                "conversation": "C123",
                "message_ts": "100.000001",
                "name": "eyes",
                "confirm": true
            }),
        )
        .await;
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["error"]["code"], "write_not_allowed",
            "{name}"
        );
    }

    let invalid_file = call_tool(
        &mut stdin,
        &mut stdout,
        28,
        "slack_get_file",
        json!({"file_id": "not-a-file"}),
    )
    .await;
    assert_eq!(invalid_file["result"]["isError"], true);
    assert_eq!(
        invalid_file["result"]["structuredContent"]["error"]["code"],
        "invalid_input"
    );

    let custom_emoji = call_tool(
        &mut stdin,
        &mut stdout,
        29,
        "slack_list_custom_emoji",
        json!({}),
    )
    .await;
    assert_eq!(custom_emoji["result"]["isError"], true);
    assert!(
        [
            "authentication",
            "http_status",
            "invalid_response",
            "timeout",
            "transport"
        ]
        .contains(
            &custom_emoji["result"]["structuredContent"]["error"]["code"]
                .as_str()
                .unwrap()
        )
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "slack_read_channel",
                "arguments": {"channel_id": "", "limit": 1}
            }
        }),
    )
    .await;
    let invalid = response_with_id(&mut stdout, 3).await;
    assert_eq!(invalid["result"]["isError"], true);
    assert_eq!(
        invalid["result"]["structuredContent"],
        json!({
            "error": {
                "code": "invalid_input",
                "message": "invalid conversation: must be a Slack conversation ID or a 1 to 128 character name"
            }
        })
    );
    assert!(
        invalid["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid conversation")
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "slack_list_conversations",
                "arguments": {"limit": 0}
            }
        }),
    )
    .await;
    let invalid_list = response_with_id(&mut stdout, 4).await;
    assert_eq!(invalid_list["result"]["isError"], true);
    assert_eq!(
        invalid_list["result"]["structuredContent"],
        json!({
            "error": {
                "code": "invalid_input",
                "message": "invalid limit: is outside the allowed range"
            }
        })
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "slack_find_conversations",
                "arguments": {"query": "", "limit": 20}
            }
        }),
    )
    .await;
    let invalid_find = response_with_id(&mut stdout, 5).await;
    assert_eq!(invalid_find["result"]["isError"], true);
    assert_eq!(
        invalid_find["result"]["structuredContent"],
        json!({
            "error": {
                "code": "invalid_input",
                "message": "invalid query: must contain 1 to 128 non-control characters"
            }
        })
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "slack_search_messages",
                "arguments": {"query": "", "limit": 20}
            }
        }),
    )
    .await;
    let invalid_search = response_with_id(&mut stdout, 6).await;
    assert_eq!(invalid_search["result"]["isError"], true);
    assert_eq!(
        invalid_search["result"]["structuredContent"],
        json!({
            "error": {
                "code": "invalid_input",
                "message": "invalid query: must contain 1 to 512 non-control characters"
            }
        })
    );

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "slack_read_inbox",
                "arguments": {"conversation_limit": 0, "message_limit": 20}
            }
        }),
    )
    .await;
    let invalid_inbox = response_with_id(&mut stdout, 7).await;
    assert_eq!(invalid_inbox["result"]["isError"], true);
    assert_eq!(
        invalid_inbox["result"]["structuredContent"],
        json!({
            "error": {
                "code": "invalid_input",
                "message": "invalid conversation_limit: is outside the allowed range"
            }
        })
    );

    drop(stdin);
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("MCP server did not stop on EOF")
        .unwrap();
    assert!(status.success());
    let mut diagnostics = String::new();
    stderr.read_to_string(&mut diagnostics).await.unwrap();
    assert!(diagnostics.is_empty(), "unexpected stderr: {diagnostics}");
}

#[tokio::test]
async fn raw_json_rpc_exposes_and_guards_enabled_file_uploads() {
    let file_root = std::env::temp_dir().join(format!(
        "lurkline-mcp-raw-upload-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&file_root).unwrap();
    let file_root = std::fs::canonicalize(file_root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lurkline"))
        .args(["mcp", "--allow-write", "--file-root"])
        .arg(&file_root)
        .env("SLACK_BASE_URL", "https://example.slack.com")
        .env("SLACK_TEAM_ID", "T000TEST")
        .env("SLACK_TOKEN", "xoxc-mcp-test-secret")
        .env("SLACK_COOKIE", "d=xoxd-mcp-test-secret; b=test")
        .env("LURKLINE_TIMEOUT_MS", "500")
        .env_remove("LURKLINE_MAX_RESPONSE_BYTES")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut stderr = child.stderr.take().unwrap();

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "raw-upload-test", "version": "1.0"}
            }
        }),
    )
    .await;
    let initialized = response_with_id(&mut stdout, 100).await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "lurkline");
    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )
    .await;
    send(
        &mut stdin,
        json!({"jsonrpc": "2.0", "id": 101, "method": "tools/list", "params": {}}),
    )
    .await;
    let tools = response_with_id(&mut stdout, 101).await;
    let upload = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "slack_upload_file")
        .expect("enabled upload tool");
    for property in [
        "conversation",
        "path",
        "thread_ts",
        "title",
        "alt_text",
        "max_bytes",
        "confirm",
    ] {
        assert!(
            upload["inputSchema"]["properties"][property].is_object(),
            "missing upload input property {property}"
        );
    }
    assert!(
        upload["inputSchema"]["properties"]["path"]["description"]
            .as_str()
            .unwrap()
            .contains("Relative regular file")
    );
    let output_schema = serde_json::to_string(&upload["outputSchema"]).unwrap();
    assert!(output_schema.contains("allocation_uncertain"));
    assert!(output_schema.contains("transfer_uncertain"));
    assert!(output_schema.contains("completion_uncertain"));
    assert!(output_schema.contains("shared"));
    assert_eq!(upload["annotations"]["readOnlyHint"], false);
    assert_eq!(upload["annotations"]["destructiveHint"], false);
    assert_eq!(upload["annotations"]["idempotentHint"], false);
    assert_eq!(upload["annotations"]["openWorldHint"], true);

    let unconfirmed = call_tool(
        &mut stdin,
        &mut stdout,
        102,
        "slack_upload_file",
        json!({
            "conversation": "C123",
            "path": "missing.txt",
            "confirm": false
        }),
    )
    .await;
    assert_eq!(unconfirmed["result"]["isError"], true);
    assert_eq!(
        unconfirmed["result"]["structuredContent"]["error"]["code"],
        "confirmation_required"
    );

    let unsafe_path = call_tool(
        &mut stdin,
        &mut stdout,
        103,
        "slack_upload_file",
        json!({
            "conversation": "C123",
            "path": "../outside.txt",
            "confirm": true
        }),
    )
    .await;
    assert_eq!(unsafe_path["result"]["isError"], true);
    assert_eq!(
        unsafe_path["result"]["structuredContent"]["error"]["code"],
        "local_file"
    );

    drop(stdin);
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("MCP server did not stop on EOF")
        .unwrap();
    assert!(status.success());
    let mut diagnostics = String::new();
    stderr.read_to_string(&mut diagnostics).await.unwrap();
    assert!(diagnostics.is_empty(), "unexpected stderr: {diagnostics}");
    std::fs::remove_dir(&file_root).unwrap();
}
