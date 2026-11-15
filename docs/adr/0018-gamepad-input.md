# ADR-0018 — Gamepad Input

**Status:** Proposed  
**Scope:** Native gamepad backends in `spawn-platform` and their consumption by `spawn-input`.  
**Related:** [ADR-0001](0001-in-house-platform-layer.md) (platform layer), [ADR-0002](0002-platform-owns-raw-input-events.md) (raw event ownership).

## Context

ADR-0001 left gamepads to a later ADR. Gamepads matter for broader audiences and for advanced features (gyro aiming, adaptive triggers, haptics, touchpad on DualSense; gyro on Switch Pro). Each OS exposes gamepads differently, and mapping thousands of controller models to a standard layout is a data problem.

## Decision

1. **Location:** native backends in `spawn-platform` behind a `gamepad` Cargo feature, independent of the window feature. They emit Spawn raw events: connect/disconnect (with device identity and capabilities), buttons, axes, and, where supported, sensors (gyro, accelerometer) and touchpad. Output commands (rumble, trigger effects, LED) go through `spawn-platform` as well.
2. **Windows backends:** XInput for Xbox-compatible controllers; Raw Input HID for other controllers (DualSense, Switch Pro). GameInput is evaluated when this ADR is implemented (availability and redistribution requirements must be verified).
3. **Linux backends:** evdev for standard gamepad input (devices are normally accessible to the active-session user through logind/uaccess); hidraw for advanced DualSense/Switch Pro features (requires udev permissions, typically installed by Steam; absence degrades gracefully to evdev features).
4. **Mappings:** the SDL GameControllerDB (zlib) as data, translated to Spawn's standard layout; user overrides allowed.
5. **`spawn-input`** treats gamepads as devices like keyboard and mouse: dead zones, response curves, gyro-to-aim mapping, rebinding. No genre actions in either crate.
6. **Steam Input coexistence:** when Steam Input remaps a controller, Spawn sees the virtual device; behavior is documented and tested.

## Alternatives

- **SDL3 gamepad subsystem only:** best-in-class support with gyro and triggers; C dependency and less control. Remains the fallback if native backends prove too costly.
- **`gilrs`:** pure Rust; no sensors or advanced HID features.
- **XInput only on Windows:** Xbox controllers only.

## References

SDL3 source (zlib; porting allowed with notice per [ADR-0008](0008-third-party-license-policy.md)), SDL GameControllerDB, `gilrs` (MIT/Apache-2.0), Linux `hid-playstation` driver documentation, public DualSense HID report documentation.

## Consequences

- Gamepad support stays optional per game and absent from headless builds.
- Advanced features exist only on controllers and OSes that expose them; capabilities are reported to games.

## Risks

- **HID report formats** are reverse-engineered for some controllers. Mitigation: test devices; capability flags instead of assumptions.
- **Permissions on Linux** for hidraw. Mitigation: graceful degradation and documentation.
- **Large testing matrix** of controllers. Mitigation: support list limited to tested devices.

## Revisit triggers

Native backend effort exceeds value compared with SDL3; Windows GameInput becomes the clear standard.

## Acceptance conditions

Owner review before gamepad work starts.
