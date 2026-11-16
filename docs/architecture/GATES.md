# Quality Gates

Mandatory checks a change must pass before it moves forward. A gate is **passed** only when every item is satisfied or explicitly marked not applicable with a reason. Failing a gate blocks the step; it is never waived silently.

Gates reference [INVARIANTS](INVARIANTS.md) by ID. Tooling referenced here (toolchain file, lints, CI, local check command) is decided in [ADR-0007](../adr/0007-toolchain-lints-and-ci.md) and is `PLANNED` until applied; until then, gates are checked by running the commands manually.

## Gate overview

| Gate | When | Who checks |
| --- | --- | --- |
| [G0 — Local change](#g0--local-change) | Before every commit/push | Author (human or agent) |
| [G1 — Pull request](#g1--pull-request) | Before merging | CI + reviewer |
| [G2 — Special changes](#g2--special-changes) | When a change adds a dependency, `unsafe`, script API, data format, ADR, etc. | Reviewer + owner where stated |
| [G3 — Milestone exit](#g3--milestone-exit) | Before declaring a milestone complete | Owner |
| [G4 — Release](#g4--release) | Before any public build | Owner |

## G0 — Local change

- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes (SPN-INV-030, SPN-INV-033).
- [ ] `cargo test --workspace` passes; new behavior has tests.
- [ ] `cargo doc --no-deps` has no warnings; public items are documented.
- [ ] Nothing claimed as working that was not run; anything not verified is stated in the commit or PR.
- [ ] Affected `AGENTS.md` files and docs are updated in the same change (SPN-INV-101).

The local check command from ADR-0007 runs all of the above once it exists.

## G1 — Pull request

CI runs on pull requests only ([ADR-0007](../adr/0007-toolchain-lints-and-ci.md)).

- [ ] **Linux job:** fmt, clippy, tests, docs.
- [ ] **Windows job:** check and tests.
- [ ] **License job:** `cargo-deny` passes (SPN-INV-020, SPN-INV-021).
- [ ] **Layering:** no forbidden dependency edge; graph acyclic; headless crates do not pull native/GPU backends (SPN-INV-001 to SPN-INV-012).
- [ ] **Scope:** one focused change; no unrelated refactors or renames.
- [ ] **PR description** states: requirements covered, tests run (with output), platforms tested, invariants touched, known limitations.
- [ ] GPU/window/audio behavior affected → manual check on the reference hardware recorded (SPN-INV-091).

## G2 — Special changes

Apply each block whose trigger matches.

### New or changed dependency
- [ ] Listed in [DEPENDENCIES](../DEPENDENCIES.md) for this crate (SPN-INV-023); otherwise owner approval and a new entry first.
- [ ] Current version, maintenance and license verified; pinned in `[workspace.dependencies]`; default features disabled where possible.
- [ ] "In use" column updated.
- [ ] Proprietary SDK → isolated integration crate behind a feature, with its own ADR (SPN-INV-022).

### New `unsafe` code
- [ ] Needed (FFI, GPU, storage internals, measured performance); confined to the owning module.
- [ ] `// SAFETY:` comment states the invariant relied on (SPN-INV-033).
- [ ] Tested where feasible; Miri for pure-Rust `unsafe`; `loom` for concurrency primitives.

### Untrusted-input parser (packets, files, assets, packages, script values)
- [ ] Sizes and nesting bounded before allocation (SPN-INV-034).
- [ ] `clippy::indexing_slicing` denied in the module.
- [ ] Fuzz target added and run.

### Simulation code used in prediction
- [ ] Uses only the deterministic math subset (SPN-INV-044).
- [ ] Parallel results combined in fixed order (SPN-INV-045).
- [ ] Determinism test with recorded inputs; cross-platform comparison when Windows CI can run it.
- [ ] Multiplayer declaration: owner, replication, prediction, misprediction behavior (SPN-INV-065).

### New script API (Luau)
- [ ] Allowlisted deliberately; documented (signature, semantics, errors, client/server availability, API version).
- [ ] Every argument validated (SPN-INV-071); sandbox-escape and limit tests added.
- [ ] Script API changelog updated; version bumped if breaking.

### New data definition or file format
- [ ] `version` field and migration path; unknown fields rejected (SPN-INV-080).
- [ ] Errors name file, field and reason (SPN-INV-081).
- [ ] Round-trip and invalid-input tests.

### Error types
- [ ] Location, stable code and cause chain present (SPN-INV-032); snapshot test of the report format.

### Ported or adapted external code
- [ ] Source license allows it; notice kept; `THIRD_PARTY_NOTICES.md` updated (SPN-INV-024).

### New crate or tool
- [ ] Approved by the owner (and by an ADR where one requires it).
- [ ] `AGENTS.md` with the standard sections; added to the root map and to [DESIGN](DESIGN.md#2-crate-dependency-plan).
- [ ] Inherits workspace version, edition and lints.

### ADR change
- [ ] Status changes only by explicit owner decision.
- [ ] ADR index, [DECISIONS](DECISIONS.md), root `AGENTS.md` and affected crate `AGENTS.md` files updated together (SPN-INV-101).
- [ ] [INVARIANTS](INVARIANTS.md) and gates updated if a rule changes.
- [ ] All relative links and anchors resolve.

## G3 — Milestone exit

- [ ] Every requirement of the milestone specification has passing verification (e.g. [foundation VERIFICATION](../foundation/VERIFICATION.md#exit-gate)).
- [ ] All applicable invariants verified; any exception documented with an owner decision.
- [ ] Manual smoke test on the reference hardware recorded (OS, compositor, GPU, driver, versions) (SPN-INV-090, SPN-INV-091).
- [ ] Vulkan validation layers report no errors in the tested scenarios (SPN-INV-092).
- [ ] Performance measured against the milestone's budgets on the reference hardware; regressions explained.
- [ ] ADRs used by the milestone are accepted and match the implementation.
- [ ] Status tags in `AGENTS.md` files updated (`SCAFFOLD` → `EXISTS` only with source and tests, SPN-INV-100).
- [ ] Known limitations and untested platforms listed.

## G4 — Release

- [ ] G3 passed for every included milestone.
- [ ] Windows smoke test recorded before Windows is claimed as supported (SPN-INV-090).
- [ ] `cargo-deny` clean; `THIRD_PARTY_NOTICES.md` complete; proprietary SDK terms verified for redistribution.
- [ ] Release symbols archived per build; crash reporting works and uploads only with consent (SPN-INV-037).
- [ ] Terms of Service and Privacy Policy cover anti-cheat data, human review and penalties, user-generated content and crash reports; legal review done (LGPD/GDPR) ([ADR-0019](../adr/0019-anti-cheat-and-client-trust.md)).
- [ ] Shipping build loads only cooked assets (SPN-INV-083).
- [ ] Security review of network protocol, package loading and script sandbox.
- [ ] Release notes list supported platforms, minimum hardware, known issues.
