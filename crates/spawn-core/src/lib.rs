//! `spawn-core`: lifecycle and orchestration contracts shared by subsystems.
//!
//! Current contents:
//!
//! - [`FixedStepClock`], [`FixedStepConfig`], [`FixedStepUpdate`] and [`Tick`]:
//!   the fixed-step simulation timing contract of ADR-0003. The clock never
//!   reads OS time; the caller injects monotonic elapsed durations.
//! - [`ClockError`] / [`ClockErrorKind`]: located, coded errors (ADR-0009).
//!
//! This crate depends on no other `spawn-*` crate and does not log. Ownership,
//! boundaries and rules are in this crate's `AGENTS.md`.

mod error;
mod fixed_step;

pub use error::{ClockError, ClockErrorKind};
pub use fixed_step::{FixedStepClock, FixedStepConfig, FixedStepUpdate, Tick, Ticks};
