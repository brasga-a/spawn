# Spawn Engineering Documentation

> Status: **foundation slice `EXISTS` on Linux/Wayland; [Milestone 2 (First 3D and Free Camera)](first-3d/README.md) and wider engine architecture `PLANNED`**. `spawn-core`, `spawn-platform` (Wayland), `spawn-input`, `spawn-render` (Vulkan 1.3 clear frame) and the root `spawn-engine` composition binary implement the [foundation milestone](foundation/README.md); all other crates and Milestone 2 capabilities (`spawn-math`, `spawn-cli` Slang shader cook, 3D buffers/depth/pipelines, and free-fly camera) remain `SCAFFOLD` or `PLANNED`. The repository's root [AGENTS.md](../AGENTS.md) and crate-level `AGENTS.md` files remain the authoritative ownership and agent-behavior rules.

## Start here

| Document | What it answers |
| --- | --- |
| [Engine invariants](architecture/INVARIANTS.md) | What must never be violated anywhere in the engine? |
| [Engine design](architecture/DESIGN.md) | How do all subsystems fit together? |
| [Quality gates](architecture/GATES.md) | What must pass before a commit, merge, milestone or release? |
| [Decision register](architecture/DECISIONS.md) | What has been decided, what is pending, and when to revisit? |
| [Foundation overview](foundation/README.md) | What was built in Milestone 1 (`EXISTS` on Linux/Wayland), and what was excluded? |
| [Foundation specification](foundation/SPEC.md) | What must the first native runtime slice do? |
| [Foundation design](foundation/DESIGN.md) | How do the foundation subsystems cooperate without coupling? |
| [Foundation invariants](foundation/INVARIANTS.md) | What milestone rules govern the foundation slice? |
| [Foundation verification](foundation/VERIFICATION.md) | How were the foundation requirements proven? |
| [Foundation delivery plan](foundation/PLAN.md) | In what order did the foundation increments land? |
| [First 3D overview](first-3d/README.md) | What is planned next in Milestone 2 (`PLANNED`), and what stays out of scope? |
| [First 3D specification](first-3d/SPEC.md) | What must the initial 3D rendering, shader cook, math, and free-camera slice do? |
| [First 3D design](first-3d/DESIGN.md) | How do `spawn-math`, `spawn-cli`, `spawn-render`, and `spawn-engine` deliver 3D without coupling? |
| [First 3D invariants](first-3d/INVARIANTS.md) | What spatial, shader, GPU, and simulation/presentation rules govern Milestone 2? |
| [First 3D verification](first-3d/VERIFICATION.md) | How will we prove math precision, offline shader cooking, reversed-Z occlusion, and camera determinism? |
| [First 3D delivery plan](first-3d/PLAN.md) | In what order (Phases 0–6) will Milestone 2 land? |
| [Architecture decisions](adr/README.md) | Which choices are proposed or accepted? |
| [Approved dependencies](DEPENDENCIES.md) | Which third-party crates may be added, where, and which are rejected? |

## Documentation hierarchy

- **AGENTS.md:** enduring rules for coding agents, boundaries, and ownership.
- **docs/architecture:** engine-wide invariants (binding), technical design, quality gates and the decision register.
- **docs/DEPENDENCIES.md:** approved, rejected and deferred third-party crates.
- **docs/foundation:** specification and verification record of Spawn's first running slice (`EXISTS` on Linux/Wayland).
- **docs/first-3d:** time-bounded, testable specification of Milestone 2 (`PLANNED`: spatial math, offline Slang shader cook, Vulkan 1.3 3D pipeline/depth/allocator slice, and genre-free 60 Hz `f64` free-fly camera).
- **docs/adr:** rationale and status of engineering decisions; proposed ADRs **do not authorize** implementation.
- **Rust code and Cargo manifests:** the actual implemented state. If documentation disagrees with code, flag and resolve the conflict, do not assume the document was implemented.

## Change policy

When a milestone requirement changes, update its requirement ID, related invariants, verification cases, and affected ADRs in the same change. Do not silently promote *Proposed* decisions to *Accepted*. Do not invent API types or record speculative features as delivered.

Scope at this stage:
- **Implemented ([foundation/](foundation/README.md), `EXISTS` on Linux/Wayland):** native application lifecycle, window, raw input events (including relative pointer motion and cursor constraints), semantic input sampling, a Vulkan 1.3 clear frame, graceful recovery, and headless timing/input/composition tests.
- **Next planned slice ([first-3d/](first-3d/README.md), `PLANNED`):** `spawn-math` (`f32` primitives, `f64` world positions, reversed-Z, single-site Vulkan Y-flip, `libm` deterministic subset), offline Slang → SPIR-V shader cooking in `spawn-cli` ([ADR-0014](adr/0014-shader-toolchain.md)), in-house Vulkan 1.3 block allocator, buffers, `D32_SFLOAT` reversed-Z depth, and dynamic-rendering mesh pipeline in `spawn-render`, plus a genre-free 60 Hz `f64` free-fly camera with `alpha` presentation interpolation and camera-relative `f32` submission in `spawn-engine`. No genre-specific behavior.
