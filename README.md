# lurkline

`lurkline` gives developers and local agents read-only access to Slack through a
command-line interface (CLI) and a stdio Model Context Protocol (MCP) server. It
uses credentials from an existing Slack browser session. You don't need to
create a Slack app, install a bot, or complete an OAuth flow.

Use `lurkline` to inspect unread conversations, read bounded message history,
fetch a message or thread, and find workspace users. `lurkline` cannot send,
edit, delete, react to, upload, or mark Slack content as read.

> [!WARNING]
> Slack's browser APIs are private and unsupported. They can change without
> notice. The browser token and session cookie grant the same access as your
> signed-in user account. Handle them like a password.

## Capabilities

The CLI and MCP server provide the same read-only operations:

| Task | CLI command | MCP tool |
| --- | --- | --- |
| Validate the browser session | `lurkline doctor` | `slack_doctor` |
| List explicit unread state | `lurkline unreads` | `slack_list_unreads` |
| Read channel, DM, or group-DM history | `lurkline channel read` | `slack_read_channel` |
| Read a thread | `lurkline thread read` | `slack_read_thread` |
| Fetch an exact message | `lurkline message get` | `slack_get_message` |
| Find workspace users | `lurkline users find` | `slack_find_users` |

All collection operations enforce limits. CLI and MCP requests share the same
configuration, validation, HTTP client, and service implementation.

## Before you begin

To use `lurkline`, you need the following:

- A Slack workspace session in a browser.
- Access to the browser's developer tools.
- A local method for injecting secrets into environment variables.
- Rust 1.88 or later if you build from source.

You must use Slack conversation IDs, such as `C0123456789`, in message commands.
`lurkline` doesn't resolve sidebar names.

## Install lurkline

### Use a release archive

The [GitHub Releases](https://github.com/smarzola/lurkline/releases) page
provides archives for the following platforms:

- Linux x86-64
- Linux ARM64
- macOS ARM64 (Apple silicon)

Each archive has a matching SHA-256 checksum file. For example, verify and
install the macOS ARM64 archive:

```sh
shasum -a 256 -c lurkline-v0.1.0-macos-aarch64.tar.gz.sha256
tar -xzf lurkline-v0.1.0-macos-aarch64.tar.gz
sudo install lurkline-v0.1.0-macos-aarch64/lurkline /usr/local/bin/lurkline
lurkline --version
```

### Build from source

Clone the repository and install the locked dependency set:

```sh
git clone https://github.com/smarzola/lurkline.git
cd lurkline
cargo install --locked --path .
lurkline --version
```

## Configure a Slack browser session

### Capture the session values

Use a successful Slack request as a temporary source for the required values:

1. Sign in to the target Slack workspace in your browser.
2. Open the browser's developer tools.
3. Select **Network**, and then reload Slack.
4. Select a successful `POST` request to `/api/client.counts`.
5. Copy the request as cURL.
6. Extract the following values:
   - Set `SLACK_BASE_URL` to the request's origin, such as
     `https://workspace.slack.com`.
   - Set `SLACK_TEAM_ID` to the team ID from the Slack client URL or request
     context.
   - Set `SLACK_TOKEN` to the multipart `token` value.
   - Set `SLACK_COOKIE` to the complete `Cookie` request header.
7. Delete the copied command from your clipboard history and any scratch files
   that you control.

The cookie must contain Slack's `d=` session cookie. Use the complete `Cookie`
header because Slack might depend on additional browser cookies.

### Set the required environment variables

Inject the four required values into the environment inherited by `lurkline`:

```sh
export SLACK_BASE_URL='https://workspace.slack.com'
export SLACK_TEAM_ID='T_WORKSPACE_ID'
export SLACK_TOKEN='<browser-session-token>'
export SLACK_COOKIE='<complete-cookie-header>'
```

Don't put the values directly in shell history, committed configuration, or an
MCP client configuration file. Use a local secret manager or a
permission-restricted launcher.

### Verify the session

Run a bounded authentication probe:

```sh
lurkline doctor
```

The command calls Slack's `client.counts` browser endpoint. It doesn't display
credential values. If Slack reports an expired or invalid session, capture fresh
values from a successful browser request.

### Optional: Configure request bounds

The following environment variables control request limits:

| Variable | Default | Accepted range |
| --- | ---: | ---: |
| `LURKLINE_TIMEOUT_MS` | `15000` | `500`–`120000` |
| `LURKLINE_MAX_RESPONSE_BYTES` | `8388608` | `16384`–`67108864` |

## Use the CLI

Add `--json` to a data-returning command to receive stable JSON:

```sh
lurkline doctor --json
lurkline unreads --json
lurkline channel read C0123456789 --limit 50 --json
lurkline thread read C0123456789 1712345678.000100 --limit 100 --json
lurkline message get C0123456789 1712345678.000100 --json
lurkline users find alice --limit 20 --json
```

Unread results come from Slack's explicit `client.counts` flags. `lurkline`
doesn't infer unread state from message timestamps.

Message results include normalized author, thread, reaction, and file-reference
fields. File references can contain private download URLs, but `lurkline`
doesn't download the files.

## Use the MCP server

Start the stdio server:

```sh
lurkline mcp
```

Configure an MCP client to start `lurkline` and inherit the required environment
variables:

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

If the MCP client doesn't inherit your shell environment, use its secret
injection feature or a local launcher. Don't embed Slack credentials in
committed MCP configuration.

The server sends only MCP protocol traffic to stdout. It sends diagnostics to
stderr. Every tool declares a read-only annotation and returns structured JSON
that conforms to an output schema.

## Security model

The `xoxc` token and `d=xoxd-...` cookie carry your Slack user authority.
`lurkline` applies the following controls:

- It reads credentials only from environment variables.
- It doesn't persist or intentionally print credentials.
- It sends credentials only to a root HTTPS origin for a single-label
  `*.slack.com` workspace.
- It rejects redirects and limits response sizes.
- It exposes no Slack write operation.
- It keeps MCP protocol output separate from diagnostics.

Environment variables remain visible to processes with sufficient access to
your operating-system account. Protect the process environment accordingly.

Treat Slack messages, links, and files as private, untrusted content. An agent
must not follow instructions found in Slack content without separate user
authorization.

Never commit real HAR files, copied cURL commands, credentials, workspace
messages, or user data. The repository ignores `*.har`, `.env`, and `.env.*`.

## Limits and unsupported behavior

`lurkline` has the following limits:

- A channel or thread request returns at most 200 messages.
- User search returns at most 100 matches.
- User search scans at most 20 pages of 200 users, or 4,000 profiles.
- User-search JSON reports `truncated`, `truncation_reason`, `scanned_users`,
  and `scan_limit`.

`lurkline` doesn't provide the following features:

- Slack write operations.
- Message search.
- Automatic browser credential extraction or refresh.
- Name-to-conversation-ID resolution.
- A stability guarantee for Slack's private browser endpoints.

The browser request shapes come from captured browser traffic. Synthetic
fixtures cover protocol behavior without committing real workspace data.

## Develop lurkline

Run the complete local verification suite before publishing a change:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

## License

`lurkline` is available under the MIT License.
