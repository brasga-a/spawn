# ADR-0001 — In-House Platform Layer for Windows and Linux

**Status:** Accepted (foundation scope; revisited when a later milestone or a measured limitation requires it).  
**Scope:** Native windows, event loop, raw keyboard/pointer events, and later native input devices in `spawn-platform`.  
**Related:** [ADR-0002](0002-platform-owns-raw-input-events.md) (event ownership), [ADR-0003](0003-fixed-step-loop-and-threading.md) (loop ownership), [ADR-0004](0004-in-house-graphics-api.md) (graphics API).

## Context

The validation project is a competitive multiplayer FPS. Its feel depends on the "last mile" of platform behavior: mouse input read in batches at high polling rates, precise per-event timestamps (sub-tick input), late input reads, precise frame pacing and sleep, OS scheduling hints, and advanced gamepad features (gyro, adaptive triggers, haptics). General-purpose windowing libraries cover typical needs but limit control over exactly these points. The owner prefers full control and accepts the extra implementation time.

## Decision (owner direction)

`spawn-platform` implements its own platform layer directly on the OS APIs:

- **Supported:** Windows (Win32) and Linux.
- **Not supported:** macOS. No code, CI, or design constraint is kept for it. Reconsidering this requires a new ADR.
- **Spawn owns the event loop:** a classic poll-driven loop (pump OS events, then update, then render) instead of a library calling into the engine (see [ADR-0003](0003-fixed-step-loop-and-threading.md)).
- Each OS backend sits behind a Cargo feature of `spawn-platform`; the raw event types compile without any backend ([ADR-0002](0002-platform-owns-raw-input-events.md)).
- The window type implements the `raw-window-handle` traits so `spawn-render` can create a surface without depending on `spawn-platform` ([ADR-0004](0004-in-house-graphics-api.md)).

"In-house" means Spawn writes the platform logic. It does not rule out thin crates that only *declare* OS APIs or implement a wire protocol; see detailed decision 2 below.

## Detailed decisions (owner-confirmed)

1. **Linux display servers: Wayland and X11.** Wayland is the current desktop default (the first smoke-test machine runs Hyprland). X11 is needed for older setups and because games on Steam Deck in game mode (gamescope) usually run under XWayland.
2. **Binding level: thin declaration/protocol crates, no hand-written FFI declarations.**
   - Windows: `windows-sys` (Microsoft's raw API declarations, no runtime logic).
   - Wayland: `wayland-client` + `wayland-protocols` (protocol implementation only).
   - X11: `x11rb` (protocol implementation only).
   - Linux keymaps: `xkbcommon` (or a dlopen variant); no in-house keymap/compose implementation.

   Platform logic remains entirely Spawn's. Each crate is still justified individually under the root dependency rules when adopted.
3. **Foundation scope: basics first.** One window, close/resize/focus/DPI events, keyboard and mouse (including relative motion and cursor lock), and a window handle for the renderer. Raw Input batching, high-precision timestamps, frame-pacing timers, IME and gamepads are later increments, each with its own measurement and, where it adds a backend, its own ADR.

## Alternatives

| Option | Strengths | Weaknesses |
| --- | --- | --- |
| **In-house, Windows + Linux** (chosen direction) | Full control of input timing, batching, pacing and OS tuning; Spawn owns the loop; no third-party event model; prepares the structure needed for consoles later | Per-OS code to maintain; Wayland behavior differs per compositor (protocol support varies); larger testing matrix; first frame takes longer |
| **`winit`** | Pure Rust; de facto standard; Wayland, X11, Windows, macOS; implements `raw-window-handle` | No gamepad support; callback-driven loop (inversion of control); every event goes through the window message loop, limiting batched high-rate mouse reads; less control over timing details |
| **SDL3** (`sdl3` bindings) | Mature, widely used in shipped games; window, input, **gamepad**, haptics, audio; poll-driven loop | C library; broad scope overlaps `spawn-input` and `spawn-audio`; internal behavior not under Spawn's control |
| **GLFW** (`glfw` bindings) | Small, stable, poll-driven | C library; limited gamepad support; aimed mainly at demos and tools |

## Consequences if accepted

- `spawn-platform` contains `unsafe` FFI code; every block follows the root `// SAFETY:` rule, and OS calls are confined to per-OS modules.
- The foundation's Phase 2 (platform) grows significantly compared with using a library.
- Wayland features without universal compositor support (tearing control, high-resolution input timestamps, pointer constraints) need a fallback path; compositor support is recorded per tested environment.
- The smoke-test matrix becomes: Windows 10/11, and Linux on Wayland (at least Hyprland) and X11.
- Gamepad support: [ADR-0018](0018-gamepad-input.md) (proposed), inside `spawn-platform` behind its own feature.

## Acceptance conditions

Explicit owner acceptance. The first smoke test (window, close, resize, keyboard, mouse) on Linux is implementation evidence recorded afterwards, not a precondition, so Phase 0 is not blocked on implementation. Windows smoke testing is required before Windows is claimed as supported.
