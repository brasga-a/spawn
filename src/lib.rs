//! `spawn-engine`: composition root of the Spawn foundation runtime slice.
//!
//! Wires together `spawn-core` (`FixedStepClock`), `spawn-platform` (native
//! window and raw event loop), `spawn-input` (device tracking and tick-boundary
//! command sampling), and `spawn-render` (GPU surface clear and recovery)
//! without introducing upward or lateral dependencies between engine crates
//! (`SPN-INV-001`..`SPN-INV-006`).
//!
//! Diagnostics are written to `stderr` via `std` only until `spawn-profiler`
//! provides the structured logging facade (ADR-0009).

#![forbid(unsafe_code)]

mod error;
mod native;
mod options;
mod runtime;

pub use error::{EngineError, EngineErrorKind};
pub use native::run_native;
pub use options::RuntimeOptions;
pub use runtime::{
    ExitReason, RenderSurface, RuntimeBackend, SessionSummary, WindowSurface, run_session,
};

/// Validates environment options and executes the native foundation runtime.
///
/// # Errors
///
/// Returns a located, coded [`EngineError`] if options are invalid, no native
/// backend is available on this OS, or initialization/runtime execution fails.
pub fn run() -> Result<SessionSummary, EngineError> {
    let options = RuntimeOptions::from_env()?;
    run_native(&options)
}
