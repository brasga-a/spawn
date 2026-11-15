# ADR-0016 — Networking Transport and Replication Model

**Status:** Proposed  
**Scope:** `spawn-network`; roadmap step 6 and later scale work.  
**Related:** [ADR-0003](0003-fixed-step-loop-and-threading.md) (tick index), [ADR-0005](0005-physics-strategy.md) (lag compensation), [ADR-0011](0011-ecs-architecture.md) (change detection), [ADR-0019](0019-anti-cheat-and-client-trust.md) (client trust).

## Context

Multiplayer is foundational and server-authoritative (root [Multiplayer](../../AGENTS.md#multiplayer-foundational)). The validation project targets large battles (tens of players, vehicles) with responsive movement and fair hit registration. Clients send inputs, never state.

## Decision

### Transport

1. **UDP with an in-house protocol:** connection handshake, sequence numbers and acknowledgements, per-channel delivery modes (unreliable, unreliable-sequenced, reliable-ordered), fragmentation of large messages, and congestion/bandwidth control.
2. **Security:** connect tokens issued by a backend (netcode.io-style) to prevent spoofed connections; packets encrypted and authenticated with an audited AEAD from a reviewed crate (e.g. RustCrypto ChaCha20-Poly1305). **No in-house cryptographic primitives.**
3. **Loopback transport** for listen-server and tests.

### Replication

4. **Server tick = simulation tick** (60 Hz default, [ADR-0003](0003-fixed-step-loop-and-threading.md)). The snapshot send rate per client is configurable (e.g. 30 or 60 Hz) and adapts to bandwidth.
5. **Snapshots with delta compression** against the last snapshot acknowledged by each client (Quake 3 model), using ECS change detection to find modified state.
6. **Client prediction and reconciliation** for the local player's movement: client keeps unacknowledged inputs, server returns authoritative state with the last processed input tick, client rewinds and replays (requires deterministic movement, [ADR-0005](0005-physics-strategy.md)).
7. **Interpolation** of remote entities with a buffer of a few snapshots (~2 send intervals by default, tuned by measurement).
8. **Lag compensation** for hitscan on the server: rewind to the shooter's view time within a maximum rewind window (default ~200 ms), using physics pose history.
9. **Interest management:** per-client relevancy (spatial grid plus game-provided rules) and a priority accumulator, so each snapshot fits a per-client bandwidth budget.
10. **Replicated state declaration:** each replicated component declares ownership, prediction, quantization and priority. Quantization (positions, angles) is explicit.

## Alternatives

- **QUIC (`quinn`):** encryption and streams built in, plus unreliable datagrams; TLS stack and complexity are heavy for a game protocol and reduce control over packet layout.
- **GameNetworkingSockets (Valve, BSD-3), ENet, netcode.io / yojimbo (BSD-3):** proven C/C++ libraries; references, not dependencies.
- **TCP or WebSockets:** head-of-line blocking; unacceptable for real-time movement.
- **Deterministic lockstep:** minimal bandwidth, requires full determinism and waits for the slowest player; unsuitable for large FPS battles.

## References

Glenn Fiedler's "Networking for Game Programmers" and netcode.io (BSD-3), Valve Developer Wiki "Source Multiplayer Networking" and "Latency Compensating Methods", "Overwatch Gameplay Architecture and Netcode" (GDC 2017), "I Shot You First: Networking the Gameplay of Halo: Reach" (GDC 2011), Tribes networking model paper, Quake 3 networking (GPL: study only, ADR-0008).

## Consequences

- Every gameplay feature declares server/client ownership, replication and prediction (root rule) using the declaration mechanism of point 10.
- A backend service issuing connect tokens is needed for public servers (separate decision).

## Risks

- **Protocol bugs** (reliability, fragmentation) are hard to diagnose. Mitigation: simulated loss/latency/jitter tests, packet capture tooling, fuzzing of packet parsing.
- **Bandwidth at scale.** Mitigation: budgets measured with bot clients at target player counts.
- **Lag-compensation fairness** ("shot behind cover"). Mitigation: bounded rewind window; tuning by play-testing.

## Revisit triggers

Measured bandwidth or CPU limits at target player counts; a platform requiring a specific transport.

## Acceptance conditions

Owner review before roadmap step 6.
