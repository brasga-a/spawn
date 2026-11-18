# Verification and Acceptance — Foundation

**Status:** test plan. Implemented and passing: FND-T01, FND-T02 (`crates/spawn-core/tests/fixed_step_verification.rs`); FND-T05 and the platform part of FND-T10 (`crates/spawn-platform/src/linux/translate.rs`, headless, no backend feature). FND-T06 and FND-T07 (`crates/spawn-render/src/presentation.rs`, headless, no backend feature), also exercised on the reference GPU by the opt-in tests in `crates/spawn-render/src/vulkan/mod.rs` (`cargo test -p spawn-render --features vulkan -- --ignored`). The composition-root part of FND-T10 (shutdown on close) belongs to Phase 5. All other cases are not yet implemented.

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
| FND-T09 | NFR-01,02 / INV-01,02,13,14 | Inspect manifests, public API, crate imports, and `cargo tree` for `spawn-input` and `spawn-core`. | Acyclic dependencies, no out-of-scope contracts, and no native window/GPU backend in headless crates. |
| FND-T10 | FR-03 / INV-04,12 | Deliver repeated close requests. | Idempotent request handling, orderly stop. |

Prefer fake clocks, synthetic event queues and mock surface state machines. GPU-driven behavior is not suitable as a mandatory CI-only unit test.

## Manual smoke tests (record environment)

On **at least one explicitly named tested platform**, record (first platform: the reference hardware in root [Accepted Decisions](../../AGENTS.md#accepted-decisions), Linux/Wayland/Hyprland):
- OS/version, compositor (e.g., Wayland/X11 where applicable), graphics device/driver and backend version.
- Window appears; clear color is visible; repeated frames present without runaway CPU usage.
- Drag resize, change DPI/scale if available, minimize/restore, close using native control.
- Keyboard press/release, focus loss and regain; no stuck action.
- Expected behavior on unavailable display/GPU; structured failure rather than panic or hang.
- Repeat launch/close and note unexpected logs, leaks or lingering processes.

A documented Linux-only smoke run does **not** imply Windows support. macOS is not a target.

## Recorded smoke runs

| Date | Phase | Environment | Command | Observed | Not verified |
| --- | --- | --- | --- | --- | --- |
| 2026-11-17 | 2 (platform) | Omarchy 4.0.6 (Arch-based), kernel 7.2.15, Hyprland 0.56.2 (Wayland, native, not XWayland), monitor scale 1.0; reference hardware | `SPAWN_SMOKE_SECONDS=8 cargo run -p spawn-platform --features wayland --example wayland_window` | Window mapped (`hyprctl clients`: `mapped: true`, `xwayland: false`) with stand-in `wl_shm` content; ordered `Focused`, `Resized` (tiling 1265→1257→624 px wide, repainted), `Key` (Alt/Super press and release), `PointerEntered`/`PointerMoved`/`PointerLeft` events; auto-close exited 0 with no lingering process. With `WAYLAND_DISPLAY` pointing nowhere: `error[PLAT-0001]` report, exit 1, no panic. | Close through the compositor's close control (`xdg_toplevel.close`); fractional/integer scale > 1 and scale changes; `Occluded` (`suspended`, xdg-shell v6); compositors other than Hyprland; X11; Windows. |
| 2026-11-18 | 3 (renderer) | Same machine and compositor; AMD Radeon RX 460 (RADV POLARIS11), Mesa 26.2.5, Vulkan 1.4.354 device, loader 1.4.363; **Khronos validation layer not installed** | `SPAWN_SMOKE_SECONDS=8 cargo run --example vulkan_clear` | Window mapped (`hyprctl clients`: `mapped: true`, `xwayland: false`); center pixel sampled with `grim` from the window's own region: RGB (39, 132, 152), the teal clear color (linear 0.02, 0.25, 0.30 → sRGB ≈ 39, 137, 149; small difference attributed to compositor color handling, not investigated); 407 frames presented in 8 s (≈ 51/s, FIFO), 0 suspended, 0 skipped; tiling resize 632 → 616 px handled; CPU 0.24 s user+sys in 8.1 s; exited 0 with no lingering process. Without a driver (`VK_DRIVER_FILES` pointing nowhere): `error[REND-0001]` with an actionable detail, exit 1, no panic. Opt-in GPU tests (headless surface): 3/3 pass. | Validation layer (not installed); drag resize; hide/minimize (`Occluded`) and zero-size suspension in a real window (covered only by headless tests); scale > 1; device loss; compositor close control; other GPUs, compositors, X11, Windows. |

## Traceability

- Functional requirements: [SPEC.md](SPEC.md) `FND-FR-01..11`.
- Non-functional requirements: [SPEC.md](SPEC.md) `FND-NFR-01..06`.
- Safety and architecture invariants: [INVARIANTS.md](INVARIANTS.md) `FND-INV-01..15`.
- Delivery order: [PLAN.md](PLAN.md).

Every completed implementation PR should state: requirements covered, tests run (with output), platform tested, invariant changes (if any), and known limitations. Never report scaffold builds or empty test runs as validation of the renderer or event loop.

## Exit gate

The foundation milestone is **complete only if**:
1. Window starts and closes on a documented platform.
2. Repeated clear frames present at valid size.
3. Resize/minimize and defined recoverable errors behave safely.
4. Input state and fixed-timestep behavior pass headless tests.
5. Boundary checks show no prohibited dependencies.
6. All relevant ADRs are accepted and implementation matches them.
7. Any unsupported platforms and untested recovery cases are called out explicitly.

Open items against this gate: relative pointer motion and cursor lock ([PLAN](PLAN.md#scope-notes) increment 2b, required by ADR-0001 foundation scope); manual test of closing through the compositor's close control (Phase 2); a clear-frame run with the Khronos validation layer reporting no errors, required as evidence by ADR-0004 (Phase 3); manual drag resize and hide/minimize with the Vulkan example (Phase 3).
