# Goal: Deliver Nullable Identities And Complete Activity Scope

Work in `/Users/smarzola/projects/lurkline`.

Address every GitHub issue open at the start of this goal through a prioritized
sequence of complete, issue-scoped releases. Deliver the identity correctness
bug before extending activity scope. Each issue must have its own branch,
reviewed pull request, squash merge, annotated version tag, successful release
workflow, GitHub Release, and independently verified native assets.

Source of truth: GitHub issues
[#27](https://github.com/smarzola/lurkline/issues/27) and
[#26](https://github.com/smarzola/lurkline/issues/26), read from GitHub on
2026-07-30.

## Target State

When this goal is complete:

- Missing Slack usernames and display names remain absent from structured
  output as JSON `null`; empty values follow the same policy, while a genuine
  four-character identity equal to `"null"` remains a string.
- Author, mention, unread-conversation, ordinary-conversation, and public user
  output apply one truthful identity policy without losing stable IDs,
  resolution state, or deliberate conversation fallbacks.
- Activity callers can filter eligible conversations by kind before the cap
  and traverse every conversation in the bounded discovered scope through an
  explicit, deterministic, stale-safe continuation.
- Activity remains read-only, response- and request-bounded, CLI/MCP-aligned,
  and honest about hard directory-scan limits and cross-scope merge semantics.
- `main` contains two sequential issue-scoped squash merges, and releases
  `v0.14.0` and `v0.15.0` are published and verified.

## Current-State Evidence

Verified before implementation:

- Starting branch: `main`; first issue branch:
  `fix/nullable-user-identities`.
- Goal base:
  `521d95ee1cfc53833afbd4cc8d97a2229094a71a`, identical to `origin/main`
  and annotated tag `v0.13.0`.
- The worktree was clean and GitHub reported no open pull requests.
- GitHub reported exactly two open issues: bug #27 and enhancement #26.
- `Cargo.toml`, `Cargo.lock`, and `packaging/mcp/server.json` report `0.13.0`.
- `RawUser.name`, `RawUserProfile.display_name`, and the corresponding public
  `User` fields are non-nullable strings. Missing raw fields default to empty;
  explicit JSON null is not accepted at that boundary.
- Author and mention fields are already `Option<String>` and serialize true
  absence as JSON null. Their directory enrichment filters empty or unsafe
  labels, while public user output still exposes non-nullable strings.
- Ordinary conversations intentionally retain required name/display fields:
  DMs fall back to a stable user ID and expose `name_is_fallback`. Unread
  conversation names are nullable and expose typed resolution.
- Activity scans at most 20 conversation pages of 200, ranks eligible
  conversations by validated `latest` metadata then stable ID, selects at most
  50, and marks omitted selection explicitly. It cannot continue into the
  omitted conversation slice.
- Existing activity continuation freezes one selected slice, time interval,
  ordering, limits, last message key, and bounded message snapshot. One
  reply-inclusive `conversations.history` request is made per selected
  conversation; no read-state mutation endpoint is used.
- `.github/workflows/ci.yml` covers formatting, strict locked Clippy, locked
  tests, release build, credential scanning, package metadata, Rust 1.88, and
  macOS ARM64. `.github/workflows/release.yml` publishes three native archives
  plus three matching checksum assets from a semantic tag on exact `main`.
- The complete `v0.13.0` suite passed immediately before this goal: 275
  library tests, 12 CLI-process tests, 2 raw-MCP tests, and 1 package-metadata
  test, plus all static, release, compatibility, and packaging gates.

Unknowns affecting implementation details but not the target state:

- The signed-in workspace may not currently contain every missing/empty
  identity combination or more than 50 conversations of one kind. Synthetic
  fixtures are authoritative for those edge cases; live smoke may validate
  aggregate schema and scope behavior without printing workspace data.
- Slack cannot provide an immutable whole-workspace snapshot across separate
  calls. Continuations must detect changes in the bounded reconstructed scope
  before reading history and document what cannot be proven after a completed
  scope slice.

## Priority And Release Queue

1. Issue #27, `v0.14.0`: correct the public identity schema before the activity
   extension consumes the same user directory.
2. Issue #26, `v0.15.0`: add kind filtering and complete bounded scope
   traversal on top of the corrected schema.

The #27 bug requires nullable public user fields, so it is a pre-1.0 minor
release rather than a patch. Do not combine the issues or begin #26 before
`v0.14.0` is independently verified.

## Constraints And Non-Goals

Follow `AGENTS.md`.

- Keep CLI and MCP behavior on the same typed service layer.
- Keep normal Slack behavior read-only. Neither issue adds a Slack mutation.
- Preserve stable IDs, typed resolution states, canonical text, permalink and
  mention behavior, time bounds, per-conversation history bounds, response byte
  limits, cursor integrity, terminal escaping, credential safety, and existing
  guarded write behavior outside this goal.
- Never commit, log, snapshot, or print real Slack credentials, messages,
  identities, conversation names, URLs, or file contents. Use synthetic
  fixtures in tests, documentation, reviews, and commits.
- Live smoke may use the signed-in `sfera` profile to read any workspace scope,
  but output only aggregate structure and boolean invariants. Slack writes, if
  genuinely necessary, are authorized only in `smarzola`'s self-DM and must use
  minimum uniquely labeled synthetic content with truthful residue reporting.
- Apply one boundary normalization policy instead of scattered serializer
  special cases. Never reinterpret the valid string `"null"` as absence.
- Activity traversal must remain caller-driven and bounded per call. Do not add
  an automatic unbounded whole-workspace history fan-out, persistent cache,
  background job, search dependency, or new configuration.
- Implement the smallest coherent complete design consistent with repository
  patterns. Do not add speculative abstractions, dependencies, adjacent
  identity fields, or unrelated activity filters.
- Protect unrelated user work. Start each issue from the newly released exact
  `origin/main`; never stack the second issue on an unmerged branch.

## Authorization And Decisions

This goal authorizes repository inspection, in-scope local edits,
non-destructive verification, Conventional Commits, typed branch creation,
branch pushes, one pull request per issue, squash merges after green CI,
annotated tags, and publication and verification of `v0.14.0` and `v0.15.0`.
It authorizes GitHub issue closure through each pull request's `Closes #N`
reference and the bounded live Slack reads described above.

Require confirmation before destructive actions outside safely proven
synthetic artifacts, Slack writes outside the authorized self-DM, external
writes unrelated to this delivery, credential or permission changes,
purchases, or material scope expansion.

Continue through routine implementation choices using repository evidence.
Ask only when an ambiguity materially changes user-visible behavior,
architecture, compatibility, security, or authorization. Exhaust safe
in-scope alternatives before reporting a blocker.

## Success Criteria

The goal is complete only when:

1. Omitted, JSON-null, empty, and whitespace-only Slack usernames and profile
   display names normalize to absence once at the user boundary and serialize
   as JSON null in public user, author, and mention output where no documented
   fallback supplies a label.
2. A literal valid username or display name equal to `"null"` remains the JSON
   string `"null"`. Stable IDs, author/mention/name resolution states, DM ID
   fallback, and unread naming semantics remain unchanged.
3. Human user output gives an explicit safe placeholder for missing identity
   fields, while CLI JSON and MCP schemas agree on nullable types.
4. Activity accepts a normalized repeated kind filter for channel, direct
   message, and group direct message; applies it before includes, excludes, and
   the conversation cap; and rejects contradictory explicit selectors with
   actionable errors.
5. A single opaque activity continuation stream can traverse disjoint slices
   of every eligible conversation found by the bounded directory scan. It
   freezes interval, normalized kinds, resolved selectors, limits, scope
   offset, and a digest of the fully ordered eligible scope.
6. Scope changes make continuation stale before history calls. Each call reads
   at most 50 conversations, each selected conversation receives exactly one
   bounded reply-inclusive history request, and directory-scan truncation
   remains explicit and non-continuable rather than being called complete.
7. Structured and human output distinguish message continuation from
   conversation-scope continuation, expose scope progress, and document that
   items from multiple scope slices must be merged with the canonical
   timestamp/conversation-ID comparator for whole-scope global ordering.
8. Existing activity message paging, response-byte shortening, partial and
   inaccessible states, exact `[after, before)` semantics, CLI/MCP parity, and
   no-mark-read behavior remain covered.
9. Each issue has synthetic regression coverage, current focused
   documentation, clean retained-review and fresh-audit results, green CI, one
   squash-merged PR, and one verified release.
10. Both releases align all three version sources, tag the exact merged `main`
    commit, publish exactly three archives and three checksums, and pass
    independent checksum, layout, and binary-version verification.
11. After `v0.15.0`, issues #26 and #27 are closed, the final suite passes on
    exact `main`, a fresh integration audit is clean, and the worktree is
    clean.

## Milestones

- [x] Milestone 1: Normalize nullable identities and release `v0.14.0` for #27.
- [ ] Milestone 2: Traverse filtered activity scope and release `v0.15.0` for #26.

### Per-Issue Checkpoint And Delivery Protocol

For each milestone:

1. Refresh `origin/main`, verify the prior release, and create the recorded
   typed issue branch from exact current `origin/main`.
2. Record the milestone-start commit and finalized user-visible design below.
3. Implement the issue, synthetic tests, focused docs, and version alignment.
4. Run narrow verification, freeze main-agent writes, and obtain a clean review
   from the retained adversarial reviewer. Repair and re-review until clean.
5. Run the full release gate and obtain a clean fresh context-independent audit.
6. Mark the milestone `[x]`, add a dated evidence note, and commit all remaining
   issue work with focused Conventional Commits.
7. Push, open one pull request containing `Closes #N`, wait for green checks,
   and squash-merge only the reviewed expected head.
8. Switch to `main`, fast-forward from `origin/main`, verify the squash result,
   create and push the annotated version tag, then wait for the tagged workflow.
9. Verify the workflow, GitHub Release, exact six-asset set, every checksum,
   archive layout, and native version before beginning the next issue.

Do not treat a checkbox, local test, pull request, tag, workflow, or release
object alone as delivery proof.

## Milestone 1: Nullable User Identities (`v0.14.0`, #27)

Acceptance criteria:

- Raw username, profile display-name, and real-name inputs accept omission,
  explicit JSON null, empty strings, whitespace, ordinary values, and the
  literal `"null"` without deserialization ambiguity.
- Public `User` identity labels are nullable and normalized once. Author and
  mention enrichment reuses them, retaining the existing real-name fallback
  only when a safe real name exists.
- Required ordinary-conversation labels keep the stable ID fallback and
  `name_is_fallback`; unread conversation labels remain nullable with typed
  resolution.
- User search, human output, CLI JSON, MCP results, and generated schemas agree.
- Synthetic tests cover the complete input matrix and unchanged IDs/statuses.

Likely touchpoints: `src/model.rs`, `src/service.rs`, `src/cli.rs`,
`src/mcp.rs`, `tests/cli_process.rs`, `tests/mcp_raw_stdio.rs`, `README.md`,
and version metadata.

Narrow verification:

```bash
cargo test --locked service::tests
cargo test --locked cli::tests
cargo test --locked --test cli_process
cargo test --locked --test mcp_raw_stdio
```

Status: Completed and independently verified on 2026-07-30. Started from
`521d95ee1cfc53833afbd4cc8d97a2229094a71a`, exact `v0.13.0`
`origin/main`, on `fix/nullable-user-identities`.

Design decisions:

- Represent Slack username, profile display name, and real name as optional
  raw/public labels. Normalize missing, null, empty, and whitespace-only input
  to `None`; retain trimmed valid strings including `"null"`.
- Keep conversation labels non-nullable where the model promises a stable
  fallback. Do not add sentinel strings or serializer-only rewrites.
- Use a readable explicit placeholder in human user rows while preserving JSON
  null and generated nullable schemas.

Local release-candidate evidence:

- Implementation commit
  `35c207983d714a964b84a68620464af933f708a7` makes the raw and public
  identity labels optional, normalizes safe values once, reuses those typed
  options for author, mention, and unread enrichment, preserves ordinary
  conversation fallbacks, aligns generated schemas and human output, documents
  the compatibility change, and sets all version sources to `0.14.0`.
- The retained adversarial reviewer found one inconsistent second-stage label
  filter. The repair moved control-character and 256-byte enforcement into the
  user boundary and added public-user, author, mention, unread, real-name
  fallback, overlong-label, and literal-`"null"` regressions. Re-review was
  clean.
- Formatting, strict locked Clippy, all 278 library tests, 12 CLI-process
  tests, 2 raw-MCP tests, the package-metadata test, locked release build,
  Rust 1.88 compatibility, credential scan, and diff checks passed. The
  sandboxed all-target attempt could not bind loopback listeners; its permitted
  rerun passed every test.
- Two local macOS ARM64 packages were byte-identical. Checksum, exact
  versioned archive layout, permissions, and packaged binary readback as
  `lurkline 0.14.0` passed.
- The fresh context-independent audit at implementation commit `35c2079` was
  clean across issue scope, UX, correctness, security and privacy, CLI/MCP
  parity, schemas, compatibility, documentation, tests, code quality, and
  release readiness.
- An authorized read-only `sfera` smoke returned 100 typed user records,
  including 8 genuine JSON-null display names and zero invalid identity field
  types. Only aggregate counts were emitted; no Slack write occurred, and the
  mode-0600 raw temporary file was removed with zero residue.
- [PR #28](https://github.com/smarzola/lurkline/pull/28) passed CI run
  `30567649318` at reviewed head
  `eba7ebefef505c657b5802fcc5932a3c5b1137cd` and squash-merged to
  `c645cc25772e1da647631d5a9863c2f9f7ff1a3b`, closing issue #27 as
  completed.
- Annotated tag `v0.14.0` dereferences to that exact merged `main` commit.
  Main CI `30567894536`, tag CI `30567956512`, and release workflow
  `30567956735` all completed successfully.
- Published GitHub Release `362618362` is neither draft nor prerelease and
  contains exactly the three documented native archives plus their three
  checksums. Independent downloads passed every checksum, exact versioned
  `lurkline`/`README.md`/`LICENSE` layout and executable-mode check. The native
  asset is a linker-signed ARM64 Mach-O and reports `lurkline 0.14.0`; all
  verification artifacts were removed.

## Milestone 2: Filtered Complete Activity Scope (`v0.15.0`, #26)

Acceptance criteria:

- Repeated typed kind filters are normalized, deduplicated, and applied before
  the conversation cap. Empty means all current kinds.
- Includes may opt in a visible unjoined channel only when its kind is allowed;
  mismatched kind/include intent and include/exclude overlap fail actionably.
- The cursor format has explicit message and scope phases. Continuation
  reconstructs the bounded directory, verifies the ordered-scope digest before
  history calls, and selects the next disjoint slice without duplication or
  omission while the scope is stable.
- Report fields expose normalized kinds, eligible count, scope offset,
  message-versus-scope continuation kind, continuable scope remainder, and hard
  directory-scan truncation. Existing fields remain meaningful and additive.
- Every response remains ordered within its current scope slice. Full traversal
  merge ordering is deterministic and documented for both directions.
- Synthetic tests cover more than 50 mixed and same-kind conversations,
  filters, selectors, ties, scope transitions including empty slices, stale and
  tampered cursors, exact bounds, byte shortening, scan truncation, and
  read-only HTTP shapes.

Likely touchpoints: `src/model.rs`, `src/service.rs`, `src/cli.rs`,
`src/mcp.rs`, focused HTTP assertions, `tests/cli_process.rs`,
`tests/mcp_raw_stdio.rs`, `README.md`, and version metadata.

Narrow verification:

```bash
cargo test --locked service::tests
cargo test --locked http::tests
cargo test --locked cli::tests
cargo test --locked --test cli_process
cargo test --locked --test mcp_raw_stdio
```

Status: Started on 2026-07-30 from
`c645cc25772e1da647631d5a9863c2f9f7ff1a3b`, exact verified `v0.14.0`
`origin/main`, on `feat/activity-scope-pagination`.

Design decisions:

- Add a repeated typed kind selector covering channel, direct message, and
  group direct message. Normalize and deduplicate it once, apply it before the
  activity conversation cap, and make explicit selectors outside the allowed
  kinds fail actionably.
- Continue through conversation scope with the existing single opaque activity
  cursor instead of adding a second pagination interface. Give the cursor a
  tagged message or scope phase, and expose which phase the next cursor
  represents.
- Reconstruct the complete bounded eligible scope on continuation and protect
  it with a digest plus offset. Scope drift must fail stale before any history
  read; the cursor must not embed an unbounded list of conversation IDs.
- Order eligible scope by stable conversation ID and digest only ordered
  ID/kind eligibility data. Do not let display-name changes or mutable Slack
  `latest` metadata outside the frozen interval make a valid traversal stale.
- Treat `--conversations` as a per-call slice bound, including for explicit
  includes. Canonicalize kinds in enum order and resolved selector IDs in
  sorted order before cursor creation.
- Keep each response to at most 50 conversations and one bounded,
  reply-inclusive history request per selected conversation. Expose eligible
  count, scope progress, remaining scope, and hard directory-scan truncation.
- Preserve canonical ordering within each slice and document the canonical
  timestamp/conversation-ID merge comparator for callers combining multiple
  scope slices.

## Final Verification

Run before each pull request from `/Users/smarzola/projects/lurkline`:

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

Also build two deterministic local packages for the current native platform,
compare them byte-for-byte, verify the checksum, inspect the exact versioned
`lurkline`, `README.md`, and `LICENSE` layout, and confirm version alignment in
`Cargo.toml`, `Cargo.lock`, and `packaging/mcp/server.json`.

After each tag, verify the release workflow to completion. Confirm the GitHub
Release contains exactly the three documented platform archives and their
three checksum files. Download all six to an ephemeral directory, verify every
checksum and archive layout, and run the native release binary's version
readback.

## Resume Protocol

On resume, read this goal, `AGENTS.md`, git status and history, live open
issue/PR state, milestone notes, and the latest release. Verify completed
checkpoints rather than redoing them, then continue from the first unchecked
milestone without silently weakening or combining the queue.

## Final Report

Lead with `Achieved` or `Not achieved`, then report:

- both issue outcomes and success-criteria status;
- branch, checkpoint commits, PR, squash commit, tag, workflow, release, and
  asset evidence for each issue;
- exact local and CI verification results;
- retained reviewer and fresh auditor findings and repairs;
- live read coverage, any authorized self-DM writes, and residue;
- residual risks, follow-ups, or blockers.
