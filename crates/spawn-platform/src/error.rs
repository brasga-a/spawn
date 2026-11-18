//! Error types of `spawn-platform` (ADR-0009: hand-written, located, coded).

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;

/// What went wrong in a [`PlatformError`].
///
/// Each kind has a stable error code (see [`PlatformErrorKind::code`]); codes
/// are never reused or renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlatformErrorKind {
    /// `PLAT-0001`: no display server could be reached (not running, not
    /// configured in the environment, or its client library is missing).
    DisplayUnavailable,
    /// `PLAT-0002`: the display server lacks a protocol feature Spawn requires.
    UnsupportedDisplay,
    /// `PLAT-0003`: the connection to the display server failed or was closed,
    /// or the server reported a protocol error.
    ConnectionLost,
    /// `PLAT-0004`: a requested window size is zero or above
    /// [`crate::WindowDescriptor::MAX_EXTENT`].
    InvalidWindowSize,
    /// `PLAT-0005`: the display server did not answer within the time limit.
    Timeout,
    /// `PLAT-0006`: cursor lock or confinement could not be applied (the
    /// display server lacks the protocol, no pointer device is available, the
    /// window was destroyed, or the server rejected the request).
    CursorGrabFailed,
}

impl PlatformErrorKind {
    /// Stable, searchable error code of this kind.
    pub const fn code(self) -> &'static str {
        match self {
            Self::DisplayUnavailable => "PLAT-0001",
            Self::UnsupportedDisplay => "PLAT-0002",
            Self::ConnectionLost => "PLAT-0003",
            Self::InvalidWindowSize => "PLAT-0004",
            Self::Timeout => "PLAT-0005",
            Self::CursorGrabFailed => "PLAT-0006",
        }
    }

    /// One-line description of this kind.
    pub const fn message(self) -> &'static str {
        match self {
            Self::DisplayUnavailable => "no display server is available",
            Self::UnsupportedDisplay => "the display server lacks a required feature",
            Self::ConnectionLost => "the connection to the display server failed",
            Self::InvalidWindowSize => {
                "window size must be non-zero and within the supported maximum"
            }
            Self::Timeout => "the display server did not respond in time",
            Self::CursorGrabFailed => "the window system could not lock or confine the cursor",
        }
    }
}

/// Error returned by `spawn-platform`.
///
/// Carries the kind, the source location where it was created (inside
/// `spawn-platform` where the failure was detected; for invalid arguments such
/// as a [`crate::WindowDescriptor`], the caller's code), an optional detail naming what was involved, the
/// underlying cause as opaque text (backend error types never cross the public
/// API) and, when enabled through `RUST_BACKTRACE`, a backtrace. `Display`
/// renders a compiler-like report:
///
/// ```text
/// error[PLAT-0002]: the display server lacks a required feature
///   --> crates/spawn-platform/src/linux/wayland.rs:120:17
///    = detail: global `xdg_wm_base` (version 1 or later) is not advertised
/// ```
#[derive(Debug)]
pub struct PlatformError {
    kind: PlatformErrorKind,
    location: &'static Location<'static>,
    detail: Option<String>,
    cause: Option<Cause>,
    backtrace: Backtrace,
}

impl PlatformError {
    /// Creates an error of `kind`, recording the caller's source location.
    #[track_caller]
    pub(crate) fn new(kind: PlatformErrorKind) -> Self {
        Self {
            kind,
            location: Location::caller(),
            detail: None,
            cause: None,
            backtrace: Backtrace::capture(),
        }
    }

    /// Adds a detail line naming what was involved.
    #[cfg_attr(not(feature = "wayland"), allow(dead_code))]
    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Records `cause` (and its whole `source()` chain) as opaque text, so no
    /// backend error type escapes through [`std::error::Error::source`].
    #[cfg_attr(not(feature = "wayland"), allow(dead_code))]
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
    pub fn kind(&self) -> PlatformErrorKind {
        self.kind
    }

    /// Stable error code (e.g. `PLAT-0001`).
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Source location where the error was created.
    pub fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// What was involved (a protocol name, a size), if recorded.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Backtrace captured when the error was created; empty unless enabled
    /// through `RUST_BACKTRACE` / `RUST_LIB_BACKTRACE`.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for PlatformError {
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

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

/// Underlying cause of a [`PlatformError`], flattened to text.
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
            f.write_str("outer failure")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("inner failure")
        }
    }
    impl std::error::Error for Inner {}

    #[test]
    fn codes_are_stable_and_distinct() {
        let kinds = [
            PlatformErrorKind::DisplayUnavailable,
            PlatformErrorKind::UnsupportedDisplay,
            PlatformErrorKind::ConnectionLost,
            PlatformErrorKind::InvalidWindowSize,
            PlatformErrorKind::Timeout,
            PlatformErrorKind::CursorGrabFailed,
        ];
        let codes: Vec<_> = kinds.iter().map(|kind| kind.code()).collect();
        assert_eq!(
            codes,
            [
                "PLAT-0001",
                "PLAT-0002",
                "PLAT-0003",
                "PLAT-0004",
                "PLAT-0005",
                "PLAT-0006",
            ]
        );
    }

    #[test]
    fn report_has_code_location_detail_and_flattened_cause() {
        let error = PlatformError::new(PlatformErrorKind::DisplayUnavailable)
            .with_detail("Wayland")
            .with_cause(&Outer(Inner));
        let line = line!() - 3;
        let report = error.to_string();
        let expected_head = format!(
            "error[PLAT-0001]: no display server is available\n  --> {}:{line}:21\n   = detail: Wayland\ncaused by: outer failure: inner failure",
            file!()
        );
        assert!(report.starts_with(&expected_head), "report was:\n{report}");
        assert_eq!(error.code(), "PLAT-0001");
        assert_eq!(error.detail(), Some("Wayland"));
        let source = error.source().map(ToString::to_string);
        assert_eq!(source.as_deref(), Some("outer failure: inner failure"));
    }

    #[test]
    fn report_without_detail_or_cause_is_two_lines() {
        let error = PlatformError::new(PlatformErrorKind::InvalidWindowSize);
        let report = error.to_string();
        let mut lines = report.lines();
        assert_eq!(
            lines.next(),
            Some("error[PLAT-0004]: window size must be non-zero and within the supported maximum")
        );
        assert!(lines.next().is_some_and(|line| line.starts_with("  --> ")));
        // Only a backtrace (when enabled through the environment) may follow.
        assert!(lines.next().is_none_or(|line| line == "backtrace:"));
        assert!(error.source().is_none());
    }

    #[test]
    fn cursor_grab_failed_report_carries_code_location_and_detail() {
        let error = PlatformError::new(PlatformErrorKind::CursorGrabFailed)
            .with_detail("Wayland global `zwp_pointer_constraints_v1` is not available");
        let line = line!() - 2;
        let report = error.to_string();
        let expected_head = format!(
            "error[PLAT-0006]: the window system could not lock or confine the cursor\n  --> {}:{line}:21\n   = detail: Wayland global `zwp_pointer_constraints_v1` is not available",
            file!()
        );
        assert!(report.starts_with(&expected_head), "report was:\n{report}");
        assert_eq!(error.kind(), PlatformErrorKind::CursorGrabFailed);
        assert_eq!(error.code(), "PLAT-0006");
    }
}
