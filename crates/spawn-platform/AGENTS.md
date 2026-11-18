# spawn-platform

Local rules for `spawn-platform` (crate). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Operating-system integration: windows, surfaces, system/window events, event loop, clipboard/cursor/display queries, and other platform-specific abstractions.

## Responsibilities

- Window creation and lifecycle, display/monitor information, surface handles for rendering (Wayland backend `EXISTS`; X11, Win32 and display/monitor queries `PLANNED`).
- Event loop and delivery of raw OS events (window, keyboard, mouse, gamepad connection) to consumers.
- Platform-specific differences hidden behind one abstraction (Windows and Linux; see [ADR-0001](../../docs/adr/0001-in-house-platform-layer.md)).
- In-house OS backends for Windows (Win32) and Linux (Wayland and X11); macOS is not supported. Spawn owns the event loop. Accepted in [ADR-0001](../../docs/adr/0001-in-house-platform-layer.md). Per [ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md) (accepted), each native backend is compiled only behind its Cargo feature (`wayland` exists); raw event types are available without it.

## Non-Responsibilities

- Semantic input: actions, mappings, input state, device abstraction for gameplay (`spawn-input`). This crate emits *raw* platform events only.
- Rendering/GPU work (`spawn-render`) beyond handing over a surface/window handle.
- Filesystem and stream access (`spawn-io`), audio devices (`spawn-audio`), sockets (`spawn-network`).

## Architectural Boundaries

- Must not depend on `spawn-render`, `spawn-input`, or `spawn-ui`; they consume platform events/handles. Raw event types live here ([ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md), accepted); never depend on `spawn-input`.
- Headless operation must be possible: the engine (server, tests, CLI) runs without linking a window backend.

## Public API Guidelines

- Expose engine-defined types, not backend types (no OS handles, binding-crate types, or protocol objects in the public API).
- Exception: the window type implements the `raw-window-handle` traits so `spawn-render` can create a surface (SPN-INV-011, [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md)). Expose only those traits/handle contracts, nothing else from backend crates.
- Event delivery is explicit about thread and ordering guarantees.

## Internal Design Principles

- OS calls are confined to per-OS backend modules behind Cargo features; all `unsafe`/FFI lives there with `// SAFETY:` comments.
- Window/GPU code sits behind thin layers so logic stays headless-testable.

## Performance and Safety

- Event pump must not stall the fixed-timestep simulation; handle lost-surface/minimize/resize without panicking.

## Testing Requirements

- Headless tests of event translation and queues with synthetic events; window creation tests are manual or opt-in (not required in CI).

## Governing Decisions

- **Accepted (binding):** [ADR-0001](../../docs/adr/0001-in-house-platform-layer.md) (platform layer), [ADR-0002](../../docs/adr/0002-platform-owns-raw-input-events.md) (raw event ownership), [ADR-0003](../../docs/adr/0003-fixed-step-loop-and-threading.md) (fixed-step loop, tick index, threading). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0018](../../docs/adr/0018-gamepad-input.md) (gamepad input).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `windows-sys`, `wayland-client`, `wayland-protocols`, `x11rb`, `xkbcommon`, `raw-window-handle`, `rustix`, `evdev`, `udev`, `hidapi`. Any other crate needs owner approval first.

## Current Implementation Status

`EXISTS` (raw events and the Wayland backend); X11 and Win32 backends `PLANNED`.

**Cargo features:** `default = []`. `wayland` enables the Wayland backend on Linux (ignored on other targets) and pulls `wayland-client` (features `dlopen`, `system`), `wayland-protocols` (`client`, `staging`, `unstable`), `rustix` (`event`, `std`) and `raw-window-handle`. `libwayland-client.so` is loaded at runtime, so building needs no system packages and a missing library is `PLAT-0001`. Without features the crate has no dependencies (`cargo tree -p spawn-platform -e normal`).

| Item | Module | Availability | Notes |
| --- | --- | --- | --- |
| `Event`, `WindowEvent`, `KeyEvent`, `ElementState`, `ScrollDelta` | `event` | always | Raw events in window-system order; no actions or bindings |
| `WindowId`, `PhysicalSize`, `LogicalSize`, `PhysicalPosition` | `event` | always | Ids never reused within a platform instance; sizes and positions for the renderer in physical pixels |
| `KeyCode`, `MouseButton` | `key` | always | Physical keys named by US-layout position; unknown keys keep their `native_code` |
| `WindowDescriptor` | `window` | always | Title, optional app id, logical size validated to `1..=16384` |
| `PlatformError`, `PlatformErrorKind` | `error` | always | Located, coded, detail line, cause chain flattened to opaque text (no backend type escapes), optional backtrace (ADR-0009) |
| `Platform`, `Window`, `PumpMode` | `platform` | `wayland` on Linux | Public wrapper; names no backend type. `Platform::pump_events` appends events to a caller buffer (`Poll`, `WaitTimeout`, `Wait`). `Platform` and `Window` implement `raw-window-handle` traits. Neither is `Send`/`Sync` |
| Evdev key/button mapping | `linux/evdev` | Linux, `wayland` or tests | Pure tables |
| `Translator` | `linux/translate` | Linux, `wayland` or tests | Pure state machine: configure, size/scale arithmetic, focus routing, scroll frame grouping, event order |
| Wayland glue | `linux/wayland` | `wayland` on Linux | Only module using protocol types and `unsafe` (two `raw-window-handle` borrows) |

**Error codes:**

| Code | Kind | Meaning |
| --- | --- | --- |
| `PLAT-0001` | `DisplayUnavailable` | No display server reachable, or its client library is missing |
| `PLAT-0002` | `UnsupportedDisplay` | A required protocol global (`wl_compositor`, `xdg_wm_base`) is missing |
| `PLAT-0003` | `ConnectionLost` | Connection failure or protocol error (detail names code, object and message) |
| `PLAT-0004` | `InvalidWindowSize` | Requested window size is zero or above `WindowDescriptor::MAX_EXTENT` |
| `PLAT-0005` | `Timeout` | The compositor did not configure a new window within 5 s |

Error locations point at the line in `spawn-platform` that detected the failure, except `WindowDescriptor::new` (`#[track_caller]`: the caller's line).

**Event and thread contract:**

- Single-threaded: events are produced only inside `Platform::pump_events`, on the calling thread; no callbacks, no background threads.
- Order: the order the compositor sent the messages; scroll axis messages are grouped and emitted at the pointer `frame` that ends them.
- `Resized` carries the physical size and is not sent for the initial size (`Platform::window_size` has it once `create_window` returns). `ScaleFactorChanged` precedes the `Resized` it causes.
- `CloseRequested` is delivered for every request; the platform never closes a window itself. Dropping `Window` destroys it; dropping it before `Platform` is the intended order, but either order is safe.
- Focus: `Focused(false)` is sent on keyboard leave; keys held at that moment get no synthetic release (resetting held state is `spawn-input`'s job, FND-T04). Keys already held when focus enters are not reported.
- Key and button transitions only: no key repeat, no text, no modifiers state, no keymap (no `xkbcommon`).

**Wayland protocols:** `wl_compositor` (required, up to v6), `xdg_wm_base` (required, up to v6, `suspended` → `Occluded`), `wl_seat` (optional, up to v8, first seat at startup only; keyboard and pointer follow capability changes), `wp_viewporter` + `wp_fractional_scale_v1` (optional; integer `preferred_buffer_scale` used when fractional scale is absent; without the viewporter the scale stays 1.0 and the buffer scale is never set), `zxdg_decoration_manager_v1` (optional; requests server-side decorations; without it the window is undecorated, no client-side decorations).

**Tests (all passing, headless, no backend feature needed):** 32 unit tests: error codes and report format (3), descriptor validation (3), event types (2), evdev mapping (5), translator (19) including FND-T05 (`mixed_sequence_preserves_protocol_order`) and the platform part of FND-T10 (`repeated_close_requests_are_delivered_and_harmless`). `scripts/check.sh` and CI also lint, test and document the crate with `--features wayland`.

**Manual smoke test:** `examples/wayland_window.rs` (requires `wayland`). It stands in for the renderer by attaching a `wl_shm` buffer through the `raw-window-handle` contract, prints every event and exits on close request or Escape (`SPAWN_SMOKE_SECONDS=<n>` auto-closes). Recorded run: [VERIFICATION](../../docs/foundation/VERIFICATION.md#recorded-smoke-runs) (Hyprland; supported claims are limited to what that record lists).

**Not implemented / deferred:**

- X11 and Win32 backends; Windows support is not claimed.
- Relative pointer motion and cursor lock/confinement: **in ADR-0001's foundation scope** but not in Phase 2; scheduled as foundation increment 2b ([PLAN scope notes](../../docs/foundation/PLAN.md#scope-notes)). Still owed before the foundation milestone completes.
- Raw Input batching, high-precision timestamps, IME/text input, gamepads ([ADR-0018](../../docs/adr/0018-gamepad-input.md), proposed): later increments per ADR-0001 decision 3.
- Cursor shape, key repeat, keymaps (`xkbcommon`): not required by the foundation SPEC; added when a consumer needs them.
- Validation: closing through the compositor's close control is a pending manual test ([VERIFICATION](../../docs/foundation/VERIFICATION.md#recorded-smoke-runs)).
- Display/monitor queries, clipboard, multiple seats, globals added after startup.

## Agent Instructions

- Do not add input mapping or action logic.
- The backends are decided ([ADR-0001](../../docs/adr/0001-in-house-platform-layer.md)): Win32 on Windows, Wayland and X11 on Linux, an in-house abstraction, and a Spawn-owned event loop; macOS is out of the target matrix. Do not reopen this choice without a new ADR.
- Decided is not implemented: only the Wayland backend exists; implement each further backend behind its own Cargo feature and record its smoke test before claiming support.
- Keep protocol types and `unsafe` inside the backend module; put decisions in the pure translator and test them with synthetic sequences.
- New error kinds get the next `PLAT-` code; codes are never reused or renumbered.
