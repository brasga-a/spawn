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

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No command, argument parser, or binary target (it is a library package with no `[[bin]]`) exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- If logic would be useful to the engine or other tools, put it in the owning crate and call it.
- Do not add third-party dependencies (e.g. an argument parser) without [root: Workspace](../../AGENTS.md#workspace) justification.
