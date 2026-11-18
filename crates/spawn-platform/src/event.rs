//! Raw platform event types (ADR-0002).
//!
//! These types compile without any native backend, so `spawn-input`, tests and
//! dedicated servers can use them without linking windowing code. They describe
//! what the operating system reported, in delivery order; they carry no
//! semantic actions, key bindings or gameplay meaning.

use crate::key::{KeyCode, MouseButton};

/// Identity of a window created by the platform layer.
///
/// Identifiers are unique for the lifetime of one platform instance and are
/// never reused, so a stale identifier never names a different window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u64);

impl WindowId {
    /// Creates an identifier from its raw value (synthetic events and tests).
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value of this identifier.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Size in physical pixels: the extent of the drawable surface.
///
/// The renderer configures its swapchain with this size, never with the
/// logical size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysicalSize {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
}

impl PhysicalSize {
    /// Creates a physical size.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Whether either dimension is zero (nothing can be presented).
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Size in logical (scale-independent) units, as requested by the application
/// and as used by the window system for layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LogicalSize {
    /// Width in logical units.
    pub width: u32,
    /// Height in logical units.
    pub height: u32,
}

impl LogicalSize {
    /// Creates a logical size.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// Position in physical pixels relative to the top-left corner of the window's
/// drawable surface. Sub-pixel precision is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PhysicalPosition {
    /// Horizontal position; grows to the right.
    pub x: f64,
    /// Vertical position; grows downwards.
    pub y: f64,
}

impl PhysicalPosition {
    /// Creates a physical position.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Whether a key or button went down or up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementState {
    /// The key or button was pressed.
    Pressed,
    /// The key or button was released.
    Released,
}

/// Amount scrolled by one scroll event.
///
/// Positive `y` scrolls content down (wheel rotated towards the user); positive
/// `x` scrolls content right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDelta {
    /// Discrete wheel movement in lines/notches; high-resolution wheels report
    /// fractions of a notch.
    Lines {
        /// Horizontal notches.
        x: f64,
        /// Vertical notches.
        y: f64,
    },
    /// Continuous movement (touchpad, smooth scrolling) in physical pixels.
    Pixels {
        /// Horizontal pixels.
        x: f64,
        /// Vertical pixels.
        y: f64,
    },
}

/// A raw keyboard key transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    /// Layout-independent physical key, named after its position on a US
    /// keyboard.
    pub key: KeyCode,
    /// The platform's native code for the key (the Linux evdev code on Linux).
    /// Stable per platform; lets callers bind keys that [`KeyCode`] does not
    /// name.
    pub native_code: u32,
    /// Press or release.
    pub state: ElementState,
}

/// Something that happened to one window, as reported by the window system.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum WindowEvent {
    /// The user or the window system asked to close the window.
    ///
    /// The platform layer never closes the window by itself; the owner decides.
    /// Repeated requests are delivered each time and are harmless.
    CloseRequested,
    /// The drawable surface changed to this size in physical pixels.
    ///
    /// Not sent for the initial size, which is available from the window when
    /// it is created.
    Resized(PhysicalSize),
    /// The scale factor (physical pixels per logical unit) changed. A
    /// [`WindowEvent::Resized`] with the new physical size follows when the
    /// physical size changed.
    ScaleFactorChanged {
        /// New scale factor, greater than zero.
        scale_factor: f64,
    },
    /// The window gained (`true`) or lost (`false`) keyboard focus.
    ///
    /// The platform does not synthesize key releases on focus loss; consumers
    /// reset held state (see `spawn-input`).
    Focused(bool),
    /// The window system reports the window as hidden (`true`), for example
    /// minimized or on an invisible workspace, or as visible again (`false`).
    /// Support depends on the window system; absence of this event does not
    /// imply visibility.
    Occluded(bool),
    /// A keyboard key was pressed or released while the window had focus.
    Key(KeyEvent),
    /// The pointer entered the window at this position.
    PointerEntered(PhysicalPosition),
    /// The pointer left the window.
    PointerLeft,
    /// The pointer moved to this position inside the window.
    PointerMoved(PhysicalPosition),
    /// A pointer button was pressed or released.
    PointerButton {
        /// Which button.
        button: MouseButton,
        /// Press or release.
        state: ElementState,
    },
    /// The scroll wheel or touchpad scrolled.
    Scroll(ScrollDelta),
}

/// A raw platform event.
///
/// Within one platform instance, events are delivered in the order the window
/// system reported them. No ordering is guaranteed between different platform
/// instances.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// An event concerning one window.
    Window {
        /// The window the event concerns.
        window: WindowId,
        /// What happened.
        event: WindowEvent,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_id_round_trips_raw_value() {
        assert_eq!(WindowId::from_raw(7).raw(), 7);
    }

    #[test]
    fn physical_size_emptiness() {
        assert!(PhysicalSize::new(0, 10).is_empty());
        assert!(PhysicalSize::new(10, 0).is_empty());
        assert!(!PhysicalSize::new(1, 1).is_empty());
    }
}
