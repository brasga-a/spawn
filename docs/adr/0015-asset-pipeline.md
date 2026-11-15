# ADR-0015 — Asset Pipeline and Asset Identity

**Status:** Accepted  
**Scope:** Import, cooking, identity and runtime loading of assets (`spawn-assets`, `spawn-cli`).  
**Related:** [ADR-0013](0013-declarative-data-format.md) (data format), [ADR-0014](0014-shader-toolchain.md) (shaders), [ADR-0020](0020-memory-strategy.md) (memory budgets).

## Context

Meshes, skeletons, animations, textures, materials, audio and definitions must be imported from authoring formats, converted to formats efficient for the runtime, referenced stably across renames, and loaded asynchronously within the memory budget of the reference hardware (4 GB VRAM, ~2.8 GB usable budget reported by the driver; 16 GB RAM).

## Decision

1. **Two stages:**
   - **Import/cook (offline, `spawn-cli`):** source files → cooked binary assets.
   - **Load (runtime, `spawn-assets`):** cooked assets only in shipping builds; development builds may cook on demand for hot reload.
2. **Source formats:**
   - **3D: glTF 2.0 (primary) and FBX (secondary).**
     - **glTF 2.0** (`.gltf`/`.glb`): the reference format, parsed by an in-house importer. Preferred for all content Spawn's own pipeline produces; Blender exports it natively.
     - **FBX** through **ufbx** (single-file C library, MIT), used **only inside `spawn-cli`** (development tooling; never linked into games or servers). It gives access to FBX-based content: animation libraries such as Mixamo, asset stores, and Maya/3ds Max pipelines. Bindings are either thin in-house FFI or an existing binding crate, chosen at implementation under [ADR-0008](0008-third-party-license-policy.md). Both importers produce the same internal representation, so cooked assets do not depend on the source format.
     - The Autodesk FBX SDK is not used: ufbx covers the need, and the SDK's terms restrict redistribution with an MIT-licensed, publicly distributed tool.
   - **Textures: three source formats, each with a role.** All are **cooked to GPU block-compressed formats** (BC7 for color, BC5 for normals, BC4 for single-channel, BC6H for HDR) with mipmaps; the runtime never reads PNG, JPEG or EXR.

     | Format | Role | Rules |
     | --- | --- | --- |
     | **PNG** | Default for every texture | Required for normal maps, masks (roughness/metallic/AO), height maps and anything needing alpha or 16 bits per channel |
     | **JPEG** | Photo-based color (albedo) textures that already exist as JPEG | Accepted with an import warning (double lossy compression); **rejected** for normal maps, masks and height maps |
     | **EXR / HDR** | High dynamic range: skyboxes, environment maps, lighting data | Cooked to BC6H or kept as float data where required |

     Decoders run only in `spawn-cli`; pure-Rust decoder crates are preferred, chosen at implementation under ADR-0008. Compressor choice is part of the first texture import task.
   - Audio: source formats decoded by the decoder chosen in [ADR-0006](0006-in-house-audio.md).
3. **Asset identity: UUID per asset**, stored in a sidecar `.meta` file (RON) next to the source. Renames and moves keep identity. Runtime references use asset IDs; files are never referenced by path in game data.
4. **Cooking cache:** cooked output is keyed by a content hash of the source, the `.meta` settings and the cooker version, so unchanged assets are not re-cooked.
5. **Cooked formats:** in-house, versioned binary layouts designed for direct loading (minimal parsing, aligned data that can be uploaded to the GPU without restructuring). Every cooked file has a header with magic, format version and asset ID.
6. **Runtime loading:** asynchronous on background jobs ([ADR-0012](0012-job-system.md)), completion applied at sync points; reference-counted handles; explicit memory budgets per asset class.
7. **Packaging:** shipping builds pack cooked assets into archive files through `spawn-io` virtual mounts; format decided when the first shipping build is needed.

## Alternatives

- **Using source files at runtime:** simple, slow loading and no compression; rejected for shipping.
- **Path-based identity:** simple, breaks on rename; rejected.
- **FBX as primary source:** dominant in AAA pipelines, but proprietary and inconsistently specified; kept secondary through ufbx.
- **FBX through the Autodesk FBX SDK:** proprietary with redistribution restrictions, and unnecessary given ufbx; rejected.
- **PNG only:** simplest, but forces re-encoding of photo textures that only exist as JPEG; JPEG is allowed for that role only.
- **KTX2/Basis Universal:** portable compressed textures; desktop-only targets make BCn simpler; may be reconsidered.

## References

glTF 2.0 specification (Khronos), ufbx (MIT) and Godot's ufbx-based FBX importer, Unity's `.meta`/GUID model, Bevy asset system (MIT/Apache-2.0), "Our Machinery" blog on asset pipelines, Intel ISPC texture compressor and `bc7enc` (check licenses per ADR-0008).

## Consequences

- `spawn-cli` owns import and cooking; the editor reuses the same code.
- `.meta` files are committed with sources.

## Risks

- **Orphaned or duplicated `.meta` files** (copied assets keeping the same UUID). Mitigation: `spawn-cli` validation command detecting duplicates and orphans.
- **VRAM exhaustion** on the reference GPU. Mitigation: budgets, texture streaming planned before large maps.
- **glTF variations** across exporters. Mitigation: importer validation with clear errors; test corpus of exported files.
- **FBX inconsistencies** (units, axes, scale, version differences). Mitigation: conversion to Spawn conventions ([ADR-0010](0010-math-library.md)) in one place, a test corpus including Mixamo exports, and errors naming the file and node.
- **Native C code in `spawn-cli`** (ufbx). Mitigation: confined to the importer module, never in runtime crates; the C compiler requirement documented for tool builds.

## Revisit triggers

Load times or memory exceed budgets; a content workflow cannot be expressed with glTF.

## Acceptance conditions

Accepted by the owner. Applying it is an implementation task.
