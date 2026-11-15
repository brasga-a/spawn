# Technical Design — Foundation Runtime

**Status:** DESIGN PROPOSAL. No API names or backends in this document are claims of implementation.

## Architecture

```text
spawn-engine executable (composition root)
   ├── spawn-platform   native window + event pump + raw events
   ├── spawn-input      translated device state + sampled actions
   ├── spawn-render     GPU device/surface + clear + present
   └── spawn-core       generic lifecycle and fixed-step timing contract

Dependency rules: platform !→ render/input; input !→ render;
                 core !→ platform/render/input; all crates !→ game frameworks.
```

Arrows above show the executable **uses** libraries, not literal Rust dependencies inside `spawn-core`. Each crate owns its implementation and errors. The executable composes them; the engine's public API should not leak a particular native backend by accident.

## Lifecycle proposal

1. Validate runtime options (do not build a large configuration system).
2. Establish diagnostics and a monotonic timing source. The composition root owns both: it writes diagnostics to stderr using `std` only and reads monotonic time, passing elapsed durations to `spawn-core` (core never reads OS time itself). Structured logging belongs to `spawn-profiler` and needs its own dependency decision; it is not part of this milestone.
3. Create the event loop and window on the thread required by the selected platform backend.
4. Obtain an appropriate window/surface capability without exposing a renderer implementation through `spawn-platform`.
5. Initialize rendering and configure the current non-zero surface dimensions.
6. Initialize input state and begin the event-driven runtime.
7. For each event batch: translate raw events, update input transitions, process close/resize, advance fixed-step scheduler, and present when the surface is drawable and redraw is required.
8. Shutdown in reverse ownership order: stop presentation, release renderer/surface resources, then window/event-loop resources. Every startup failure enters a controlled cleanup path.

Actual callback/event-loop constraints depend on the backend and must be verified once selected. Never assume the event loop can be restarted on every OS or that all platforms deliver events in identical order.

## Boundary contracts (conceptual, not invented Rust traits)

| Producer → Consumer | Data crossing boundary | Restrictions |
| --- | --- | --- |
| Native backend → `spawn-platform` | OS window, pointer, keyboard, resize, focus, close events | Backend-specific objects stay private where practical. |
| `spawn-platform` → `spawn-input` | Ordered raw device events + device/window identity as needed | No gameplay actions, keybind policy, or render types in platform. |
| `spawn-platform` → `spawn-render` | Surface/window capability and dimensions | Renderer owns GPU setup; platform does not own GPU command buffers. |
| `spawn-input` → composition root | Tick-indexed action/sample state | Actions are intents, not authoritative gameplay outcomes. |
| `spawn-core` → composition root | Time advance / simulation tick policy | Core does not call OS or GPU APIs. |
| `spawn-render` → composition root | Presentation outcome or structured recoverable/unrecoverable error | Renderer does not run the event loop. |

These are responsibilities, **not** required new trait names or modules. Choose the smallest APIs needed for the first slice, documenting adopted ownership in the crate's AGENTS.md.

## Frame and simulation timing

- Read time from a monotonic source; compute elapsed duration safely. Wall-clock timestamps are not suitable for step accumulation.
- Keep `fixed_dt` configurable but fixed for a running session. Suggested **initial test configuration**: 60 simulation ticks/second. This is not a guaranteed delivered frequency under load.
- Maintain a bounded accumulator. For each update, execute at most `max_catch_up_steps` simulation ticks. Suggested initial test configuration: 4 steps, to be measured later.
- Excess backlog is explicitly dropped/clamped or handled by a documented policy; never process an unlimited number of ticks after suspend/debugger pause.
- Sample input according to the tick boundary contract. Do not clear press/release transitions once per rendered frame if that loses a transition during multiple simulation ticks or redraw stalls.
- Presentation may run at a different cadence from simulation. Do not drive physics/simulation from frame presentation callbacks alone.
- Keep interpolation factor bounded to the documented range and derived from the residual accumulator if future rendering interpolation needs it. No ECS state required yet.

## Presentation state machine

Proposed coarse states:

| State | Allowed behavior |
| --- | --- |
| Uninitialized | No attempt to present. |
| Ready | Surface configured with non-zero dimensions; may clear/present. |
| Suspended | Zero dimensions, minimized or temporarily non-drawable; skip acquire/present and preserve application liveness. |
| Recovering | Lost/outdated surface or recoverable condition; retry bounded reconfigure path. |
| Fatal | Non-recoverable graphics failure; return structured error and initiate shutdown. |
| Stopped | Native resources released; no frames submitted. |

Resize must use the current physical pixel extent, not assume logical size equals drawable size; scale-factor changes should be handled explicitly. Do not recreate the GPU device on every resize. Surface texture acquisition failure must not trigger a tight busy loop.

## Event and focus policy

- Close request initiates shutdown; duplicate close requests must be harmless.
- Focus loss must not leave a key logically held forever. Define reset/synthetic release behavior and test it.
- Consecutive pointer events may be coalesced only when ordering-dependent input semantics remain intact.
- Do not install genre-specific default actions inside `spawn-input`; the sample executable may bind a development-only action without turning it into an engine primitive.
- Platform-specific differences (Wayland/X11, DPI, event-loop threading) are backend concerns, not assumptions hidden in input state.

## Headless seams

Simulation-clock behavior and input state changes must accept synthetic sequences in unit tests. The initial demo needs a GPU, but tests for timing, ordering, lifecycle and error policies must not. Future dedicated servers must not need to link or initialize `spawn-render`.

## Deferred topics

No ECS/world update implementation, animation, physics, threaded render graph, GPU instancing, multi-window editor architecture, frame scheduling for networking, or scripting in this milestone. Decisions for those features belong to later specs and ADRs.
