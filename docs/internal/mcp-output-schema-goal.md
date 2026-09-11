# Goal: Make MCP tool discovery compatible with strict clients

Approval: user requested the pursue-goal workflow and “fix this” on 2026-09-11,
with a screenshot reporting 27 dropped tools due to `outputSchema.type`.
This authorizes the bounded schema fix, verification, independent review, and
draft-to-ready PR delivery. Follow the pursue-goal skill for execution.
The user then requested “Release” on 2026-09-11, authorizing merge and patch
release `v0.17.2`, including publication and artifact verification.

Repository: `/Users/smarzola/projects/lurkline`. Starting branch: `main`.
Base: `81ad84fda52c8aa439b2566e524889bc7c22cb4e`; checkout clean and matches remote.
Work branch: `fix/mcp-output-schema`; GitHub base: `main`.
Draft PR: https://github.com/smarzola/lurkline/pull/42.
Commit this goal and evidence, following existing tracked internal goals.

## Baseline and decision

The real v0.17.1 executable advertises 27 tools by default. Every output schema
has `$defs`, `$schema`, `anyOf`, and `title`, but no root `type`. The shared
untagged `ToolOutput<T>` enum generates that schema for all 30 tools, including
the three optional file tools. The current tests only check schema presence and
selected fields. MCP 2025-11-25 requires an object root:
https://modelcontextprotocol.io/specification/2025-11-25/schema#tool.

Add `type: object` through Schemars metadata on the shared output enum. Preserve
the success/error alternatives, all nested schemas, serialized results, tool
names, and safety gates. Do not wrap results or add a schema-rewriting layer.
No storage, API payload, dependency, or Slack permission changes are needed.
All runtime inputs are synthetic; no real Slack credentials or data are read.

Runtime discovery found a second defect: the default schema generator describes
deserialization, making `RenderedMessage.outbound_mentions` required even when
serialization omits it. The official SDK rejects the successful render result.
Generate all tool output schemas with Schemars' serialization contract through
one shared helper. This corrects the schema without changing result payloads.

## Delivery and success criteria

1. Every tool returned by actual `tools/list` has an explicit object output root,
   in default and fully enabled configurations. Existing input schemas remain
   valid. Add assertions to the existing raw stdio tests to catch missing root
   types and a schema that requires the omitted `outbound_mentions` field.
2. Launch the real MCP executable through an official TypeScript SDK client.
   Default discovery returns 27 accepted tools; fully enabled discovery returns
   30. A local Markdown render succeeds, invalid input returns a structured
   error, and blocked writes still fail before Slack access. Validate returned
   structured results against advertised schemas. Use a scratch SDK installation.
3. Formatting, strict locked Clippy, all-target tests, release build, credential
   scan, and published-head CI (including Rust 1.88) pass.
4. One fresh independent Sol reviewer inspects the full diff and personally
   launches the executable for discovery and calls. No material findings remain.
   The reviewer may use disposable runtime files but cannot edit tracked source.
5. Keep the PR draft during implementation/review; mark ready after verification
   and published-head checks pass. Record any separately authorized delivery.
6. Publish `v0.17.2` from the reviewed product merge. Align Cargo and MCP version
   metadata, verify final PR and tagged-source checks, verify all three platform
   archives and checksums, and run the published macOS binary through the SDK
   discovery/result-validation walkthrough. Preserve existing review evidence
   while product source remains unchanged.

Both roles should use the simplest complete fix and proportional verification.
The wire-level regression closes an actual protocol-validation gap; no new
standalone test framework or production dependency is needed.

## Status and evidence

- [x] Shared schema fix and raw stdio regression pass.
- [x] Required local checks and implementer runtime acceptance pass.
- [x] Independent review and reviewer runtime acceptance pass.

The linked PR records published-head CI and draft/ready status. Mark it ready
only after the final published head passes the required checks.

Baseline runtime: real `target/release/lurkline mcp`, version 0.17.1;
27 tools returned, 27 missing object output roots, clean EOF shutdown.
Implementation evidence:

- The added stdio assertions failed on missing object roots before the fix in
  both configurations. A second regression failed on required `outbound_mentions`
  before the serialization-contract fix. Both now pass.
- Format, strict locked Clippy, all-target tests (315 library, 13 CLI, 2 raw MCP,
  1 metadata), locked release build, credential scan, and diff checks passed.
  No new dependency was added to the repository.
- Raw evidence: `/tmp/lurkline-mcp-schema-{red,green,clippy,tests,build}.log`,
  `/tmp/lurkline-mcp-output-contract-red.log`, and
  `/tmp/lurkline-mcp-sdk-{red,green}.log`.

Implementer runtime: personally launched the built release executable on macOS
ARM64 using `/tmp/lurkline-mcp-sdk-check/walkthrough.mjs`, official MCP TypeScript
SDK 1.27.1, and Ajv 8 in a disposable scratch installation. The published v0.17.1
baseline was rejected at `outputSchema.type` in both configurations. The patched
binary returned 27/30 accepted tools; all schemas compiled and rejected null and
scalar results. Markdown rendering returned valid structured output; invalid
user input and blocked/unconfirmed writes returned schema-valid errors. A
deliberately mistyped render result was rejected. No Slack requests or writes
were needed; server diagnostics stayed empty and the disposable file root was
removed. The SDK's validator logs ignored Rust integer format annotations; JSON
integer/range constraints still apply. The scratch driver initially needed a
canonical file-root path and the actual draft argument shape; these were fixed.

Reviewer runtime: fresh Sol reviewer personally built and launched
`ecdb999b3f0f5dfe0671f22303ff9ed9adb12d7e` with the official SDK on macOS ARM64.
Discovery accepted 27 default and 30 fully enabled tools; all schemas compiled
and rejected non-object outputs. Render success and invalid-input/write-guard
errors validated, and a mistyped result was rejected. No Slack access occurred,
server diagnostics were empty, temporary data was removed, and the worktree
remained clean. Source review confirmed unchanged payloads and permissions.

Final review: no material blocking findings; no substantial avoidable complexity
or ineffective tests. Implementation commit: `ecdb999b3f0f5dfe0671f22303ff9ed9adb12d7e`.
Current status: implementation and both runtime walkthroughs complete. The PR
is the source of truth for the final CI results and readiness transition.

## Release v0.17.2

PR #42 was verified ready at `57fd025` with all six CI checks passing before
release approval. It returns to draft for the version update. Product source is
unchanged from the independently reviewed `ecdb999` implementation.
Cargo and MCP metadata now agree on `0.17.2`. Formatting, strict Clippy, all
331 tests, release build, credential scan, and whitespace checks passed again.
A focused independent review found only the intended metadata and authorization
changes. The reviewer personally confirmed `lurkline --version` and MCP
`serverInfo.version` both report `0.17.2`; the prior full implementation review
and SDK walkthrough remain applicable because product source is unchanged.

The linked PR records final-head CI, readiness, and merge. The
[v0.17.2 release notes](https://github.com/smarzola/lurkline/releases/tag/v0.17.2)
are the delivery record for the product tag, release workflow, platform assets,
and post-download verification required by criterion 6. This keeps external
delivery evidence with the published artifacts.
