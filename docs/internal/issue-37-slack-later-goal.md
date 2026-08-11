# Goal: Deliver A First-Class Slack Later Inbox

Work in `/Users/smarzola/projects/lurkline`.

Resolve GitHub issue
[#37](https://github.com/smarzola/lurkline/issues/37) as one focused enhancement
release. A caller should be able to review Slack's personal Later list as a
bounded, resumable inbox, understand each saved message without opening Slack,
and deliberately save, complete, or remove an item through the same guarded
CLI and MCP service behavior.

Source of truth: issue #37, read from GitHub on 2026-08-11.

## Target State

When this goal is complete:

- `lurkline later list` and its MCP equivalent page the signed-in user's
  in-progress, completed, or archived Later items with useful defaults,
  deterministic order, an opaque continuation cursor, aggregate counts, and
  explicit truncation state.
- Each item exposes its stable conversation/message identity, Later state and
  timestamps, reminder metadata when Slack supplies it, hydrated message and
  thread context, author/conversation metadata, attached files, and canonical
  message permalink using existing typed read models and bounded enrichment.
- Saving a message, marking it complete, and removing it use explicit
  subcommands and MCP tools, require the existing write gate plus per-action
  confirmation, and return exact post-write reconciliation rather than trusting
  a browser API acknowledgement alone.
- A file saved through Slack is represented truthfully as its containing
  message with that message's file attachments. Lurkline does not invent a
  separate file-level Later identity that the observed Slack contract does not
  provide.
- CLI help, human output, JSON, MCP schemas, and focused documentation explain
  states, pagination, reminders, confirmations, reconciliation, and the
  provisional browser-API boundary.
- `main` contains one squash-merged pull request for #37 and release `v0.17.0`
  is published and independently verified.

## Current-State Evidence

Verified before implementation:

- Starting `main` was clean at
  `881c52ca432b6e346f2b3de08325f03e51a159a2`, identical to `origin/main` after
  fetch and fast-forward pull. Branch `feat/issue-37-slack-later` starts at
  that exact commit.
- GitHub reported exactly one open issue (#37), no open pull requests, and
  `v0.16.0` as the latest release. Issue #37 is therefore the complete
  goal-start queue and highest-priority delivery.
- `Cargo.toml`, the Lurkline package entry in `Cargo.lock`, and
  `packaging/mcp/server.json` report `0.16.0`.
- Lurkline has no Later model, HTTP methods, service operation, CLI command, or
  MCP tool. Existing message, conversation, author, permalink, file,
  pagination, confirmation, and reconciliation patterns can be reused.
- A bounded signed-in browser spike observed `saved.list` with a server cursor,
  state filters, counts, and saved-item metadata; `messages.list` hydrates a
  batch of conversation/message identities for the Later view.
- The same spike observed `saved.add`, `saved.update`, and `saved.delete` for a
  self-DM message. Exact list readback proved save and complete, then removal
  restored the original Later state. No conversation message was created.
- Slack's file-level Save for later action produced a message Later item keyed
  by the containing conversation and message timestamp. Files must therefore
  remain attachments on hydrated message context, not a second resource kind.
- The observed browser contract includes in-progress, completed, and archived
  views; active rows expose complete, reminder, archive, and remove controls.
  Issue #37 requires list, save, complete, and remove, so archive/reminder
  mutation is outside this smallest complete release.
- CI and release workflows gate format, strict locked Clippy, all-target tests,
  release build, credential scanning, Rust 1.88, macOS ARM64, version alignment,
  and three native archives plus their checksum files.

## User Experience And Design Decisions

- The default list is the in-progress inbox. Callers opt into `completed` or
  `archived`; API-specific `saved` naming is not exposed as the human concept.
- The list limit applies to Later items and remains bounded. The continuation
  cursor is opaque, tied to the chosen state, and rejected when malformed or
  reused with incompatible selection so pages cannot silently overlap.
- Hydrate each page in bounded batches, then use existing shared conversation,
  user, thread, permalink, and file normalization. Do not issue one network
  lookup per item when Slack supplies a batch route or a bounded directory can
  be shared.
- Preserve a useful Later row when source context is deleted or unavailable:
  return its stable identity and metadata with explicit context-resolution
  state. Unknown response shapes and unknown resource kinds fail clearly
  rather than being silently dropped.
- Save accepts a conversation reference plus exact message timestamp, matching
  other message-targeting commands. The service resolves the conversation once,
  verifies the source message before mutation, and reports the canonical target.
- Complete is idempotent when the exact item is already completed. Remove is
  idempotent when the exact target is provably absent. Every successful mutation
  reports requested action, observed before/after state, and whether readback
  reconciled; incomplete bounded proof fails actionably instead of overstating
  success.
- Human output is an inbox-oriented summary with state, conversation, author,
  time/reminder, message preview, and permalink. Stable `--json` and MCP output
  retain the complete structured item and continuation data.

## Constraints And Non-Goals

Follow `AGENTS.md`.

- CLI and MCP must delegate to the same typed service operations. Do not add
  path-specific parsing, raw browser payload output, or duplicated behavior.
- Keep all Slack writes behind the current opt-in write gate and an exact
  per-call confirmation. Listing and source verification remain read-only and
  must not alter Slack read state.
- Bound page size, response bytes, hydration work, reconciliation scans, user
  and conversation resolution, message text, files, blocks, and attachments.
  Preserve useful continuation and partial-resolution evidence.
- Treat private Slack browser routes as a provisional, tested compatibility
  contract. Validate required fields and item kinds, tolerate documented
  additive fields, and return recovery-oriented errors on incompatible changes.
- Never commit, log, snapshot, or print real Slack credentials, cookies, HAR
  payloads, workspace messages, names, identifiers, URLs, or file contents.
  Use synthetic fixtures in code, tests, docs, review packets, and commits.
- Live smoke may read the signed-in `sfera` workspace but expose only aggregate
  or boolean evidence. Slack writes are authorized only to the personal Later
  state of a uniquely identified synthetic message in `smarzola`'s self-DM;
  remove every temporary Later item and report exact residue.
- Do not add archive mutation, reminder scheduling, Later search, bulk actions,
  background polling, persistent caching, configuration, dependencies, or
  adjacent Slack mutations.
- Implement the smallest coherent complete design. Simplicity must not omit
  source context, confirmations, reconciliation, tests, error handling,
  documentation, review, or release validation.

## Authorization And Decisions

This goal authorizes repository inspection, in-scope local edits,
non-destructive verification, Conventional Commits, the typed branch above,
branch push, one ready pull request containing `Closes #37`, squash merge after
green CI, annotated tag `v0.17.0`, release publication and verification, issue
closure, and the bounded live smoke described above.

Require confirmation before destructive actions outside safely proven
synthetic artifacts, Slack writes outside the authorized self-DM Later state,
credential or permission changes, purchases, unrelated external writes, or
material scope expansion. Continue autonomously through routine implementation
and delivery choices.

Ask only if new evidence materially changes user-visible syntax,
compatibility, security, or authorization. Exhaust safe in-scope alternatives
before reporting a blocker, and never claim completion without evidence.

## Success Criteria

The goal is complete only when:

1. CLI and MCP list in-progress, completed, and archived Later items with a
   useful default, validated limits, exact aggregate counts, deterministic
   ordering, opaque state-bound cursors, and truthful continuation state.
2. Every supported row preserves stable Later identity and metadata and, when
   available, exposes bounded hydrated source message, thread context, author,
   conversation, attachments/files, and canonical permalink. Missing context is
   explicit and unknown contract shapes fail actionably.
3. Save, complete, and remove share typed service behavior, require both the
   global write opt-in and exact confirmation, verify source/target identity,
   handle already-satisfied state safely, and return exact bounded readback
   reconciliation.
4. A Slack-saved file appears through its containing message and file metadata;
   no unsupported standalone file identity or undocumented API guarantee is
   presented.
5. Synthetic HTTP, service, CLI, process, MCP, schema, pagination, contract-
   drift, confirmation, idempotency, and reconciliation tests cover success and
   actionable failure without private fixtures or credentials.
6. Focused docs and help teach the Later inbox journey, states, cursor use,
   message/file representation, write safety, readback semantics, limitations,
   and recovery from stale or changed Slack contracts.
7. All version sources report `0.17.0`; narrow and full release gates pass; the
   retained adversarial reviewer and a fresh context-independent auditor are
   clean on the exact candidate.
8. The issue-scoped PR is squash-merged, issue #37 is closed, annotated tag
   `v0.17.0` peels to the exact product merge on `main`, tagged CI and release
   workflows succeed, and all archives plus checksums pass checksum, layout,
   mode, architecture, and native version readback verification.
9. Final `origin/main` passes applicable checks, the worktree is clean, live
   testing reports exact scope and residue, and no issue or pull request from
   the goal-start queue remains open.

## Milestone

- [ ] Milestone 1: Deliver the complete Slack Later inbox and release `v0.17.0`
  for #37.

Acceptance criteria are success criteria 1 through 6. Likely touchpoints are
`src/http.rs`, `src/model.rs`, `src/service.rs`, `src/cli.rs`, `src/mcp.rs`,
`tests/cli_process.rs`, `tests/mcp_raw_stdio.rs`, `README.md`, and version
metadata.

Narrow verification:

```bash
cargo test --locked http::tests
cargo test --locked service::tests
cargo test --locked cli::tests
cargo test --locked mcp::tests
cargo test --locked --test cli_process
cargo test --locked --test mcp_raw_stdio
```

### Checkpoint And Delivery Protocol

1. Commit this goal before implementation and obtain retained-reviewer
   readiness on the goal, issue, spike evidence, and baseline.
2. Implement typed models and browser API validation, shared service behavior,
   CLI/MCP surfaces, synthetic tests, focused docs, and `0.17.0` version
   alignment using Conventional Commits.
3. Run narrow verification, freeze main-agent writes, and repair retained
   adversarial review findings until clean.
4. Run format, strict locked all-target Clippy and tests, locked release build,
   Rust 1.88 compatibility, credential scan, version/package checks,
   deterministic native packaging, and diff/status checks.
5. Run the minimum authorized self-DM Later smoke needed for end-to-end proof;
   expose only aggregate/boolean evidence and remove every temporary item.
6. Obtain a clean fresh context-independent audit, mark the milestone done, and
   commit exact local completion evidence.
7. Push one branch, open one ready pull request containing `Closes #37`, wait
   for every check, and squash-merge only the reviewed exact head.
8. Fast-forward local `main`, create and push annotated tag `v0.17.0`, wait for
   main/tag/release workflows, and independently verify all release assets.
9. Record immutable release evidence through a documentation-only finalization
   pull request if needed, then recheck exact `origin/main`, the issue/PR queue,
   final CI, and worktree cleanliness.

## Final Verification

Run from `/Users/smarzola/projects/lurkline`:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
rustup run 1.88.0 cargo check --locked --all-targets
target/release/lurkline --version
python3 scripts/check-no-secrets.py
git diff --check
git status --short
```

Build two deterministic native packages, compare them byte-for-byte, verify the
checksum and exact versioned `lurkline`/`README.md`/`LICENSE` layout, inspect
binary mode, architecture, and signature, and confirm all three version
sources. After tagging, verify exact tag/main ancestry, every workflow, the
GitHub Release, all downloaded assets, each checksum, archive layout, platform
architecture, executable mode, and native binary version in a removed
ephemeral directory.

## Resume And Final Report

Update this file after each verified checkpoint with commit IDs, reviewer and
auditor outcomes, exact test counts, live-smoke scope and residue, PR/merge/tag
identities, workflow runs, release-asset verification, and final queue state.
Do not mark the milestone complete while delivery evidence is pending. A final
report must distinguish local implementation, external delivery, and any
remaining risk without exposing private Slack data.
