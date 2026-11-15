# ADR-0007 — Toolchain, Lints and Continuous Integration

**Status:** Accepted  
**Scope:** Whole workspace. Applies from the first implementation commit (Phase 1).  
**Related:** [ADR-0008](0008-third-party-license-policy.md) (license checks in CI), [ADR-0009](0009-error-handling-and-diagnostics.md) (panic policy).

## Context

The workspace has no `rust-toolchain.toml`, no `rustfmt`/`clippy` configuration, and no CI. Root [Engineering Standards](../../AGENTS.md#engineering-standards) require `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, documented `unsafe`, and no `unwrap()` on fallible runtime paths, but nothing enforces them. The repository is hosted on GitHub.

## Decision

1. **Toolchain pin:** `rust-toolchain.toml` pins an exact stable release (initially the local toolchain, Rust 1.99.1) with `rustfmt` and `clippy` components. The pinned version is the minimum supported Rust version. Upgrades are deliberate, one commit, with the full check suite passing.
2. **Formatting:** `rustfmt` with default style; a `rustfmt.toml` only if a concrete need appears.
3. **Lints in `[workspace.lints]`**, inherited by every package with `[lints] workspace = true`:
   - `unsafe_op_in_unsafe_fn = "deny"`
   - `clippy::undocumented_unsafe_blocks = "deny"` (enforces `// SAFETY:` comments)
   - `clippy::unwrap_used = "deny"`, `clippy::expect_used = "deny"`, `clippy::panic = "deny"`, `clippy::todo = "deny"`, `clippy::unimplemented = "deny"`; allowed only in tests
   - `clippy::indexing_slicing = "deny"` in modules that parse untrusted data (files, packets, script values), where `get()` must be used; not enabled globally, because bounds-proven indexing in math and storage code would become noise
   - `missing_docs = "warn"` for public items (promoted to errors by `-D warnings`)
   - `clippy::all = "deny"`; `clippy::pedantic` not enabled globally (too noisy); individual pedantic lints added when they catch a real problem.
4. **Fallible functions return `Result`.** Any function that can fail for reasons outside the caller's control (I/O, parsing, resource creation, untrusted input, OS/GPU calls) returns `Result` with a typed error ([ADR-0009](0009-error-handling-and-diagnostics.md)). This is mandatory for foundation crates (`spawn-core`, `spawn-math`, `spawn-io`, `spawn-ecs`, `spawn-platform`, …), which never panic on behalf of their callers. Absence of a value that is not an error uses `Option`. Functions that cannot fail do not return `Result` (no artificial error paths). Precondition violations that indicate a bug in the caller (e.g. invalid indices proven by construction) may use `debug_assert!`; if a precondition depends on runtime data, it is an error, not an assertion.
5. **CI (GitHub Actions), on pull requests only** (plus manual `workflow_dispatch`), required to pass before merging:
   - **Linux job:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo doc --no-deps` with warnings denied.
   - **Windows job:** `cargo check --workspace --all-targets` and `cargo test --workspace` on a Windows runner (catches Win32 backend compile errors).
   - **License/dependency job:** `cargo-deny` per [ADR-0008](0008-third-party-license-policy.md).
   - **Headless guarantee:** CI has no GPU and no display. Tests that need a GPU or window are `#[ignore]`d and run manually on the reference hardware.
6. **Local check command:** a single documented command (e.g. a Cargo alias) runs the same checks as the Linux CI job locally, since CI does not run on direct pushes.
7. **Builds:** dev profile with `opt-level = 1` for the workspace and `opt-level = 3` for dependencies once dependencies exist (games are unusable at `opt-level = 0`); release profile with `lto = "thin"` and `codegen-units = 1` evaluated by measurement.

## Alternatives

- **CI on every push:** catches problems earlier; rejected by the owner in favor of pull-request-only runs.
- **Floating `stable` channel:** always current, but breaks reproducibility and can introduce new lint failures unexpectedly.
- **Nightly toolchain:** access to `std::simd` and other unstable features, but unstable APIs and breakage; rejected while stable suffices.
- **Lints per crate (`#![deny(...)]`):** easy to forget in new crates; workspace lints are central.
- **Self-hosted CI with GPU:** needed eventually for render tests; deferred until render code exists.

## Consequences

- Every package manifest gains `[lints] workspace = true` (implementation change, done when this ADR is applied).
- New crates inherit lints automatically when following the root documentation rules.
- GPU/window behavior is never verified by CI; manual smoke tests on the reference hardware remain mandatory (see [VERIFICATION](../foundation/VERIFICATION.md)).

## Risks

- **Direct pushes bypass CI.** Mitigation: work through pull requests; run the local check command before pushing.
- Strict lints slow down early prototyping. Mitigation: `#[allow]` with a written justification at the narrowest scope, never crate-wide.
- Windows CI covers compilation and headless tests only, not real Windows behavior. Mitigation: Windows smoke test before claiming Windows support (ADR-0001).

## Revisit triggers

A required feature needs nightly; GPU tests become necessary for regressions; CI time per pull request becomes a bottleneck.

## Acceptance conditions

Accepted by the owner. Applying it (configuration files, manifests, code) is an implementation task.
