# Implementation Plan — Foundation

**Status:** PLANNED. Tasks are incremental; each step should keep the workspace buildable.

## Ordered increments

| Phase | Deliverable | Exit evidence |
| --- | --- | --- |
| 0. Decisions | Accept ADRs 0001–0004 and choose the smoke-test platform. **Done:** ADRs accepted; first platform Linux/Wayland (Hyprland). | Accepted ADRs and recorded platform scope. |
| 1. Clock contract | Fixed-step timing policy in `spawn-core` with injected elapsed durations. **Done:** `FixedStepClock` implemented. | T01, T02 headless tests passing (`crates/spawn-core/tests/fixed_step_verification.rs`). |
| 2. Platform | Small native window/event pump with raw event mapping in `spawn-platform`: one window; close, resize, focus, scale (DPI) and occlusion events; keyboard keys; pointer enter/leave/motion/buttons/scroll; window handle for the renderer. Relative motion and cursor lock are **not** in this phase (see 2b and [Scope notes](#scope-notes)). **Implemented:** raw event types (no backend) and the Wayland backend behind feature `wayland`; X11 and Win32 not started. **Validation incomplete.** | Window visible, close works, T05 and T10 as applicable. Done: T05 and T10 (platform part) pass headless (`crates/spawn-platform/src/linux/translate.rs`); window mapped on Hyprland ([smoke record](VERIFICATION.md#recorded-smoke-runs)). **Pending:** manual test of closing through the compositor's close control (owner). Phase 2 is not validated until it passes. |
| 2b. Pointer capture | Relative pointer motion and cursor lock/confinement in `spawn-platform` (Wayland: `zwp_relative_pointer_v1`, `zwp_pointer_constraints_v1`). Required by [ADR-0001](../adr/0001-in-house-platform-layer.md) decision 3 (foundation scope); no `FND-FR` requirement depends on it. **Implemented:** `WindowEvent::PointerRelativeMotion`, `CursorGrabMode`, `WindowEvent::CursorGrabChanged`, `Platform::set_cursor_grab` / `cursor_grab`, wired into `spawn-input`. | Done: headless translation tests for relative motion, lock/confinement lifecycle, focus loss while constrained and seat capability loss (`crates/spawn-platform/src/linux/translate.rs`), plus end-to-end `spawn-input` integration test; bounded live smoke runs on Hyprland for `Locked` and `Confined` ([smoke record](VERIFICATION.md#recorded-smoke-runs)). |
| 3. Renderer | Surface/device setup, clear/present and resize/recovery policy in `spawn-render`. **Implemented:** `Renderer` over a private Vulkan 1.3 RHI behind feature `vulkan` (Wayland surfaces only), pure presentation controller and selection logic. Depends on Phase 2 (PR #5) for the smoke example. **Validation incomplete.** | Clear frame; T06, T07, manual resize/minimize. Done: T06 and T07 pass headless (`crates/spawn-render/src/presentation.rs`) and on the reference GPU through 4 opt-in headless-surface tests with `VK_LAYER_KHRONOS_validation` + sync validation reporting 0 errors; clear frame presented repeatedly in a Wayland window on Hyprland with a tiling resize and 0 validation errors ([smoke record](VERIFICATION.md#recorded-smoke-runs)). **Pending:** manual drag resize and hiding/minimizing the window (owner). |
| 4. Input | Device transitions and tick-based input sampling in `spawn-input`. **Done:** `DeviceState`, `InputMap`, `InputState`, `InputCommand`, and `InputError` implemented (`#![forbid(unsafe_code)]`, zero native/GPU dependencies). | T03, T04 pass without window (`crates/spawn-input/tests/input_verification.rs`). |
| 5. Composition | Root `spawn-engine` binary wires subsystems, handles errors and cleanup. | T08, T09 and full smoke run; no unrelated crates changed. |
| 6. Completion | Document tested platforms, collect evidence, update statuses. | [Verification exit gate](VERIFICATION.md) satisfied. |

## Scope notes

**Relative pointer motion and cursor lock (Phase 2 → 2b).** [ADR-0001](../adr/0001-in-house-platform-layer.md) (accepted), detailed decision 3, lists "keyboard and mouse (including relative motion and cursor lock)" in the foundation scope. The milestone requirements in [SPEC](SPEC.md) do not need them: FND-FR-02 and FND-FR-09 ask for raw pointer events and pointer motion, and the exit evidence is a clear frame, not camera control. They are therefore moved out of Phase 2 into the separate increment 2b. This changes only the phase split, not the accepted decision: 2b remains part of the foundation milestone, and the [exit gate](VERIFICATION.md#exit-gate) (item 6, implementation matches accepted ADRs) is not met until 2b is done. Moving these features out of the foundation milestone altogether would change ADR-0001 and needs an explicit owner decision recorded in that ADR; it is not made here.

These are feature increments, not necessarily one pull request each. Keep each change reviewable; do not add all backends, editor UI, ECS or gameplay to get a clear frame.

## Implementation guardrails

- Follow [root AGENTS.md](../../AGENTS.md) and each affected crate's local AGENTS.md.
- Proposed ADRs are review inputs; do not silently implement them before acceptance.
- Avoid speculative generic interfaces. Document the concrete boundary only after the first consumer exists.
- The default binary is an integration/demo harness, not a production game launcher or editor.
- Make errors visible and resource lifetimes testable before adding convenience abstractions.
- If an external crate is selected, pin and justify it according to workspace dependency rules; check current upstream documentation before coding.
- Keep changes unrelated to this milestone out of the PR.

## Deferred follow-up

The **next** milestone may render a triangle, then mesh/camera; subsequently ECS/world integration, gameplay and multiplayer. Those need their own specs and acceptance criteria and must not be implied by success of this foundation slice.
