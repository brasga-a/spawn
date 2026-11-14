# Verification and Acceptance — Foundation

**Status:** test plan only. No foundation test results have been run or claimed here.

## Required automated checks

Once implementation exists, run the normal workspace formatting/lint/test gates and focused headless tests. At minimum:

| Test ID | Requirement / invariant | Scenario | Pass condition |
| --- | --- | --- | --- |
| FND-T01 | FR-07 / INV-07 | Feed identical elapsed-duration sequences into a test clock. | Same number and order of fixed ticks, independent of render scheduling. |
| FND-T02 | FR-08 / INV-08 | Feed a large pause/suspend delta. | Tick count is bounded by configured catch-up limit, no unbounded work. |
| FND-T03 | FR-09 / INV-10 | Press, hold and release between redraws / across multiple ticks. | Each edge observed according to explicit sampling contract, never duplicated or dropped. |
| FND-T04 | FR-09 / INV-11 | Focus loss or device disconnect with held input. | Held states reset/released predictably. |
| FND-T05 | FR-02 / INV-02 | Inject ordered synthetic raw events. | Translation preserves declared ordering and distinguishes raw from actions. |
| FND-T06 | FR-05 / INV-05 | Resize to zero, then back to valid pixels. | No acquire/present at zero size; resumes on valid size. |
| FND-T07 | FR-06 / INV-06,12 | Inject recoverable surface failure and reconfigure request. | Bounded recovery without resetting simulation clock; diagnostic status returned. |
| FND-T08 | FR-10 / INV-04,12 | Fail successive initialization steps. | Prior owned resources released exactly once, no panic. |
| FND-T09 | NFR-01,02 / INV-01,02,13,14 | Inspect manifests, public API and crate imports. | Acyclic dependencies and no out-of-scope contracts. |
| FND-T10 | FR-03 / INV-04,12 | Deliver repeated close requests. | Idempotent request handling, orderly stop. |

Prefer fake clocks, synthetic event queues and mock surface state machines. GPU-driven behavior is not suitable as a mandatory CI-only unit test.

## Manual smoke tests (record environment)

On **at least one explicitly named tested platform**, record:
- OS/version, compositor (e.g., Wayland/X11 where applicable), graphics device/driver and backend version.
- Window appears; clear color is visible; repeated frames present without runaway CPU usage.
- Drag resize, change DPI/scale if available, minimize/restore, close using native control.
- Keyboard press/release, focus loss and regain; no stuck action.
- Expected behavior on unavailable display/GPU; structured failure rather than panic or hang.
- Repeat launch/close and note unexpected logs, leaks or lingering processes.

A documented Linux-only smoke run does **not** imply Windows/macOS support.

## Traceability

- Functional requirements: [SPEC.md](SPEC.md) `FND-FR-01..11`.
- Non-functional requirements: [SPEC.md](SPEC.md) `FND-NFR-01..06`.
- Safety and architecture invariants: [INVARIANTS.md](INVARIANTS.md) `FND-INV-01..15`.
- Delivery order: [PLAN.md](PLAN.md).

Every completed implementation PR should state: requirements covered, tests run (with output), platform tested, invariant changes (if any), and known limitations. Never report the preexisting `cargo new` stub tests as validation of the renderer or event loop.

## Exit gate

The foundation milestone is **complete only if**:
1. Window starts and closes on a documented platform.
2. Repeated clear frames present at valid size.
3. Resize/minimize and defined recoverable errors behave safely.
4. Input state and fixed-timestep behavior pass headless tests.
5. Boundary checks show no prohibited dependencies.
6. All relevant ADRs are accepted and implementation matches them.
7. Any unsupported platforms and untested recovery cases are called out explicitly.
