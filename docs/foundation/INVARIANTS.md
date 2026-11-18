# Foundation Invariants

**Status:** normative requirements for the foundation milestone, verified across automated tests `FND-T01..T10` (`crates/*/tests/`, `tests/foundation_verification.rs`) and recorded Linux/Wayland + Khronos validation smoke runs ([VERIFICATION.md](VERIFICATION.md)).

Each invariant has an ID and an intended verification approach. A change that weakens one requires explicit review and corresponding test/document updates.

| ID | Invariant | How to verify |
| --- | --- | --- |
| FND-INV-01 | `spawn-core` does not import `spawn-platform`, `spawn-input`, `spawn-render`, or gameplay crates. | Dependency graph review. |
| FND-INV-02 | `spawn-platform` never depends on `spawn-render` or `spawn-input`; `spawn-input` never depends on rendering/gameplay crates. | `cargo metadata`, source audit. |
| FND-INV-03 | Native OS and graphics backends cannot accidentally become mandatory for headless timing/input unit tests. | Headless CI matrix. |
| FND-INV-04 | At most one subsystem owns any individual GPU surface, window and lifecycle state at a given time; ownership and teardown order are explicit. | State-machine/failure tests, code review. |
| FND-INV-05 | No frame is acquired or presented while the drawable surface size is zero or the renderer is suspended. | Resize and minimize tests. |
| FND-INV-06 | Surface resize/recovery does not implicitly create a new simulation world or reset simulation time. | Controlled resize and clock test. |
| FND-INV-07 | Simulation advancement is governed by a fixed step and monotonic elapsed time, never by a variable render-frame delta as simulation step size. | Deterministic clock tests using injected elapsed durations. |
| FND-INV-08 | A single wakeup processes no more than the configured catch-up step limit. | Large time-jump test. |
| FND-INV-09 | Rendering/presentation is not required for simulation-clock progression or input transition testing. | Headless tests, boundary audit. |
| FND-INV-10 | Input transitions are consumed according to a documented tick-based policy; redraw frequency must not duplicate or silently erase pressed/released events. | Synthetic event/tick schedule tests. |
| FND-INV-11 | Focus loss, window close, and device-disconnect paths cannot leave permanently held synthetic input states. | Input lifecycle tests. |
| FND-INV-12 | Expected runtime errors return explicit status and contextual diagnostics; no unchecked panic on device creation, acquire, resize, or teardown failure. | Error-injection tests, code review. |
| FND-INV-13 | Client/UI/gameplay semantics such as weapon, player or team cannot appear inside foundation subsystem contracts. | Public API review. |
| FND-INV-14 | Native platform details and backend concrete types do not leak into general-purpose public APIs without an accepted ADR. | API review. |
| FND-INV-15 | An implementation claim is documented only after it exists and has corresponding verification; proposed ADRs are not binding backend selections. | Docs/code audit. |

## Failure policy

- **Recoverable:** minimize/zero size, surface outdated/lost, transient resize. Suspend or attempt bounded recovery; never spin indefinitely.
- **Fatal:** unsupported window/display or graphics capability, unrecoverable device failure. Report context, unwind owned resources and terminate with failure.
- **Logic bug/invariant violation:** fail tests and correct the implementation; do not disguise the defect with silent success.

Do not claim full engine determinism or automatic GPU device-loss recovery merely from having a fixed timestep. The selected backend's actual failure modes must be handled explicitly.

## Change control

For every invariant change, update affected rows in [SPEC.md](SPEC.md) and [VERIFICATION.md](VERIFICATION.md). Link the accepted ADR if it modifies ownership, timing, or backend responsibilities.
