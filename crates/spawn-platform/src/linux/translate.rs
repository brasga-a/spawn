//! Backend-neutral translation of Wayland protocol events into raw events.
//!
//! The Wayland glue (`wayland.rs`) decodes protocol messages into calls on
//! [`Translator`] and applies the [`SurfaceUpdate`]s it returns. Every decision
//! (configure handling, scale and size arithmetic, focus routing, scroll frame
//! grouping, event order) lives here, uses no protocol types and is tested with
//! synthetic sequences, without a display server.
//!
//! Order guarantee: events are appended in the order the calls arrive, which is
//! the order the display server sent the messages, except that scroll axis
//! messages are grouped and emitted at the pointer `frame` that ends them.

use crate::event::{
    ElementState, Event, KeyEvent, LogicalSize, PhysicalPosition, PhysicalSize, ScrollDelta,
    WindowEvent, WindowId,
};
use crate::linux::evdev;

/// Scale factors are carried in 120ths, as in `wp_fractional_scale_v1`.
const SCALE_DENOMINATOR: u32 = 120;

/// A window's size and scale as currently applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SurfaceMetrics {
    /// Size in logical units (surface-local coordinates).
    pub logical: LogicalSize,
    /// Scale in 120ths (120 = 1.0).
    pub scale_120: u32,
}

impl SurfaceMetrics {
    /// Scale factor as a float.
    pub(crate) fn scale_factor(self) -> f64 {
        f64::from(self.scale_120) / f64::from(SCALE_DENOMINATOR)
    }

    /// Physical size: logical size times scale, rounded half away from zero
    /// (the rule of `wp_fractional_scale_v1`), saturated to `u32`.
    pub(crate) fn physical(self) -> PhysicalSize {
        let scale = |extent: u32| -> u32 {
            let scaled = (u64::from(extent) * u64::from(self.scale_120)
                + u64::from(SCALE_DENOMINATOR / 2))
                / u64::from(SCALE_DENOMINATOR);
            u32::try_from(scaled).unwrap_or(u32::MAX)
        };
        PhysicalSize::new(scale(self.logical.width), scale(self.logical.height))
    }

    /// Converts a surface-local position to physical pixels.
    fn to_physical(self, x: f64, y: f64) -> PhysicalPosition {
        let scale = self.scale_factor();
        PhysicalPosition::new(x * scale, y * scale)
    }
}

/// Protocol requests the glue must send after a configure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceUpdate {
    /// Acknowledge the configure with this serial.
    pub ack_serial: u32,
    /// Set the viewport destination (the logical size) when it changed.
    pub viewport_destination: Option<LogicalSize>,
}

#[derive(Debug, Clone, Copy, Default)]
struct PendingConfigure {
    /// Size suggested by the latest toplevel configure; `None` means "client
    /// chooses" (a zero dimension in the protocol).
    width: Option<u32>,
    height: Option<u32>,
    suspended: bool,
}

#[derive(Debug)]
struct WindowRecord {
    id: WindowId,
    metrics: SurfaceMetrics,
    pending: PendingConfigure,
    configured: bool,
    occluded: bool,
}

/// Scroll accumulated between pointer frames, in surface-local units.
#[derive(Debug, Default)]
struct PendingScroll {
    /// High-resolution wheel steps in 120ths of a notch, per axis (x, y).
    value_120: [i64; 2],
    /// Continuous scroll distance, per axis (x, y).
    continuous: [f64; 2],
}

/// A scroll axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    fn index(self) -> usize {
        match self {
            Self::Horizontal => 0,
            Self::Vertical => 1,
        }
    }
}

/// Translation state for one display connection with one seat.
#[derive(Debug, Default)]
pub(crate) struct Translator {
    windows: Vec<WindowRecord>,
    next_id: u64,
    keyboard_focus: Option<WindowId>,
    pointer_focus: Option<WindowId>,
    scroll: PendingScroll,
    events: Vec<Event>,
}

impl Translator {
    /// Registers a new window with its requested logical size and scale 1.
    pub(crate) fn add_window(&mut self, requested: LogicalSize) -> WindowId {
        let id = WindowId::from_raw(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.windows.push(WindowRecord {
            id,
            metrics: SurfaceMetrics {
                logical: requested,
                scale_120: SCALE_DENOMINATOR,
            },
            pending: PendingConfigure::default(),
            configured: false,
            occluded: false,
        });
        id
    }

    /// Forgets a window destroyed by its owner. Focus on it is cleared without
    /// events, and queued events for it are discarded.
    pub(crate) fn remove_window(&mut self, id: WindowId) {
        self.windows.retain(|window| window.id != id);
        if self.keyboard_focus == Some(id) {
            self.keyboard_focus = None;
        }
        if self.pointer_focus == Some(id) {
            self.pointer_focus = None;
            self.scroll = PendingScroll::default();
        }
        self.events.retain(|event| match event {
            Event::Window { window, .. } => *window != id,
        });
    }

    /// Whether the window has received and applied its first configure.
    pub(crate) fn is_configured(&self, id: WindowId) -> bool {
        self.window(id).is_some_and(|window| window.configured)
    }

    /// Current metrics of a live window.
    pub(crate) fn metrics(&self, id: WindowId) -> Option<SurfaceMetrics> {
        self.window(id).map(|window| window.metrics)
    }

    /// Moves every queued event, in order, to the end of `out`.
    pub(crate) fn drain_into(&mut self, out: &mut Vec<Event>) {
        out.append(&mut self.events);
    }

    /// `xdg_toplevel.configure`: stores a suggested size (zero = client
    /// chooses) and state, applied at the next [`Self::surface_configure`].
    pub(crate) fn toplevel_configure(
        &mut self,
        id: WindowId,
        width: i32,
        height: i32,
        suspended: bool,
    ) {
        let suggest = |extent: i32| u32::try_from(extent).ok().filter(|&extent| extent > 0);
        if let Some(window) = self.window_mut(id) {
            window.pending = PendingConfigure {
                width: suggest(width),
                height: suggest(height),
                suspended,
            };
        }
    }

    /// `xdg_surface.configure`: applies the pending state and returns the
    /// requests to send. Emits `Resized` and `Occluded` on change, except for
    /// the size of the first configure (the initial size).
    pub(crate) fn surface_configure(&mut self, id: WindowId, serial: u32) -> Option<SurfaceUpdate> {
        let window = self.windows.iter_mut().find(|window| window.id == id)?;
        let before = window.metrics;
        let logical = LogicalSize::new(
            window.pending.width.unwrap_or(before.logical.width),
            window.pending.height.unwrap_or(before.logical.height),
        );
        window.metrics.logical = logical;
        let first = !window.configured;
        window.configured = true;
        let resized = !first && window.metrics.physical() != before.physical();
        let occlusion_changed = window.pending.suspended != window.occluded;
        window.occluded = window.pending.suspended;
        let (physical, occluded) = (window.metrics.physical(), window.occluded);

        if resized {
            self.push(id, WindowEvent::Resized(physical));
        }
        if occlusion_changed {
            self.push(id, WindowEvent::Occluded(occluded));
        }
        Some(SurfaceUpdate {
            ack_serial: serial,
            viewport_destination: (first || logical != before.logical).then_some(logical),
        })
    }

    /// Preferred scale from `wp_fractional_scale_v1` (in 120ths) or from
    /// `wl_surface.preferred_buffer_scale` (integer, multiplied by 120).
    /// Zero is invalid and ignored. Before the first configure the scale is
    /// applied silently as part of the initial state.
    pub(crate) fn preferred_scale(&mut self, id: WindowId, scale_120: u32) {
        if scale_120 == 0 {
            return;
        }
        let Some(window) = self.window_mut(id) else {
            return;
        };
        let before = window.metrics;
        if before.scale_120 == scale_120 {
            return;
        }
        window.metrics.scale_120 = scale_120;
        if !window.configured {
            return;
        }
        let after = window.metrics;
        self.push(
            id,
            WindowEvent::ScaleFactorChanged {
                scale_factor: after.scale_factor(),
            },
        );
        if after.physical() != before.physical() {
            self.push(id, WindowEvent::Resized(after.physical()));
        }
    }

    /// `xdg_toplevel.close`: every request is delivered; the owner decides.
    pub(crate) fn close_requested(&mut self, id: WindowId) {
        if self.window(id).is_some() {
            self.push(id, WindowEvent::CloseRequested);
        }
    }

    /// `wl_keyboard.enter`. Keys already held at enter are not reported as
    /// presses (they were pressed elsewhere).
    pub(crate) fn keyboard_enter(&mut self, id: WindowId) {
        if self.window(id).is_none() {
            return;
        }
        if let Some(previous) = self.keyboard_focus.filter(|&previous| previous != id) {
            self.push(previous, WindowEvent::Focused(false));
        }
        if self.keyboard_focus != Some(id) {
            self.keyboard_focus = Some(id);
            self.push(id, WindowEvent::Focused(true));
        }
    }

    /// `wl_keyboard.leave`.
    pub(crate) fn keyboard_leave(&mut self, id: WindowId) {
        if self.keyboard_focus == Some(id) {
            self.keyboard_focus = None;
            self.push(id, WindowEvent::Focused(false));
        }
    }

    /// `wl_keyboard.key` with an evdev code. Dropped without keyboard focus.
    pub(crate) fn key(&mut self, native_code: u32, state: ElementState) {
        if let Some(id) = self.keyboard_focus {
            let key = evdev::key_code(native_code);
            self.push(
                id,
                WindowEvent::Key(KeyEvent {
                    key,
                    native_code,
                    state,
                }),
            );
        }
    }

    /// `wl_pointer.enter` at a surface-local position.
    pub(crate) fn pointer_enter(&mut self, id: WindowId, x: f64, y: f64) {
        let Some(metrics) = self.metrics(id) else {
            return;
        };
        if let Some(previous) = self.pointer_focus.filter(|&previous| previous != id) {
            self.push(previous, WindowEvent::PointerLeft);
        }
        self.pointer_focus = Some(id);
        self.scroll = PendingScroll::default();
        self.push(id, WindowEvent::PointerEntered(metrics.to_physical(x, y)));
    }

    /// `wl_pointer.leave`.
    pub(crate) fn pointer_leave(&mut self, id: WindowId) {
        if self.pointer_focus == Some(id) {
            self.flush_scroll();
            self.pointer_focus = None;
            self.push(id, WindowEvent::PointerLeft);
        }
    }

    /// `wl_pointer.motion` to a surface-local position.
    pub(crate) fn pointer_motion(&mut self, x: f64, y: f64) {
        if let Some((id, metrics)) = self.pointer_target() {
            self.push(id, WindowEvent::PointerMoved(metrics.to_physical(x, y)));
        }
    }

    /// `wl_pointer.button` with an evdev code.
    pub(crate) fn pointer_button(&mut self, native_code: u32, state: ElementState) {
        if let Some((id, _)) = self.pointer_target() {
            let button = evdev::mouse_button(native_code);
            self.push(id, WindowEvent::PointerButton { button, state });
        }
    }

    /// `wl_pointer.axis`: continuous distance in surface-local units.
    pub(crate) fn pointer_axis(&mut self, axis: Axis, value: f64) {
        if value.is_finite() {
            self.scroll.continuous[axis.index()] += value;
        }
    }

    /// `wl_pointer.axis_value120`: wheel steps in 120ths of a notch.
    pub(crate) fn pointer_axis_value_120(&mut self, axis: Axis, value_120: i32) {
        let slot = &mut self.scroll.value_120[axis.index()];
        *slot = slot.saturating_add(i64::from(value_120));
    }

    /// `wl_pointer.frame`: emits the scroll accumulated since the last frame.
    /// When a frame carries wheel steps they are reported as lines (the
    /// continuous value of the same frame describes the same motion);
    /// otherwise the continuous distance is reported in physical pixels.
    /// Pointers older than version 5 have no frames: the glue calls this
    /// after every axis message.
    pub(crate) fn pointer_frame(&mut self) {
        self.flush_scroll();
    }

    fn flush_scroll(&mut self) {
        let scroll = std::mem::take(&mut self.scroll);
        let Some((id, metrics)) = self.pointer_target() else {
            return;
        };
        let delta = if scroll.value_120 != [0, 0] {
            // i64 → f64: wheel step sums stay far below 2^53.
            #[allow(clippy::cast_precision_loss)]
            let notches = |steps: i64| steps as f64 / f64::from(SCALE_DENOMINATOR);
            ScrollDelta::Lines {
                x: notches(scroll.value_120[0]),
                y: notches(scroll.value_120[1]),
            }
        } else if scroll.continuous != [0.0, 0.0] {
            let scale = metrics.scale_factor();
            ScrollDelta::Pixels {
                x: scroll.continuous[0] * scale,
                y: scroll.continuous[1] * scale,
            }
        } else {
            return;
        };
        self.push(id, WindowEvent::Scroll(delta));
    }

    fn pointer_target(&self) -> Option<(WindowId, SurfaceMetrics)> {
        let id = self.pointer_focus?;
        Some((id, self.metrics(id)?))
    }

    fn push(&mut self, window: WindowId, event: WindowEvent) {
        self.events.push(Event::Window { window, event });
    }

    fn window(&self, id: WindowId) -> Option<&WindowRecord> {
        self.windows.iter().find(|window| window.id == id)
    }

    fn window_mut(&mut self, id: WindowId) -> Option<&mut WindowRecord> {
        self.windows.iter_mut().find(|window| window.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{KeyCode, MouseButton};
    use ElementState::{Pressed, Released};

    fn drain(translator: &mut Translator) -> Vec<Event> {
        let mut out = Vec::new();
        translator.drain_into(&mut out);
        out
    }

    fn window_events(translator: &mut Translator, id: WindowId) -> Vec<WindowEvent> {
        drain(translator)
            .into_iter()
            .map(|Event::Window { window, event }| {
                assert_eq!(window, id);
                event
            })
            .collect()
    }

    /// A translator with one window that received its first configure.
    fn configured(size: LogicalSize, scale_120: u32) -> (Translator, WindowId) {
        let mut translator = Translator::default();
        let id = translator.add_window(size);
        translator.preferred_scale(id, scale_120);
        translator.toplevel_configure(id, 0, 0, false);
        let update = translator.surface_configure(id, 1);
        assert_eq!(
            update,
            Some(SurfaceUpdate {
                ack_serial: 1,
                viewport_destination: Some(size)
            })
        );
        assert!(
            drain(&mut translator).is_empty(),
            "first configure emits no events"
        );
        (translator, id)
    }

    #[test]
    fn physical_size_rounds_half_away_from_zero() {
        let metrics = |w, h, scale_120| SurfaceMetrics {
            logical: LogicalSize::new(w, h),
            scale_120,
        };
        assert_eq!(
            metrics(1280, 720, 120).physical(),
            PhysicalSize::new(1280, 720)
        );
        assert_eq!(
            metrics(1280, 720, 240).physical(),
            PhysicalSize::new(2560, 1440)
        );
        // 1.25: 1001 * 1.25 = 1251.25 → 1251; 3 * 1.25 = 3.75 → 4.
        assert_eq!(metrics(1001, 3, 150).physical(), PhysicalSize::new(1251, 4));
        // 1.5: 3 * 1.5 = 4.5 → 5 (half away from zero).
        assert_eq!(metrics(3, 1, 180).physical(), PhysicalSize::new(5, 2));
        assert_eq!(metrics(u32::MAX, 1, 240).physical().width, u32::MAX);
    }

    #[test]
    fn first_configure_uses_requested_size_when_compositor_leaves_choice() {
        let (translator, id) = configured(LogicalSize::new(800, 600), 120);
        assert!(translator.is_configured(id));
        let metrics = translator.metrics(id);
        assert_eq!(
            metrics.map(SurfaceMetrics::physical),
            Some(PhysicalSize::new(800, 600))
        );
    }

    #[test]
    fn first_configure_adopts_compositor_size_without_resize_event() {
        let mut translator = Translator::default();
        let id = translator.add_window(LogicalSize::new(800, 600));
        assert!(!translator.is_configured(id));
        translator.toplevel_configure(id, 1920, 1080, false);
        let update = translator.surface_configure(id, 9);
        assert_eq!(
            update.and_then(|u| u.viewport_destination),
            Some(LogicalSize::new(1920, 1080))
        );
        assert!(drain(&mut translator).is_empty());
    }

    #[test]
    fn later_configure_with_new_size_emits_resized_in_physical_pixels() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 180);
        translator.toplevel_configure(id, 1000, 500, false);
        let update = translator.surface_configure(id, 2);
        assert_eq!(
            update,
            Some(SurfaceUpdate {
                ack_serial: 2,
                viewport_destination: Some(LogicalSize::new(1000, 500))
            })
        );
        assert_eq!(
            window_events(&mut translator, id),
            [WindowEvent::Resized(PhysicalSize::new(1500, 750))]
        );
    }

    #[test]
    fn configure_with_unchanged_size_only_acks() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        translator.toplevel_configure(id, 800, 600, false);
        let update = translator.surface_configure(id, 3);
        assert_eq!(
            update,
            Some(SurfaceUpdate {
                ack_serial: 3,
                viewport_destination: None
            })
        );
        assert!(drain(&mut translator).is_empty());
    }

    #[test]
    fn suspended_state_reports_occlusion_changes_once() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        for (serial, suspended) in [(2, true), (3, true), (4, false)] {
            translator.toplevel_configure(id, 0, 0, suspended);
            translator.surface_configure(id, serial);
        }
        assert_eq!(
            window_events(&mut translator, id),
            [WindowEvent::Occluded(true), WindowEvent::Occluded(false)]
        );
    }

    #[test]
    fn scale_change_emits_scale_then_resize() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        translator.preferred_scale(id, 150);
        translator.preferred_scale(id, 150);
        translator.preferred_scale(id, 0);
        assert_eq!(
            window_events(&mut translator, id),
            [
                WindowEvent::ScaleFactorChanged { scale_factor: 1.25 },
                WindowEvent::Resized(PhysicalSize::new(1000, 750)),
            ]
        );
        let metrics = translator.metrics(id);
        assert_eq!(metrics.map(|m| m.logical), Some(LogicalSize::new(800, 600)));
    }

    #[test]
    fn scale_before_first_configure_is_initial_state() {
        let mut translator = Translator::default();
        let id = translator.add_window(LogicalSize::new(100, 100));
        translator.preferred_scale(id, 240);
        assert!(drain(&mut translator).is_empty());
        translator.toplevel_configure(id, 0, 0, false);
        translator.surface_configure(id, 1);
        assert!(drain(&mut translator).is_empty());
        let physical = translator.metrics(id).map(SurfaceMetrics::physical);
        assert_eq!(physical, Some(PhysicalSize::new(200, 200)));
    }

    #[test]
    fn negative_suggested_size_is_treated_as_client_choice() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        translator.toplevel_configure(id, -5, 400, false);
        translator.surface_configure(id, 2);
        let logical = translator.metrics(id).map(|m| m.logical);
        assert_eq!(logical, Some(LogicalSize::new(800, 400)));
    }

    /// FND-T10 (platform part): repeated close requests are each delivered,
    /// change no state and leave the window usable.
    #[test]
    fn repeated_close_requests_are_delivered_and_harmless() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        for _ in 0..3 {
            translator.close_requested(id);
        }
        assert_eq!(
            window_events(&mut translator, id),
            [WindowEvent::CloseRequested; 3]
        );
        assert!(translator.is_configured(id));
        translator.toplevel_configure(id, 640, 480, false);
        assert!(translator.surface_configure(id, 2).is_some());
        translator.remove_window(id);
        translator.close_requested(id);
        assert!(drain(&mut translator).is_empty());
    }

    #[test]
    fn keys_route_to_focused_window_and_are_dropped_without_focus() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        translator.key(17, Pressed);
        translator.keyboard_enter(id);
        translator.keyboard_enter(id);
        translator.key(17, Pressed);
        translator.key(17, Released);
        translator.keyboard_leave(id);
        translator.key(30, Pressed);
        let w = |state| {
            WindowEvent::Key(KeyEvent {
                key: KeyCode::W,
                native_code: 17,
                state,
            })
        };
        assert_eq!(
            window_events(&mut translator, id),
            [
                WindowEvent::Focused(true),
                w(Pressed),
                w(Released),
                WindowEvent::Focused(false)
            ]
        );
    }

    #[test]
    fn keyboard_focus_moving_between_windows_unfocuses_the_previous() {
        let mut translator = Translator::default();
        let first = translator.add_window(LogicalSize::new(10, 10));
        let second = translator.add_window(LogicalSize::new(10, 10));
        assert_ne!(first, second);
        translator.keyboard_enter(first);
        translator.keyboard_enter(second);
        translator.keyboard_leave(first);
        let events = drain(&mut translator);
        let focus = |window, focused| Event::Window {
            window,
            event: WindowEvent::Focused(focused),
        };
        assert_eq!(
            events,
            [focus(first, true), focus(first, false), focus(second, true)]
        );
    }

    #[test]
    fn pointer_positions_are_scaled_to_physical_pixels() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 240);
        translator.pointer_motion(1.0, 1.0);
        translator.pointer_enter(id, 10.5, 20.0);
        translator.pointer_motion(11.0, 21.25);
        translator.pointer_button(0x110, Pressed);
        translator.pointer_button(0x110, Released);
        translator.pointer_leave(id);
        translator.pointer_motion(1.0, 1.0);
        translator.pointer_button(0x111, Pressed);
        assert_eq!(
            window_events(&mut translator, id),
            [
                WindowEvent::PointerEntered(PhysicalPosition::new(21.0, 40.0)),
                WindowEvent::PointerMoved(PhysicalPosition::new(22.0, 42.5)),
                WindowEvent::PointerButton {
                    button: MouseButton::Left,
                    state: Pressed
                },
                WindowEvent::PointerButton {
                    button: MouseButton::Left,
                    state: Released
                },
                WindowEvent::PointerLeft,
            ]
        );
    }

    #[test]
    fn wheel_frame_reports_lines_and_ignores_duplicate_continuous_value() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        translator.pointer_enter(id, 0.0, 0.0);
        drain(&mut translator);
        translator.pointer_axis_value_120(Axis::Vertical, 120);
        translator.pointer_axis(Axis::Vertical, 15.0);
        translator.pointer_frame();
        translator.pointer_axis_value_120(Axis::Horizontal, -60);
        translator.pointer_frame();
        translator.pointer_frame();
        assert_eq!(
            window_events(&mut translator, id),
            [
                WindowEvent::Scroll(ScrollDelta::Lines { x: 0.0, y: 1.0 }),
                WindowEvent::Scroll(ScrollDelta::Lines { x: -0.5, y: 0.0 }),
            ]
        );
    }

    #[test]
    fn continuous_scroll_is_reported_in_physical_pixels() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 180);
        translator.pointer_enter(id, 0.0, 0.0);
        drain(&mut translator);
        translator.pointer_axis(Axis::Vertical, 2.0);
        translator.pointer_axis(Axis::Horizontal, -4.0);
        translator.pointer_axis(Axis::Vertical, f64::NAN);
        translator.pointer_frame();
        assert_eq!(
            window_events(&mut translator, id),
            [WindowEvent::Scroll(ScrollDelta::Pixels { x: -6.0, y: 3.0 })]
        );
    }

    #[test]
    fn scroll_without_pointer_focus_is_discarded() {
        let (mut translator, _id) = configured(LogicalSize::new(800, 600), 120);
        translator.pointer_axis_value_120(Axis::Vertical, 120);
        translator.pointer_frame();
        assert!(drain(&mut translator).is_empty());
    }

    #[test]
    fn removing_a_window_clears_focus_and_its_queued_events() {
        let mut translator = Translator::default();
        let kept = translator.add_window(LogicalSize::new(10, 10));
        let removed = translator.add_window(LogicalSize::new(10, 10));
        translator.close_requested(kept);
        translator.keyboard_enter(removed);
        translator.pointer_enter(removed, 1.0, 1.0);
        translator.remove_window(removed);
        translator.key(1, Pressed);
        translator.pointer_motion(2.0, 2.0);
        assert_eq!(
            drain(&mut translator),
            [Event::Window {
                window: kept,
                event: WindowEvent::CloseRequested
            }]
        );
        assert!(translator.metrics(removed).is_none());
    }

    #[test]
    fn window_ids_are_never_reused() {
        let mut translator = Translator::default();
        let first = translator.add_window(LogicalSize::new(10, 10));
        translator.remove_window(first);
        let second = translator.add_window(LogicalSize::new(10, 10));
        assert_ne!(first, second);
    }

    /// FND-T05: a mixed synthetic protocol sequence yields raw events in the
    /// same order, with no actions or bindings (the event types have none).
    #[test]
    fn mixed_sequence_preserves_protocol_order() {
        let (mut translator, id) = configured(LogicalSize::new(800, 600), 120);
        translator.keyboard_enter(id);
        translator.pointer_enter(id, 5.0, 6.0);
        translator.key(30, Pressed);
        translator.pointer_motion(7.0, 8.0);
        translator.pointer_button(0x111, Pressed);
        translator.key(30, Released);
        translator.pointer_axis_value_120(Axis::Vertical, -240);
        translator.pointer_frame();
        translator.toplevel_configure(id, 400, 300, false);
        translator.surface_configure(id, 2);
        translator.pointer_button(0x111, Released);
        translator.close_requested(id);
        translator.keyboard_leave(id);
        let a = |state| {
            WindowEvent::Key(KeyEvent {
                key: KeyCode::A,
                native_code: 30,
                state,
            })
        };
        let right = |state| WindowEvent::PointerButton {
            button: MouseButton::Right,
            state,
        };
        assert_eq!(
            window_events(&mut translator, id),
            [
                WindowEvent::Focused(true),
                WindowEvent::PointerEntered(PhysicalPosition::new(5.0, 6.0)),
                a(Pressed),
                WindowEvent::PointerMoved(PhysicalPosition::new(7.0, 8.0)),
                right(Pressed),
                a(Released),
                WindowEvent::Scroll(ScrollDelta::Lines { x: 0.0, y: -2.0 }),
                WindowEvent::Resized(PhysicalSize::new(400, 300)),
                right(Released),
                WindowEvent::CloseRequested,
                WindowEvent::Focused(false),
            ]
        );
    }
}
