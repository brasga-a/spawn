# spawn-editor

Local rules for `spawn-editor` (tool). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Graphical development tools: scene editing, inspectors, asset browsing, and viewport integration. Development tooling, never part of a shipped game runtime.

## Responsibilities

- Editor application shell, panels, inspectors, and scene/asset editing workflows (PLANNED).
- Viewport integration displaying engine rendering output.
- Editing operations expressed as engine-level commands with undo/redo.
- Editor UI framework: `egui` is permitted here (unadopted).

## Non-Responsibilities

- The game runtime UI (`spawn-ui`) and HUDs; the editor's UI framework must never leak into it.
- Scene format, validation, asset import, and world logic (owned by `spawn-scene`, `spawn-assets`, `spawn-world`); CLI workflows (`spawn-cli`); MCP protocol (`spawn-mcp`).

## Architectural Boundaries

- Depends on engine crates; no engine crate may depend on `spawn-editor` or `egui`. Game builds must compile and link without it.
- Uses `spawn-reflect` metadata for inspectors; does not receive privileged access to engine internals beyond public APIs.

## Public API Guidelines

- Extension points for editor plugins are `PLANNED`; do not design them speculatively. Shared editing operations used by `spawn-mcp` should live in a layer both can use, decided explicitly.

## Internal Design Principles

- Editor state is separate from game state; edits go through commands so they can be validated, undone, and automated.

## Performance and Safety

- Loading untrusted projects/assets must not crash the editor; autosave/recovery must not corrupt data.

## Testing Requirements

- Headless tests of command/undo logic and data models; UI rendering is not required in CI.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No editor UI, viewport, or inspector exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Never add editor-only types or `egui` to engine crates.
- Do not create sibling directories that duplicate this package; workspace membership is only `tools/*`.
