//! Hand-written located error types of `spawn-cli` (ADR-0009, `SPN-INV-032`, `SPN-INV-081`).

#![forbid(unsafe_code)]

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;
use std::path::Path;

/// Category of failure in a [`CliError`].
///
/// Each variant maps to a stable `CLI-XXXX` code that is never reused or renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CliErrorKind {
    /// `CLI-0001`: invalid command-line arguments, options, or directory paths.
    InvalidArguments,
    /// `CLI-0002`: external `slangc` compiler executable could not be found or spawned.
    CompilerUnavailable,
    /// `CLI-0003`: installed `slangc` version does not match the pinned `SLANGC_VERSION`.
    CompilerVersionMismatch,
    /// `CLI-0004`: unreadable or malformed `.slang` shader source or entry-point declaration.
    InvalidShaderSource,
    /// `CLI-0005`: `slangc` exited with a non-zero status while compiling `.slang` to SPIR-V.
    ShaderCompilationFailed,
    /// `CLI-0006`: SPIR-V reflection extraction, layout validation, or cooked artifact I/O failed.
    ShaderReflectionFailed,
}

impl CliErrorKind {
    /// Stable, searchable error code for this kind (`CLI-0001`..).
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidArguments => "CLI-0001",
            Self::CompilerUnavailable => "CLI-0002",
            Self::CompilerVersionMismatch => "CLI-0003",
            Self::InvalidShaderSource => "CLI-0004",
            Self::ShaderCompilationFailed => "CLI-0005",
            Self::ShaderReflectionFailed => "CLI-0006",
        }
    }

    /// One-line summary message for this kind.
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid command-line arguments or options",
            Self::CompilerUnavailable => "Slang shader compiler (`slangc`) is unavailable",
            Self::CompilerVersionMismatch => {
                "installed `slangc` version does not match repository pin"
            }
            Self::InvalidShaderSource => "invalid shader source file",
            Self::ShaderCompilationFailed => "offline Slang shader compilation failed",
            Self::ShaderReflectionFailed => {
                "SPIR-V reflection or cooked artifact validation failed"
            }
        }
    }

    /// Non-zero process exit code associated with this error kind.
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::InvalidArguments => 2,
            Self::CompilerUnavailable
            | Self::CompilerVersionMismatch
            | Self::InvalidShaderSource
            | Self::ShaderCompilationFailed
            | Self::ShaderReflectionFailed => 1,
        }
    }
}

/// Structured, compiler-style diagnostic error returned by `spawn-cli` (`ADR-0009`).
///
/// Boxed internally so `Result<T, CliError>` stays pointer-sized while carrying
/// the originating Rust location (`#[track_caller]`), stable `CLI-XXXX` code,
/// target file/path, field or node, human-readable detail, flattened cause chain,
/// and optional backtrace (`SPN-INV-081`).
#[derive(Debug)]
pub struct CliError(Box<CliErrorInner>);

#[derive(Debug)]
struct CliErrorInner {
    kind: CliErrorKind,
    location: &'static Location<'static>,
    file: Option<String>,
    field: Option<String>,
    detail: Option<String>,
    cause: Option<Cause>,
    backtrace: Backtrace,
}

impl CliError {
    /// Creates a [`CliError`] of `kind`, recording the caller's source location.
    #[track_caller]
    pub fn new(kind: CliErrorKind) -> Self {
        Self(Box::new(CliErrorInner {
            kind,
            location: Location::caller(),
            file: None,
            field: None,
            detail: None,
            cause: None,
            backtrace: Backtrace::capture(),
        }))
    }

    /// Attaches the path or resource name involved (`SPN-INV-081`).
    #[must_use]
    pub fn with_file(mut self, file: impl AsRef<Path>) -> Self {
        self.0.file = Some(file.as_ref().display().to_string());
        self
    }

    /// Attaches the field, attribute, or entry-point identifier involved (`SPN-INV-081`).
    #[must_use]
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.0.field = Some(field.into());
        self
    }

    /// Attaches a human-readable explanation of what failed (`SPN-INV-081`).
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.0.detail = Some(detail.into());
        self
    }

    /// Records `cause` (and its full `source()` chain) as opaque diagnostic text.
    #[must_use]
    pub fn with_cause(mut self, cause: &(dyn std::error::Error + 'static)) -> Self {
        let mut message = cause.to_string();
        let mut next = cause.source();
        while let Some(inner) = next {
            message.push_str(": ");
            message.push_str(&inner.to_string());
            next = inner.source();
        }
        self.0.cause = Some(Cause(message));
        self
    }

    /// Error category.
    pub fn kind(&self) -> CliErrorKind {
        self.0.kind
    }

    /// Stable error code (`CLI-0001`..).
    pub fn code(&self) -> &'static str {
        self.0.kind.code()
    }

    /// Recommended non-zero process exit status.
    pub fn exit_code(&self) -> u8 {
        self.0.kind.exit_code()
    }

    /// Source location where the error was constructed.
    pub fn location(&self) -> &'static Location<'static> {
        self.0.location
    }

    /// File or executable path named by this error, if recorded.
    pub fn file(&self) -> Option<&str> {
        self.0.file.as_deref()
    }

    /// Field, attribute, or node named by this error, if recorded.
    pub fn field(&self) -> Option<&str> {
        self.0.field.as_deref()
    }

    /// Diagnostic reason text, if recorded.
    pub fn detail(&self) -> Option<&str> {
        self.0.detail.as_deref()
    }

    /// Captured backtrace (empty unless enabled via `RUST_BACKTRACE`).
    pub fn backtrace(&self) -> &Backtrace {
        &self.0.backtrace
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "error[{}]: {}\n  --> {}:{}:{}",
            self.0.kind.code(),
            self.0.kind.message(),
            self.0.location.file(),
            self.0.location.line(),
            self.0.location.column()
        )?;
        if let Some(file) = &self.0.file {
            write!(f, "\n   = file: {file}")?;
        }
        if let Some(field) = &self.0.field {
            write!(f, "\n   = field: {field}")?;
        }
        if let Some(detail) = &self.0.detail {
            write!(f, "\n   = detail: {detail}")?;
        }
        if let Some(cause) = &self.0.cause {
            write!(f, "\ncaused by: {cause}")?;
        }
        if self.0.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\nbacktrace:\n{}", self.0.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.cause.as_ref().map(|cause| cause as _)
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
    fn cli_error_is_pointer_sized_and_codes_are_stable() {
        assert_eq!(
            std::mem::size_of::<CliError>(),
            std::mem::size_of::<usize>()
        );
        let kinds = [
            CliErrorKind::InvalidArguments,
            CliErrorKind::CompilerUnavailable,
            CliErrorKind::CompilerVersionMismatch,
            CliErrorKind::InvalidShaderSource,
            CliErrorKind::ShaderCompilationFailed,
            CliErrorKind::ShaderReflectionFailed,
        ];
        let codes: Vec<_> = kinds.iter().map(|k| k.code()).collect();
        let expected: Vec<_> = (1..=6).map(|n| format!("CLI-{n:04}")).collect();
        assert_eq!(codes, expected);
    }

    #[test]
    fn cli_error_report_includes_file_field_detail_and_cause() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        let err = CliError::new(CliErrorKind::InvalidShaderSource)
            .with_file("shaders/broken.slang")
            .with_field("entry_points")
            .with_detail("no [shader(\"...\")] attributes found")
            .with_cause(&io_err);
        let text = err.to_string();
        assert!(text.contains("error[CLI-0004]"));
        assert!(text.contains("= file: shaders/broken.slang"));
        assert!(text.contains("= field: entry_points"));
        assert!(text.contains("= detail: no [shader(\"...\")] attributes found"));
        assert_eq!(
            err.source().map(ToString::to_string).as_deref(),
            Some("missing file")
        );
    }
}
