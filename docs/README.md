# Spawn Engineering Documentation

> Status: **foundation slice `EXISTS` on Linux/Wayland; wider engine architecture `PLANNED`**. `spawn-core`, `spawn-platform` (Wayland), `spawn-input`, `spawn-render` (Vulkan 1.3 clear frame) and the root `spawn-engine` composition binary implement the [foundation milestone](foundation/README.md); all other crates remain `SCAFFOLD` or `PLANNED`. The repository's root [AGENTS.md](../AGENTS.md) and crate-level `AGENTS.md` files remain the authoritative ownership and agent-behavior rules.

## Start here

| Document | What it answers |
| --- | --- |
| [Engine invariants](architecture/INVARIANTS.md) | What must never be violated anywhere in the engine? |
| [Engine design](architecture/DESIGN.md) | How do all subsystems fit together? |
| [Quality gates](architecture/GATES.md) | What must pass before a commit, merge, milestone or release? |
| [Decision register](architecture/DECISIONS.md) | What has been decided, what is pending, and when to revisit? |
| [Foundation overview](foundation/README.md) | What are we building first, and what is not included? |
| [Specification](foundation/SPEC.md) | What must the first executable slice do? |
| [Technical design](foundation/DESIGN.md) | How do subsystems cooperate without coupling? |
| [Invariants](foundation/INVARIANTS.md) | What must never be violated? |
| [Verification plan](foundation/VERIFICATION.md) | How do we prove the requirements? |
| [Delivery plan](foundation/PLAN.md) | In what order should changes land? |
| [Architecture decisions](adr/README.md) | Which choices are proposed or accepted? |
| [Approved dependencies](DEPENDENCIES.md) | Which third-party crates may be added, where, and which are rejected? |

## Documentation hierarchy

- **AGENTS.md:** enduring rules for coding agents, boundaries, and ownership.
- **docs/architecture:** engine-wide invariants (binding), technical design, quality gates and the decision register.
- **docs/DEPENDENCIES.md:** approved, rejected and deferred third-party crates.
- **docs/foundation:** time-bounded, testable specification of Spawn's first running slice.
- **docs/adr:** rationale and status of engineering decisions; proposed ADRs **do not authorize** implementation.
- **Rust code and Cargo manifests:** the actual implemented state. If documentation disagrees with code, flag and resolve the conflict, do not assume the document was implemented.

## Change policy

When a foundation requirement changes, update its requirement ID, related invariants, verification cases, and affected ADRs in the same change. Do not silently promote *Proposed* decisions to *Accepted*. Do not invent API types or record speculative features as delivered.

Scope at this stage: native application lifecycle, window, raw input events, semantic input sampling, a GPU-backed clear frame, graceful recovery, and headless timing tests. No FPS-specific behavior.
