# ADR-0005 — Physics Strategy: In-House Collision, Queries and Character Controller

**Status:** Accepted (scope: collision, spatial queries and character controller). Rigid-body dynamics are deferred to an evaluation described below.  
**Scope:** `spawn-physics`.  
**Related:** [ADR-0003](0003-fixed-step-loop-and-threading.md) (fixed step, tick index), [ADR-0004](0004-in-house-graphics-api.md) (reference hardware).

## Context

For an FPS, "physics" covers three parts with different weight:

| Part | What it is | Impact on game feel |
| --- | --- | --- |
| Collision and spatial queries (raycast, sweep, overlap) | Hitscan, line of sight, ground detection | Very high |
| Character controller | Kinematic movement: walk, jump, slide, step up, slopes | Very high: defines movement feel |
| Rigid-body dynamics | Loose objects, ragdolls, vehicles, destruction | High for large-scale combat, but not for aiming and movement |

Shipped FPS games typically write character movement themselves even when they use a physics engine for dynamics, because movement must be predicted on the client and reconciled with the server (same code, same fixed step, deterministic), tuned precisely, and independent of the rigid-body solver.

## Decision

1. **In-house, now:** collision detection (broadphase, narrowphase, shapes, layers/filters), spatial queries, and a kinematic character controller in `spawn-physics`. This is what roadmap step 4 requires.
2. **Character controller split:**
   - `spawn-physics` owns the generic mechanism: collide-and-slide, step-up, slope limits, ground detection, depenetration.
   - The FPS framework owns genre rules: acceleration curves, sprint, slide, mantle, movement tuning.
3. **Lag-compensation support:** `spawn-physics` provides the mechanism for spatial queries against historical poses (a bounded history indexed by tick, see [ADR-0003](0003-fixed-step-loop-and-threading.md)). The hit-registration rules that use it belong to the FPS framework on the server.
4. **Determinism for movement:** code used in prediction/reconciliation must produce identical results on Windows and Linux for the same inputs. Transcendental and approximate functions (`sin`, `cos`, `atan2`, `exp`, …) differ between platform math libraries (MSVC vs. glibc), so this code uses the deterministic subset of `spawn-math` ([ADR-0010](0010-math-library.md)). Compiler transformations that change floating-point results (e.g. automatic FMA contraction) are controlled for that code. Full-simulation determinism is still not assumed (root [Multiplayer](../../AGENTS.md#multiplayer-foundational)).
5. **Rigid-body dynamics: deferred.** No solver is written and no physics library is added until the dynamics milestone (props, ragdolls, vehicles). At that milestone, the evaluation below decides the approach.
6. **Destruction:** not a goal for the first milestones; desirable later. Collision and dynamics designs must not preclude it (e.g. collision shapes that can be replaced at runtime).

## Deferred evaluation: rigid-body dynamics

Run at the dynamics milestone, with results recorded in a new ADR:

| Candidate | Description |
| --- | --- |
| **In-house Rust solver, Jolt as reference** | Rust implementation that follows Jolt's design (solver, broadphase, islands, job model) |
| **Jolt** (C++, MIT) through bindings | Used in shipped AAA titles and integrated into Godot; fast, multi-threaded, wheeled and tracked vehicles, deterministic |
| **Rapier** (Rust, Apache-2.0) | Pure Rust, optional cross-platform determinism, serializable state |

Benchmark scenarios, run on the reference hardware: stacking stability, thousands of active bodies, ragdolls, wheeled and tracked vehicles, sleeping/waking, and step-time scaling with thread count. Metrics: step time, stability, determinism (same inputs → same state), memory, integration cost. The decision weighs measured results against the maintenance cost of each option.

## References

| Reference | Use | License note |
| --- | --- | --- |
| Jolt Physics source | Primary reference for solver, broadphase, character controller, vehicles | MIT. A Rust implementation that follows Jolt's code closely is a derivative work: keep Jolt's copyright and license notice |
| Rapier / Parry | Rust-idiomatic APIs, query design, determinism mode | Apache-2.0 |
| Box2D v3 and Erin Catto's articles | Solver design, soft step, continuous collision | MIT |
| *Real-Time Collision Detection* (Christer Ericson) | Collision algorithms, GJK, spatial structures | Book |
| Quake / Source movement code | Movement-feel concepts | **GPL / Source SDK license: study only, never copy** |
| Havok, PhysX documentation | Feature set and API concepts of AAA engines | Havok proprietary (docs only); PhysX BSD-3 |

## Alternatives

- **Integrate a full physics engine for everything now:** fastest to dynamics, but the most important parts for an FPS (movement, queries) would still need custom work and would be coupled to a third-party solver.
- **In-house dynamics now:** possible, but builds a large system before any feature needs it (root rule: no infrastructure without a concrete requirement).
- **Havok:** used by major FPS engines, but proprietary and paid; excluded by the no-royalties rule.

## Consequences

- Movement and hit queries are fully under Spawn's control from the first physics milestone.
- `spawn-math` must provide deterministic implementations of the functions used by movement code.
- Physics geometry for hit registration and movement can exist before any dynamics solver.

## Risks

- **Effort and AI-assisted development.** AI assistance speeds up writing code, but not validating it: solver stability, edge cases (tunneling, jitter, stacking), and movement feel still require tests, benchmarks, and manual play-testing. Mitigation: every physics feature lands with headless tests and a measured scenario; nothing is claimed working without them.
- **Cross-platform determinism drift** from compiler or library changes. Mitigation: determinism tests that compare movement results across Windows and Linux builds for recorded input sequences.
- **Lag-compensation memory and cost** grow with player count and history length. Mitigation: bounded history, measured with the target player count.
- **Late dynamics decision** may require adapting collision data structures. Mitigation: the evaluation includes integration cost with the existing collision layer.

## Acceptance conditions

Accepted by the owner. The dynamics evaluation produces its own ADR before any solver or physics library is added.
