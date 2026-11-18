//! Error types of `spawn-math` (ADR-0009: hand-written, located, coded).

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;

/// Category of failure in a [`MathError`].
///
/// Each variant maps to a stable, searchable error code (see [`MathErrorKind::code`]);
/// assigned codes are never reused or renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MathErrorKind {
    /// `MATH-0001`: vertical field-of-view angle is non-finite, `<= 0`, or `>= π` radians.
    InvalidFov,
    /// `MATH-0002`: viewport aspect ratio (`width / height`) is non-finite or `<= 0`.
    InvalidAspectRatio,
    /// `MATH-0003`: near or far clip distance is non-finite, `near <= 0`, or `far <= near`.
    InvalidClipPlanes,
    /// `MATH-0004`: a required direction, normal, or basis vector is non-finite, zero-length,
    /// or collinear with a reference axis.
    DegenerateDirection,
    /// `MATH-0005`: geometric bounds are non-finite, inverted (`min > max`), or have a negative radius.
    InvalidBounds,
}

impl MathErrorKind {
    /// Stable, searchable error code of this kind (for example `"MATH-0001"`).
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidFov => "MATH-0001",
            Self::InvalidAspectRatio => "MATH-0002",
            Self::InvalidClipPlanes => "MATH-0003",
            Self::DegenerateDirection => "MATH-0004",
            Self::InvalidBounds => "MATH-0005",
        }
    }

    /// One-line human-readable summary of this kind.
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidFov => "vertical field of view must be finite and within (0, pi) radians",
            Self::InvalidAspectRatio => {
                "viewport aspect ratio must be finite and strictly positive"
            }
            Self::InvalidClipPlanes => {
                "clip plane distances must be finite with 0 < near (and near < far for finite projections)"
            }
            Self::DegenerateDirection => {
                "direction or basis vector must be finite, non-zero, and linearly independent"
            }
            Self::InvalidBounds => {
                "geometric bounds must be finite with non-inverted extents and non-negative radius"
            }
        }
    }
}

/// Error returned by validated constructors in `spawn-math`.
///
/// Carries [`MathErrorKind`], the caller's [`Location`] captured via `#[track_caller]`,
/// an optional detail naming the offending parameter and value, an optional flattened
/// cause chain exposed via [`std::error::Error::source`], and a [`Backtrace`] when
/// enabled through `RUST_BACKTRACE`.
///
/// `Display` renders a compiler-style diagnostic report:
///
/// ```text
/// error[MATH-0003]: clip plane distances must be finite with 0 < near (and near < far for finite projections)
///   --> crates/spawn-math/src/mat.rs:512:19
///    = detail: near = 10.0, far = 2.0
/// ```
#[derive(Debug)]
pub struct MathError {
    kind: MathErrorKind,
    location: &'static Location<'static>,
    detail: Option<String>,
    cause: Option<Cause>,
    backtrace: Backtrace,
}

impl MathError {
    /// Creates an error of `kind`, recording the caller's source location.
    #[track_caller]
    pub(crate) fn new(kind: MathErrorKind) -> Self {
        Self {
            kind,
            location: Location::caller(),
            detail: None,
            cause: None,
            backtrace: Backtrace::capture(),
        }
    }

    /// Attaches context naming the offending parameter and value.
    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Records `cause` (and its full `source()` chain) as opaque diagnostic text.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_cause(mut self, cause: &(dyn std::error::Error + 'static)) -> Self {
        let mut message = cause.to_string();
        let mut next = cause.source();
        while let Some(inner) = next {
            message.push_str(": ");
            message.push_str(&inner.to_string());
            next = inner.source();
        }
        self.cause = Some(Cause(message));
        self
    }

    /// Error category.
    pub const fn kind(&self) -> MathErrorKind {
        self.kind
    }

    /// Stable error code (for example `"MATH-0001"`).
    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Caller location recorded when the error was constructed.
    pub const fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// Offending parameter and value context, if attached.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Backtrace captured at construction; empty unless enabled via `RUST_BACKTRACE`.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for MathError {
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

impl std::error::Error for MathError {
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

    #[derive(Debug)]
    struct OuterError(InnerError);
    #[derive(Debug)]
    struct InnerError;

    impl fmt::Display for OuterError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("camera config parse failed")
        }
    }

    impl std::error::Error for OuterError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    impl fmt::Display for InnerError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("negative near clip distance")
        }
    }

    impl std::error::Error for InnerError {}

    fn normalize_report_for_snapshot(report: &str) -> String {
        let mut out = Vec::new();
        for line in report.lines() {
            if line.starts_with("backtrace:") {
                break;
            }
            if let Some(rest) = line.strip_prefix("  --> ") {
                let normalized_path = rest.replace('\\', "/");
                let file_only = normalized_path
                    .rsplit_once(':')
                    .and_then(|(prefix, _col)| prefix.rsplit_once(':'))
                    .map_or(normalized_path.as_str(), |(path, _line)| path);
                out.push(format!("  --> {file_only}:<LINE>:<COL>"));
            } else {
                out.push(line.to_owned());
            }
        }
        out.join("\n")
    }

    #[test]
    fn codes_are_stable_and_distinct() {
        let kinds = [
            MathErrorKind::InvalidFov,
            MathErrorKind::InvalidAspectRatio,
            MathErrorKind::InvalidClipPlanes,
            MathErrorKind::DegenerateDirection,
            MathErrorKind::InvalidBounds,
        ];
        let codes: Vec<_> = kinds.iter().map(|k| k.code()).collect();
        assert_eq!(
            codes,
            [
                "MATH-0001",
                "MATH-0002",
                "MATH-0003",
                "MATH-0004",
                "MATH-0005",
            ]
        );
    }

    #[test]
    fn error_report_matches_adr_0009_snapshot() {
        let err = MathError::new(MathErrorKind::InvalidClipPlanes)
            .with_detail("near = 10.0, far = 2.0")
            .with_cause(&OuterError(InnerError));
        assert_eq!(err.kind(), MathErrorKind::InvalidClipPlanes);
        assert_eq!(err.code(), "MATH-0003");
        assert_eq!(err.detail(), Some("near = 10.0, far = 2.0"));
        assert_eq!(err.location().file(), file!());
        assert_eq!(
            err.source().map(ToString::to_string).as_deref(),
            Some("camera config parse failed: negative near clip distance")
        );

        let normalized = normalize_report_for_snapshot(&err.to_string());
        insta::assert_snapshot!("math_error_report", normalized);
    }
}
