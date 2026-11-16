//! Error types of `spawn-core` (ADR-0009: hand-written, located, coded).

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;

/// What went wrong in a [`ClockError`].
///
/// Each kind has a stable error code (see [`ClockErrorKind::code`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClockErrorKind {
    /// `CORE-0001`: the configured tick duration is zero.
    ZeroTickDuration,
    /// `CORE-0002`: the configured tick rate is zero ticks per second.
    ZeroTickRate,
    /// `CORE-0003`: the configured maximum number of ticks per update is zero.
    ZeroMaxTicksPerUpdate,
    /// `CORE-0004`: advancing would overflow the `u64` tick index.
    TickIndexExhausted,
}

impl ClockErrorKind {
    /// Stable, searchable error code of this kind.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ZeroTickDuration => "CORE-0001",
            Self::ZeroTickRate => "CORE-0002",
            Self::ZeroMaxTicksPerUpdate => "CORE-0003",
            Self::TickIndexExhausted => "CORE-0004",
        }
    }

    /// One-line description of this kind.
    pub const fn message(self) -> &'static str {
        match self {
            Self::ZeroTickDuration => "fixed-step tick duration must be greater than zero",
            Self::ZeroTickRate => "fixed-step tick rate must be greater than zero",
            Self::ZeroMaxTicksPerUpdate => "maximum ticks per update must be greater than zero",
            Self::TickIndexExhausted => "advancing the clock would overflow the u64 tick index",
        }
    }
}

/// Error returned by the fixed-step clock and its configuration.
///
/// Carries the kind, the source location of the failing call into `spawn-core`
/// (constructors use `#[track_caller]`, so the location points at the caller's
/// code, like a compiler diagnostic) and, when
/// enabled through `RUST_BACKTRACE`, a backtrace. `Display` renders a
/// compiler-like report:
///
/// ```text
/// error[CORE-0001]: fixed-step tick duration must be greater than zero
///   --> crates/spawn-core/src/fixed_step.rs:42:13
/// ```
#[derive(Debug)]
pub struct ClockError {
    kind: ClockErrorKind,
    location: &'static Location<'static>,
    backtrace: Backtrace,
}

impl ClockError {
    /// Creates an error of `kind`, recording the caller's source location.
    #[track_caller]
    pub(crate) fn new(kind: ClockErrorKind) -> Self {
        Self {
            kind,
            location: Location::caller(),
            backtrace: Backtrace::capture(),
        }
    }

    /// What went wrong.
    pub fn kind(&self) -> ClockErrorKind {
        self.kind
    }

    /// Stable error code (e.g. `CORE-0001`).
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Source location of the failing call into `spawn-core`.
    pub fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// Backtrace captured when the error was created; empty unless enabled
    /// through `RUST_BACKTRACE` / `RUST_LIB_BACKTRACE`.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ClockError {
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
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\nbacktrace:\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for ClockError {}
