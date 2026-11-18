//! Error types of `spawn-input` (ADR-0009: hand-written, located, coded).

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;

/// What went wrong in an [`InputError`].
///
/// Each kind has a stable error code (see [`InputErrorKind::code`]); codes are
/// never reused or renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InputErrorKind {
    /// `INPT-0001`: an axis dead zone is non-finite, negative, or `>= 1.0`.
    InvalidDeadZone,
    /// `INPT-0002`: an axis sensitivity is non-finite, `<= 0.0`, or exceeds
    /// [`crate::AxisConfig::MAX_SENSITIVITY`].
    InvalidSensitivity,
    /// `INPT-0003`: an axis clamp range or magnitude cap has non-finite bounds,
    /// `min > max`, or excludes neutral `0.0` (`min <= 0.0 <= max`).
    InvalidAxisRange,
    /// `INPT-0004`: a binding parameter is invalid (for example binding
    /// [`spawn_platform::KeyCode::Unidentified`] instead of a native code,
    /// using a non-finite or zero axis scale, pairing identical negative and
    /// positive triggers, or exceeding the per-map binding capacity).
    InvalidBinding,
    /// `INPT-0005`: an action, axis, or context identifier exceeds the bounded
    /// capacity of [`crate::InputCommand`] or [`crate::InputMap`].
    IdOutOfRange,
    /// `INPT-0006`: an untrusted [`crate::InputCommand`] failed validation
    /// (non-finite or out-of-range axis value, unconfigured action or axis bit
    /// set, or an impossible press/hold/release transition state).
    InvalidCommand,
}

impl InputErrorKind {
    /// Stable, searchable error code of this kind.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidDeadZone => "INPT-0001",
            Self::InvalidSensitivity => "INPT-0002",
            Self::InvalidAxisRange => "INPT-0003",
            Self::InvalidBinding => "INPT-0004",
            Self::IdOutOfRange => "INPT-0005",
            Self::InvalidCommand => "INPT-0006",
        }
    }

    /// One-line description of this kind.
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidDeadZone => "axis dead zone must be finite and within [0.0, 1.0)",
            Self::InvalidSensitivity => {
                "axis sensitivity must be finite, positive, and within the supported maximum"
            }
            Self::InvalidAxisRange => {
                "axis range bounds must be finite and include neutral zero (min <= 0.0 <= max)"
            }
            Self::InvalidBinding => "input binding configuration is invalid",
            Self::IdOutOfRange => "input identifier exceeds the supported capacity",
            Self::InvalidCommand => "input command failed validation",
        }
    }
}

/// Error returned by `spawn-input`.
///
/// Carries the kind, the source location of the failing call (constructors use
/// `#[track_caller]`, so invalid caller parameters point directly at the
/// caller's line), an optional detail naming the offending field and value, an
/// optional underlying cause flattened to opaque text, and, when enabled
/// through `RUST_BACKTRACE`, a backtrace. `Display` renders a compiler-like
/// report:
///
/// ```text
/// error[INPT-0001]: axis dead zone must be finite and within [0.0, 1.0)
///   --> crates/spawn-input/src/action.rs:88:17
///    = detail: dead_zone = NaN
/// ```
#[derive(Debug)]
pub struct InputError {
    kind: InputErrorKind,
    location: &'static Location<'static>,
    detail: Option<String>,
    cause: Option<Cause>,
    backtrace: Backtrace,
}

impl InputError {
    /// Creates an error of `kind`, recording the caller's source location.
    #[track_caller]
    pub(crate) fn new(kind: InputErrorKind) -> Self {
        Self {
            kind,
            location: Location::caller(),
            detail: None,
            cause: None,
            backtrace: Backtrace::capture(),
        }
    }

    /// Adds a detail line naming the offending field, identifier, or value.
    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Records `cause` (and its whole `source()` chain) as opaque text.
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

    /// What went wrong.
    pub const fn kind(&self) -> InputErrorKind {
        self.kind
    }

    /// Stable error code (for example `INPT-0001`).
    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Source location where the error was created.
    pub const fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// Offending field, identifier, or value, if recorded.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Backtrace captured when the error was created; empty unless enabled
    /// through `RUST_BACKTRACE` / `RUST_LIB_BACKTRACE`.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for InputError {
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

impl std::error::Error for InputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

/// Underlying cause of an [`InputError`], flattened to opaque text.
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
    struct Outer(Inner);
    #[derive(Debug)]
    struct Inner;

    impl fmt::Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("packet decode error")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("truncated payload")
        }
    }
    impl std::error::Error for Inner {}

    #[test]
    fn codes_are_stable_and_distinct() {
        let kinds = [
            InputErrorKind::InvalidDeadZone,
            InputErrorKind::InvalidSensitivity,
            InputErrorKind::InvalidAxisRange,
            InputErrorKind::InvalidBinding,
            InputErrorKind::IdOutOfRange,
            InputErrorKind::InvalidCommand,
        ];
        let codes: Vec<_> = kinds.iter().map(|kind| kind.code()).collect();
        assert_eq!(
            codes,
            [
                "INPT-0001",
                "INPT-0002",
                "INPT-0003",
                "INPT-0004",
                "INPT-0005",
                "INPT-0006",
            ]
        );
    }

    #[test]
    fn report_has_code_location_detail_and_flattened_cause() {
        let error = InputError::new(InputErrorKind::InvalidCommand)
            .with_detail("axis 2 is NaN")
            .with_cause(&Outer(Inner));
        let line = line!() - 3;
        let report = error.to_string();
        let expected_head = format!(
            "error[INPT-0006]: input command failed validation\n  --> {}:{line}:21\n   = detail: axis 2 is NaN\ncaused by: packet decode error: truncated payload",
            file!()
        );
        assert!(report.starts_with(&expected_head), "report was:\n{report}");
        assert_eq!(error.kind(), InputErrorKind::InvalidCommand);
        assert_eq!(error.code(), "INPT-0006");
        assert_eq!(error.detail(), Some("axis 2 is NaN"));
        assert_eq!(error.location().file(), file!());
        let source = error.source().map(ToString::to_string);
        assert_eq!(
            source.as_deref(),
            Some("packet decode error: truncated payload")
        );
    }

    #[test]
    fn report_without_detail_or_cause_is_two_lines() {
        let error = InputError::new(InputErrorKind::InvalidDeadZone);
        let report = error.to_string();
        let mut lines = report.lines();
        assert_eq!(
            lines.next(),
            Some("error[INPT-0001]: axis dead zone must be finite and within [0.0, 1.0)")
        );
        assert!(lines.next().is_some_and(|line| line.starts_with("  --> ")));
        assert!(lines.next().is_none_or(|line| line == "backtrace:"));
        assert!(error.source().is_none());
    }
}
