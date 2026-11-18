//! Top-level composition error types of `spawn-engine` (ADR-0009).

#![forbid(unsafe_code)]

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;

/// Actionable build hint included when cooked shader artifacts are missing on disk.
pub const SHADER_COOK_HINT: &str = "run 'cargo run -p spawn-cli -- shaders build' first";

/// Category of an [`EngineError`].
///
/// Each kind has a stable, searchable error code (see [`EngineErrorKind::code`]);
/// codes are never reused or renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EngineErrorKind {
    /// `ENGN-0001`: a startup option or environment variable is malformed or
    /// out of range.
    InvalidOption,
    /// `ENGN-0002`: no native window/graphics backend is compiled for this
    /// target platform.
    UnsupportedPlatform,
    /// `ENGN-0003`: fixed-step simulation clock configuration or advancement
    /// failed.
    Clock,
    /// `ENGN-0004`: platform connection, window creation, event pumping, or
    /// cursor constraint failed.
    Platform,
    /// `ENGN-0005`: renderer initialization or frame presentation failed
    /// fatally.
    Render,
    /// `ENGN-0006`: the graphics validation layer reported one or more errors
    /// (`SPN-INV-092`).
    Validation,
    /// `ENGN-0007`: input binding configuration failed.
    Input,
    /// `ENGN-0008`: cooked shader artifact file loading, header framing, or
    /// pipeline reflection validation failed (`ADR-0014`, `SPN-INV-034`, `081`).
    Shader,
    /// `ENGN-0009`: free-fly camera or perspective projection parameters are
    /// invalid or non-finite (`ADR-0010`).
    Camera,
    /// `ENGN-0010`: procedural 3D scene geometry or draw extraction failed
    /// validation.
    Scene,
}

impl EngineErrorKind {
    /// Stable, searchable error code of this kind (`ENGN-0001`..`ENGN-0010`).
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidOption => "ENGN-0001",
            Self::UnsupportedPlatform => "ENGN-0002",
            Self::Clock => "ENGN-0003",
            Self::Platform => "ENGN-0004",
            Self::Render => "ENGN-0005",
            Self::Validation => "ENGN-0006",
            Self::Input => "ENGN-0007",
            Self::Shader => "ENGN-0008",
            Self::Camera => "ENGN-0009",
            Self::Scene => "ENGN-0010",
        }
    }

    /// One-line summary of this kind.
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidOption => "invalid runtime option",
            Self::UnsupportedPlatform => "no native runtime backend is available for this platform",
            Self::Clock => "simulation clock failed",
            Self::Platform => "window or event-loop operation failed",
            Self::Render => "rendering operation failed",
            Self::Validation => "graphics validation reported errors",
            Self::Input => "input configuration failed",
            Self::Shader => "cooked shader artifact load or validation failed",
            Self::Camera => "invalid camera or projection configuration",
            Self::Scene => "procedural 3D scene validation failed",
        }
    }
}

/// Top-level error aggregated and reported by the `spawn-engine` composition
/// root (ADR-0009).
///
/// Carries the [`EngineErrorKind`], caller [`Location`], optional context
/// detail, flattened cause chain (never exposing backend concrete types), and
/// optional [`Backtrace`].
#[derive(Debug)]
pub struct EngineError {
    kind: EngineErrorKind,
    location: &'static Location<'static>,
    detail: Option<String>,
    cause: Option<Cause>,
    recoverable_cursor_grab: bool,
    backtrace: Backtrace,
}

impl EngineError {
    /// Creates an error of `kind`, recording the caller's source location.
    #[track_caller]
    pub fn new(kind: EngineErrorKind) -> Self {
        Self {
            kind,
            location: Location::caller(),
            detail: None,
            cause: None,
            recoverable_cursor_grab: false,
            backtrace: Backtrace::capture(),
        }
    }

    /// Attaches actionable context naming the operation, option, or resource.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Records `cause` (and any unrendered `source()` links) as opaque text.
    #[must_use]
    pub fn with_cause(mut self, cause: &(dyn std::error::Error + 'static)) -> Self {
        let mut message = cause.to_string();
        let mut next = cause.source();
        while let Some(inner) = next {
            let inner_text = inner.to_string();
            if !message.contains(&inner_text) {
                message.push_str(": ");
                message.push_str(&inner_text);
            }
            next = inner.source();
        }
        self.cause = Some(Cause(message));
        self
    }

    /// Marks an [`EngineErrorKind::Platform`] cursor-constraint rejection (for
    /// example missing compositor pointer-constraints support) as recoverable
    /// when requested interactively (`docs/first-3d/INVARIANTS.md` failure policy).
    ///
    /// Explicit smoke-test cursor grab requests (`SPAWN_SMOKE_GRAB`) still
    /// treat the error as fatal.
    #[must_use]
    pub const fn with_recoverable_cursor_grab(mut self) -> Self {
        self.recoverable_cursor_grab = true;
        self
    }

    /// Whether this error represents an optional cursor-grab request rejected by
    /// the window system without losing the display connection.
    pub const fn is_recoverable_cursor_grab(&self) -> bool {
        self.recoverable_cursor_grab
    }

    /// Category of failure.
    pub const fn kind(&self) -> EngineErrorKind {
        self.kind
    }

    /// Stable error code (`ENGN-0001`..`ENGN-0010`).
    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Source location where the error was created.
    pub const fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// Optional context detail.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Captured backtrace (populated when `RUST_BACKTRACE` is enabled).
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "error[{}]: {}\n  --> {}:{}:{}",
            self.kind.code(),
            self.kind.message(),
            self.location.file(),
            self.location.line(),
            self.location.column()
        )?;
        if let Some(detail) = &self.detail {
            write!(f, "\n   = detail: {detail}")?;
        }
        if let Some(cause) = &self.cause {
            write!(f, "\ncaused by: {cause}")?;
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\nbacktrace:\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

#[derive(Debug)]
struct Cause(String);

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Cause {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    fn normalize_location(report: &str) -> String {
        report
            .lines()
            .take_while(|line| *line != "backtrace:")
            .map(|line| {
                if let Some(rest) = line.strip_prefix("  --> ") {
                    let file_only = rest
                        .rsplit_once(':')
                        .and_then(|(prefix, _col)| prefix.rsplit_once(':'))
                        .map_or(rest, |(file, _line)| file)
                        .replace('\\', "/");
                    format!("  --> {file_only}:<LINE>:<COL>")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn codes_are_stable_and_distinct() {
        use EngineErrorKind as K;
        let kinds = [
            K::InvalidOption,
            K::UnsupportedPlatform,
            K::Clock,
            K::Platform,
            K::Render,
            K::Validation,
            K::Input,
            K::Shader,
            K::Camera,
            K::Scene,
        ];
        let codes: Vec<_> = kinds.iter().map(|kind| kind.code()).collect();
        let expected: Vec<_> = (1..=10).map(|n| format!("ENGN-{n:04}")).collect();
        assert_eq!(codes, expected);
    }

    #[test]
    fn report_includes_code_location_detail_and_non_duplicated_cause() {
        let clock_err = spawn_core::FixedStepConfig::new(std::time::Duration::ZERO, 4).unwrap_err();
        let line = line!() + 1;
        let error = EngineError::new(EngineErrorKind::Clock)
            .with_detail("initializing fixed-step clock")
            .with_cause(&clock_err);
        let report = error.to_string();
        assert!(
            report.starts_with(&format!(
                "error[ENGN-0003]: simulation clock failed\n  --> {}:{line}:",
                file!()
            )),
            "unexpected report:\n{report}"
        );
        assert!(report.contains("= detail: initializing fixed-step clock"));
        assert!(report.contains("caused by: error[CORE-0001]"));
        assert!(error.source().is_some());
    }

    #[test]
    fn phase_5_shader_camera_and_scene_error_reports_match_snapshots() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "entity not found");
        let shader_err = EngineError::new(EngineErrorKind::Shader)
            .with_detail(format!(
                "file `target/shaders/mesh_3d.vs_main.spnshdr` (stage `vertex`): failed to read cooked shader artifact; {SHADER_COOK_HINT}"
            ))
            .with_cause(&io_err);
        insta::assert_snapshot!(
            normalize_location(&shader_err.to_string()),
            @r#"
error[ENGN-0008]: cooked shader artifact load or validation failed
  --> src/error.rs:<LINE>:<COL>
   = detail: file `target/shaders/mesh_3d.vs_main.spnshdr` (stage `vertex`): failed to read cooked shader artifact; run 'cargo run -p spawn-cli -- shaders build' first
caused by: entity not found
"#
        );

        let math_err = spawn_math::Mat4::perspective_reversed_z_rh(0.0, 1.0, 0.1, 100.0)
            .expect_err("zero fov must fail");
        let camera_err = EngineError::new(EngineErrorKind::Camera)
            .with_detail("field `fov_y_radians`: perspective vertical field of view is invalid")
            .with_cause(&math_err);
        insta::assert_snapshot!(
            normalize_location(&camera_err.to_string()),
            @r#"
error[ENGN-0009]: invalid camera or projection configuration
  --> src/error.rs:<LINE>:<COL>
   = detail: field `fov_y_radians`: perspective vertical field of view is invalid
caused by: error[MATH-0001]: vertical field of view must be finite and within (0, pi) radians
  --> src/error.rs:<LINE>:<COL>
   = detail: fov_y_radians = 0
"#
        );

        let mesh_err = spawn_render::MeshDescriptor::new("empty_mesh", &[], &[])
            .validate()
            .expect_err("empty mesh must fail");
        let scene_err = EngineError::new(EngineErrorKind::Scene)
            .with_detail("procedural mesh `empty_mesh` failed validation")
            .with_cause(&mesh_err);
        insta::assert_snapshot!(
            normalize_location(&scene_err.to_string()),
            @r#"
error[ENGN-0010]: procedural 3D scene validation failed
  --> src/error.rs:<LINE>:<COL>
   = detail: procedural mesh `empty_mesh` failed validation
caused by: error[REND-0014]: invalid render resource or target descriptor
  --> src/error.rs:<LINE>:<COL>
   = detail: mesh `empty_mesh`: vertex slice cannot be empty
"#
        );
    }
}
