//! `spawn-platform`: OS integration: windows, system events, surfaces.
//!
//! Always available (no native code, ADR-0002):
//!
//! - Raw events: [`Event`], [`WindowEvent`], [`KeyEvent`], [`KeyCode`],
//!   [`MouseButton`], [`ScrollDelta`], [`ElementState`], [`CursorGrabMode`]
//!   and the size/position types. They describe what the window system
//!   reported, in order; they carry no actions or bindings (`spawn-input`
//!   builds those).
//! - [`WindowDescriptor`] and the located, coded [`PlatformError`] (ADR-0009).
//!
//! With the `wayland` Cargo feature on Linux, the native backend adds
//! `Platform` (connection and Spawn-owned event pump, `PumpMode`) and
//! `Window` (implements the `raw-window-handle` traits for the renderer).
//!
//! Ownership, boundaries and rules are in this crate's `AGENTS.md`.

mod error;
mod event;
mod key;
// Translation logic is compiled for its tests even without a backend.
#[cfg(all(target_os = "linux", any(feature = "wayland", test)))]
mod linux;
#[cfg(all(feature = "wayland", target_os = "linux"))]
mod platform;
mod window;

pub use error::{PlatformError, PlatformErrorKind};
pub use event::{
    CursorGrabMode, ElementState, Event, KeyEvent, LogicalSize, PhysicalPosition, PhysicalSize,
    ScrollDelta, WindowEvent, WindowId,
};
pub use key::{KeyCode, MouseButton};
#[cfg(all(feature = "wayland", target_os = "linux"))]
pub use platform::{Platform, PumpMode, Window};
pub use window::WindowDescriptor;
