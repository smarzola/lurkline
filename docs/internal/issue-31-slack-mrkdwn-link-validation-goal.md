# Goal: Reject Ambiguous Slack-Native Links Before Authoring

Work in `/Users/smarzola/projects/lurkline`.

Resolve GitHub issue
[#31](https://github.com/smarzola/lurkline/issues/31) as one focused patch
release. Lurkline must reject Slack-native `<URL|label>` syntax in prose before
it can produce a misleading preview, malformed destination, draft, message, or
reply. The recovery must be immediately actionable: use standard Markdown
`[label](URL)`.

Source of truth: issue #31, read from GitHub on 2026-08-01.

## Target State

When this goal is complete:

- Unescaped `<http://...|label>` and `<https://...|label>` forms in prose fail
  locally with one stable diagnostic that names the unsupported Slack syntax
  and demonstrates standard Markdown.
- Validation follows the parsed Markdown structure. Inline, fenced, and
  indented code examples remain literal; standard Markdown links and their
  destinations remain valid.
- Local rendering and every CLI/MCP authoring path receive the same behavior
  through the shared renderer and typed service layer, before any Slack
  request.
- `main` contains one squash-merged pull request for #31 and release `v0.15.1`
  is published and independently verified.

## Current-State Evidence

Verified before implementation:

- Goal base `bd7a0570a9f10ee06124fbdfe4d85e92b7356557` is clean and identical
  to `origin/main`; branch `fix/reject-slack-mrkdwn-links` starts there.
- GitHub reports exactly one open issue (#31) and no open pull requests.
- The latest product release is `v0.15.0`; `Cargo.toml`, the Lurkline package
  entry in `Cargo.lock`, and `packaging/mcp/server.json` all report `0.15.0`.
- The current parser leaves `<https://example.com/one|One Label>` visible as
  literal text, but treats `<https://example.com/two|Two>` as an autolink whose
  destination includes the pipe. Neither outcome warns the author.
- `pulldown-cmark` offset events distinguish prose text, autolinks, standard
  Markdown link destinations, inline code, and code blocks without a raw
  whole-input regular expression.
- `render_markdown` backs the local renderer, service message/reply/draft
  creation and update paths, and the corresponding MCP tools.
- CI and release workflows already gate formatting, strict locked Clippy,
  locked tests, release builds, credential scanning, Rust 1.88, three native
  archives, and their three checksums.

## Constraints And Non-Goals

Follow `AGENTS.md`.

- Keep CLI and MCP authoring behavior on the same renderer and typed service
  layer. Do not add path-specific string checks.
- Validate parsed/tokenized Markdown structure, not the entire source with a
  raw regex. Preserve escaped examples and all inline, fenced, and indented
  code examples.
- Reject only unescaped Slack-native HTTP(S) labeled-link syntax. Do not reject
  ordinary angle autolinks, standard Markdown links, encoded pipes, or link
  destination characters that the existing renderer supports.
- Fail before conversation resolution or a Slack request. Keep the diagnostic
  concise and useful in both human and structured MCP errors.
- Use only synthetic fixtures in tests, docs, reviews, and commits. Never log
  or print real Slack credentials, messages, identities, URLs, or file data.
- Live smoke may use the signed-in `sfera` profile to read any workspace scope.
  If a write is needed to validate authoring, it is authorized only in
  `smarzola`'s self-DM, with minimal uniquely labeled synthetic content and
  truthful residue reporting.
- Keep the change patch-sized. Do not redesign Markdown support, add a parser
  dependency, or broaden Slack write capabilities.

## Authorization

This goal authorizes in-scope edits, non-destructive verification,
Conventional Commits, branch push, one pull request containing `Closes #31`,
squash merge after green CI, annotated tag `v0.15.1`, release publication and
verification, issue closure, and the bounded live smoke described above.

Require confirmation before destructive actions outside safely proven
synthetic artifacts, Slack writes outside the authorized self-DM, credential
or permission changes, purchases, unrelated external writes, or material scope
expansion. Continue autonomously through routine implementation choices.

## Success Criteria

The goal is complete only when:

1. Prose forms with HTTP and HTTPS, labels with and without spaces, multiple
   occurrences, and surrounding punctuation fail deterministically.
2. The error names unsupported Slack-native link syntax and says to use
   standard Markdown `[label](URL)`.
3. Escaped forms and inline, fenced, and indented code render unchanged.
4. Standard Markdown links, ordinary autolinks, and supported destination
   characters render unchanged.
5. CLI render and raw MCP tests prove the same public diagnostic, while
   service tests prove invalid message/reply/draft authoring reaches no Slack
   API operation.
6. Focused renderer documentation explains the accepted syntax and recovery.
7. All three version sources report `0.15.1`; local narrow and full release
   gates pass; retained review and fresh final audit are clean.
8. PR #31's issue-scoped pull request is squash-merged, the issue is closed,
   tag `v0.15.1` points to exact released `main`, the tagged workflow succeeds,
   and exactly three archives plus three matching checksums pass checksum,
   layout, architecture, mode, and native `--version` verification.
9. Exact final `origin/main` passes its applicable checks and the worktree is
   clean with no open issues or pull requests.

## Milestone

- [x] Milestone 1: Reject Slack-native labeled links and release `v0.15.1` for
  issue #31.

### Implementation Shape

- Add a bounded validation pass to `src/markdown.rs` using parser offset events.
  Scan only prose-owned source ranges and autolink ranges, exclude inline and
  block code plus parser-owned standard link destinations, and honor escaped
  opening delimiters.
- Keep one static `InvalidInput` reason so CLI and MCP outputs remain stable.
- Add focused renderer unit tests, one CLI-process regression, one raw MCP
  regression, and service no-request coverage for shared write paths.
- Update the Markdown section of `README.md` and align release metadata.

Likely touchpoints: `src/markdown.rs`, `src/service.rs`,
`tests/cli_process.rs`, `tests/mcp_raw_stdio.rs`, `README.md`, `Cargo.toml`,
`Cargo.lock`, and `packaging/mcp/server.json`.

Narrow verification:

```bash
cargo test --locked markdown::tests
cargo test --locked service::tests
cargo test --locked --test cli_process
cargo test --locked --test mcp_raw_stdio
```

### Checkpoint And Delivery Protocol

1. Commit this goal before implementation and obtain retained-reviewer
   readiness on the goal, issue, and baseline.
2. Implement the smallest coherent design, synthetic tests, focused docs, and
   version alignment using Conventional Commit checkpoints.
3. Run narrow verification, freeze main-agent writes, and repair retained
   adversarial review findings until clean.
4. Run `cargo fmt --check`, strict locked Clippy, locked all-target tests,
   release build, Rust 1.88 compatibility, credential scan, package metadata
   checks, and a final diff check.
5. Obtain a clean fresh context-independent audit, mark this milestone done,
   and commit the final local evidence.
6. Push one branch, open one pull request containing `Closes #31`, wait for
   every required check, and squash-merge only the reviewed exact head.
7. Fast-forward local `main`, create and push annotated tag `v0.15.1`, wait for
   the tagged workflow and GitHub Release, then independently verify all six
   assets.
8. Record immutable PR, merge, tag, workflow, release, and asset evidence in
   this goal through a documentation-only finalization pull request if needed.
9. Recheck exact `origin/main`, the issue/PR queue, final CI, and worktree
   cleanliness before completion.

## Progress Notes

- 2026-08-01: Goal established from exact `origin/main` at
  `bd7a0570a9f10ee06124fbdfe4d85e92b7356557`. Issue #31 is the sole open
  issue and therefore the highest-priority delivery.
- 2026-08-01: Local release candidate completed at
  `05f56c4f61924464ea0f84815f39e1058fdd1ef2`. Parsed visible-projection
  validation rejects prose and partial-inline-code constructions while
  preserving whole code examples, escapes, standard destinations, and
  ordinary autolinks. Retained review repaired one range-merge bypass, one
  destination false positive, and one partial-code bypass before declaring
  the whole milestone clean. A separate context-independent final audit was
  also clean.
- 2026-08-01: Final local gates passed: formatter, strict locked all-target
  Clippy, 287 library tests, 12 CLI-process tests, 2 raw-MCP tests, 1 package
  metadata test, release build, Rust 1.88 all-target check, credential scan,
  and native package checksum smoke. The release binary rejected a uniquely
  labeled invalid `sfera` self-DM send locally with the documented diagnostic;
  aggregate readback found zero matching messages, so the smoke left no Slack
  residue. External PR, merge, tag, workflow, release, and asset verification
  remained pending at this checkpoint.
- 2026-08-01: External delivery completed. Ready PR
  [#32](https://github.com/smarzola/lurkline/pull/32) passed both CI matrices
  and squash-merged exact reviewed head
  `ff315a59229aa4f67a8bdb93f209ea5b4d344483` as
  `59d34d5f2f3a9a3e70ddbe6033d34f208d353107`; its `Closes #31` reference
  closed the issue. The merge tree exactly matched the audited head tree.
- 2026-08-01: Annotated tag `v0.15.1` peels to exact merge commit
  `59d34d5f2f3a9a3e70ddbe6033d34f208d353107`. Release workflow
  [30701991258](https://github.com/smarzola/lurkline/actions/runs/30701991258)
  succeeded through tag validation, reusable quality gates, all three native
  builds, artifact-set validation, and publication. The non-draft,
  non-prerelease [GitHub Release](https://github.com/smarzola/lurkline/releases/tag/v0.15.1)
  was published at 2026-08-01T13:41:38Z.
- 2026-08-01: Independent release readback found exactly the expected Linux
  x86-64, Linux ARM64, and macOS ARM64 archives plus their three checksum
  files. Every checksum passed. Each archive contained only its platform
  directory with executable-mode `lurkline`, `README.md`, and `LICENSE`; file
  inspection confirmed the advertised architecture, and the native macOS
  binary reported `lurkline 0.15.1`.
