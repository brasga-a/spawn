//! `spawn-cli`: command-line development tooling for Spawn (`ADR-0014`, `ADR-0015`).
//!
//! Currently implements offline Slang → Vulkan 1.3 SPIR-V + reflection shader
//! cooking (`spawn shaders build`, `ADR-0014`), keeping `spawn-render` and
//! `spawn-engine` free of runtime or `build.rs` shader compilation (`SPN-INV-083`,
//! `SPN-INV-084`). Headless by construction (`SPN-INV-009`).

#![forbid(unsafe_code)]

mod error;
mod reflect;
mod shaders;
mod slang_scan;

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use clap::{Arg, Command, error::ErrorKind as ClapErrorKind};

pub use error::{CliError, CliErrorKind};
pub use shaders::{
    COOKED_SHADER_EXTENSION, CookedArtifactRecord, DEFAULT_SHADER_OUT_DIR,
    DEFAULT_SHADER_SOURCE_DIR, ProcessSlangCompiler, SLANGC_VERSION, ShaderBuildOptions,
    ShaderBuildReport, SlangCompileRequest, SlangCompiler, build_shaders,
    build_shaders_with_compiler, verify_slangc_version,
};

/// Parsed command-line action for the `spawn` CLI binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// Display help or version text already formatted to `String` and exit `0`.
    PrintInfo(String),
    /// Execute `spawn shaders build` with the parsed options.
    ShadersBuild(ShaderBuildOptions),
}

/// Parses raw command-line arguments (`argv`, including `argv[0]`) into a [`CliCommand`].
///
/// Third-party `clap` types stay private to `spawn-cli` (`SPN-INV-011`).
pub fn parse_cli_args<I, T>(args: I) -> Result<CliCommand, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let app = cli_app();
    let parsed = match app.try_get_matches_from(args) {
        Ok(m) => m,
        Err(err) => {
            return match err.kind() {
                ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => {
                    Ok(CliCommand::PrintInfo(err.to_string()))
                }
                _ => Err(CliError::new(CliErrorKind::InvalidArguments)
                    .with_field("cli_args")
                    .with_detail(err.to_string().trim().to_owned())),
            };
        }
    };

    match parsed.subcommand() {
        Some(("shaders", shaders_sub)) => match shaders_sub.subcommand() {
            Some(("build", build_sub)) => {
                let source_dir = build_sub
                    .get_one::<PathBuf>("source-dir")
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_SHADER_SOURCE_DIR));
                let out_dir = build_sub
                    .get_one::<PathBuf>("out")
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_SHADER_OUT_DIR));
                let slangc_path = build_sub.get_one::<PathBuf>("slangc").cloned();
                Ok(CliCommand::ShadersBuild(ShaderBuildOptions {
                    source_dir,
                    out_dir,
                    slangc_path,
                }))
            }
            _ => Err(CliError::new(CliErrorKind::InvalidArguments)
                .with_field("subcommand")
                .with_detail("expected `spawn shaders build`")),
        },
        _ => Err(CliError::new(CliErrorKind::InvalidArguments)
            .with_field("subcommand")
            .with_detail("missing subcommand (expected `shaders build`)")),
    }
}

/// Executes the CLI with `args`, writing human/CI summary lines to `stdout`.
pub fn run_cli<I, T>(args: I, stdout: &mut dyn Write) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match parse_cli_args(args)? {
        CliCommand::PrintInfo(info) => {
            let _ = write!(stdout, "{info}");
            Ok(())
        }
        CliCommand::ShadersBuild(options) => {
            let report = build_shaders(&options)?;
            for artifact in &report.artifacts {
                let _ = writeln!(
                    stdout,
                    "cooked [{}] {}::{} -> {} ({} bytes)",
                    artifact.stage,
                    artifact.source_path.display(),
                    artifact.entry_point,
                    artifact.output_path.display(),
                    artifact.byte_len
                );
            }
            let _ = writeln!(
                stdout,
                "spawn shaders build: {} artifact(s) cooked with slangc {}",
                report.artifacts.len(),
                report.compiler_version
            );
            Ok(())
        }
    }
}

fn cli_app() -> Command {
    Command::new("spawn")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Spawn Engine development and asset-pipeline CLI")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("shaders")
                .about("Offline Slang shader toolchain commands (ADR-0014)")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("build")
                        .about(
                            "Compile .slang shaders offline to Vulkan 1.3 SPIR-V + reflection artifacts",
                        )
                        .arg(
                            Arg::new("source-dir")
                                .long("source-dir")
                                .visible_alias("manifest")
                                .value_name("DIR")
                                .default_value(DEFAULT_SHADER_SOURCE_DIR)
                                .value_parser(clap::value_parser!(PathBuf))
                                .help("Directory containing authored .slang shader sources"),
                        )
                        .arg(
                            Arg::new("out")
                                .long("out")
                                .value_name("DIR")
                                .default_value(DEFAULT_SHADER_OUT_DIR)
                                .value_parser(clap::value_parser!(PathBuf))
                                .help("Uncommitted output directory for cooked .spnshdr artifacts"),
                        )
                        .arg(
                            Arg::new("slangc")
                                .long("slangc")
                                .value_name("PATH")
                                .value_parser(clap::value_parser!(PathBuf))
                                .help("Explicit path to pinned slangc executable (overrides SLANGC and PATH)"),
                        ),
                ),
        )
}
