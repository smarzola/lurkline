# Goal: Accept Slack MPIM Conversation IDs

Work in `/Users/smarzola/Documents/Codex/2026-07-26/use-browser-to-read-sferait-ws/lurkline-fix`.

Fix Lurkline discovery and inbox so Slack group direct messages are accepted when Slack returns either a `C`-prefixed or `G`-prefixed conversation ID. Preserve the typed, bounded, read-only CLI and MCP behavior already implemented for v0.2.0.

Source of truth: the current user request and the authenticated SferaIT live failure reproduced on 2026-07-26.

## Target State

When this goal is complete:

- `conversations list`, `conversations find`, and `inbox` accept real Slack MPIM records whose `is_mpim` flag is true and whose ID begins with `C` or `G`.
- CLI and MCP return the same typed group-DM behavior through the shared service layer.
- Other invalid conversation-kind and ID-prefix combinations remain rejected.

## Current-State Evidence

Verified before implementation:

- `src/service.rs::is_valid_conversation_id`: `GroupDirectMessage` currently accepts only `G`-prefixed IDs.
- `src/service.rs::normalize_conversations`: a live `conversations.list` response is rejected when `is_mpim=true` is paired with a `C`-prefixed ID.
- Authenticated, bounded live smoke tests: CLI and MCP doctor, one-page conversation listing, and message search pass; conversation find and inbox fail with `invalid_response` from `conversations.list`.
- A structural live-response diagnostic found three `C`-prefixed records with `is_mpim=true`; no credentials, identifiers, names, users, or messages were persisted or printed.
- Baseline branch `feat/discovery-inbox` is clean at `e201c6d4162c7850c9236a77d4dc824337a5dd7c`.

Unknowns that may affect implementation details, but not the target state:

- Slack's unsupported browser-session API may introduce additional response-shape drift later; this goal covers only the verified MPIM ID-prefix incompatibility.

## Constraints And Non-Goals

Follow `AGENTS.md`.

- Keep Slack behavior read-only and keep CLI/MCP behavior in the shared typed service layer.
- Never commit, log, snapshot, or print real credentials, HAR payloads, workspace messages, identifiers, or user data.
- Preserve `C` and `G` channel compatibility, `D` direct-message compatibility, pagination bounds, cursor checks, and kind-disagreement rejection.
- Use synthetic fixtures for committed regression tests.
- Do not add dependencies, credential handling, new commands, API methods, configuration, version changes, publishing, tags, pushes, or releases.
- Implement the smallest coherent complete fix. Prefer a direct validator correction plus focused regression coverage; do not add speculative abstractions.

## Authorization And Decisions

This goal authorizes repository inspection, in-scope local edits, focused Conventional Commit checkpoints, non-destructive verification, and ephemeral read-only live verification using the browser credentials already authorized by the user.

Pushing, tagging, publishing, releasing, destructive actions, credential persistence, permission changes, and material scope expansion remain unauthorized.

Starting branch: `feat/discovery-inbox`.

Goal branch: `fix/mpim-conversation-ids`.

Base commit: `e201c6d4162c7850c9236a77d4dc824337a5dd7c`.

Continue through routine implementation choices using repository evidence. Ask only when an ambiguity materially changes user-visible behavior, architecture, compatibility, security, or authorization. Exhaust safe in-scope alternatives before reporting a blocker.

## Success Criteria

The goal is complete only when:

1. The shared conversation-ID validator accepts both `C`- and `G`-prefixed group DMs while continuing to reject incompatible prefixes and malformed IDs.
2. Synthetic service tests cover `C`-prefixed MPIM discovery and inbox behavior, retain `G` compatibility, and prove an invalid prefix still fails.
3. CLI and MCP authenticated smoke tests pass for doctor, conversation listing, conversation finding, message search, and bounded inbox without exposing or persisting credentials or workspace data.
4. Formatting, strict lint, the full locked test suite, and the locked release build pass.
5. The milestone is adversarially reviewed, checked off with exact verification evidence, and committed with a focused Conventional Commit; a fresh independent final audit reports no blocking findings.

## Milestones

- [ ] Milestone 1: MPIM discovery and inbox compatibility

### Checkpoint Protocol

At the end of the milestone:

1. Satisfy its acceptance criteria.
2. Run its verification commands and inspect the results.
3. Freeze main-agent writes and obtain a clean adversarial review; repair and re-review valid findings.
4. Mark the checklist item `[x]` and add a dated status note containing the outcome, exact commands, and results.
5. Commit the implementation, tests, and this prompt update together with a focused Conventional Commit.
6. Report the resulting commit hash.

If verification fails, leave the milestone unchecked and do not commit it. Diagnose and repair in-scope failures rather than weakening coverage.

## Milestone 1: MPIM Discovery And Inbox Compatibility

Why this matters:

- A valid Slack response currently makes complete discovery and inbox unusable in a real workspace.

Acceptance criteria:

- `C` and `G` MPIM IDs normalize as `GroupDirectMessage` in list/find flows.
- Explicit MPIM unread state using a `C`-prefixed ID can be enriched and read by inbox.
- A `D`-prefixed record marked as MPIM and malformed IDs remain invalid.
- Existing public/private channel and direct-message behavior is unchanged.
- Authenticated CLI and MCP discovery, search, and inbox smokes pass with bounded limits and emit only non-sensitive result metadata.

Likely touchpoints (non-exhaustive):

- `src/service.rs`
- `docs/internal/mpim-conversation-id-compatibility-goal.md`

Verification:

```bash
cargo fmt --check
cargo test mpim --locked
cargo test inbox --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

Authenticated smoke commands run with credentials injected only into the subprocess environment:

```bash
./target/release/lurkline doctor --json
./target/release/lurkline conversations list --limit 10 --json
./target/release/lurkline conversations find general --limit 5 --json
./target/release/lurkline search messages 'has:link' --limit 5 --json
./target/release/lurkline inbox --conversations 3 --messages 3 --json
./target/release/lurkline mcp
```

Status: Not started.

## Final Verification

Run from the goal worktree:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
git diff --check e201c6d4162c7850c9236a77d4dc824337a5dd7c...HEAD
git log --format=%s e201c6d4162c7850c9236a77d4dc824337a5dd7c..HEAD
git status --short --branch
```

Repeat the authenticated bounded CLI and MCP smoke checks above. Inspect only response shapes, counts, and pass/fail state; do not print or persist credentials or Slack content.

## Resume Protocol

On a resumed session, first read this prompt, `AGENTS.md`, `git status`, milestone status notes, and recent commits. Verify completed checkpoints and continue from the first unchecked milestone without redoing completed work. New evidence may refine implementation details but must not silently weaken the target state or success criteria.

## Final Report

Lead with `Achieved` or `Not achieved`, then report target-state status, the milestone commit, files changed, exact verification results, reviewer rounds and disposition, live-smoke disposition, residual private-API risks, and the unauthorized external delivery step that remains.
