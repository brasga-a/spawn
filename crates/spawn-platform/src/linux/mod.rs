//! Linux backends. Only Wayland exists so far; X11 is planned (ADR-0001).

pub(crate) mod evdev;
pub(crate) mod translate;
#[cfg(feature = "wayland")]
pub(crate) mod wayland;
