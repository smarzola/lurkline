# Goal: Resolve Slack Message Authors

Work in
`/Users/smarzola/Documents/Codex/2026-07-26/use-browser-to-read-sferait-ws/lurkline-v050`.

Make Lurkline's normal message-navigation output identify known Slack users by
username while preserving stable author IDs and truthful resolution metadata
for agents. Apply at most one bounded user-directory lookup across conversation
routing and author enrichment for each operation, never one lookup per message,
and preserve already-addressable message reads when auxiliary identity
resolution is incomplete or unavailable.

Source of truth:
[GitHub issue #12](https://github.com/smarzola/lurkline/issues/12) and the
accepted implementation scope in the current Codex task.

## Target State

When this goal is complete:

- Channel, thread, exact-message, and message-search reads resolve known user
  IDs through Lurkline's existing bounded user directory.
- Name-based conversation routing reuses its already-loaded user directory for
  author enrichment instead of starting a second scan.
- Human output renders `@username` when available and labels raw-ID fallbacks
  as unresolved, incomplete, or unavailable instead of presenting opaque IDs
  as if they were names.
- JSON and MCP results retain `author_id`, expose resolved username and display
  name when available, and carry explicit per-message resolution state.
- Directly supplied Slack author names, including bot names, remain usable
  without an unnecessary directory scan.

## Current-State Evidence

Verified before implementation:

- Starting branch: `feat/resolve-message-authors`.
- Starting and goal base commit:
  `43c343af7e426e215e9c204814fba49c101cd59a`, identical to `origin/main` and
  tagged `v0.8.2`.
- `src/cli.rs::print_messages` and `format_search_match` prefer `author_id`
  before `author_name`, so normal output shows an opaque ID even when Slack
  supplied a name.
- `src/service.rs::normalize_message` and `normalize_search_matches` populate
  `author_name` only from the message payload's optional `username`; ordinary
  user messages commonly omit that field.
- `src/service.rs::load_user_directory` already retrieves at most 20 pages of
  200 users, deduplicates cursors, validates user IDs, and reports whether the
  directory is complete.
- `src/service.rs::resolve_named_conversation` currently loads that directory
  before resolving any non-ID conversation reference, but discards it before
  the message result is normalized.
- `src/model.rs::Message` and `MessageSearchMatch` expose `author_id` and
  nullable `author_name`, but no display name or resolution status.
- MCP read tools return the same typed service models as the CLI.

Unknowns affecting implementation details but not the target state:

- Slack may omit or return unusable names for deleted, app, or system authors.
  Preserve a bounded escaped raw-ID fallback and report its resolution state;
  do not invent an identity.

## Constraints And Authorization

Follow `AGENTS.md`.

- Keep CLI and MCP behavior backed by the same typed service layer.
- Preserve existing message text, Block Kit, attachments, thread, reaction,
  file, pagination, and stable-ID behavior.
- Treat user-directory lookup as auxiliary enrichment: a lookup failure must
  not make an otherwise valid, already-addressable message read fail. A
  directory failure needed to resolve a caller-supplied conversation name or
  DM participant remains a routing failure because no conversation ID is yet
  available.
- Reuse a directory already loaded while resolving the conversation reference.
  Otherwise use the existing bounded `users.list` scan for author enrichment.
  Perform no per-message network loop, add no cache or dependency, and start no
  auxiliary scan when every returned author is already named or absent.
- Accept only bounded non-control usernames and display names in enriched
  output. Keep terminal escaping for all human-readable fields.
- Use synthetic identities and messages in tests and documentation. Never
  commit, log, snapshot, or print real Slack credentials, workspace content,
  or user data.
- Keep structured changes additive. Preserve `author_id` and the existing
  meaning of `author_name` as the Slack username/name when known.
- Implement the smallest coherent complete design consistent with existing
  service and model patterns. Do not add generalized identity services,
  background work, persistence, configuration, or adjacent Slack features.
- Preserve unrelated user changes. The baseline is clean.

This goal authorizes repository inspection, in-scope local edits, focused
Conventional Commit checkpoints, non-destructive verification, and reviewer
subagents. It does not authorize pushing, opening or merging a pull request,
publishing, tagging, releasing, live Slack access, credential changes, or
destructive actions.

Continue through routine implementation choices using repository evidence.
Ask only when an ambiguity materially changes user-visible behavior,
architecture, data compatibility, security posture, or authorization. Exhaust
safe in-scope alternatives before declaring a blocker, then report concrete
evidence without claiming completion.

## Success Criteria

The goal is complete only when:

1. Known directory users are resolved with at most one user-directory scan
   across routing and enrichment for each channel, thread, exact-message, or
   search operation, with username and display name in typed JSON/MCP output.
2. Human message and search output prefers escaped `@username`; display-only
   identities remain readable; unresolved IDs explicitly distinguish complete,
   incomplete, and unavailable resolution.
3. Direct Slack author names require no auxiliary directory lookup, and an
   auxiliary lookup failure preserves the primary message result; directory
   failures required to resolve a conversation reference remain routing
   failures.
4. Directory pagination remains bounded and cursor-safe, partial results can
   still resolve users already scanned, and no operation performs a
   per-message lookup.
5. Existing message data and pagination behavior remain compatible, MCP output
   schemas expose the additive identity fields, and README behavior is
   task-focused and current.
6. Synthetic unit, CLI, and raw MCP tests cover resolved, direct, authorless,
   unresolved, partial, unavailable, display-only, and
   control-character-safe behavior, including directory-call counts for named
   channels, named DMs, and filtered search.
7. Every milestone has passing verification, a clean retained-reviewer result,
   a checked status note, and a focused Conventional Commit.
8. Final formatting, strict locked Clippy, all locked tests, release build,
   credential scan, and diff checks pass, followed by a clean fresh
   context-independent audit.

## Milestones

- [x] Milestone 1: Deliver bounded typed author resolution.
- [x] Milestone 2: Deliver ergonomic CLI/MCP behavior and operator guidance.

### Checkpoint Protocol

For each milestone:

1. Meet its acceptance criteria and run the listed verification.
2. Freeze writes and obtain a clean review from the retained adversarial
   reviewer; repair and re-review until no blocking finding remains.
3. Mark the milestone `[x]` and add a dated status note with exact commands and
   results.
4. Commit implementation, tests, documentation, and this goal update together
   with a focused Conventional Commit.
5. Report the resulting hash before starting the next milestone.

Do not mark or commit a failed milestone. Diagnose and repair in-scope failures
instead of weakening tests.

## Milestone 1: Bounded Typed Author Resolution

Acceptance criteria:

- Typed message and search results preserve `author_id` and expose bounded
  username, display name, and explicit resolution state.
- Channel, thread, exact-message, and search services reuse any user directory
  needed for reference routing and perform at most one bounded directory scan
  across the whole operation.
- Directly named authors and authorless messages skip an auxiliary directory
  scan; Slack-shaped references therefore require no directory at all for
  those results.
- Complete misses, scan-limit misses, and lookup failures are distinguishable;
  auxiliary lookup failures do not discard otherwise valid messages, while
  routing failures retain their existing errors.
- Synthetic service tests cover batching, bounds, fallback states, existing
  message/pagination behavior, and exact directory-call counts for named
  channel, named DM, and search-with-conversation paths.

Likely touchpoints: `src/model.rs`, `src/service.rs`, and synthetic unit tests.

Verification:

```bash
cargo fmt --all -- --check
cargo test --locked service::tests
cargo clippy --locked --all-targets -- -D warnings
```

Status: Complete (2026-07-29). Typed message and search results now preserve
stable IDs and expose bounded names, display names, and `provided`,
`directory`, `not_attempted`, `unresolved`, `incomplete`, `unavailable`, or
`unknown` resolution state. `not_attempted` keeps inbox and write
acknowledgements truthful without adding auxiliary reads. Name-based routing
reuses its directory; Slack-shaped routes scan only when unresolved IDs need
enrichment. Unusable supplied names become absent and take the normal fallback
path. Auxiliary directory errors preserve the message as `unavailable`, while
name-routing errors retain their existing failure behavior.

Verification passed `cargo fmt --all -- --check`,
`cargo test --locked service::tests` (116 passed),
`cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check`.
The retained reviewer found two blocking truthfulness defects: unusable
supplied names discarded primary messages, and unenriched inbox/send results
claimed a complete miss. Repairs made unusable names fall back safely and
introduced `not_attempted`; re-review reported no blocking findings.

## Milestone 2: Ergonomic CLI/MCP Behavior And Guidance

Acceptance criteria:

- Human channel, thread, exact-message, and search output prefers
  `@username`, supports a display-only identity, and marks every raw-ID
  fallback truthfully.
- CLI and raw MCP coverage proves escaping and the additive structured schema.
- README guidance explains resolved author fields and fallbacks without
  implementation-history prose.
- Full repository gates pass without real Slack data.

Likely touchpoints: `src/cli.rs`, `src/mcp.rs`, `tests/cli_process.rs`,
`tests/mcp_raw_stdio.rs`, and `README.md`.

Verification:

```bash
cargo fmt --all -- --check
cargo test --locked cli::tests
cargo test --locked --test cli_process
cargo test --locked --test mcp_raw_stdio
python3 scripts/check-no-secrets.py
git diff --check
```

Status: Complete (2026-07-29). Channel, thread, exact-message, inbox, and
search rows share one terminal-safe author formatter. It prefers `@username`,
then a display name, and otherwise appends a truthful resolution label to the
stable Slack ID. Raw MCP schema coverage checks the additive fields and every
resolution state across read, inbox, search, and sent-message results. README
guidance documents the human-to-structured state mapping and bounded lookup
contract.

Verification passed `cargo fmt --all -- --check`,
`cargo test --locked cli::tests` (12 passed),
`cargo test --locked --test cli_process` (12 passed),
`cargo test --locked --test mcp_raw_stdio` (2 passed),
`cargo clippy --locked --all-targets -- -D warnings`,
`python3 scripts/check-no-secrets.py`, and `git diff --check`. The retained
reviewer found one documentation mismatch between human labels and serialized
enum values plus an overstated scan count. The repair maps every value
explicitly and documents the 20-page, up-to-4,000-user bound; re-review reported
no blocking findings.

## Final Verification

Run from
`/Users/smarzola/Documents/Codex/2026-07-26/use-browser-to-read-sferait-ws/lurkline-v050`:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
target/release/lurkline --version
python3 scripts/check-no-secrets.py
git diff --check
git status --short
```

Inspect every failure and fix in-scope regressions rather than weakening tests.
Document an unrelated pre-existing failure only with command output and
evidence that this branch did not cause it.

## Resume Protocol

On resume, first read this prompt, `AGENTS.md`, `git status`, milestone status
notes, and recent commits. Verify completed checkpoints and continue from the
first unchecked milestone without redoing completed work. New evidence may
refine implementation details but must not weaken the target state or success
criteria silently.

## Final Report

Lead with `Achieved` or `Not achieved`, then report:

- target state and success-criteria status;
- milestone checkpoint commits;
- files changed;
- exact verification commands and results;
- reviewer rounds and dispositions;
- residual risks, follow-ups, and unauthorized external delivery remaining.
