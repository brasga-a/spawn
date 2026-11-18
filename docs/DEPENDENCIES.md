# Approved Dependencies

Registry of third-party dependencies approved by the owner for Spawn. **Approved** means a crate may be added when real code needs it; it does not mean it is already in a `Cargo.toml`. The manifests are the only evidence of what is actually used.

Rules ([ADR-0008](adr/0008-third-party-license-policy.md), root [Workspace](../AGENTS.md#workspace)):

- Before adding a crate: verify its current version, maintenance and license; pin it in `[workspace.dependencies]` with `default-features = false` where possible.
- Add it only to the crates listed in its **Where** column; using it elsewhere needs an update to this file.
- A crate not listed here needs owner approval and an entry in this file before it is added.
- When a dependency is actually added, mark it in the **In use** column in the same change.

**Kind:** `dep` = engine or game runtime; `tool` = development tools only (never linked into games or servers); `test` = tests and benchmarks only; `oracle` = reference implementation compared against in tests; `fallback` = approved replacement if the in-house implementation is not viable.

## Decided by ADRs

| Crate | License | Kind | Where | Purpose | ADR | In use |
| --- | --- | --- | --- | --- | --- | --- |
| `windows-sys` | MIT/Apache-2.0 | dep | `spawn-platform`, `spawn-audio` | Win32 and WASAPI declarations | [0001](adr/0001-in-house-platform-layer.md), [0006](adr/0006-in-house-audio.md) | no |
| `wayland-client` | MIT | dep | `spawn-platform` | Wayland protocol | [0001](adr/0001-in-house-platform-layer.md) | yes (`spawn-platform`, feature `wayland`; `dlopen` + `system`) |
| `wayland-protocols` | MIT | dep | `spawn-platform` | Wayland protocol extensions | [0001](adr/0001-in-house-platform-layer.md) | yes (`spawn-platform`, feature `wayland`) |
| `x11rb` | MIT/Apache-2.0 | dep | `spawn-platform` | X11 protocol | [0001](adr/0001-in-house-platform-layer.md) | no |
| `xkbcommon` | MIT | dep | `spawn-platform` | Linux keyboard keymaps | [0001](adr/0001-in-house-platform-layer.md) | no |
| `raw-window-handle` | MIT/Apache-2.0/Zlib | dep | `spawn-platform`, `spawn-render` | Window handle contract | [0004](adr/0004-in-house-graphics-api.md) | yes (`spawn-platform`, feature `wayland`; `spawn-render`, feature `vulkan`) |
| `ash` | MIT/Apache-2.0 | dep | `spawn-render` | Vulkan bindings | [0004](adr/0004-in-house-graphics-api.md) | yes (`spawn-render`, feature `vulkan`; 0.38, `loaded` + `std`) |
| `serde` | MIT/Apache-2.0 | dep | data-defining crates | Serialization framework | [0013](adr/0013-declarative-data-format.md) | no |
| `ron` | MIT/Apache-2.0 | dep | data-defining crates | Authored data format | [0013](adr/0013-declarative-data-format.md) | no |
| `mlua` (feature `luau`) | MIT | dep | `spawn-scripting` | Luau integration | [0017](adr/0017-luau-scripting-runtime.md) (proposed) | no |
| ufbx bindings | MIT | tool | `spawn-cli` | FBX import | [0015](adr/0015-asset-pipeline.md) | no |
| `slangc` (external compiler) | Apache-2.0 WITH LLVM-exception | tool | `spawn-cli` | Slang → SPIR-V | [0004](adr/0004-in-house-graphics-api.md), [0014](adr/0014-shader-toolchain.md) | yes (`spawn-cli`, external binary pinned at `2026.22`) |
| `cargo-deny` | MIT/Apache-2.0 | tool | CI | License and advisory checks | [0008](adr/0008-third-party-license-policy.md) | yes (CI) |

## Testing and quality

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `proptest` | MIT/Apache-2.0 | test | all crates | Property-based tests | yes (`spawn-math`, `spawn-render`, `spawn-cli`, `spawn-engine` dev-deps) |
| `cargo-fuzz` | MIT/Apache-2.0 | test | parsers (packets, RON, glTF, cooked assets, packages) | Fuzzing | yes (`crates/spawn-render/fuzz`, excluded standalone fuzz package) |
| `arbitrary` | MIT/Apache-2.0 | test | parsers | Structured fuzz inputs | yes (`spawn-render` dev-deps and `crates/spawn-render/fuzz`) |
| `loom` | MIT | test | `spawn-jobs`, lock-free code | Concurrency model checking | no |
| `criterion` or `divan` (choose one) | MIT/Apache-2.0 | test | all crates | Benchmarks on the reference hardware | no |
| `insta` | Apache-2.0 | test | all crates | Snapshot tests (e.g. error report format) | yes (`spawn-math`, `spawn-render`, `spawn-cli`, `spawn-engine` dev-deps) |
| `cargo-nextest` | MIT/Apache-2.0 | tool | CI, local | Faster test runner | no |

## Profiling and diagnostics

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `tracy-client` | MIT/Apache-2.0 (Tracy: BSD-3) | dep, behind a feature | `spawn-profiler` | Frame profiler backend for the logging/timing facade | no |
| `renderdoc` | MIT/Apache-2.0 | dep, behind a feature | `spawn-render` | Trigger RenderDoc captures | no |
| `minidump-writer` | MIT | dep | root binary | Native crash minidumps ([ADR-0009](adr/0009-error-handling-and-diagnostics.md)) | no |
| `minidump-processor` | MIT | tool | crash tooling | Read and symbolize minidumps | no |

## Platform, input and system

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `rustix` | Apache-2.0/MIT | dep | `spawn-platform`, `spawn-io`, `spawn-audio` | Linux system calls without raw libc | yes (`spawn-platform`, feature `wayland`) |
| `evdev` | MIT/Apache-2.0 | dep | `spawn-platform` | Linux gamepads ([ADR-0018](adr/0018-gamepad-input.md)) | no |
| `udev` | MIT | dep | `spawn-platform` | Device hotplug | no |
| `hidapi` | MIT (C library: BSD) | dep | `spawn-platform` | HID access (DualSense, Switch Pro) | no |
| `pipewire` | MIT | dep | `spawn-audio` | Linux audio output ([ADR-0006](adr/0006-in-house-audio.md)) | no |
| `socket2` | MIT/Apache-2.0 | dep | `spawn-network` | UDP socket options | no |
| `core_affinity` | MIT/Apache-2.0 | dep | `spawn-jobs` | Thread-to-core pinning | no |

## Math, memory and jobs

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `libm` | MIT/Apache-2.0 | dep | `spawn-math` | Deterministic transcendental functions ([ADR-0010](adr/0010-math-library.md)) | yes (`spawn-math`; `default-features = false`) |
| `glam` | MIT/Apache-2.0 | oracle | `spawn-math` tests | Compare math results | yes (`spawn-math` dev-dependency) |
| `crossbeam-deque` | MIT/Apache-2.0 | dep | `spawn-jobs` | Work-stealing deques ([ADR-0012](adr/0012-job-system.md)) | no |
| `crossbeam-utils` | MIT/Apache-2.0 | dep | `spawn-jobs` | Cache padding, utilities | no |
| `bytemuck` | MIT/Apache-2.0/Zlib | dep | `spawn-render`, `spawn-assets`, `spawn-math` | Safe plain-data byte casting | yes (`spawn-render`, feature `vulkan`; 1.25, `derive`) |
| `bumpalo` | MIT/Apache-2.0 | dep | crates with per-frame data | Frame arenas ([ADR-0020](adr/0020-memory-strategy.md)) | no |
| `smallvec` | MIT/Apache-2.0 | dep | any | Small inline vectors | no |
| `arrayvec` | MIT/Apache-2.0 | dep | any | Fixed-capacity vectors | no |
| `bitflags` | MIT/Apache-2.0 | dep | any | Typed bit flags | no |
| `rustc-hash` | MIT/Apache-2.0 | dep | any | Fast hashing for **internal keys only**; keys from network, files or mods use the standard hasher | no |

## Assets and pipeline

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `uuid` | MIT/Apache-2.0 | dep | `spawn-assets`, `spawn-scene` | Asset and node IDs ([ADR-0015](adr/0015-asset-pipeline.md)) | no |
| `blake3` | CC0/Apache-2.0 | dep | `spawn-assets`, packages | Content hashes and integrity | no |
| `lz4_flex` | MIT | dep | `spawn-assets`, `spawn-network` | Fast compression | no |
| `zstd` or `ruzstd` (choose one) | BSD-3 / MIT | dep | packaging | Strong compression for archives | no |
| `gltf` | MIT/Apache-2.0 | tool | `spawn-cli` | glTF parsing under the in-house importer | no |
| `meshopt` | MIT | tool | `spawn-cli` | Vertex optimization, LODs, meshlets | no |
| `mikktspace` | Zlib/Apache-2.0 | tool | `spawn-cli` | MikkTSpace tangents (match DCC tools) | no |
| `png` | MIT/Apache-2.0 | tool | `spawn-cli` | PNG decoding | no |
| `zune-jpeg` | MIT/Apache-2.0/Zlib | tool | `spawn-cli` | JPEG decoding | no |
| `exr` | BSD-3 | tool | `spawn-cli` | EXR (HDR) decoding | no |
| `intel_tex_2` | MIT | tool | `spawn-cli` | BC7/BC6H/BC5 compression | no |
| `rspirv` | Apache-2.0 | tool | `spawn-cli` | SPIR-V inspection if Slang reflection is insufficient | yes (`spawn-cli` 0.13) |
| `notify` | CC0 | tool | `spawn-cli`, `spawn-editor` | File watching for hot reload | no |
| `gpu-allocator` | MIT/Apache-2.0 | fallback | `spawn-render` | Replacement if the in-house allocator is not viable | no |

## Audio

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `opus` | BSD-3 (libopus) | dep | `spawn-audio` | Opus decoding | no |
| `lewton` | MIT/Apache-2.0 | dep | `spawn-audio` | Vorbis decoding (pure Rust) | no |
| `rubato` | MIT | dep | `spawn-audio` | Resampling | no |
| `realfft` | MIT/Apache-2.0 | dep | `spawn-audio` | FFT for HRTF convolution | no |
| `rustfft` | MIT/Apache-2.0 | dep | `spawn-audio` | FFT (used by `realfft`) | no |
| `sofar` | MIT/Apache-2.0 | dep | `spawn-audio` | SOFA HRTF loading and convolution | no |

## Networking and security

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `chacha20poly1305` | MIT/Apache-2.0 | dep | `spawn-network` | Authenticated encryption ([ADR-0016](adr/0016-networking-and-replication.md)) | no |
| `x25519-dalek` | BSD-3 | dep | `spawn-network` | Key exchange | no |
| `getrandom` | MIT/Apache-2.0 | dep | `spawn-network` | OS randomness | no |
| `tract` or `candle` (choose one) | MIT/Apache-2.0 | dep | anti-cheat analysis (server side) | Small-model inference ([ADR-0019](adr/0019-anti-cheat-and-client-trust.md)) | no |

## UI, text and editor

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `cosmic-text` | MIT/Apache-2.0 | dep | `spawn-ui` | Text layout | no |
| `rustybuzz` | MIT | dep | `spawn-ui` | Text shaping | no |
| `ttf-parser` | MIT/Apache-2.0 | dep | `spawn-ui` | Font parsing | no |
| `swash` | MIT/Apache-2.0 | dep | `spawn-ui` | Font rasterization | no |
| `unicode-segmentation` | MIT/Apache-2.0 | dep | `spawn-ui` | Text segmentation | no |
| `unicode-bidi` | MIT/Apache-2.0 | dep | `spawn-ui` | Bidirectional text | no |
| `egui` | MIT/Apache-2.0 | tool | `spawn-editor` | Editor UI (never in the game runtime) | no |
| `egui_dock` | MIT | tool | `spawn-editor` | Dockable panels | no |

## Tools

| Crate | License | Kind | Where | Purpose | In use |
| --- | --- | --- | --- | --- | --- |
| `clap` | MIT/Apache-2.0 | tool | `spawn-cli` | Command-line arguments | yes (`spawn-cli` 4.6) |
| `rmcp` | MIT/Apache-2.0 | tool | `spawn-mcp` | MCP SDK (confirm maturity before adoption) | no |

## Rejected

| Crate | Reason |
| --- | --- |
| `symphonia` | MPL-2.0; not approved by the owner. `opus` and `lewton` cover decoding. |
| `bevy_ecs` | Rejected as a dependency ([ADR-0011](adr/0011-ecs-architecture.md)); design reference only. |
| `wgpu` | Rejected as a dependency ([ADR-0004](adr/0004-in-house-graphics-api.md)); design reference only. |
| `winit` | Rejected ([ADR-0001](adr/0001-in-house-platform-layer.md)). |
| FMOD, Wwise, Havok | Royalties or revenue-tied fees ([ADR-0008](adr/0008-third-party-license-policy.md)). |
| Autodesk FBX SDK | Redistribution restrictions; ufbx covers the need ([ADR-0015](adr/0015-asset-pipeline.md)). |

## Deferred

| Crates | Note |
| --- | --- |
| `tokio`, `axum`, `tower`, `sqlx` | For future online backend services only; never in engine crates or the game's UDP path. Requires its own ADR. |
