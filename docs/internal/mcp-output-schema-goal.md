# Goal: Make MCP tool discovery compatible with strict clients

Approval: user requested the pursue-goal workflow and “fix this” on 2026-09-11,
with a screenshot reporting 27 dropped tools due to `outputSchema.type`.
This authorizes the bounded schema fix, verification, independent review, and
draft-to-ready PR delivery. Follow the pursue-goal skill for execution.

Repository: `/Users/smarzola/projects/lurkline`. Starting branch: `main`.
Base: `81ad84fda52c8aa439b2566e524889bc7c22cb4e`; checkout clean and matches remote.
Work branch: `fix/mcp-output-schema`; GitHub base: `main`; draft PR pending.
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

## Delivery and success criteria

1. Every tool returned by actual `tools/list` has an explicit object output root,
   in default and fully enabled configurations. Existing input schemas remain
   valid. Add assertions to the existing raw stdio tests to catch this defect.
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

Both roles should use the simplest complete fix and proportional verification.
The wire-level regression closes an actual protocol-validation gap; no new
standalone test framework or production dependency is needed.

## Status and evidence

- [ ] Shared schema fix and raw stdio regression pass.
- [ ] Required checks and implementer runtime acceptance pass.
- [ ] Independent review and reviewer runtime acceptance pass.
- [ ] Published PR head passes CI and is ready for review.

Baseline runtime: real `target/release/lurkline mcp`, version 0.17.1;
27 tools returned, 27 missing object output roots, clean EOF shutdown.
Implementer runtime: pending. Reviewer runtime: pending. Final review: pending.
Current status: baseline reproduced; implementation pending.
