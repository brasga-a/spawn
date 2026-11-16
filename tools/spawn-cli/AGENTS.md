# spawn-cli

Local rules for `spawn-cli` (tool). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Command-line tooling for development workflows: project creation, building, running, validation, and asset-related tasks. Development tooling, not engine runtime.

## Responsibilities

- Project scaffolding, build/run orchestration, and project validation commands (PLANNED).
- Asset and data validation/import commands that invoke `spawn-assets`, `spawn-scene`, and related crates headlessly.
- Exit codes and machine-readable output suitable for CI.

## Non-Responsibilities

- Reimplementing engine logic: parsing, validation rules, and import behavior live in engine crates; the CLI calls them.
- Graphical authoring (`spawn-editor`), MCP protocol handling (`spawn-mcp`), runtime play of games (games link the engine themselves).

## Architectural Boundaries

- Depends on engine crates, never the reverse. Must not be depended on by engine crates. Headless: must not require `spawn-platform` windowing or `spawn-render`, except for explicitly graphical commands.
- Any capability shared with the editor or MCP server belongs in an engine crate or a deliberately agreed shared layer, not copy-pasted between tools.

## Public API Guidelines

- The CLI surface (commands, flags, output formats) is the public contract: version it, document it, keep it stable; the library portion is for tool reuse only and stays minimal.

## Internal Design Principles

- Thin command layer over engine APIs; commands are non-interactive by default and scriptable.

## Performance and Safety

- Treat project files and inputs as untrusted; destructive operations (overwrite/delete) require explicit flags or confirmation; no network calls without explicit command intent.

## Testing Requirements

- Integration tests invoking commands on temp project directories; golden output tests; error-path and exit-code tests.

## Governing Decisions

- **Accepted (binding):** [ADR-0015](../../docs/adr/0015-asset-pipeline.md) (asset pipeline), [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md) (graphics API), [ADR-0013](../../docs/adr/0013-declarative-data-format.md) (data format). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0014](../../docs/adr/0014-shader-toolchain.md) (shader toolchain), [ADR-0024](../../docs/adr/0024-packages-and-game-extensions.md) (packages and extensions), [ADR-0025](../../docs/adr/0025-mods-and-workshop.md) (mods and Workshop).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `clap`, `gltf`, `ufbx` bindings, `meshopt`, `mikktspace`, `png`, `zune-jpeg`, `exr`, `intel_tex_2`, `rspirv`, `notify`, `slangc` (external, license to verify). Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No command, argument parser, or binary target (it is a library package with no `[[bin]]`) exists.

## Agent Instructions

- If logic would be useful to the engine or other tools, put it in the owning crate and call it.
- Use only crates approved for this tool (see Governing Decisions); native-code importers (ufbx) and decoders stay confined to this tool.
