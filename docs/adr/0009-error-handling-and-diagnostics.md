# ADR-0009 — Error Handling, Panics and Diagnostics

**Status:** Accepted  
**Scope:** All crates; logging owned by `spawn-profiler`.  
**Related:** [ADR-0007](0007-toolchain-lints-and-ci.md) (lints), [DESIGN](../foundation/DESIGN.md) (foundation diagnostics).

## Context

Root rules require typed errors and no panics on fallible runtime paths, but do not define how errors are written, how they cross crates, how panics behave, or how diagnostics are emitted. The foundation design already chose stderr with `std` only for its milestone.

## Decision

### Errors

1. **Hand-written error types**, no `thiserror` or `anyhow` in engine crates. Each crate defines error enums per operation area (not one giant enum), implementing `Display` and `std::error::Error` with `source()` chaining. Boilerplate is cheap with AI assistance and avoids a proc-macro dependency.
2. **Errors carry context** needed to act on them: operation, resource identity (path, asset ID, handle), and the underlying cause. They never expose backend types (Vulkan results, OS error structs) in public APIs; backend details are converted into Spawn variants or kept as opaque source text.
3. **Compiler-like traceability.** Every error must lead a developer to the exact place it originated:
   - **Origin location:** error constructors use `#[track_caller]` and store `std::panic::Location` (file, line, column) of where the error was created. Cost: one pointer.
   - **Error codes:** each error variant has a stable code per crate (e.g. `ASSET-0003`), documented, so reports and logs are searchable.
   - **Cause chain:** `source()` links every underlying error; reporting prints the whole chain.
   - **Backtraces:** captured with `std::backtrace::Backtrace` in development builds (and when explicitly enabled in release), not on hot paths.
   - **Report format** modeled on compiler diagnostics:

     ```text
     error[ASSET-0003]: failed to load texture
       --> crates/spawn-assets/src/load.rs:142:17
        = asset: 3f2a9c1e-… (textures/wall.png)
     caused by: io error: No such file or directory
     ```

4. **Recoverable vs. fatal** is expressed by the caller's handling, not by the type; the foundation failure policy ([INVARIANTS](../foundation/INVARIANTS.md#failure-policy)) applies.
5. The root binary may aggregate errors into one top-level type for reporting.

### Panics

6. Panics are reserved for invariant violations (bugs). Untrusted input (files, packets, scripts) never panics.
7. **Panic strategy: `unwind`** in all profiles, so the panic hook can write a crash report and owned resources are released.
8. **Crash reports.**
   - **Panics:** the root binary installs a panic hook that writes a crash report file: panic message, source location, thread name, full backtrace, build version and commit hash, OS and GPU/driver information, and the last log lines kept in a ring buffer.
   - **Native crashes** (segmentation faults or access violations in FFI, OS or GPU driver code): a crash handler writes a minidump plus the same metadata. Implementation (in-house writer or an MIT/Apache crate such as `minidump-writer`) is decided when native code first ships, under [ADR-0008](0008-third-party-license-policy.md).
   - **Symbols:** release builds keep split debug information per build, archived by version, so crash reports and minidumps can be symbolized to exact file and line.
   - **Upload:** never automatic without player consent; disclosure follows the Terms of Service and Privacy Policy ([ADR-0019](0019-anti-cheat-and-client-trust.md)). Local crash files are always written.

### Diagnostics

9. **Own minimal logging facade in `spawn-profiler`**: levels (error, warn, info, debug, trace), a target (crate/subsystem), and structured key-value fields. Compiled-out levels in release builds have zero cost. Output: stderr in development; file output added when needed.
10. Until `spawn-profiler` provides the facade, the foundation uses stderr through `std` only, as already decided in DESIGN.
11. In development builds, timing scopes and logs can be forwarded to the **Tracy** profiler (`tracy-client`, behind a feature, approved in [DEPENDENCIES](../DEPENDENCIES.md)).
12. `tracing` is a design reference, not a dependency. If a future tool integration requires it, an adapter from the facade is added rather than replacing it.

## Alternatives

- **`thiserror`:** less boilerplate; adds a proc-macro dependency and compile time. Acceptable fallback if hand-written errors prove error-prone.
- **`anyhow` in libraries:** loses typed errors; rejected (allowed only in development tools if justified).
- **`tracing` from the start:** mature ecosystem and tooling; rejected for control and dependency weight, kept as reference.
- **`panic = "abort"`:** smaller and faster binaries, but no cleanup or context logging without a crash reporter.

## Consequences

- `spawn-profiler` becomes a dependency of most crates once the facade exists; it must stay a foundation-level crate with no `spawn-*` dependencies. `spawn-core`, `spawn-math` and `spawn-jobs` do not log and do not depend on it; they expose hooks instead ([ADR-0012](0012-job-system.md)).
- Error types are part of each crate's public API and follow its documentation rules.

## Risks

- **Inconsistent error design across crates.** Mitigation: this ADR as the pattern; review new error types against it.
- **Error-size growth** from locations and optional backtraces in hot error paths. Mitigation: location is one pointer; backtraces captured only in development or on demand.
- **Logging on hot paths** (per entity, per frame) costs performance. Mitigation: level checks before formatting; no logging in inner loops; counters instead of logs for high-frequency events.
- **Real-time audio thread** must never log directly. Mitigation: audio reports through a lock-free queue drained elsewhere.

## Revisit triggers

The first shipped native code requires the minidump implementation decision; external tooling requires `tracing`; error boilerplate becomes a measurable maintenance problem.

## Acceptance conditions

Accepted by the owner. Applying it (configuration files, manifests, code) is an implementation task.
