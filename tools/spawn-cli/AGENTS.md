# spawn-cli

Local rules for `spawn-cli` (tool). Global rules: [root AGENTS.md](../../AGENTS.md).

## Purpose

Command-line tooling for development workflows: offline shader compilation (`EXISTS`), project creation, building, running, validation, and asset-related tasks (`PLANNED`). Development tooling, not engine runtime.

## Responsibilities

- Offline Slang → Vulkan 1.3 SPIR-V (`spirv_1_6`, column-major matrices) + reflection shader cooking (`spawn shaders build`, [ADR-0014](../../docs/adr/0014-shader-toolchain.md)) (`EXISTS`).
- Project scaffolding, build/run orchestration, and project validation commands (`PLANNED`).
- Asset and data validation/import commands that invoke `spawn-assets`, `spawn-scene`, and related crates headlessly (`PLANNED`).
- Exit codes and machine-readable output suitable for CI.

## Non-Responsibilities

- Reimplementing engine logic: cooked-shader format framing and pipeline-layout validation rules live in `spawn-render`; `spawn-cli` invokes them.
- Graphical authoring (`spawn-editor`), MCP protocol handling (`spawn-mcp`), runtime play of games (games link the engine themselves).

## Architectural Boundaries

- Depends on engine crates, never the reverse. Must not be depended on by engine crates. Headless: must not require `spawn-platform` windowing or `spawn-render` GPU backends (`vulkan` feature); depends on headless `spawn-render` (`default-features = false`) for shared shader/format contracts (`SPN-INV-009`).
- Any capability shared with the editor or MCP server belongs in an engine crate or a deliberately agreed shared layer, not copy-pasted between tools.

## Public API Guidelines

- The CLI surface (commands, flags, output formats, exit codes) is the public contract: version it, document it, keep it stable; the library portion is for tool reuse and testing only, stays minimal, and never leaks third-party types (`clap`, `rspirv`) in public signatures (`SPN-INV-011`).

## Internal Design Principles

- Thin command layer over engine APIs; commands are non-interactive by default and scriptable.
- External tool execution (`slangc`) sits behind the `SlangCompiler` trait so CI and `./scripts/check.sh` run headlessly without `slangc` installed.

## Performance and Safety

- `#![forbid(unsafe_code)]`; `#![deny(clippy::indexing_slicing)]` in source scanning and SPIR-V reflection modules (`SPN-INV-030`, `SPN-INV-034`).
- Treat project files, `.slang` sources, and compiler outputs as untrusted: bound file sizes before allocation, delete temporary `.spv` intermediates via RAII guards, and never make network calls without explicit command intent.

## Testing Requirements

- Integration tests invoking commands on unique `std::env::temp_dir()` subdirectories (mock `SlangCompiler` always run in CI + opt-in/auto-detected live `slangc` verification, `F3D-T06`); `insta` error-report snapshot tests; exit-code and `F3D-T13` headless-tree/`build.rs`/uncommitted-`.spv` audit tests.

## Governing Decisions

- **Accepted (binding):** [ADR-0015](../../docs/adr/0015-asset-pipeline.md) (asset pipeline), [ADR-0004](../../docs/adr/0004-in-house-graphics-api.md) (graphics API), [ADR-0009](../../docs/adr/0009-error-handling-and-diagnostics.md) (located errors), [ADR-0013](../../docs/adr/0013-declarative-data-format.md) (data format), [ADR-0014](../../docs/adr/0014-shader-toolchain.md) (shader toolchain). Global rules: [INVARIANTS](../../docs/architecture/INVARIANTS.md), [GATES](../../docs/architecture/GATES.md).
- **Proposed (not binding yet; do not implement contrary designs):** [ADR-0024](../../docs/adr/0024-packages-and-game-extensions.md) (packages and extensions), [ADR-0025](../../docs/adr/0025-mods-and-workshop.md) (mods and Workshop).
- **Approved third-party crates** ([DEPENDENCIES](../../docs/DEPENDENCIES.md)): `clap`, `gltf`, `ufbx` bindings, `meshopt`, `mikktspace`, `png`, `zune-jpeg`, `exr`, `intel_tex_2`, `rspirv`, `notify`, `slangc` (external, `Apache-2.0 WITH LLVM-exception`, pinned at `SLANGC_VERSION = "2026.22"`). Any other crate needs owner approval first.

## Current Implementation Status

`EXISTS` (Milestone 2 Phase 2: offline shader build command); project scaffolding, asset/glTF/texture cooking, and hot-reload file watching are `PLANNED`.

**Binary target:** `[[bin]] name = "spawn"` (`src/main.rs`), invoked as `spawn shaders build` or `cargo run -p spawn-cli -- shaders build` (`--source-dir` / `--manifest`, default `shaders`; `--out`, default `target/shaders`; optional `--slangc`, overriding `SLANGC` and `PATH`).

**Dependencies:** `clap` 4.6 (builder API, no proc-macros), `rspirv` 0.13, and `spawn-render` (`default-features = false`, pulling zero GPU or windowing backends); dev-dependencies `insta` and `proptest`.

| Item | Module | Notes |
| --- | --- | --- |
| `SLANGC_VERSION`, `COOKED_SHADER_EXTENSION` | `shaders` | Pinned `2026.22` (`Apache-2.0 WITH LLVM-exception`); `.spnshdr` uncommitted output |
| `SlangCompiler`, `ProcessSlangCompiler`, `SlangCompileRequest` | `shaders` | Injectable compiler seam; invokes `slangc -target spirv -profile spirv_1_6 -matrix-layout-column-major -fvk-use-entrypoint-name` (never `-fvk-invert-y`) |
| `ShaderBuildOptions`, `ShaderBuildReport`, `CookedArtifactRecord`, `build_shaders`, `build_shaders_with_compiler`, `verify_slangc_version` | `shaders` | Verifies `slangc` version, scans `.slang` `[shader("...")]` entry points, extracts SPIR-V reflection via `rspirv`, validates and emits `.spnshdr` bundles |
| `CliCommand`, `parse_cli_args`, `run_cli` | `lib` | CLI parser and command dispatcher; hides `clap` types from public API |
| `CliError`, `CliErrorKind` | `error` | Pointer-sized, located (`#[track_caller]`), naming file, field, detail, cause chain, and optional backtrace (`ADR-0009`, `SPN-INV-081`) |

**Error codes:**

| Code | Kind | Exit | Meaning |
| --- | --- | --- | --- |
| `CLI-0001` | `InvalidArguments` | 2 | Invalid CLI flags, subcommands, or directory paths |
| `CLI-0002` | `CompilerUnavailable` | 1 | `slangc` binary missing or failed to spawn |
| `CLI-0003` | `CompilerVersionMismatch` | 1 | Installed `slangc` version does not match `SLANGC_VERSION` (`2026.22`) |
| `CLI-0004` | `InvalidShaderSource` | 1 | Unreadable or malformed `.slang` file or `[shader("...")]` attribute |
| `CLI-0005` | `ShaderCompilationFailed` | 1 | `slangc` exited nonzero; forwards file/line compiler diagnostics |
| `CLI-0006` | `ShaderReflectionFailed` | 1 | SPIR-V reflection extraction, layout validation, or artifact write failed |

**Tests:** 5 unit tests (`error`, `slang_scan`) plus 5 headless integration/audit tests (`F3D-T06` mock cook, SPIR-V array-of-matrices/stride/signed-constant/duplicate-entry-point edge cases, version/flag/diagnostic/`insta` snapshot/subprocess exit codes, auto-detected live `slangc` cook, and `F3D-T13` headless tree/`build.rs`/uncommitted-`.spv` audit) and 1 opt-in `--ignored` live `slangc` test.

## Agent Instructions

- If logic would be useful to the engine or other tools, put it in the owning crate and call it.
- Use only crates approved for this tool (see Governing Decisions); native-code importers (ufbx) and decoders stay confined to this tool.
- New error kinds get the next `CLI-` code; codes are never reused or renumbered.
