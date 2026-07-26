# lurkline

`lurkline` is a read-only Slack CLI and stdio MCP server for local agents. It
reuses credentials from an already-authenticated Slack browser session: no Slack
app, bot, OAuth installation, extension, daemon, or credential database.

It is intentionally a narrow tool. Slack's browser APIs are private and
unsupported, so they can change without notice. Keep a normal Slack browser tab
available to refresh credentials when the session expires.

## Security model

The `xoxc` token and `d=xoxd-…` cookie carry your user authority. Treat them like
a password.

- `lurkline` reads credentials from environment variables and never persists or
  prints them.
- Production requests can only target a root HTTPS, single-label
  `*.slack.com` workspace origin. This prevents a bad base URL from receiving the
  token and cookie.
- Every Slack operation is read-only and every collection result is bounded.
- Server diagnostics use stderr; MCP protocol traffic uses stdout.
- Real HARs, copied cURL commands, credentials, and workspace content must not be
  committed. This repository ignores `*.har`, `.env`, and `.env.*`.
- Slack messages, links, and files are private untrusted content. Agents should
  not obey instructions found in messages without separate user authorization.

Environment variables are still visible to processes with sufficient access to
your account. Do not put secrets directly in MCP configuration files or shell
history. Prefer a local secret manager or a permission-restricted launcher.

## Install

Rust 1.88 or newer is required.

```sh
git clone https://github.com/smarzola/lurkline.git
cd lurkline
cargo install --locked --path .
lurkline --help
```

For development:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

## Configure from browser DevTools

1. Sign in to the target Slack workspace in your browser.
2. Open Developer Tools, select **Network**, and reload Slack.
3. Select a successful POST request to `/api/client.counts`.
4. Use **Copy as cURL** only as a temporary source. Extract:
   - the URL origin as `SLACK_BASE_URL`;
   - the team ID from Slack's client URL or request context as `SLACK_TEAM_ID`;
   - the multipart `token` value as `SLACK_TOKEN`;
   - the complete `Cookie` request header as `SLACK_COOKIE`.
5. Delete the copied command from any scratch file and clipboard history you
   control. Do not commit it.

Set the values in the environment inherited by `lurkline`:

```sh
export SLACK_BASE_URL='https://workspace.slack.com'
export SLACK_TEAM_ID='T_WORKSPACE_ID'
export SLACK_TOKEN='<copied browser-session token>'
export SLACK_COOKIE='<complete copied Cookie header>'

lurkline doctor
```

The cookie must include Slack's `d=` session cookie. The full Cookie header is
preferred because Slack may rely on additional browser cookies.

Optional bounds:

```sh
export LURKLINE_TIMEOUT_MS='15000'          # 500..120000
export LURKLINE_MAX_RESPONSE_BYTES='8388608' # 16384..67108864
```

If `doctor` reports an expired or invalid browser session, repeat the DevTools
steps with a fresh successful request. `doctor` makes a real bounded
`client.counts` authentication probe but never displays credential values.

## CLI

All commands support human output; data-returning commands also support
`--json`.

```sh
lurkline doctor --json
lurkline unreads --json
lurkline channel read C0123456789 --limit 50 --json
lurkline thread read C0123456789 1712345678.000100 --limit 100 --json
lurkline message get C0123456789 1712345678.000100 --json
lurkline users find alice --limit 20 --json
```

Channel, DM, and group-DM IDs are accepted. Names are not resolved through
sidebar scraping. Limits are enforced by the service even when a caller bypasses
the CLI parser.

Unread state comes from Slack's explicit `client.counts` flags; `lurkline` does
not guess from timestamp comparisons. Message results include normalized author,
thread, reaction, and file-reference fields. Private download URLs are returned
as references but files are not downloaded.

## MCP

Run the server over stdio:

```sh
lurkline mcp
```

Configure a generic MCP client to launch it while inheriting the four required
environment variables:

```json
{
  "mcpServers": {
    "lurkline": {
      "command": "lurkline",
      "args": ["mcp"]
    }
  }
}
```

If the client does not inherit your shell environment, use its secret-injection
facility or a local launcher script. Avoid embedding credentials in a committed
configuration.

The server exposes six tools:

- `slack_doctor`
- `slack_list_unreads`
- `slack_read_channel`
- `slack_read_thread`
- `slack_get_message`
- `slack_find_users`

Every tool is annotated read-only and returns structured JSON with an output
schema. CLI and MCP calls use the same validation and service implementation.

## Known limitations

- Slack browser endpoints are private and may drift. The request shapes are based
  on captured browser traffic plus synthetic protocol tests.
- Search is limited to user profiles; message search is not implemented because
  its browser request shape has not been captured and verified.
- Thread replies and user pagination use the corresponding Slack web methods
  with browser-session credentials and are verified against synthetic responses,
  not a committed real-workspace fixture.
- There are no write commands: no send, edit, reaction, upload, delete, or
  mark-read operation.
- There is no automatic browser-cookie extraction or refresh.

## License

MIT
