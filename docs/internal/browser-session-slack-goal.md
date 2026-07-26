# Lurkline Browser-Session Slack Goal

## Mission

Deliver `lurkline`, a single Rust binary that gives humans and local agents useful,
read-only Slack access through credentials copied out of an already-authenticated
browser session. It must not require a Slack app, bot, OAuth installation, browser
extension, daemon, or database.

The final repository lives at `/Users/smarzola/projects/lurkline` and is pushed to a
private GitHub repository named `smarzola/lurkline`. Private is the conservative
default because repository visibility was not specified.

## Starting State

- Staging repository: `/Users/smarzola/Documents/Codex/2026-07-26/use-browser-to-read-sferait-ws/lurkline`
- Final repository: `/Users/smarzola/projects/lurkline`
- Baseline commit: `4e3f73a8c06e97aadd67c395cac176e43da6e713`
- Goal branch: `feat/browser-session-slack`
- Initial remote: none
- Rust toolchain: stable Rust 1.97.1, edition 2024
- Reference evidence: private sanitized HARs outside this repository; they must
  never be copied into Git.

## Target State

One `lurkline` executable provides:

- a human-friendly CLI;
- stable JSON output suitable for scripts;
- a stdio MCP server exposing the same bounded, typed, read-only operations;
- a shared service layer, so CLI and MCP semantics cannot drift;
- Slack web-session HTTP access using `SLACK_BASE_URL`, `SLACK_TEAM_ID`,
  `SLACK_TOKEN`, and `SLACK_COOKIE`;
- useful authentication diagnostics without exposing secrets;
- exact unread channel/DM state from Slack's `client.counts` response;
- bounded channel history, thread replies, individual-message lookup, and user
  search;
- synthetic tests and documentation sufficient for a new user to install,
  configure, validate, and connect the MCP server.

## Scope

### In scope

- Private Slack Web API requests authenticated with a browser-session `xoxc`
  token plus cookie header.
- Read-only methods needed by the commands and MCP tools below.
- Multipart form requests carrying the token, compatible browser request
  metadata, timeouts, response-size limits, and redacted errors.
- Credential extraction instructions based on browser DevTools "Copy as cURL".
- Linux/macOS-compatible Rust source and local macOS verification.
- A private GitHub repository, commits, push, and remote read-back verification.

### Out of scope

- Slack apps, bot tokens, OAuth, socket mode, or the official Slack MCP.
- Sending or editing messages, reactions, uploads, deletions, or marking content
  read.
- Automated cookie extraction, browser control, credential refresh, background
  daemons, or credential persistence.
- Committing real credentials, HARs, workspace identifiers, user data, or message
  content.
- Claiming compatibility with Slack's unsupported private endpoints beyond the
  behavior verified by synthetic protocol tests and the captured response shapes.

Simple is better than complex, but simple must still be complete. Prefer a small
typed HTTP/service/adapter design over speculative abstractions, without omitting
required validation, boundedness, or secret handling.

## User-Facing Contract

### Environment

- `SLACK_BASE_URL`: required `https://<workspace>.slack.com` origin.
- `SLACK_TEAM_ID`: required Slack team/workspace ID.
- `SLACK_TOKEN`: required browser-session token, normally beginning `xoxc-`.
- `SLACK_COOKIE`: required full browser `Cookie` header, including the `d=xoxd-…`
  session cookie.
- `LURKLINE_TIMEOUT_MS`: optional request timeout, bounded to a safe range.
- `LURKLINE_MAX_RESPONSE_BYTES`: optional response limit, bounded to a safe range.

No command may echo, serialize, log, or include these secret values in an error.

### CLI

- `lurkline doctor [--json]`
- `lurkline unreads [--json]`
- `lurkline channel read <channel-id> [--limit <1..200>] [--json]`
- `lurkline thread read <channel-id> <thread-ts> [--limit <1..200>] [--json]`
- `lurkline message get <channel-id> <message-ts> [--json]`
- `lurkline users find <query> [--limit <1..100>] [--json]`
- `lurkline mcp`

Channel IDs are always accepted. Names may be displayed when Slack returns them,
but the implementation must not require sidebar scraping or local state.

### MCP tools

- `slack_doctor`
- `slack_list_unreads`
- `slack_read_channel`
- `slack_read_thread`
- `slack_get_message`
- `slack_find_users`

Every collection request has an enforced maximum. Every result is structured JSON.
MCP protocol output stays on stdout; diagnostics stay on stderr.

### Failures

- Missing or malformed configuration returns actionable names-only diagnostics.
- Login HTML, redirects to authentication, `invalid_auth`, `not_authed`, and
  non-JSON responses are reported as session-expired/authentication failures.
- Slack API errors preserve the non-secret Slack error code.
- Oversized responses, timeouts, and transport failures are distinct.
- Message and user text is returned only as requested; it is never logged.

## Milestones

### Milestone 1 — Safe transport, configuration, and unread core

Deliver:

- bounded, secret-safe environment configuration;
- a private HTTP client for multipart Slack calls;
- normalized models for `client.counts`;
- shared service operations for `doctor` and unread listing;
- CLI `doctor` and `unreads`, including `--json`;
- synthetic request/response tests for auth failures, response limits, and unread
  parsing.

Verification:

- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- reviewer reports no blocking scope, secret-handling, or protocol defects.

### Milestone 2 — Complete read operations

Deliver:

- bounded channel history, thread replies, exact message lookup, and user search;
- CLI commands and stable normalized JSON for all read operations;
- human output that is concise and does not expose configuration;
- synthetic fixtures covering pagination/limits, empty results, Slack errors, and
  malformed responses.

Verification:

- milestone 1 verification remains green;
- focused CLI parser and service tests pass;
- reviewer reports the CLI contract and error semantics are complete.

### Milestone 3 — MCP parity, operator docs, and publishable artifact

Deliver:

- stdio MCP server with tools matching the CLI service operations;
- raw JSON-RPC initialize/list-tools/tool-call smoke coverage;
- installation, DevTools credential setup, security caveats, CLI examples, MCP
  configuration, and credential-expiry recovery documentation;
- repository metadata suitable for GitHub;
- release-mode build from a locked dependency graph.

Verification:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo build --release --locked`
- CLI help and raw stdio MCP smoke checks;
- persistent reviewer finds no blocking issue;
- a fresh context-independent auditor reports `CLEAN`.

## Definition of Done

- [ ] All target CLI commands exist and share one typed service layer.
- [ ] All target MCP tools exist and return structured, bounded results.
- [ ] No write-capable Slack method or automatic credential extraction exists.
- [ ] Real secrets, HARs, workspace data, and user data are absent from Git history.
- [ ] Configuration and runtime errors cannot reveal token or cookie values.
- [ ] Synthetic tests exercise request shape, parsing, bounds, authentication
      expiry, non-JSON, oversized responses, and Slack API errors.
- [ ] Formatting, lint, all tests, locked release build, CLI smoke, and raw MCP
      smoke pass from the final path.
- [ ] README documents installation, configuration, security model, commands, MCP
      setup, limitations, and recovery from expired credentials.
- [ ] Goal branch has Conventional Commit checkpoints and an adversarial review
      trail.
- [ ] Final audited branch is fast-forwarded into local `main`.
- [ ] `/Users/smarzola/projects/lurkline` exists and is the verified Git root.
- [ ] Private GitHub repository `smarzola/lurkline` exists with pushed `main`.
- [ ] Remote URL, default branch, visibility, and commit SHA are read back.

## Required Final Verification

Run from `/Users/smarzola/projects/lurkline`:

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --locked
./target/release/lurkline --help
./target/release/lurkline mcp
git status --short --branch
git rev-parse --show-toplevel
git remote -v
git rev-parse HEAD
gh repo view smarzola/lurkline --json nameWithOwner,visibility,defaultBranchRef,url
gh api repos/smarzola/lurkline/commits/main --jq .sha
```

The MCP command is exercised with a bounded raw JSON-RPC initialize, initialized,
tools/list, and one validation-error tool call rather than left running.

## Progress Ledger

- [x] 2026-07-26: selected globally unused name `lurkline`.
- [x] 2026-07-26: created staging repository and typed goal branch from baseline.
- [x] 2026-07-26: Milestone 1 implemented, reviewed, and verified (13 tests).
- [ ] Milestone 2 implemented, reviewed, verified, and committed.
- [ ] Milestone 3 implemented, reviewed, verified, and committed.
- [ ] Independent final audit passed.
- [ ] Final path, local main, GitHub repository, push, and read-back verified.

## Decision Log

- 2026-07-26: Use a single Rust binary and shared service layer to keep CLI and MCP
  behavior consistent.
- 2026-07-26: Treat credentials as out-of-band environment configuration; do not
  implement browser automation or persistence.
- 2026-07-26: Keep v1 strictly read-only.
- 2026-07-26: Use a private GitHub repository unless the user later asks for public
  visibility.
- 2026-07-26: Develop in the writable staging directory, then move the audited Git
  repository to the required durable projects directory and re-run verification.

## Review Log

Record reviewer readiness, milestone findings, repairs, re-checks, and the final
independent audit here before marking the goal complete.

- 2026-07-26 readiness: `READY`; reviewer locked real `doctor` probing, explicit
  Slack unread flags, stable JSON fixtures, redirect handling, streaming bounds,
  and secret non-disclosure into the review criteria.
- 2026-07-26 Milestone 1 review: blocking arbitrary-origin credential egress;
  high non-JSON/login-string misclassification; medium untyped thread-count JSON.
- 2026-07-26 Milestone 1 repair: production origins restricted to root HTTPS
  single-label `*.slack.com`; JSON parsed before envelope classification;
  arbitrary non-JSON treated as authentication failure; thread counts typed as
  `BTreeMap<String, u64>` with an exact serialized-schema regression.
- 2026-07-26 Milestone 1 re-check: `CLEAN`.
