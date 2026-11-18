//! Raw device state tracking for keyboard and pointer inputs.
//!
//! `DeviceState` consumes raw [`spawn_platform`] events without attaching
//! semantic or genre meaning. Transitions (`pressed`, `released`) and motion or
//! scroll deltas accumulate across zero or more event batches until consumed at
//! a simulation tick boundary, ensuring edges are neither lost on zero-tick
//! render frames nor duplicated across multi-tick catch-up updates
//! (`FND-INV-10`).

#![deny(clippy::indexing_slicing)]

use std::mem;

use spawn_platform::{
    ElementState, Event, KeyCode, KeyEvent, MouseButton, PhysicalPosition, ScrollDelta,
    WindowEvent, WindowId,
};

const KEY_WORDS: usize = 2;
const MAX_NATIVE_KEY_SLOTS: usize = 32;
const MAX_OTHER_BUTTON_SLOTS: usize = 16;

/// Sampled transition and hold state of a digital button or semantic action for
/// one simulation tick.
///
/// Within a single tick interval, both `pressed` and `released` may be `true`
/// when a rapid press-and-release tap (ending with `held == false`) or a
/// release-and-repress sequence (ending with `held == true`) occurred before
/// the tick sampled input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ButtonState {
    /// At least one upward-to-downward transition occurred since the previous
    /// sampled tick.
    pub pressed: bool,
    /// The button or action is held down at the sampled tick boundary.
    pub held: bool,
    /// At least one downward-to-upward transition occurred since the previous
    /// sampled tick.
    pub released: bool,
}

impl ButtonState {
    /// Idle unpressed state (`pressed: false`, `held: false`, `released: false`).
    pub const UP: Self = Self {
        pressed: false,
        held: false,
        released: false,
    };

    /// Whether a press transition occurred in this tick sample.
    pub const fn is_pressed(self) -> bool {
        self.pressed
    }

    /// Whether the button or action is held down at this tick boundary.
    pub const fn is_held(self) -> bool {
        self.held
    }

    /// Whether a release transition occurred in this tick sample.
    pub const fn is_released(self) -> bool {
        self.released
    }

    /// Whether the input was active during this tick (`pressed || held`),
    /// catching sub-tick taps that already released before the tick ran.
    pub const fn was_active(self) -> bool {
        self.pressed || self.held
    }
}

/// Two-dimensional pointer motion delta accumulated over a tick interval.
///
/// Positive `dx` points right; positive `dy` points down, matching
/// [`spawn_platform::PhysicalPosition`] and Phase 2b's
/// `WindowEvent::PointerRelativeMotion`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PointerDelta {
    /// Horizontal motion delta.
    pub dx: f64,
    /// Vertical motion delta.
    pub dy: f64,
}

impl PointerDelta {
    /// Zero motion delta.
    pub const ZERO: Self = Self { dx: 0.0, dy: 0.0 };

    /// Creates a motion delta, normalizing `-0.0` to `0.0`.
    pub const fn new(dx: f64, dy: f64) -> Self {
        Self {
            dx: canonical_zero_f64(dx),
            dy: canonical_zero_f64(dy),
        }
    }

    /// Whether both components are zero.
    pub fn is_zero(self) -> bool {
        self.dx == 0.0 && self.dy == 0.0
    }
}

/// Two-dimensional scroll delta accumulated over a tick interval.
///
/// Positive `y` scrolls content down; positive `x` scrolls content right,
/// matching [`spawn_platform::ScrollDelta`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollVector {
    /// Horizontal scroll amount.
    pub x: f64,
    /// Vertical scroll amount.
    pub y: f64,
}

impl ScrollVector {
    /// Zero scroll delta.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// Creates a scroll vector, normalizing `-0.0` to `0.0`.
    pub const fn new(x: f64, y: f64) -> Self {
        Self {
            x: canonical_zero_f64(x),
            y: canonical_zero_f64(y),
        }
    }

    /// Whether both components are zero.
    pub fn is_zero(self) -> bool {
        self.x == 0.0 && self.y == 0.0
    }
}

/// Category of physical input device for explicit disconnect handling
/// (`FND-INV-11`, `FND-T04`).
///
/// Marked `#[non_exhaustive]` so gamepad device identities (proposed ADR-0018)
/// can be added later without breaking callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeviceKind {
    /// Physical or virtual keyboard.
    Keyboard,
    /// Pointer / mouse device.
    Mouse,
}

#[derive(Debug, Clone, Copy, Default)]
struct BitSet128([u64; KEY_WORDS]);

impl BitSet128 {
    fn get(self, index: usize) -> bool {
        let word = index / 64;
        let bit = index % 64;
        self.0
            .get(word)
            .is_some_and(|bits| (*bits & (1_u64 << bit)) != 0)
    }

    fn set(&mut self, index: usize, value: bool) {
        let word = index / 64;
        let bit = index % 64;
        if let Some(bits) = self.0.get_mut(word) {
            if value {
                *bits |= 1_u64 << bit;
            } else {
                *bits &= !(1_u64 << bit);
            }
        }
    }

    fn bitor_assign(&mut self, rhs: Self) {
        for (dst, src) in self.0.iter_mut().zip(rhs.0) {
            *dst |= src;
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct CodeSlot {
    active: bool,
    code: u32,
    down: bool,
    pending_pressed: bool,
    pending_released: bool,
    sampled_down: bool,
    sampled_pressed: bool,
    sampled_released: bool,
}

impl CodeSlot {
    fn apply(&mut self, state: ElementState) {
        match state {
            ElementState::Pressed => {
                if !self.down {
                    self.down = true;
                    self.pending_pressed = true;
                }
            }
            ElementState::Released => {
                if self.down {
                    self.down = false;
                    self.pending_released = true;
                }
            }
        }
    }

    fn release_if_down(&mut self) {
        if self.down {
            self.down = false;
            self.pending_released = true;
        }
    }

    fn sample_tick(&mut self) {
        self.sampled_down = self.down;
        self.sampled_pressed = mem::take(&mut self.pending_pressed);
        self.sampled_released = mem::take(&mut self.pending_released);
        if !self.down && !self.sampled_pressed && !self.sampled_released {
            *self = Self::default();
        }
    }

    const fn sampled_state(self) -> ButtonState {
        if !self.active {
            return ButtonState::UP;
        }
        ButtonState {
            pressed: self.sampled_pressed,
            held: self.sampled_down,
            released: self.sampled_released,
        }
    }
}

/// Allocation-free tracker of raw keyboard, pointer button, cursor, motion,
/// scroll, and window-focus state.
///
/// Live `is_*_down` queries reflect the latest processed event immediately.
/// Per-tick queries (`key_state`, `mouse_button_state`, `pointer_delta`,
/// `scroll_lines`, `scroll_pixels`) reflect the snapshot latched by the most
/// recent tick sample (`InputState::sample_tick`).
#[derive(Debug, Clone)]
pub struct DeviceState {
    focused: bool,
    focused_window: Option<WindowId>,
    pointer_window: Option<WindowId>,

    key_down: BitSet128,
    key_pending_pressed: BitSet128,
    key_pending_released: BitSet128,
    key_sampled_down: BitSet128,
    key_sampled_pressed: BitSet128,
    key_sampled_released: BitSet128,
    native_keys: [CodeSlot; MAX_NATIVE_KEY_SLOTS],

    mouse_down: u8,
    mouse_pending_pressed: u8,
    mouse_pending_released: u8,
    mouse_sampled_down: u8,
    mouse_sampled_pressed: u8,
    mouse_sampled_released: u8,
    other_buttons: [CodeSlot; MAX_OTHER_BUTTON_SLOTS],

    cursor_position: Option<PhysicalPosition>,
    cursor_anchor_valid: bool,
    pending_cursor_delta: PointerDelta,
    sampled_cursor_delta: PointerDelta,

    pending_relative_delta: PointerDelta,
    pending_has_relative: bool,
    sampled_relative_delta: PointerDelta,
    sampled_has_relative: bool,

    pending_scroll_lines: ScrollVector,
    sampled_scroll_lines: ScrollVector,
    pending_scroll_pixels: ScrollVector,
    sampled_scroll_pixels: ScrollVector,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceState {
    /// Creates an idle device state with no held inputs, no focus, and zero
    /// pending deltas.
    pub const fn new() -> Self {
        Self {
            focused: false,
            focused_window: None,
            pointer_window: None,
            key_down: BitSet128([0; KEY_WORDS]),
            key_pending_pressed: BitSet128([0; KEY_WORDS]),
            key_pending_released: BitSet128([0; KEY_WORDS]),
            key_sampled_down: BitSet128([0; KEY_WORDS]),
            key_sampled_pressed: BitSet128([0; KEY_WORDS]),
            key_sampled_released: BitSet128([0; KEY_WORDS]),
            native_keys: [CodeSlot {
                active: false,
                code: 0,
                down: false,
                pending_pressed: false,
                pending_released: false,
                sampled_down: false,
                sampled_pressed: false,
                sampled_released: false,
            }; MAX_NATIVE_KEY_SLOTS],
            mouse_down: 0,
            mouse_pending_pressed: 0,
            mouse_pending_released: 0,
            mouse_sampled_down: 0,
            mouse_sampled_pressed: 0,
            mouse_sampled_released: 0,
            other_buttons: [CodeSlot {
                active: false,
                code: 0,
                down: false,
                pending_pressed: false,
                pending_released: false,
                sampled_down: false,
                sampled_pressed: false,
                sampled_released: false,
            }; MAX_OTHER_BUTTON_SLOTS],
            cursor_position: None,
            cursor_anchor_valid: false,
            pending_cursor_delta: PointerDelta::ZERO,
            sampled_cursor_delta: PointerDelta::ZERO,
            pending_relative_delta: PointerDelta::ZERO,
            pending_has_relative: false,
            sampled_relative_delta: PointerDelta::ZERO,
            sampled_has_relative: false,
            pending_scroll_lines: ScrollVector::ZERO,
            sampled_scroll_lines: ScrollVector::ZERO,
            pending_scroll_pixels: ScrollVector::ZERO,
            sampled_scroll_pixels: ScrollVector::ZERO,
        }
    }

    /// Whether a window currently has keyboard focus according to processed
    /// events.
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// Identifier of the currently focused window, when tracked via
    /// [`InputState::handle_event`](crate::InputState::handle_event).
    pub const fn focused_window(&self) -> Option<WindowId> {
        self.focused_window
    }

    /// Identifier of the window currently holding pointer focus, when tracked
    /// via [`InputState::handle_event`](crate::InputState::handle_event).
    pub const fn pointer_window(&self) -> Option<WindowId> {
        self.pointer_window
    }

    /// Whether `key` is physically held down right now (before the next tick
    /// sample). Returns `false` for [`KeyCode::Unidentified`].
    pub fn is_key_down(&self, key: KeyCode) -> bool {
        key_index(key).is_some_and(|index| self.key_down.get(index))
    }

    /// Whether the physical key with `native_code` is held down right now.
    pub fn is_native_key_down(&self, native_code: u32) -> bool {
        self.native_keys
            .iter()
            .any(|slot| slot.active && slot.code == native_code && slot.down)
    }

    /// Whether `button` is physically held down right now.
    pub fn is_mouse_button_down(&self, button: MouseButton) -> bool {
        if let Some(mask) = mouse_button_mask(button) {
            (self.mouse_down & mask) != 0
        } else if let MouseButton::Other(code) = button {
            self.other_buttons
                .iter()
                .any(|slot| slot.active && slot.code == code && slot.down)
        } else {
            false
        }
    }

    /// Sampled press/hold/release state of `key` for the most recent simulation
    /// tick. Returns [`ButtonState::UP`] for [`KeyCode::Unidentified`].
    pub fn key_state(&self, key: KeyCode) -> ButtonState {
        let Some(index) = key_index(key) else {
            return ButtonState::UP;
        };
        ButtonState {
            pressed: self.key_sampled_pressed.get(index),
            held: self.key_sampled_down.get(index),
            released: self.key_sampled_released.get(index),
        }
    }

    /// Sampled press/hold/release state of `native_code` for the most recent
    /// simulation tick.
    pub fn native_key_state(&self, native_code: u32) -> ButtonState {
        self.native_keys
            .iter()
            .find(|slot| slot.active && slot.code == native_code)
            .copied()
            .map_or(ButtonState::UP, CodeSlot::sampled_state)
    }

    /// Sampled press/hold/release state of `button` for the most recent
    /// simulation tick.
    pub fn mouse_button_state(&self, button: MouseButton) -> ButtonState {
        if let Some(mask) = mouse_button_mask(button) {
            ButtonState {
                pressed: (self.mouse_sampled_pressed & mask) != 0,
                held: (self.mouse_sampled_down & mask) != 0,
                released: (self.mouse_sampled_released & mask) != 0,
            }
        } else if let MouseButton::Other(code) = button {
            self.other_buttons
                .iter()
                .find(|slot| slot.active && slot.code == code)
                .copied()
                .map_or(ButtonState::UP, CodeSlot::sampled_state)
        } else {
            ButtonState::UP
        }
    }

    /// Latest known cursor position in physical pixels while inside the window,
    /// or `None` before entry or after [`WindowEvent::PointerLeft`].
    pub const fn cursor_position(&self) -> Option<PhysicalPosition> {
        self.cursor_position
    }

    /// Effective pointer motion delta sampled for the most recent simulation
    /// tick.
    ///
    /// Returns the unaccelerated relative motion delta when relative motion
    /// events were received during the tick interval, or the surface cursor
    /// displacement delta otherwise, avoiding double-counting when a platform
    /// delivers both while unlocked.
    pub const fn pointer_delta(&self) -> PointerDelta {
        if self.sampled_has_relative {
            self.sampled_relative_delta
        } else {
            self.sampled_cursor_delta
        }
    }

    /// Surface cursor displacement delta sampled for the most recent simulation
    /// tick, derived from consecutive [`WindowEvent::PointerMoved`] positions.
    pub const fn cursor_delta(&self) -> PointerDelta {
        self.sampled_cursor_delta
    }

    /// Unaccelerated relative pointer motion delta sampled for the most recent
    /// simulation tick.
    pub const fn relative_delta(&self) -> PointerDelta {
        self.sampled_relative_delta
    }

    /// Discrete scroll-wheel delta (in lines/notches) sampled for the most
    /// recent simulation tick.
    pub const fn scroll_lines(&self) -> ScrollVector {
        self.sampled_scroll_lines
    }

    /// Continuous scroll delta (in physical pixels) sampled for the most recent
    /// simulation tick.
    pub const fn scroll_pixels(&self) -> ScrollVector {
        self.sampled_scroll_pixels
    }

    pub(crate) fn handle_event(&mut self, event: &Event) {
        if let Event::Window { window, event } = event {
            self.handle_window_event_for(Some(*window), event);
        }
    }

    pub(crate) fn handle_window_event(&mut self, event: &WindowEvent) {
        self.handle_window_event_for(None, event);
    }

    fn handle_window_event_for(&mut self, window: Option<WindowId>, event: &WindowEvent) {
        match event {
            WindowEvent::Focused(true) => {
                if let (Some(prev), Some(next)) = (self.focused_window, window)
                    && prev != next
                {
                    self.release_held();
                }
                self.focused = true;
                if window.is_some() {
                    self.focused_window = window;
                }
            }
            WindowEvent::Focused(false) => {
                if window.is_none()
                    || self.focused_window.is_none()
                    || self.focused_window == window
                {
                    self.focused = false;
                    self.focused_window = None;
                    self.release_held();
                }
            }
            WindowEvent::CloseRequested => {
                if window.is_none()
                    || self.focused_window.is_none()
                    || self.focused_window == window
                {
                    self.focused = false;
                    self.focused_window = None;
                    self.release_keyboard();
                    self.cursor_anchor_valid = false;
                }
                if window.is_none()
                    || self.pointer_window.is_none()
                    || self.pointer_window == window
                {
                    self.pointer_window = None;
                    self.cursor_position = None;
                    self.cursor_anchor_valid = false;
                    self.release_mouse_buttons();
                }
            }
            WindowEvent::Key(key_event) => self.apply_key_event(*key_event),
            WindowEvent::PointerEntered(position) => {
                if let (Some(prev), Some(next)) = (self.pointer_window, window)
                    && prev != next
                {
                    self.release_mouse_buttons();
                    self.cursor_position = None;
                    self.cursor_anchor_valid = false;
                }
                if window.is_some() {
                    self.pointer_window = window;
                }
                if let (Some(x), Some(y)) = (sanitize_f64(position.x), sanitize_f64(position.y)) {
                    self.cursor_position = Some(PhysicalPosition::new(x, y));
                    self.cursor_anchor_valid = true;
                }
            }
            WindowEvent::PointerLeft => {
                if window.is_none()
                    || self.pointer_window.is_none()
                    || self.pointer_window == window
                {
                    self.pointer_window = None;
                    self.release_mouse_buttons();
                    self.cursor_position = None;
                    self.cursor_anchor_valid = false;
                }
            }
            WindowEvent::PointerMoved(position) => {
                if let (Some(prev), Some(next)) = (self.pointer_window, window)
                    && prev != next
                {
                    self.cursor_position = None;
                    self.cursor_anchor_valid = false;
                }
                if window.is_some() {
                    self.pointer_window = window;
                }
                if let (Some(x), Some(y)) = (sanitize_f64(position.x), sanitize_f64(position.y)) {
                    if self.cursor_anchor_valid
                        && let Some(prev) = self.cursor_position
                    {
                        let dx = clamp_finite_f64(x - prev.x);
                        let dy = clamp_finite_f64(y - prev.y);
                        self.pending_cursor_delta = PointerDelta::new(
                            saturating_add_f64(self.pending_cursor_delta.dx, dx),
                            saturating_add_f64(self.pending_cursor_delta.dy, dy),
                        );
                    }
                    self.cursor_position = Some(PhysicalPosition::new(x, y));
                    self.cursor_anchor_valid = true;
                }
            }
            WindowEvent::PointerRelativeMotion { dx, dy } => {
                if window.is_none()
                    || self.pointer_window.is_none()
                    || self.pointer_window == window
                {
                    if self.pointer_window.is_none() && window.is_some() {
                        self.pointer_window = window;
                    }
                    self.handle_relative_motion(*dx, *dy);
                }
            }
            WindowEvent::PointerButton { button, state } => {
                if window.is_none()
                    || self.pointer_window.is_none()
                    || self.pointer_window == window
                {
                    if self.pointer_window.is_none() && window.is_some() {
                        self.pointer_window = window;
                    }
                    self.apply_mouse_button(*button, *state);
                }
            }
            WindowEvent::Scroll(delta) => match *delta {
                ScrollDelta::Lines { x, y } => {
                    let dx = sanitize_f64(x).unwrap_or(0.0);
                    let dy = sanitize_f64(y).unwrap_or(0.0);
                    self.pending_scroll_lines = ScrollVector::new(
                        saturating_add_f64(self.pending_scroll_lines.x, dx),
                        saturating_add_f64(self.pending_scroll_lines.y, dy),
                    );
                }
                ScrollDelta::Pixels { x, y } => {
                    let dx = sanitize_f64(x).unwrap_or(0.0);
                    let dy = sanitize_f64(y).unwrap_or(0.0);
                    self.pending_scroll_pixels = ScrollVector::new(
                        saturating_add_f64(self.pending_scroll_pixels.x, dx),
                        saturating_add_f64(self.pending_scroll_pixels.y, dy),
                    );
                }
            },
            // Non-input window events (`Resized`, `ScaleFactorChanged`,
            // `Occluded`, `CursorGrabChanged`) and future `#[non_exhaustive]`
            // variants are ignored without panicking.
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Occluded(_)
            | WindowEvent::CursorGrabChanged(_)
            | _ => {}
        }
    }

    pub(crate) fn handle_relative_motion(&mut self, dx: f64, dy: f64) {
        let Some(clean_dx) = sanitize_f64(dx) else {
            return;
        };
        let Some(clean_dy) = sanitize_f64(dy) else {
            return;
        };
        if clean_dx == 0.0 && clean_dy == 0.0 {
            return;
        }
        self.pending_has_relative = true;
        self.pending_relative_delta = PointerDelta::new(
            saturating_add_f64(self.pending_relative_delta.dx, clean_dx),
            saturating_add_f64(self.pending_relative_delta.dy, clean_dy),
        );
    }

    fn apply_key_event(&mut self, event: KeyEvent) {
        if let Some(index) = key_index(event.key) {
            let was_down = self.key_down.get(index);
            match event.state {
                ElementState::Pressed => {
                    if !was_down {
                        self.key_down.set(index, true);
                        self.key_pending_pressed.set(index, true);
                    }
                }
                ElementState::Released => {
                    if was_down {
                        self.key_down.set(index, false);
                        self.key_pending_released.set(index, true);
                    }
                }
            }
        }

        // Track native codes except when `0` is used as a synthetic placeholder
        // alongside an already-named `KeyCode`.
        if event.native_code != 0 || event.key == KeyCode::Unidentified {
            update_code_slot(&mut self.native_keys, event.native_code, event.state);
        }
    }

    fn apply_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        if let Some(mask) = mouse_button_mask(button) {
            let was_down = (self.mouse_down & mask) != 0;
            match state {
                ElementState::Pressed => {
                    if !was_down {
                        self.mouse_down |= mask;
                        self.mouse_pending_pressed |= mask;
                    }
                }
                ElementState::Released => {
                    if was_down {
                        self.mouse_down &= !mask;
                        self.mouse_pending_released |= mask;
                    }
                }
            }
        } else if let MouseButton::Other(code) = button {
            update_code_slot(&mut self.other_buttons, code, state);
        }
    }

    /// Releases all currently held keyboard keys and pointer buttons, latching
    /// `released` transitions for the next tick sample (`FND-INV-11`).
    pub(crate) fn release_held(&mut self) {
        self.release_keyboard();
        self.release_mouse_buttons();
        self.cursor_anchor_valid = false;
    }

    pub(crate) fn disconnect_device(&mut self, device: DeviceKind) {
        match device {
            DeviceKind::Keyboard => self.release_keyboard(),
            DeviceKind::Mouse => {
                self.release_mouse_buttons();
                self.pointer_window = None;
                self.cursor_position = None;
                self.cursor_anchor_valid = false;
            }
        }
    }

    fn release_keyboard(&mut self) {
        self.key_pending_released.bitor_assign(self.key_down);
        self.key_down = BitSet128::default();
        for slot in &mut self.native_keys {
            if slot.active {
                slot.release_if_down();
            }
        }
    }

    fn release_mouse_buttons(&mut self) {
        self.mouse_pending_released |= self.mouse_down;
        self.mouse_down = 0;
        for slot in &mut self.other_buttons {
            if slot.active {
                slot.release_if_down();
            }
        }
    }

    /// Latches all pending transitions and deltas into the sampled tick state
    /// and clears the pending accumulators.
    pub(crate) fn sample_tick(&mut self) {
        self.key_sampled_down = self.key_down;
        self.key_sampled_pressed = mem::take(&mut self.key_pending_pressed);
        self.key_sampled_released = mem::take(&mut self.key_pending_released);
        for slot in &mut self.native_keys {
            if slot.active {
                slot.sample_tick();
            }
        }

        self.mouse_sampled_down = self.mouse_down;
        self.mouse_sampled_pressed = mem::take(&mut self.mouse_pending_pressed);
        self.mouse_sampled_released = mem::take(&mut self.mouse_pending_released);
        for slot in &mut self.other_buttons {
            if slot.active {
                slot.sample_tick();
            }
        }

        self.sampled_cursor_delta = mem::take(&mut self.pending_cursor_delta);
        self.sampled_relative_delta = mem::take(&mut self.pending_relative_delta);
        self.sampled_has_relative = mem::take(&mut self.pending_has_relative);
        self.sampled_scroll_lines = mem::take(&mut self.pending_scroll_lines);
        self.sampled_scroll_pixels = mem::take(&mut self.pending_scroll_pixels);
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }
}

fn update_code_slot(slots: &mut [CodeSlot], code: u32, state: ElementState) {
    if let Some(existing) = slots
        .iter_mut()
        .find(|slot| slot.active && slot.code == code)
    {
        existing.apply(state);
        return;
    }
    if state != ElementState::Pressed {
        return;
    }
    if let Some(free) = slots.iter_mut().find(|slot| !slot.active) {
        *free = CodeSlot {
            active: true,
            code,
            down: true,
            pending_pressed: true,
            pending_released: false,
            sampled_down: false,
            sampled_pressed: false,
            sampled_released: false,
        };
    } else if let Some(reclaimable) = slots
        .iter_mut()
        .find(|slot| !slot.down && !slot.pending_pressed && !slot.pending_released)
    {
        *reclaimable = CodeSlot {
            active: true,
            code,
            down: true,
            pending_pressed: true,
            pending_released: false,
            sampled_down: false,
            sampled_pressed: false,
            sampled_released: false,
        };
    }
}

const fn canonical_zero_f64(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn sanitize_f64(value: f64) -> Option<f64> {
    value.is_finite().then_some(canonical_zero_f64(value))
}

fn clamp_finite_f64(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        canonical_zero_f64(value.clamp(f64::MIN, f64::MAX))
    }
}

fn saturating_add_f64(lhs: f64, rhs: f64) -> f64 {
    clamp_finite_f64(lhs + rhs)
}

fn mouse_button_mask(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(1 << 0),
        MouseButton::Right => Some(1 << 1),
        MouseButton::Middle => Some(1 << 2),
        MouseButton::Back => Some(1 << 3),
        MouseButton::Forward => Some(1 << 4),
        MouseButton::Other(_) | _ => None,
    }
}

pub(crate) fn key_index(key: KeyCode) -> Option<usize> {
    let index = match key {
        KeyCode::Unidentified => return None,
        KeyCode::Escape => 0,
        KeyCode::F1 => 1,
        KeyCode::F2 => 2,
        KeyCode::F3 => 3,
        KeyCode::F4 => 4,
        KeyCode::F5 => 5,
        KeyCode::F6 => 6,
        KeyCode::F7 => 7,
        KeyCode::F8 => 8,
        KeyCode::F9 => 9,
        KeyCode::F10 => 10,
        KeyCode::F11 => 11,
        KeyCode::F12 => 12,
        KeyCode::F13 => 13,
        KeyCode::F14 => 14,
        KeyCode::F15 => 15,
        KeyCode::F16 => 16,
        KeyCode::F17 => 17,
        KeyCode::F18 => 18,
        KeyCode::F19 => 19,
        KeyCode::F20 => 20,
        KeyCode::F21 => 21,
        KeyCode::F22 => 22,
        KeyCode::F23 => 23,
        KeyCode::F24 => 24,
        KeyCode::PrintScreen => 25,
        KeyCode::ScrollLock => 26,
        KeyCode::Pause => 27,
        KeyCode::Backquote => 28,
        KeyCode::Digit1 => 29,
        KeyCode::Digit2 => 30,
        KeyCode::Digit3 => 31,
        KeyCode::Digit4 => 32,
        KeyCode::Digit5 => 33,
        KeyCode::Digit6 => 34,
        KeyCode::Digit7 => 35,
        KeyCode::Digit8 => 36,
        KeyCode::Digit9 => 37,
        KeyCode::Digit0 => 38,
        KeyCode::Minus => 39,
        KeyCode::Equal => 40,
        KeyCode::Backspace => 41,
        KeyCode::Tab => 42,
        KeyCode::Q => 43,
        KeyCode::W => 44,
        KeyCode::E => 45,
        KeyCode::R => 46,
        KeyCode::T => 47,
        KeyCode::Y => 48,
        KeyCode::U => 49,
        KeyCode::I => 50,
        KeyCode::O => 51,
        KeyCode::P => 52,
        KeyCode::BracketLeft => 53,
        KeyCode::BracketRight => 54,
        KeyCode::Backslash => 55,
        KeyCode::CapsLock => 56,
        KeyCode::A => 57,
        KeyCode::S => 58,
        KeyCode::D => 59,
        KeyCode::F => 60,
        KeyCode::G => 61,
        KeyCode::H => 62,
        KeyCode::J => 63,
        KeyCode::K => 64,
        KeyCode::L => 65,
        KeyCode::Semicolon => 66,
        KeyCode::Quote => 67,
        KeyCode::Enter => 68,
        KeyCode::ShiftLeft => 69,
        KeyCode::IntlBackslash => 70,
        KeyCode::Z => 71,
        KeyCode::X => 72,
        KeyCode::C => 73,
        KeyCode::V => 74,
        KeyCode::B => 75,
        KeyCode::N => 76,
        KeyCode::M => 77,
        KeyCode::Comma => 78,
        KeyCode::Period => 79,
        KeyCode::Slash => 80,
        KeyCode::ShiftRight => 81,
        KeyCode::ControlLeft => 82,
        KeyCode::SuperLeft => 83,
        KeyCode::AltLeft => 84,
        KeyCode::Space => 85,
        KeyCode::AltRight => 86,
        KeyCode::SuperRight => 87,
        KeyCode::ContextMenu => 88,
        KeyCode::ControlRight => 89,
        KeyCode::Insert => 90,
        KeyCode::Delete => 91,
        KeyCode::Home => 92,
        KeyCode::End => 93,
        KeyCode::PageUp => 94,
        KeyCode::PageDown => 95,
        KeyCode::ArrowUp => 96,
        KeyCode::ArrowDown => 97,
        KeyCode::ArrowLeft => 98,
        KeyCode::ArrowRight => 99,
        KeyCode::NumLock => 100,
        KeyCode::NumpadDivide => 101,
        KeyCode::NumpadMultiply => 102,
        KeyCode::NumpadSubtract => 103,
        KeyCode::NumpadAdd => 104,
        KeyCode::NumpadEnter => 105,
        KeyCode::NumpadDecimal => 106,
        KeyCode::Numpad0 => 107,
        KeyCode::Numpad1 => 108,
        KeyCode::Numpad2 => 109,
        KeyCode::Numpad3 => 110,
        KeyCode::Numpad4 => 111,
        KeyCode::Numpad5 => 112,
        KeyCode::Numpad6 => 113,
        KeyCode::Numpad7 => 114,
        KeyCode::Numpad8 => 115,
        KeyCode::Numpad9 => 116,
        _ => return None,
    };
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const ALL_NAMED_KEYS: [KeyCode; 117] = [
        KeyCode::Escape,
        KeyCode::F1,
        KeyCode::F2,
        KeyCode::F3,
        KeyCode::F4,
        KeyCode::F5,
        KeyCode::F6,
        KeyCode::F7,
        KeyCode::F8,
        KeyCode::F9,
        KeyCode::F10,
        KeyCode::F11,
        KeyCode::F12,
        KeyCode::F13,
        KeyCode::F14,
        KeyCode::F15,
        KeyCode::F16,
        KeyCode::F17,
        KeyCode::F18,
        KeyCode::F19,
        KeyCode::F20,
        KeyCode::F21,
        KeyCode::F22,
        KeyCode::F23,
        KeyCode::F24,
        KeyCode::PrintScreen,
        KeyCode::ScrollLock,
        KeyCode::Pause,
        KeyCode::Backquote,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Digit0,
        KeyCode::Minus,
        KeyCode::Equal,
        KeyCode::Backspace,
        KeyCode::Tab,
        KeyCode::Q,
        KeyCode::W,
        KeyCode::E,
        KeyCode::R,
        KeyCode::T,
        KeyCode::Y,
        KeyCode::U,
        KeyCode::I,
        KeyCode::O,
        KeyCode::P,
        KeyCode::BracketLeft,
        KeyCode::BracketRight,
        KeyCode::Backslash,
        KeyCode::CapsLock,
        KeyCode::A,
        KeyCode::S,
        KeyCode::D,
        KeyCode::F,
        KeyCode::G,
        KeyCode::H,
        KeyCode::J,
        KeyCode::K,
        KeyCode::L,
        KeyCode::Semicolon,
        KeyCode::Quote,
        KeyCode::Enter,
        KeyCode::ShiftLeft,
        KeyCode::IntlBackslash,
        KeyCode::Z,
        KeyCode::X,
        KeyCode::C,
        KeyCode::V,
        KeyCode::B,
        KeyCode::N,
        KeyCode::M,
        KeyCode::Comma,
        KeyCode::Period,
        KeyCode::Slash,
        KeyCode::ShiftRight,
        KeyCode::ControlLeft,
        KeyCode::SuperLeft,
        KeyCode::AltLeft,
        KeyCode::Space,
        KeyCode::AltRight,
        KeyCode::SuperRight,
        KeyCode::ContextMenu,
        KeyCode::ControlRight,
        KeyCode::Insert,
        KeyCode::Delete,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::NumLock,
        KeyCode::NumpadDivide,
        KeyCode::NumpadMultiply,
        KeyCode::NumpadSubtract,
        KeyCode::NumpadAdd,
        KeyCode::NumpadEnter,
        KeyCode::NumpadDecimal,
        KeyCode::Numpad0,
        KeyCode::Numpad1,
        KeyCode::Numpad2,
        KeyCode::Numpad3,
        KeyCode::Numpad4,
        KeyCode::Numpad5,
        KeyCode::Numpad6,
        KeyCode::Numpad7,
        KeyCode::Numpad8,
        KeyCode::Numpad9,
    ];

    #[test]
    fn every_named_keycode_maps_to_unique_in_bounds_bit() {
        assert_eq!(key_index(KeyCode::Unidentified), None);
        let mut seen = HashSet::new();
        for key in ALL_NAMED_KEYS {
            let idx = key_index(key).expect("named key must have index");
            assert!(idx < KEY_WORDS * 64);
            assert!(seen.insert(idx), "duplicate key index {idx} for {key:?}");
        }
        assert_eq!(seen.len(), ALL_NAMED_KEYS.len());
    }

    #[test]
    fn pointer_motion_ignores_nan_and_avoids_double_counting_relative_motion() {
        let mut state = DeviceState::new();
        state.handle_window_event(&WindowEvent::PointerEntered(PhysicalPosition::new(
            10.0, 20.0,
        )));
        state.handle_window_event(&WindowEvent::PointerMoved(PhysicalPosition::new(
            15.0, 18.0,
        )));
        state.handle_window_event(&WindowEvent::PointerMoved(PhysicalPosition::new(
            f64::NAN,
            100.0,
        )));
        state.sample_tick();

        assert_eq!(
            state.cursor_position(),
            Some(PhysicalPosition::new(15.0, 18.0))
        );
        assert_eq!(state.cursor_delta(), PointerDelta::new(5.0, -2.0));
        assert_eq!(state.pointer_delta(), PointerDelta::new(5.0, -2.0));

        // When both surface cursor motion and unaccelerated relative motion
        // arrive in the same tick, `pointer_delta()` prefers relative motion
        // rather than summing both.
        state.handle_window_event(&WindowEvent::PointerMoved(PhysicalPosition::new(
            25.0, 28.0,
        )));
        state.handle_relative_motion(7.5, -3.25);
        state.sample_tick();

        assert_eq!(state.cursor_delta(), PointerDelta::new(10.0, 10.0));
        assert_eq!(state.relative_delta(), PointerDelta::new(7.5, -3.25));
        assert_eq!(state.pointer_delta(), PointerDelta::new(7.5, -3.25));
    }

    #[test]
    fn native_key_and_other_mouse_button_slots_reclaim_after_release() {
        let mut state = DeviceState::new();
        for code in 1..=64_u32 {
            state.handle_window_event(&WindowEvent::Key(KeyEvent {
                key: KeyCode::Unidentified,
                native_code: code,
                state: ElementState::Pressed,
            }));
            state.handle_window_event(&WindowEvent::Key(KeyEvent {
                key: KeyCode::Unidentified,
                native_code: code,
                state: ElementState::Released,
            }));
            state.sample_tick();
            assert_eq!(
                state.native_key_state(code),
                ButtonState {
                    pressed: true,
                    held: false,
                    released: true,
                }
            );
            state.sample_tick();
            assert_eq!(state.native_key_state(code), ButtonState::UP);
        }

        state.handle_window_event(&WindowEvent::PointerButton {
            button: MouseButton::Other(9),
            state: ElementState::Pressed,
        });
        state.sample_tick();
        assert_eq!(
            state.mouse_button_state(MouseButton::Other(9)),
            ButtonState {
                pressed: true,
                held: true,
                released: false,
            }
        );
    }
}
