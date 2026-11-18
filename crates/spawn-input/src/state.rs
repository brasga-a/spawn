//! Stateful input coordinator combining raw [`DeviceState`], [`InputMap`],
//! active [`ContextId`]s, and tick-boundary [`InputCommand`] sampling.

#![deny(clippy::indexing_slicing)]

use std::mem;

use spawn_core::Tick;
use spawn_platform::{Event, WindowEvent};

use crate::action::{ContextId, InputMap};
use crate::command::InputCommand;
use crate::device::{DeviceKind, DeviceState};

/// Coordinates raw platform event ingestion, semantic action transitions, and
/// fixed-timestep input sampling.
///
/// # Tick-Boundary Sampling Contract (`FND-FR-09`, `FND-INV-10`)
///
/// 1. Raw events fed via [`Self::handle_event`], [`Self::handle_window_event`],
///    or [`Self::handle_relative_motion`] update physical down states
///    immediately and latch `pressed` / `released` transitions plus motion and
///    scroll deltas into pending accumulators.
/// 2. Render frames that execute **zero** simulation ticks do not call
///    [`Self::sample_tick`]; pending edges and deltas remain latched until the
///    next tick runs, so sub-tick or high-FPS presses are never lost.
/// 3. Each simulation tick calls [`Self::sample_tick`] exactly once. The first
///    tick after an event batch consumes all pending `pressed` / `released`
///    edges and transient motion/scroll deltas; subsequent catch-up ticks in
///    the same wakeup observe `held == true` (and continuous held-button axes)
///    while `pressed`, `released`, and transient deltas are `false` / `0.0`,
///    never duplicating edges or camera motion.
/// 4. Rapid sub-tick press+release sequences before a tick runs latch both
///    `pressed == true` and `released == true` on that tick (`held == false`),
///    preserving tap edges (`InputCommand::is_pressed`,
///    `InputCommand::was_active`).
///
/// # Focus Loss and Device Reset (`FND-INV-11`)
///
/// [`WindowEvent::Focused`]`(false)`, [`WindowEvent::CloseRequested`],
/// [`WindowEvent::PointerLeft`] (for pointer buttons), [`Self::release_held`],
/// and [`Self::disconnect_device`] transition all
/// affected held keys, buttons, and active semantic actions to up while
/// latching `released == true` for the next tick, ensuring no input remains
/// stuck. [`Self::reset`] clears all live, pending, and sampled input state
/// immediately while preserving the configured [`InputMap`] and active contexts.
#[derive(Debug, Clone)]
pub struct InputState {
    devices: DeviceState,
    map: InputMap,
    active_contexts: u16,
    action_down: u64,
    pending_action_pressed: u64,
    pending_action_released: u64,
    last_command: Option<InputCommand>,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    /// Creates an input state with an empty [`InputMap`] and
    /// [`ContextId::GLOBAL`] enabled.
    pub fn new() -> Self {
        Self::with_map(InputMap::new())
    }

    /// Creates an input state with `map` and [`ContextId::GLOBAL`] enabled.
    pub fn with_map(map: InputMap) -> Self {
        Self {
            devices: DeviceState::new(),
            map,
            active_contexts: ContextId::GLOBAL.bit(),
            action_down: 0,
            pending_action_pressed: 0,
            pending_action_released: 0,
            last_command: None,
        }
    }

    /// Read-only access to the underlying raw [`DeviceState`].
    pub const fn devices(&self) -> &DeviceState {
        &self.devices
    }

    /// Read-only access to the configured semantic [`InputMap`].
    pub const fn map(&self) -> &InputMap {
        &self.map
    }

    /// Replaces the [`InputMap`], reconciling currently active actions so any
    /// held action still registered in `map` that is no longer satisfied
    /// latches a `released` transition, while actions absent from `map`'s
    /// schema have their active and pending bits cleared.
    pub fn set_map(&mut self, map: InputMap) {
        self.map = map;
        self.reconcile_actions();
    }

    /// Mutates the [`InputMap`] in place (for example rebinding keys at
    /// runtime) and reconciles active action transitions before returning, even
    /// if `f` returns an error.
    pub fn mutate_map<T, E>(
        &mut self,
        f: impl FnOnce(&mut InputMap) -> Result<T, E>,
    ) -> Result<T, E> {
        let result = f(&mut self.map);
        self.reconcile_actions();
        result
    }

    /// Enables or disables an input mapping `context`, reconciling active
    /// actions so disabling a context cleanly releases actions bound only
    /// within it.
    pub fn set_context_active(&mut self, context: ContextId, active: bool) {
        if active {
            self.active_contexts |= context.bit();
        } else {
            self.active_contexts &= !context.bit();
        }
        self.reconcile_actions();
    }

    /// Whether `context` is currently enabled.
    pub const fn is_context_active(&self, context: ContextId) -> bool {
        (self.active_contexts & context.bit()) != 0
    }

    /// Processes one raw platform [`Event`] in delivery order without
    /// allocating on the heap.
    pub fn handle_event(&mut self, event: &Event) {
        self.devices.handle_event(event);
        self.reconcile_actions();
    }

    /// Processes a batch of raw platform [`Event`]s in delivery order.
    pub fn handle_events<'a>(&mut self, events: impl IntoIterator<Item = &'a Event>) {
        for event in events {
            self.handle_event(event);
        }
    }

    /// Processes one raw [`WindowEvent`] directly (convenient for single-window
    /// callers and synthetic headless tests).
    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        self.devices.handle_window_event(event);
        self.reconcile_actions();
    }

    /// Records unaccelerated relative pointer motion (`dx` positive right, `dy`
    /// positive down), matching Phase 2b's
    /// `WindowEvent::PointerRelativeMotion { dx, dy }`. Non-finite components
    /// are ignored without poisoning accumulated deltas.
    pub fn handle_relative_motion(&mut self, dx: f64, dy: f64) {
        self.devices.handle_relative_motion(dx, dy);
    }

    /// Releases all currently held keyboard keys, pointer buttons, and active
    /// semantic actions, latching `released` transitions for the next tick
    /// sample (`FND-INV-11`).
    pub fn release_held(&mut self) {
        self.devices.release_held();
        self.reconcile_actions();
    }

    /// Releases held inputs belonging to `device` and reconciles affected
    /// semantic actions (`FND-INV-11`, `FND-T04`).
    pub fn disconnect_device(&mut self, device: DeviceKind) {
        self.devices.disconnect_device(device);
        self.reconcile_actions();
    }

    /// Immediately clears all live, pending, and sampled device and action
    /// states to idle while preserving the configured [`InputMap`] and active
    /// [`ContextId`] mask.
    pub fn reset(&mut self) {
        self.devices.reset();
        self.action_down = 0;
        self.pending_action_pressed = 0;
        self.pending_action_released = 0;
        self.last_command = None;
    }

    /// Consumes pending transitions and transient motion/scroll deltas to
    /// produce the semantic [`InputCommand`] for simulation `tick`.
    pub fn sample_tick(&mut self, tick: Tick) -> InputCommand {
        self.devices.sample_tick();
        let pressed = mem::take(&mut self.pending_action_pressed);
        let released = mem::take(&mut self.pending_action_released);
        let held = self.action_down;
        let axes = self.map.sample_axes(&self.devices, self.active_contexts);

        let command = InputCommand::from_raw_parts(tick, pressed, held, released, axes);
        self.last_command = Some(command);
        command
    }

    /// Returns the [`InputCommand`] produced by the most recent
    /// [`Self::sample_tick`] call, or `None` before the first tick or after
    /// [`Self::reset`].
    pub const fn last_command(&self) -> Option<InputCommand> {
        self.last_command
    }

    fn reconcile_actions(&mut self) {
        let registered = self.map.registered_actions_mask();
        let now_down = self
            .map
            .active_action_mask(&self.devices, self.active_contexts)
            & registered;
        let newly_pressed = now_down & !self.action_down;
        let newly_released = self.action_down & !now_down;
        self.pending_action_pressed = (self.pending_action_pressed | newly_pressed) & registered;
        self.pending_action_released = (self.pending_action_released | newly_released) & registered;
        self.action_down = now_down;
    }
}
