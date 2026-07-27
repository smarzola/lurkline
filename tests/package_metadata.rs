#[test]
fn packaged_mcp_metadata_matches_the_crate_and_safe_defaults() {
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("../packaging/mcp/server.json")).unwrap();
    assert_eq!(metadata["name"], env!("CARGO_PKG_NAME"));
    assert_eq!(metadata["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(metadata["transport"]["command"], "lurkline");
    assert_eq!(metadata["transport"]["args"], serde_json::json!(["mcp"]));
    assert_eq!(metadata["policy"], "read-only-by-default");
    assert_eq!(metadata["required_env"], serde_json::json!([]));
    assert_eq!(
        metadata["optional_env"],
        serde_json::json!([
            "LURKLINE_PROFILE",
            "SLACK_BASE_URL",
            "SLACK_TEAM_ID",
            "SLACK_TOKEN",
            "SLACK_COOKIE",
            "LURKLINE_TIMEOUT_MS",
            "LURKLINE_MAX_RESPONSE_BYTES"
        ])
    );
}
