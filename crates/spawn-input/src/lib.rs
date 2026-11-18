//! `spawn-input`: device state, transitions, semantic mappings, and
//! tick-boundary input sampling.
//!
//! Consumes raw window and device events owned by `spawn-platform` (without any
//! native windowing backend feature, ADR-0002) and produces tick-indexed,
//! `Copy`, replay- and serialization-friendly [`InputCommand`] intents for
//! fixed-timestep simulation (ADR-0003).
//!
//! # Architectural Boundaries
//!
//! - **Raw vs. semantic separation (`FND-INV-02`):** [`DeviceState`] tracks
//!   physical keys, mouse buttons, cursor position, pointer motion deltas, and
//!   scroll deltas; [`InputMap`] maps raw [`ButtonTrigger`]s and
//!   [`AxisSource`]s onto caller-defined [`ActionId`], [`AxisId`], and
//!   [`ContextId`] identifiers.
//! - **Genre-free infrastructure (`SPN-INV-003`, `FND-INV-13`):** no gameplay or
//!   genre actions exist in this crate; games and frameworks define their own
//!   identifiers and bindings as data.
//! - **Tick-boundary sampling (`FND-FR-09`, `FND-INV-10`):** press/release
//!   transitions and motion/scroll deltas accumulate across zero-tick frames
//!   and are consumed on the first tick of a wakeup without duplicating edges
//!   or deltas across multi-tick catch-up steps.
//! - **Focus loss and disconnect reset (`FND-INV-11`):** losing focus,
//!   requesting window close, disconnecting a device, or disabling a mapping
//!   context predictably releases affected held inputs and active actions.
//!
//! ```
//! use spawn_core::Tick;
//! use spawn_input::{
//!     ActionId, AxisConfig, AxisId, ButtonTrigger, InputMap, InputState,
//! };
//! use spawn_platform::{ElementState, KeyCode, KeyEvent, WindowEvent};
//!
//! # fn main() -> Result<(), spawn_input::InputError> {
//! let confirm = ActionId::new(0)?;
//! let horizontal = AxisId::new(0)?;
//!
//! let mut map = InputMap::new();
//! map.bind_action(confirm, ButtonTrigger::Key(KeyCode::Enter))?;
//! map.bind_axis_digital_pair(
//!     horizontal,
//!     ButtonTrigger::Key(KeyCode::A),
//!     ButtonTrigger::Key(KeyCode::D),
//! )?;
//! map.set_axis_config(horizontal, AxisConfig::unit());
//!
//! let mut input = InputState::with_map(map);
//! input.handle_window_event(&WindowEvent::Key(KeyEvent {
//!     key: KeyCode::Enter,
//!     native_code: 28,
//!     state: ElementState::Pressed,
//! }));
//! input.handle_window_event(&WindowEvent::Key(KeyEvent {
//!     key: KeyCode::D,
//!     native_code: 32,
//!     state: ElementState::Pressed,
//! }));
//!
//! let tick0 = input.sample_tick(Tick::ZERO);
//! assert!(tick0.is_pressed(confirm));
//! assert!(tick0.is_held(confirm));
//! assert_eq!(tick0.axis(horizontal), 1.0);
//!
//! // Catch-up tick with no new events: still held, press edge not duplicated.
//! let tick1 = input.sample_tick(Tick::new(1));
//! assert!(!tick1.is_pressed(confirm));
//! assert!(tick1.is_held(confirm));
//! assert_eq!(tick1.axis(horizontal), 1.0);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod action;
mod command;
mod device;
mod error;
mod state;

pub use action::{
    ActionId, AxisConfig, AxisId, AxisSource, ButtonTrigger, ContextId, InputMap,
    MAX_ACTION_BINDINGS, MAX_ACTIONS, MAX_AXES, MAX_AXIS_BINDINGS, MAX_CONTEXTS, MotionAxis,
};
pub use command::{CommandValidation, InputCommand};
pub use device::{ButtonState, DeviceKind, DeviceState, PointerDelta, ScrollVector};
pub use error::{InputError, InputErrorKind};
pub use state::InputState;
