# spawn-ui

Local rules for `spawn-ui` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

In-game UI runtime: layout, widgets, text, HUD infrastructure, UI input routing, and integration with rendering. Independent of the editor's UI framework.

## Responsibilities

- UI tree, layout engine, styling, text, widgets, and focus/navigation (PLANNED).
- HUD infrastructure for games to build overlays on.
- Producing draw data for `spawn-render`; consuming semantic input from `spawn-input`.
- Binding points that client presentation scripts use via `spawn-scripting`.

## Non-Responsibilities

- Editor panels/inspectors and any `egui` usage (`spawn-editor`); GPU execution (`spawn-render`); raw input devices (`spawn-input`/`spawn-platform`); gameplay HUD content such as ammo or scoreboards (games/frameworks).

## Architectural Boundaries

- May depend on `spawn-math` and, through contracts, render and input; must not depend on `egui`, `spawn-editor`, physics, or gameplay crates. Game runtime must never depend on the editor's UI stack.

## Public API Guidelines

- Retained or immediate model undecided: record before implementing. Widgets are data-describable for hot reload.

## Internal Design Principles

- UI is presentation-only: it never mutates authoritative game state; it emits intents/events.

## Performance and Safety

- Layout cost bounded; text/style data from scripts or mods is untrusted and validated.

## Testing Requirements

- Headless layout tests; snapshot tests of draw lists; input-routing tests.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No layout, widget, or HUD code exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Do not import `egui` or editor code here.
- Do not add game-specific HUD elements.
