# ADR-0017 — Luau Scripting Runtime

**Status:** Proposed  
**Scope:** `spawn-scripting`; roadmap step 7.  
**Related:** root [Rust and Luau](../../AGENTS.md#rust-and-luau) and [Scripting security](../../AGENTS.md#data-driven-design-and-scripting-security) rules, [ADR-0008](0008-third-party-license-policy.md) (licenses).

## Context

Root rules already define the Rust/Luau split, the sandbox, execution limits, client/server VM separation and versioned API. This ADR fixes the concrete integration choices so the first implementation follows them.

## Decision

1. **Integration:** `mlua` with the `luau` feature, version pinned in `[workspace.dependencies]`, confined to `spawn-scripting`. Before adding it, verify the current `mlua` documentation for: Luau sandbox support, memory limits, interrupt callbacks, `send` support, and Luau native code generation (JIT) availability. Luau is MIT-licensed; `mlua` is MIT.
2. **VM layout:** one server VM per authoritative world and one client VM per client world; separate API registrations; no shared state between VMs.
3. **Sandbox:** Luau sandbox mode (read-only built-in libraries, per-script environments); only the allowlisted Spawn API is registered; no `os`, `io`, `debug`, `require` of arbitrary files, `loadstring` or dynamic code loading in untrusted contexts. Module loading goes through a Spawn loader that resolves asset IDs.
4. **Limits:** per-VM memory limit; per-event instruction budget through the interrupt callback; per-frame total script time budget; recursion/stack limits. Exceeding a limit aborts the current call with an error and marks the script for disable/restart per error-isolation rules.
5. **Execution model:** scripts react to events (gameplay events, timers, input actions on the client); they never run per entity per tick. Script effects are commands queued and applied by Rust at sync points.
6. **API versioning:** semantic version for the script API; each script package declares the version it targets; breaking changes bump the major version; a changelog lives next to the API definition.
7. **Typing:** scripts use Luau type annotations; CI runs the Luau type checker on bundled scripts. API type definitions are generated from the Rust API registration to stay in sync.
8. **Hot reload (development):** reloading a module resets its state by default; a script may export explicit state to persist, which is validated on reload.
9. **Values crossing the boundary** are validated (types, ranges, NaN/inf, string lengths, handle generations).

## Alternatives

- **Hand-written bindings to the Luau C++ API:** full control; large `unsafe` FFI surface already maintained well by `mlua`. Rejected.
- **Other languages (Lua 5.4, LuaJIT, Rhai, WASM):** Luau was chosen for its sandbox, performance and type system; WASM remains a possible future option for heavier mods.

## References

Luau documentation (sandboxing, type checking, performance), `mlua` documentation, Roblox engine scripting model (event-driven, sandboxed), Factorio and Garry's Mod modding APIs (concepts).

## Consequences

- Script-facing documentation is generated from the API registration (signature, errors, server/client availability, version).
- Gameplay prototyping is possible without recompiling Rust.

## Risks

- **`mlua` capabilities differ from assumptions** (e.g. memory limits under Luau). Mitigation: verification step before adoption.
- **Performance traps** (scripts called too often). Mitigation: per-frame budget, profiler counters per script.
- **Sandbox escapes** through exposed APIs. Mitigation: escape tests, allowlist review per API addition.

## Revisit triggers

Measured script cost too high; modding requires capabilities that cannot be sandboxed.

## Acceptance conditions

Owner review before roadmap step 7, after the `mlua` verification step.
