# spawn-mcp

Local rules for `spawn-mcp` (tool). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Model Context Protocol integration so external AI agents and development tools can inspect and drive the engine/editor through controlled capabilities. Optional development infrastructure, never a runtime requirement.

## Responsibilities

- MCP server and tool/resource definitions (PLANNED).
- Mapping each exposed capability onto stable, validated engine or editor APIs; schemas and error reporting for clients.
- Authorization/scoping of exposed capabilities (explicit allowlist, default deny).

## Non-Responsibilities

- Engine behavior, validation, or editing logic (owned by engine crates/`spawn-editor`); it must not duplicate it.
- Luau scripting API (`spawn-scripting`) and any in-game remote control.

## Architectural Boundaries

- Depends on engine/editor APIs, never the reverse; absent from game and server builds by default.
- Must not bypass validation or the Rust/Luau boundary to be convenient; no raw memory, filesystem, or shell access exposed to clients.

## Public API Guidelines

- Each MCP tool has a documented schema, semantics, errors, and version; breaking changes are versioned.
- Capabilities are narrow and task-oriented rather than generic escape hatches.

## Internal Design Principles

- Stateless where possible; all inputs validated; operations that mutate data are explicit and auditable.

## Performance and Safety

- Treat all client input as untrusted: sizes, types, ranges, paths; destructive operations need explicit scoping; do not expose secrets or personal data.

## Testing Requirements

- Protocol-level tests with a mock client; schema validation tests; negative tests for out-of-scope requests.

## Governing Decisions

- **Accepted (binding):** [ADR-0008](../../docs/adr/0008-third-party-license-policy.md) (license policy). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `rmcp` (confirm maturity before adoption). Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No MCP server, tool definition, or protocol code exists.

## Agent Instructions

- Do not add a capability without a corresponding engine/editor API and tests.
- Do not add MCP dependencies to any engine crate.
