//! Offline Slang → Vulkan 1.3 SPIR-V + reflection shader cook pipeline
//! (`ADR-0014`, `SPN-INV-081`, `SPN-INV-083`, `SPN-INV-084`).

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use spawn_render::{CookedShader, MAX_COOKED_SHADER_BYTES, ShaderStage};

use crate::error::{CliError, CliErrorKind};
use crate::reflect::extract_cooked_shader;
use crate::slang_scan::{MAX_SLANG_SOURCE_BYTES, scan_slang_entry_points};

/// Exact pinned `slangc` release version required for offline shader compilation (`ADR-0014`).
pub const SLANGC_VERSION: &str = "2026.22";

/// File extension written for cooked Spawn shader artifacts.
pub const COOKED_SHADER_EXTENSION: &str = "spnshdr";

/// Default source directory searched by `spawn shaders build`.
pub const DEFAULT_SHADER_SOURCE_DIR: &str = "shaders";

/// Default uncommitted output directory written by `spawn shaders build`.
pub const DEFAULT_SHADER_OUT_DIR: &str = "target/shaders";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Parameters for compiling one `.slang` entry point into raw SPIR-V.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlangCompileRequest<'a> {
    /// Source `.slang` file path.
    pub source_path: &'a Path,
    /// Entry-point function name inside `source_path`.
    pub entry_point: &'a str,
    /// Programmable shader stage of `entry_point`.
    pub stage: ShaderStage,
    /// Temporary path where raw SPIR-V words should be emitted.
    pub spv_output_path: &'a Path,
}

/// Injectable execution seam over `slangc` so `F3D-T06` runs headlessly in CI
/// without requiring an external compiler binary (`SPN-INV-009`).
pub trait SlangCompiler {
    /// Queries the compiler's version string (`slangc -v`).
    fn query_version(&self) -> Result<String, CliError>;

    /// Compiles `request` to Vulkan 1.3 SPIR-V (`spirv_1_6`, column-major matrices).
    fn compile_spirv(&self, request: &SlangCompileRequest<'_>) -> Result<(), CliError>;
}

/// Production [`SlangCompiler`] invoking an external `slangc` process via `std::process::Command`.
#[derive(Debug, Clone)]
pub struct ProcessSlangCompiler {
    executable: PathBuf,
}

impl ProcessSlangCompiler {
    /// Creates a process-backed `slangc` runner pointing at `executable` (for example `"slangc"`).
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// Resolves the `slangc` executable path from an explicit CLI override, the `SLANGC`
    /// environment variable (when non-empty), or `"slangc"` on `PATH`.
    pub fn from_override_or_env(cli_override: Option<&Path>) -> Self {
        if let Some(path) = cli_override {
            return Self::new(path);
        }
        if let Some(env_val) = std::env::var_os("SLANGC")
            && !env_val.is_empty()
        {
            return Self::new(env_val);
        }
        Self::new("slangc")
    }

    /// Executable path or program name invoked by this runner.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Builds the exact command-line arguments passed to `slangc` for `request`.
    ///
    /// Targets Vulkan 1.3 (`-target spirv -profile spirv_1_6`), enforces column-major
    /// matrices matching `spawn-math` (`-matrix-layout-column-major`), preserves entry-point
    /// names (`-fvk-use-entrypoint-name`), and never applies `-fvk-invert-y` (`SPN-INV-053`).
    pub fn build_compile_args(request: &SlangCompileRequest<'_>) -> Vec<OsString> {
        vec![
            OsString::from("-target"),
            OsString::from("spirv"),
            OsString::from("-profile"),
            OsString::from("spirv_1_6"),
            OsString::from("-matrix-layout-column-major"),
            OsString::from("-fvk-use-entrypoint-name"),
            OsString::from("-entry"),
            OsString::from(request.entry_point),
            OsString::from("-stage"),
            OsString::from(request.stage.as_str()),
            OsString::from("-o"),
            request.spv_output_path.as_os_str().to_owned(),
            request.source_path.as_os_str().to_owned(),
        ]
    }
}

impl Default for ProcessSlangCompiler {
    fn default() -> Self {
        Self::new("slangc")
    }
}

impl SlangCompiler for ProcessSlangCompiler {
    fn query_version(&self) -> Result<String, CliError> {
        let output = Command::new(&self.executable)
            .arg("-v")
            .output()
            .map_err(|err| {
                CliError::new(CliErrorKind::CompilerUnavailable)
                    .with_file(&self.executable)
                    .with_field("slangc")
                    .with_detail(format!(
                        "failed to invoke `slangc -v`; install pinned Slang {SLANGC_VERSION} (`Apache-2.0 WITH LLVM-exception`)"
                    ))
                    .with_cause(&err)
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = if !stdout.trim().is_empty() {
            stdout.trim().to_owned()
        } else {
            stderr.trim().to_owned()
        };

        if !output.status.success() {
            let status_desc = output.status.code().map_or_else(
                || "terminated by signal".to_owned(),
                |c| format!("exit code {c}"),
            );
            return Err(CliError::new(CliErrorKind::CompilerVersionMismatch)
                .with_file(&self.executable)
                .with_field("slangc_version")
                .with_detail(format!("`slangc -v` failed ({status_desc}): {combined}")));
        }

        Ok(combined)
    }

    fn compile_spirv(&self, request: &SlangCompileRequest<'_>) -> Result<(), CliError> {
        let args = Self::build_compile_args(request);
        let output = Command::new(&self.executable)
            .args(&args)
            .output()
            .map_err(|err| {
                CliError::new(CliErrorKind::CompilerUnavailable)
                    .with_file(&self.executable)
                    .with_field("slangc")
                    .with_detail(format!(
                        "failed to invoke `slangc` while compiling `{}`",
                        request.source_path.display()
                    ))
                    .with_cause(&err)
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let diagnostics = match (stderr.trim().is_empty(), stdout.trim().is_empty()) {
                (false, true) => stderr.trim().to_owned(),
                (true, false) => stdout.trim().to_owned(),
                (false, false) => format!("{}\n{}", stderr.trim(), stdout.trim()),
                (true, true) => "<no compiler diagnostics emitted>".to_owned(),
            };
            let status_desc = output.status.code().map_or_else(
                || "terminated by signal".to_owned(),
                |c| format!("exit code {c}"),
            );
            return Err(CliError::new(CliErrorKind::ShaderCompilationFailed)
                .with_file(request.source_path)
                .with_field(request.entry_point)
                .with_detail(format!(
                    "stage `{}` failed ({status_desc}):\n{diagnostics}",
                    request.stage
                )));
        }

        Ok(())
    }
}

/// Configuration for an offline `shaders build` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderBuildOptions {
    /// Directory containing authored `.slang` shader files.
    pub source_dir: PathBuf,
    /// Git-ignored build output directory receiving `.spnshdr` artifacts.
    pub out_dir: PathBuf,
    /// Optional explicit path to the `slangc` executable.
    pub slangc_path: Option<PathBuf>,
}

impl Default for ShaderBuildOptions {
    fn default() -> Self {
        Self {
            source_dir: PathBuf::from(DEFAULT_SHADER_SOURCE_DIR),
            out_dir: PathBuf::from(DEFAULT_SHADER_OUT_DIR),
            slangc_path: None,
        }
    }
}

/// Summary of a single emitted cooked-shader artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookedArtifactRecord {
    /// Authored `.slang` file path.
    pub source_path: PathBuf,
    /// Emitted `.spnshdr` file path.
    pub output_path: PathBuf,
    /// Compiled programmable stage.
    pub stage: ShaderStage,
    /// Entry-point function name.
    pub entry_point: String,
    /// Total byte size written to `output_path`.
    pub byte_len: usize,
    /// Validated in-memory cooked shader structure.
    pub shader: CookedShader,
}

/// Summary returned after `shaders build` finishes cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderBuildReport {
    /// Verified `slangc` version string (`SLANGC_VERSION`).
    pub compiler_version: String,
    /// Cooked shader artifacts emitted in deterministic order.
    pub artifacts: Vec<CookedArtifactRecord>,
}

/// Verifies `raw_version` reported by `slangc -v` against [`SLANGC_VERSION`].
#[track_caller]
pub fn verify_slangc_version(compiler_label: &Path, raw_version: &str) -> Result<String, CliError> {
    let trimmed = raw_version.trim();
    let normalized = trimmed.strip_prefix('v').unwrap_or(trimmed);
    if normalized == SLANGC_VERSION {
        Ok(SLANGC_VERSION.to_owned())
    } else {
        let found = if trimmed.is_empty() {
            "<empty>"
        } else {
            trimmed
        };
        Err(CliError::new(CliErrorKind::CompilerVersionMismatch)
            .with_file(compiler_label)
            .with_field("slangc_version")
            .with_detail(format!(
                "expected pinned slangc `{SLANGC_VERSION}`, found `{found}`"
            )))
    }
}

/// Runs the offline shader build using the external `slangc` process runner.
pub fn build_shaders(options: &ShaderBuildOptions) -> Result<ShaderBuildReport, CliError> {
    let compiler = ProcessSlangCompiler::from_override_or_env(options.slangc_path.as_deref());
    build_shaders_with_compiler(options, compiler.executable(), &compiler)
}

/// Runs the offline shader build using an injected [`SlangCompiler`] (`F3D-T06`).
pub fn build_shaders_with_compiler(
    options: &ShaderBuildOptions,
    compiler_label: &Path,
    compiler: &dyn SlangCompiler,
) -> Result<ShaderBuildReport, CliError> {
    let source_meta = fs::metadata(&options.source_dir).map_err(|err| {
        CliError::new(CliErrorKind::InvalidArguments)
            .with_file(&options.source_dir)
            .with_field("source_dir")
            .with_detail("shader source directory does not exist or cannot be accessed")
            .with_cause(&err)
    })?;
    if !source_meta.is_dir() {
        return Err(CliError::new(CliErrorKind::InvalidArguments)
            .with_file(&options.source_dir)
            .with_field("source_dir")
            .with_detail("shader source path is not a directory"));
    }

    let mut slang_files = collect_slang_files(&options.source_dir)?;
    slang_files.sort();
    if slang_files.is_empty() {
        return Err(CliError::new(CliErrorKind::InvalidShaderSource)
            .with_file(&options.source_dir)
            .with_field("source_dir")
            .with_detail("shader source directory contains no `.slang` files"));
    }

    let raw_version = compiler.query_version()?;
    let verified_version = verify_slangc_version(compiler_label, &raw_version)?;

    fs::create_dir_all(&options.out_dir).map_err(|err| {
        CliError::new(CliErrorKind::InvalidArguments)
            .with_file(&options.out_dir)
            .with_field("out_dir")
            .with_detail("failed to create shader output directory")
            .with_cause(&err)
    })?;

    let mut artifacts = Vec::new();
    let mut emitted_names = BTreeSet::new();

    for source_path in &slang_files {
        let stem = source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CliError::new(CliErrorKind::InvalidShaderSource)
                    .with_file(source_path)
                    .with_field("file_name")
                    .with_detail("shader filename must be valid non-empty UTF-8")
            })?;

        let file_meta = fs::metadata(source_path).map_err(|err| {
            CliError::new(CliErrorKind::InvalidShaderSource)
                .with_file(source_path)
                .with_field("source_file")
                .with_detail("failed to stat `.slang` source file")
                .with_cause(&err)
        })?;
        if file_meta.len() > MAX_SLANG_SOURCE_BYTES {
            return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                .with_file(source_path)
                .with_field("source_size")
                .with_detail(format!(
                    "shader source size {} bytes exceeds maximum {} bytes",
                    file_meta.len(),
                    MAX_SLANG_SOURCE_BYTES
                )));
        }

        let source_text = fs::read_to_string(source_path).map_err(|err| {
            CliError::new(CliErrorKind::InvalidShaderSource)
                .with_file(source_path)
                .with_field("source_file")
                .with_detail("failed to read `.slang` file as UTF-8")
                .with_cause(&err)
        })?;

        let entry_points = scan_slang_entry_points(source_path, &source_text)?;

        for entry in entry_points {
            let artifact_filename = format!("{stem}.{}.{COOKED_SHADER_EXTENSION}", entry.name);
            if !emitted_names.insert(artifact_filename.clone()) {
                return Err(CliError::new(CliErrorKind::InvalidShaderSource)
                    .with_file(source_path)
                    .with_field("entry_point")
                    .with_detail(format!(
                        "colliding output artifact name `{artifact_filename}`"
                    )));
            }

            let temp_spv = TempSpvGuard::new(&options.out_dir, stem, &entry.name);
            let request = SlangCompileRequest {
                source_path,
                entry_point: &entry.name,
                stage: entry.stage,
                spv_output_path: &temp_spv.path,
            };
            compiler.compile_spirv(&request)?;

            let spv_meta = fs::metadata(&temp_spv.path).map_err(|err| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_path)
                    .with_field(&entry.name)
                    .with_detail(format!(
                        "compiler succeeded without writing SPIR-V output `{}`",
                        temp_spv.path.display()
                    ))
                    .with_cause(&err)
            })?;
            if spv_meta.len() > MAX_COOKED_SHADER_BYTES as u64 {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_path)
                    .with_field(&entry.name)
                    .with_detail(format!(
                        "emitted SPIR-V size {} bytes exceeds maximum {} bytes",
                        spv_meta.len(),
                        MAX_COOKED_SHADER_BYTES
                    )));
            }

            let spv_bytes = fs::read(&temp_spv.path).map_err(|err| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_path)
                    .with_field(&entry.name)
                    .with_detail("failed to read emitted SPIR-V output")
                    .with_cause(&err)
            })?;

            let cooked = extract_cooked_shader(source_path, entry.stage, &entry.name, &spv_bytes)?;
            let encoded = cooked.encode().map_err(|err| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_path)
                    .with_field(&entry.name)
                    .with_detail("failed to encode cooked shader artifact")
                    .with_cause(&err)
            })?;

            let output_path = options.out_dir.join(&artifact_filename);
            fs::write(&output_path, &encoded).map_err(|err| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(&output_path)
                    .with_field("output_artifact")
                    .with_detail("failed to write cooked shader artifact")
                    .with_cause(&err)
            })?;

            artifacts.push(CookedArtifactRecord {
                source_path: source_path.clone(),
                output_path,
                stage: entry.stage,
                entry_point: entry.name,
                byte_len: encoded.len(),
                shader: cooked,
            });
        }
    }

    Ok(ShaderBuildReport {
        compiler_version: verified_version,
        artifacts,
    })
}

fn collect_slang_files(dir: &Path) -> Result<Vec<PathBuf>, CliError> {
    let entries = fs::read_dir(dir).map_err(|err| {
        CliError::new(CliErrorKind::InvalidArguments)
            .with_file(dir)
            .with_field("source_dir")
            .with_detail("failed to enumerate shader source directory")
            .with_cause(&err)
    })?;

    let mut files = Vec::new();
    for entry_res in entries {
        let entry = entry_res.map_err(|err| {
            CliError::new(CliErrorKind::InvalidArguments)
                .with_file(dir)
                .with_field("source_dir")
                .with_detail("failed to read directory entry")
                .with_cause(&err)
        })?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("slang") {
            files.push(path);
        }
    }
    Ok(files)
}

/// RAII guard removing intermediate raw `.spv` files from the build directory on drop.
struct TempSpvGuard {
    path: PathBuf,
}

impl TempSpvGuard {
    fn new(out_dir: &Path, stem: &str, entry_name: &str) -> Self {
        let pid = std::process::id();
        let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let filename = format!(".tmp.{stem}.{entry_name}.{pid}.{seq}.spv");
        Self {
            path: out_dir.join(filename),
        }
    }
}

impl Drop for TempSpvGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
