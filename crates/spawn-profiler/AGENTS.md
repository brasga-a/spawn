# spawn-profiler

Local rules for `spawn-profiler` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Observability infrastructure: frame and system timing, counters, structured spans/logging, and diagnostic output (a `tracing`-style approach is a candidate, unadopted).

## Responsibilities

- Timing scopes, counters, and statistics (PLANNED).
- Structured logging/span integration and export to developer tools (e.g. editor panels, trace files).
- Debug-overlay data sources (data only).

## Non-Responsibilities

- Rendering overlays (`spawn-render`/`spawn-ui`), editor UI (`spawn-editor`), gameplay metrics, error handling policy of other crates.

## Architectural Boundaries

- Foundation-level: depends on no higher `spawn-*` crate; any crate may use it. Must be removable/no-op in release configurations so subsystems are not forced to link heavy tooling.

## Public API Guidelines

- Minimal macros/functions; zero-cost when disabled; stable names for spans used by tools.

## Internal Design Principles

- Thread-local buffering; no global locks on the hot path.

## Performance and Safety

- Overhead per scope measured and documented; no allocation per sample in steady state; no sensitive data (paths, user info) in default output.

## Testing Requirements

- Tests for aggregation correctness and disabled-mode no-op behavior; overhead benchmarks.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` is the unmodified `cargo new --lib` stub (an `add` function and an `it_works` test); `Cargo.toml` declares no dependencies and inherits `version`/`edition` from the workspace. No timer, counter, or span exists. The stub is not API and must not be depended on or preserved.

## Agent Instructions

- Build only when the first real system needs instrumentation ([root: Engineering Standards](../../AGENTS.md#engineering-standards)); no speculative framework.
- Do not add gameplay-specific metrics.
- Proposed decisions affecting this crate (follow them once accepted; until then, do not implement contrary designs): [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (logging facade), [ADR-0020](../../docs/adr/0020-memory-strategy.md) (allocation tracking).
