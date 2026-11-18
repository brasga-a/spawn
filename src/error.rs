//! Top-level composition error types of `spawn-engine` (ADR-0009).

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;

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
}

impl EngineErrorKind {
    /// Stable, searchable error code of this kind.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidOption => "ENGN-0001",
            Self::UnsupportedPlatform => "ENGN-0002",
            Self::Clock => "ENGN-0003",
            Self::Platform => "ENGN-0004",
            Self::Render => "ENGN-0005",
            Self::Validation => "ENGN-0006",
            Self::Input => "ENGN-0007",
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
            backtrace: Backtrace::capture(),
        }
    }

    /// Attaches actionable context naming the operation, option, or resource.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Records `cause` (and any unrendered `source()` links) as opaque text.
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

    /// Category of failure.
    pub const fn kind(&self) -> EngineErrorKind {
        self.kind
    }

    /// Stable error code (`ENGN-0001`..`ENGN-0007`).
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
        ];
        let codes: Vec<_> = kinds.iter().map(|kind| kind.code()).collect();
        let expected: Vec<_> = (1..=7).map(|n| format!("ENGN-{n:04}")).collect();
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
}
