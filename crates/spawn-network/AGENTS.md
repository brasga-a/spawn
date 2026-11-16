# spawn-network

Local rules for `spawn-network` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Networking infrastructure for the server-authoritative model: transport, connections, messaging, replication, and client prediction/reconciliation support.

## Responsibilities

- Transport and connection management, reliability/ordering channels (PLANNED).
- Serialization of network messages with versioning and bounds.
- Replication: snapshots, delta compression, interest management, bandwidth budgeting (design for these; build after basic connectivity works).
- Support for client prediction and reconciliation, input command transport, time synchronization.

## Non-Responsibilities

- Gameplay rules, hit/damage decisions, game modes (frameworks/Luau server scripts).
- Physics simulation (`spawn-physics`), input sampling (`spawn-input`), ECS storage (`spawn-ecs`), authoritative world state ownership (`spawn-world`).
- Sockets exposed to scripts: Luau gets no network access.

## Architectural Boundaries

- May depend on `spawn-math`, and use ECS/reflect contracts for replicated data; must not depend on render, audio, ui, or scripting internals.
- Clients send intents, never state; this crate must not provide a path for client-authored outcomes.

## Public API Guidelines

- Message types are versioned plain data; replicated state descriptors are explicit about who owns, predicts, and replicates each field.

## Internal Design Principles

- Fixed-timestep tick numbering; transport abstraction allows in-memory loopback for tests and listen-server.

## Performance and Safety

- Every packet is untrusted: bounded allocation, no panics, rate limits, replay/spoof resistance as designed; bandwidth is budgeted.

## Testing Requirements

- Loopback/simulated-latency/loss tests; serialization round-trip and fuzz tests; prediction/reconciliation scenario tests.

## Governing Decisions

- **Accepted (binding):** [ADR-0019](../../docs/adr/0019-anti-cheat-and-client-trust.md) (anti-cheat and client trust), [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (fixed-step loop, tick index, threading), [ADR-0010](../../docs/adr/0010-math-library.md) (math and spatial conventions). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0016](../../docs/adr/0016-networking-and-replication.md) (networking and replication).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `socket2`, `chacha20poly1305`, `x25519-dalek`, `getrandom`, `lz4_flex`. Any other crate needs owner approval first.

## Current Implementation Status

`SCAFFOLD`. `src/lib.rs` contains only a crate-level doc comment (no items, no tests); `Cargo.toml` declares no dependencies and inherits `version`, `edition`, `license`, `rust-version` and lints from the workspace. No transport, connection, or replication code exists.

## Agent Instructions

- Answer 'what does the server do, what does the client predict, what is replicated' in design notes for any new feature.
- Do not add gameplay semantics; do not add dependencies without [root: Workspace](../../AGENTS.md#workspace) justification.
