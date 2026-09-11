# Goal: Restore MCP client compatibility with rmcp 3 and release v0.18.0

Repository: `/Users/smarzola/projects/lurkline`.
Source of truth: https://github.com/smarzola/lurkline/issues/43 and this plan.
Plan revision: 2, updated 2026-09-11 after the user proposed a minor release.
Approval: **Approved** on 2026-09-11. The user approved revision 2 with
"Ensure that you test the mcp as you were using it. Approved". Scope includes
implementation, personal MCP usage, independent review, merge, and v0.18.0 release.
Execution: approved implementation and delivery in progress.
Goal-file commit policy: commit this goal and evidence, following the comparable
tracked internal goals.
PR delivery: GitHub `smarzola/lurkline`, base `main`, planned branch
`fix/issue-43-rmcp-3`; draft PR https://github.com/smarzola/lurkline/pull/44.

## Outcome and delivery plan

Current MCP clients can connect to the released Lurkline executable over stdio,
discover its tools, and invoke them. Existing clients retain working discovery,
schema validation, and calls. Slack permissions and guarded writes retain their
current behavior.

| Step | Deliverable | Success condition |
|---|---|---|
| 1 | Upgrade both rmcp dependencies and adapt affected API uses | Cargo locks rmcp 3.3.0; the project builds on its declared Rust 1.88 minimum |
| 2 | Verify current and legacy MCP behavior | Actual stdio discovery and calls work through both lifecycles; schema and write-guard checks pass |
| 3 | Review and publish the minor release | Independent review and CI pass; v0.18.0 is published and downloaded artifacts are verified |

## Decisions for human review

| Decision and origin | Proposed choice | Why and tradeoff | Alternative |
|---|---|---|---|
| SDK target, requested by issue #43 | Set runtime and dev requirements to `3.3`, lock 3.3.0, retain existing feature selection | Latest verified stable 3.x includes lifecycle compatibility fixes; a major upgrade requires API and runtime checks | An older 3.x release omits subsequent fixes; maintaining a 2.x fork adds ongoing work |
| Compatibility, agent proposal based on upstream changes | Exercise both the 2026-07-28 lifecycle and existing 2025-11-25 initialization; retain object output schemas and serialization contracts | Serves newer clients while preserving strict older clients; needs two protocol paths in verification | Testing only the new Rust client risks missing old-client regressions |
| Scope, repository constraint | Make the smallest necessary SDK/API adaptations through the existing MCP adapter and typed service layer | Limits behavior changes; no new transports, Slack capabilities, credentials format, or optional SDK features | A broader MCP redesign is unnecessary for this issue |
| Release, user request and minor-version proposal | Publish v0.18.0 after review and checks | Newer MCP protocol support and a major SDK migration justify a distinct minor release while preserving existing client behavior | A patch could describe a narrow compatibility repair, but communicates the protocol expansion less clearly |

## Baseline, scope, and constraints

- Clean `main` matched `origin/main` at
  `a28393914b5ca971331d5a1826e71cd8c860088d`; package version 0.17.2.
- Issue #43 is the sole open issue; no open PRs were found during discovery.
- `Cargo.toml` declares rmcp 2.2 for the server and test client, with
  `transport-io` and `client` respectively. `Cargo.lock` resolves rmcp 2.2.0 and
  Schemars 1.2.1. The project declares Rust 1.88.
- `src/mcp.rs` uses SDK handler/router macros and stdio transport. Its shared
  output-schema helper describes serialization and explicitly declares an
  object root. Tool discovery exposes 27 default tools and 30 with all current
  options enabled.
- `tests/mcp_raw_stdio.rs` currently uses protocol 2025-11-25. Existing tests
  cover tool discovery, structured results, optional tools, and guarded writes.
- Upstream 3.3.0 declares Rust 1.88. The 3.x changelog includes the 2026-07-28
  lifecycle and model changes; 3.2.0 fixes legacy `initialize` compatibility.
  Sources: https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.3.0
  and https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.3.0/crates/rmcp/CHANGELOG.md.
- `.github/workflows/ci.yml` covers formatting, strict Clippy, all-target tests,
  release build, credential scanning, packaging, Rust 1.88, and macOS ARM64.
  `.github/workflows/release.yml` publishes three platform archives and checksums.

Unknowns: issue #43 gives no named failing client or captured protocol exchange.
Do not claim that its exact failure has been reproduced. Resolve API adaptation
needs by compiling after approval. Establish a concrete baseline by exercising
the new lifecycle against the old binary, then repeat against the upgraded
binary. Verify with a current official SDK client and a strict legacy client;
record exact versions and negotiated protocol behavior.

Protected work: no unrelated changes observed. Use synthetic fixtures and owned
temporary state. Never print or retain real Slack credentials or user data.

## Approval and authority

Revision 2 received explicit approval before product edits, branch creation,
commits, or PR publication. Proceed autonomously through implementation, verification, a
fresh independent review, draft-to-ready PR delivery, merge, tag, and release.
The user's explicit request to release supplies the external delivery authority
once this plan is approved; routine implementation choices need no further gate.

Open the draft PR promptly after approval using the approved goal checkpoint or
smallest meaningful implementation commit. Keep it draft while work remains.
Material changes to outcome, compatibility, architecture, or scope need approval
before dependent work. Do not silently raise Rust requirements or drop legacy
client support to make the upgrade pass.

## Engineering, verification, and review

Use direct code and existing patterns. Avoid new abstractions, production
dependencies, transports, or configuration unless required by this migration.
Do not add tests that merely inspect dependency version strings or mirror SDK
internals. Reuse valuable existing coverage. Add a focused wire-level regression
only for a demonstrated lifecycle/negotiation gap, with meaningful discovery or
call assertions; retain existing schema and guarded-write coverage.

Required commands, from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
rustup run 1.88.0 cargo check --locked --all-targets
python3 scripts/check-no-secrets.py
git diff --check
```

Also run the existing release-package smoke procedure and final published-head
CI. Repeat checks only for changed code, inputs, environment, stale evidence,
failures, or unresolved concerns. No unrelated cleanup.

One fresh independent Sol reviewer (`gpt-5.6-sol`) must inspect the final diff
for correctness, completeness, simplicity, and test quality, and independently
perform the runtime acceptance below. The reviewer may build and create owned
scratch artifacts but must not edit tracked source or git state. Resolve all
material findings. Do not treat missing review or acceptance as a pass.

## Hands-on runtime acceptance

Both implementer and reviewer personally launch the actual release executable,
using official SDK consumers in disposable scratch projects and synthetic auth.
The new Rust SDK client can exercise the modern lifecycle; retain an independent
strict TypeScript client for legacy schema/result compatibility. Record actual
client versions, launch commands, protocol paths, and observed results.

| Scenario | Expected observation |
|---|---|
| Modern stdio connection/discovery and legacy initialization | Successful negotiation followed by working tool discovery and calls |
| Default and fully enabled configurations | 27 and 30 tools respectively, with valid object input/output schemas |
| Local Markdown rendering | Successful structured result matches its advertised schema |
| Invalid input | Structured validation error remains valid for the advertised output schema |
| Write without `--allow-write`, or a guarded mutation without confirmation | Existing denial occurs before Slack access |
| EOF/shutdown | Clean process termination with protocol-only stdout |

No real Slack request is required for these scenarios. Canonicalize disposable
file roots on macOS and clean owned temporary runtime state after use. Running
tests or reading the other role's report does not replace personal acceptance.

## Milestones and completion

- [x] Revision 2 approved; draft PR #44 opened from goal checkpoint `a2581c5`.
- [x] SDK migration, meaningful regression coverage, and implementer acceptance pass.
- [ ] Required checks and fresh independent review/acceptance pass.
- [ ] Final PR head passes CI, PR is ready and merged, and v0.18.0 is published.
- [ ] All three downloaded archives match their checksums and expected contents;
  the published macOS binary reports v0.18.0 and passes stdio discovery/calls.

Use focused Conventional Commits, stage explicit paths, and inspect staged
diffs. Keep concise implementation/check evidence here. Rewrite the final PR
description around the verified result and close issue #43 through that PR.
Release notes record the source commit, workflow results, asset verification,
and published-binary acceptance. Keep documentation precise and understandable
without this conversation, following the repository's writing conventions.

Implementer runtime: passed; actual SDK consumers launched and used both binaries.
Reviewer runtime and final review: pending.
Goal status: implementation and local verification complete; independent review pending.
PR/release status: PR #44 remains draft; release pending review and CI.

## Implementation and verification evidence

Both dependencies and the lockfile now use rmcp 3.3.0. The only Rust API adaptation
is in the existing test: discovered server identity is now optional in the SDK,
so assert its presence before checking the name and version. Runtime adapter,
schemas, service behavior, and safety flags are unchanged. Cargo and packaged
MCP metadata both report 0.18.0. README documents current and legacy support.

Added one raw stdio regression for modern discovery without `initialize`, tool
listing, successful rendering, invalid input, write denial, modern result shape,
and clean EOF. Existing legacy coverage now asserts negotiated 2025-11-25 and
the absence of the modern `resultType` field on discovery/call results.

Local verification on macOS ARM64 passed: formatting, strict locked Clippy,
all 332 tests (315 library, 13 CLI, 3 raw MCP, 1 package metadata), locked release
build, Rust 1.88 all-target check, credential scan, whitespace check, and release
packaging/checksum smoke test. Logs are under `/tmp/lurkline-issue43/`:
`tests-final.log`, `clippy-final.log`, `build.log`, and `msrv.log`.

Personal runtime acceptance used the actual v0.17.2 baseline and upgraded release
binary through two public SDK clients:

- Official Rust rmcp 3.3.0 consumer at `/tmp/lurkline-issue43/client/`, launched
  as `client/target/debug/lurkline-mcp-acceptance <absolute-binary-path>`.
  With `--baseline`, v0.17.2 closes modern discovery and exits 1 in both default
  and fully enabled configurations. The upgraded executable negotiates
  2026-07-28, reports v0.18.0, discovers 27/30 tools, renders Markdown into rich
  text, returns `invalid_input` for an empty query, and returns
  `write_not_allowed` or `confirmation_required` for guarded operations.
  Both upgraded processes exit 0 with empty stderr.
- Official TypeScript SDK 1.27.1 with Ajv 8.20.0, launched as
  `node /tmp/lurkline-mcp-sdk-check/walkthrough.mjs <absolute-binary-path>`.
  Baseline and upgraded binaries both pass strict legacy discovery: 27/30 tools,
  valid object schemas and structured successes/errors, with a deliberately
  mistyped result rejected. Write denials remain intact. Rust integer format
  annotations produce ignored-format validator diagnostics, as on the baseline.

Runtime logs: `modern-baseline.log`, `modern-upgraded.log`,
`legacy-baseline.log`, and `legacy-upgraded.log`. Both consumers remove inherited
Slack configuration and use synthetic inputs; no live Slack request is needed.
Owned file roots are removed after use. This reproduces the modern-discovery
compatibility failure, without claiming the issue's unnamed client was tested.
