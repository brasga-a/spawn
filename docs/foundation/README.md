# Foundation Milestone — First Running Frame

**Milestone:** Spawn Foundation / first native runtime slice  
**Status:** in progress: phase 1 (clock) implemented and validated; phase 2 (platform, Wayland backend) implemented, validation pending (manual compositor-close test); 2b (relative pointer motion, cursor lock) and phases 3–6 PLANNED ([plan](PLAN.md))  
**Target:** A small generic executable that creates a window, polls events, updates a bounded fixed-step simulation clock, presents a clear frame, handles input and resize, and exits cleanly.

The first demonstrated frame can be a solid clear color; triangle, mesh, camera, gameplay, and editor UI are subsequent work.

## Ownership

| Crate | Scope for this milestone |
| --- | --- |
| `spawn-core` | Portable loop/timing/lifecycle **contracts**, not OS event loop and not subsystem implementations |
| `spawn-platform` | Native window/event loop and raw OS event translation |
| `spawn-input` | Normalize input, track transitions, sample semantic actions at simulation tick boundaries |
| `spawn-render` | Device/surface lifecycle, clear/present, resize/reconfigure and recoverable surface errors |
| `spawn-engine` (root binary) | Composition root: initializes and drives selected subsystems, owns shutdown order |

No dependency inversion is allowed to become an import cycle. The renderer consumes a platform surface/window capability; the platform crate does not import rendering. Input consumes translated events; the platform crate does not import input. Core stays engine-subsystem agnostic.

## Read in order

1. [Specification](SPEC.md)
2. [Design](DESIGN.md)
3. [Invariants](INVARIANTS.md)
4. [Verification](VERIFICATION.md)
5. [Implementation plan](PLAN.md)
6. [ADRs](../adr/README.md)

The details marked **proposal** are not adopted until the relevant ADR is accepted. Build the smallest working vertical slice and keep unrelated crates untouched.
