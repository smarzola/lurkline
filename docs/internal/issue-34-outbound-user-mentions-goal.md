# Goal: Publish Explicit, Verifiable Slack User Mentions

Work in `/Users/smarzola/projects/lurkline`.

Resolve GitHub issue
[#34](https://github.com/smarzola/lurkline/issues/34) as one focused enhancement
release. Authors must be able to deliberately express a notifying Slack user
mention in Markdown, verify the exact resolved user before publication, and
publish the same structured mention through CLI and MCP. Mention-looking text
must never become a notification by accident.

Source of truth: issue #34, read from GitHub on 2026-08-03.

## Target State

When this goal is complete:

- `[@label](slack-user:reference)` is the explicit outbound mention extension.
  `reference` accepts an exact Slack user ID, username, or display name; a
  destination containing spaces uses CommonMark angle-destination form.
- One bounded workspace user-directory scan resolves every mention in a
  document. Resolution is exact user ID first, then case-insensitive username,
  then case-insensitive display name. Missing, incomplete, deleted, conflicting,
  and ambiguous results fail before publication with recovery guidance.
- Each resolved mention emits a Slack rich-text `user` element and an ordered
  structured proof record containing its source label, reference, user ID, and
  resolution kind. Human dry-render output also identifies real mentions.
- `message render`, root sends, thread replies, drafts, and corresponding MCP
  tools use the same parsed document and typed service resolution. Ordinary
  Markdown rendering remains local and credential-free when no explicit
  mention extension is present.
- Literal `@text`, email addresses, canonical `<@USER_ID>` text, ordinary
  links, and mention-looking syntax inside inline, fenced, or indented code
  remain literal and non-notifying.
- `main` contains one squash-merged pull request for #34 and release `v0.16.0`
  is published and independently verified.

## Current-State Evidence

Verified before implementation:

- Starting branch `main` was clean at
  `c0ba7aed234dc798b29ed6630c2b73294b2d8170`, identical to `origin/main` after
  `git fetch --prune origin` and `git pull --ff-only`. Issue branch
  `feat/outbound-user-mentions` starts at that commit.
- GitHub reported exactly one open issue (#34), no open pull requests, and
  `v0.15.1` as the latest release. The issue is therefore the complete current
  queue and highest-priority delivery.
- `Cargo.toml`, the Lurkline package entry in `Cargo.lock`, and
  `packaging/mcp/server.json` report `0.15.1`.
- `src/markdown.rs::render_markdown` emits only text and link inline elements.
  Both `@alice` and `<@USER_ID>` therefore remain text. The proposed explicit
  link currently degrades to text such as `@alice (slack-user:alice)`.
- `src/service.rs::scan_user_directory` already performs one bounded scan of at
  most 20 pages of 200 users, normalizes optional identities, detects
  conflicting IDs, and reports whether the directory is complete.
- `src/service.rs::send_message` and draft create/update paths render before
  conversation resolution or publication. CLI and MCP delegate publication to
  the same typed service.
- Baseline `cargo test --locked markdown::tests` passed 9 tests and the focused
  send/reply service regression passed. CI and release workflows gate format,
  strict locked Clippy, all tests, release build, credential scanning, Rust
  1.88, macOS ARM64, and three archives plus three checksums.

## Constraints And Non-Goals

Follow `AGENTS.md`.

- Keep CLI and MCP behavior on the same typed service and parsed Markdown
  representation. Do not add path-specific replacements or arbitrary Block Kit
  input.
- Treat the `slack-user:` destination as the sole opt-in. Do not reinterpret
  bare `@word`, email addresses, canonical Slack tokens, or code examples.
- Require a plain bounded label beginning with `@`. Preserve the label in the
  text fallback, but make the resolved Slack user ID—not the label—the source
  of notification identity.
- Resolve all distinct references with one shared bounded directory. Active
  exact IDs may succeed when found in a bounded incomplete scan; name-based
  matches require a complete scan because unseen users could make them
  ambiguous. Unknown IDs in an incomplete scan remain unprovable and fail.
- Allow active human and bot user identities, but reject deleted users. Do not
  add user-group, channel, broadcast, or automatic mention support.
- Keep all existing Markdown byte, nesting, rendered-size, element-count,
  terminal-escaping, write-confirmation, reconciliation, and response-byte
  safeguards.
- Never commit, log, snapshot, or print real Slack credentials, messages,
  identities, conversation names, URLs, or file contents. Use synthetic
  fixtures in tests, documentation, reviews, and commits.
- Live smoke may use the configured signed-in profile to read any workspace
  scope, but emit only aggregate or boolean evidence. Slack writes are
  authorized only in the verified current-user self-DM and must use minimum
  uniquely labeled synthetic content with truthful residue reporting.
- Implement the smallest coherent complete design using existing parser,
  directory, model, service, CLI, and MCP patterns. Do not add dependencies,
  configuration, generalized templating, persistent caches, background work,
  or adjacent mention kinds.

## Authorization And Decisions

This goal authorizes repository inspection, in-scope local edits,
non-destructive verification, Conventional Commits, the typed branch above,
branch push, one ready pull request containing `Closes #34`, squash merge after
green CI, annotated tag `v0.16.0`, release publication and verification, issue
closure, and the bounded live smoke described above.

Require confirmation before destructive actions outside safely proven
synthetic artifacts, Slack writes outside the authorized self-DM, credential or
permission changes, purchases, unrelated external writes, or material scope
expansion. Continue autonomously through routine implementation choices.

Ask only if new evidence would materially change user-visible syntax,
compatibility, security, or authorization. Exhaust safe in-scope alternatives
before reporting a blocker, and never claim completion without evidence.

## Success Criteria

The goal is complete only when:

1. Explicit mention syntax parses structurally in prose and emits one Slack
   `user` element per occurrence while preserving surrounding Markdown order,
   list/quote placement, fallback text, and existing bounds.
2. Ordered `outbound_mentions` records make labels, references, resolved IDs,
   and ID/username/display-name resolution mechanically verifiable in CLI JSON
   and MCP output; human dry-render distinguishes notifying mentions.
3. One directory scan resolves multiple references with exact deterministic
   precedence. Unknown, deleted, conflicting, ambiguous, and incomplete
   identities fail actionably before any conversation lookup or mutation.
4. Ordinary `@text`, email addresses, canonical tokens, standard links,
   escaped Markdown, inline code, fenced code, and indented code retain their
   current literal/non-notifying behavior without a user-directory request.
5. Message render, root publication, thread reply, draft authoring, and MCP use
   the same typed resolution path. Root/reply request tests and exact readback
   prove the structured mention survives publication.
6. Focused docs explain syntax, resolution precedence, local-versus-workspace
   rendering, verifiable output, failure recovery, and the distinction between
   visible `@text` and a notifying mention.
7. All version sources report `0.16.0`; narrow and full release gates pass;
   retained adversarial review and a fresh context-independent audit are clean.
8. The issue-scoped PR is squash-merged, issue #34 is closed, annotated tag
   `v0.16.0` peels to the exact product merge on `main`, tagged CI and release
   workflows succeed, and all three archives plus checksums pass checksum,
   layout, mode, architecture, and native `--version` verification.
9. Exact final `origin/main` passes applicable checks, the worktree is clean,
   authorized live testing reports its exact scope and residue, and no open
   issue or pull request remains from the goal-start queue.

## Milestone

- [x] Milestone 1: Complete the local explicit outbound user mention candidate
  for `v0.16.0` and #34.

Acceptance criteria are success criteria 1 through 6. Likely touchpoints are
`src/markdown.rs`, `src/model.rs`, `src/service.rs`, `src/cli.rs`, `src/mcp.rs`,
`tests/cli_process.rs`, `tests/mcp_raw_stdio.rs`, `README.md`, and version
metadata.

Narrow verification:

```bash
cargo test --locked markdown::tests
cargo test --locked service::tests
cargo test --locked cli::tests
cargo test --locked mcp::tests
cargo test --locked --test cli_process
cargo test --locked --test mcp_raw_stdio
```

### Local Completion Evidence

- Goal commit `0592e58bc501606f20e18f30ccb8d33d5c3d6ea7`, implementation
  commit `5c953ca08cd54b12845f9bd60654e2f2b624c954`, and audit-repair
  commit `314310fb31a895ce0b615b50c2d47541a6bda8d6` form the exact local
  candidate.
- The retained adversarial reviewer completed one finding-and-repair round,
  then returned clean. A fresh context-independent auditor completed a
  separate finding-and-repair round, then returned clean on the audit repair.
- Exact-candidate gates passed: format, strict locked all-target Clippy, 297
  library tests, 13 CLI process tests, two raw MCP tests, one metadata test,
  locked release build, Rust 1.88 all-target check, credential scan, version
  readback, and diff/status checks.
- Two independently generated native packages were byte-identical with
  identical checksum files. The checksum, epoch timestamps, exact versioned
  directory and three-file layout, `0755` binary mode, `0644` documentation
  modes, ARM64 Mach-O architecture, linker signature, and packaged `0.16.0`
  version readback all passed; both temporary package directories were removed.
- Read-only signed-in CLI and MCP renders on the exact candidate each produced
  one ordered username-resolution proof, one structured Slack user element,
  and the literal fallback label. No Slack write was made by the exact-head
  rerun. Earlier publication-path smoke created exactly one minimal synthetic
  root message in the verified current-user self-DM; exact readback proved the
  destination, content, root placement, blocks, one structured self mention,
  complete mention resolution, and absence of files or attachments. That one
  self-DM message is the only unavoidable Slack residue.
- At that local checkpoint, pull-request, merge, tag, workflow, release-asset,
  and final queue evidence remained pending and was not claimed.

### External Delivery Evidence

- Ready PR [#35](https://github.com/smarzola/lurkline/pull/35) linked #34,
  contained audited head `0d8260dcbb43a4d0a67fafec4918c2c775fec280`,
  passed both triggered copies of all three CI jobs, had no review comments,
  and squash-merged as product commit
  `0d315d169db57ed503c26330974d08e740ee0cbf`. GitHub closed issue #34.
- Post-merge main CI run
  [30857856458](https://github.com/smarzola/lurkline/actions/runs/30857856458)
  passed on the product commit. Annotated tag `v0.16.0` peels exactly to that
  commit, whose tree matches the independently audited branch tree.
- Tag CI run
  [30858139011](https://github.com/smarzola/lurkline/actions/runs/30858139011)
  and release run
  [30858139279](https://github.com/smarzola/lurkline/actions/runs/30858139279)
  passed. The release workflow revalidated version agreement, format, strict
  lint, all tests, release build, credential scan, package smoke, Rust 1.88,
  and macOS ARM64 before building all three targets.
- Published release
  [`v0.16.0`](https://github.com/smarzola/lurkline/releases/tag/v0.16.0) is
  neither draft nor prerelease and contains exactly three platform archives
  plus their three checksum files. Independent downloads matched both GitHub
  asset digests and companion checksums. Every archive had the exact versioned
  directory plus `lurkline`/`README.md`/`LICENSE`, epoch timestamps, `0755` and
  `0644` modes, matching source documentation, and the declared Linux x86_64,
  Linux ARM64, or macOS ARM64 architecture. The native linker-signed binary
  reported `lurkline 0.16.0`; the temporary download directory was removed.
- At the product-release checkpoint, local `main`, `origin/main`, and the
  peeled tag all resolved to the product commit; the worktree was clean and
  GitHub reported zero open issues and zero open pull requests. This
  documentation-only finalization intentionally leaves the release tag on the
  product commit.

### Checkpoint And Delivery Protocol

1. Commit this goal before implementation and obtain retained-reviewer
   readiness on the goal, issue, and baseline.
2. Implement the smallest coherent parsed/resolved design, synthetic tests,
   focused docs, and `0.16.0` version alignment using Conventional Commits.
3. Run narrow verification, freeze main-agent writes, and repair retained
   adversarial review findings until clean.
4. Run `cargo fmt --all -- --check`, strict locked all-target Clippy and tests,
   locked release build, Rust 1.88 compatibility, credential scan, version and
   package metadata checks, deterministic native packaging, and diff/status
   checks.
5. Perform the authorized live smoke with minimal self-DM content if local and
   synthetic evidence is insufficient; expose only aggregate/boolean evidence
   and report unavoidable residue.
6. Obtain a clean fresh context-independent audit, mark the milestone done,
   and commit the final local evidence.
7. Push one branch, open one ready pull request containing `Closes #34`, wait
   for every check, and squash-merge only the reviewed exact head.
8. Fast-forward local `main`, create and push annotated tag `v0.16.0`, wait for
   main/tag/release workflows, and independently verify all six assets.
9. Record immutable release evidence in this goal through a documentation-only
   finalization pull request if needed, then recheck exact `origin/main`, the
   issue/PR queue, final CI, and worktree cleanliness.

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
binary mode/architecture/signature, and confirm all three version sources.
After tagging, verify exact tag/main ancestry, all workflows, the GitHub Release,
all six downloaded assets, every checksum, archive layout, platform
architecture, executable mode, and native binary version in an ephemeral
directory that is removed afterward.

## Resume And Final Report

On resume, read this goal, `AGENTS.md`, git status, recent commits, live issue/PR
state, and the first unchecked milestone. Verify completed checkpoints rather
than redoing them, and do not weaken or combine the issue boundary.

Lead the final report with `Achieved` or `Not achieved`. Include the goal file,
branch and commits, changed files, exact local/CI/release verification, retained
review and fresh-audit rounds, live-smoke scope and residue, release evidence,
and any residual risk or blocker.
