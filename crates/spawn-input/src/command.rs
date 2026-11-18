//! Tick-indexed semantic input commands and untrusted-command validation.
//!
//! [`InputCommand`] is small (96 bytes), `Copy`, and free of heap allocations,
//! pointers, or raw OS device codes, making it directly suitable for
//! fixed-timestep simulation, client-side prediction history buffers, network
//! serialization, and deterministic input replay.

#![deny(clippy::indexing_slicing)]

use spawn_core::Tick;

use crate::action::{ActionId, AxisId, MAX_AXES};
use crate::device::ButtonState;
use crate::error::{InputError, InputErrorKind};

/// Tick-indexed semantic intent snapshot consumed by fixed-timestep simulation.
///
/// Commands express caller-defined intents (`ActionId`, `AxisId`), never
/// authoritative gameplay outcomes (`SPN-INV-060`). Commands received from
/// untrusted sources (network packets, external replay streams) must be checked
/// with [`Self::validate`] or [`crate::InputMap::validate_command`] before use
/// (`SPN-INV-034`, `SPN-INV-061`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputCommand {
    tick: Tick,
    pressed: u64,
    held: u64,
    released: u64,
    axes: [f32; MAX_AXES],
}

impl InputCommand {
    /// Creates an idle command at `tick` with all actions up and all axes zero.
    pub const fn empty(tick: Tick) -> Self {
        Self {
            tick,
            pressed: 0,
            held: 0,
            released: 0,
            axes: [0.0; MAX_AXES],
        }
    }

    /// Reconstructs a command from decoded wire or replay fields.
    ///
    /// Call [`Self::validate`] (or [`crate::InputMap::validate_command`]) before
    /// feeding an untrusted command into simulation.
    pub const fn from_raw_parts(
        tick: Tick,
        pressed: u64,
        held: u64,
        released: u64,
        axes: [f32; MAX_AXES],
    ) -> Self {
        Self {
            tick,
            pressed,
            held,
            released,
            axes,
        }
    }

    /// Simulation tick index this command targets.
    pub const fn tick(&self) -> Tick {
        self.tick
    }

    /// Sampled press/hold/release state of `action` at this tick.
    pub const fn action_state(&self, action: ActionId) -> ButtonState {
        let bit = action.bit();
        ButtonState {
            pressed: (self.pressed & bit) != 0,
            held: (self.held & bit) != 0,
            released: (self.released & bit) != 0,
        }
    }

    /// Whether `action` transitioned upward-to-downward during this tick's
    /// sampling window.
    pub const fn is_pressed(&self, action: ActionId) -> bool {
        (self.pressed & action.bit()) != 0
    }

    /// Whether `action` is held down at this tick boundary.
    pub const fn is_held(&self, action: ActionId) -> bool {
        (self.held & action.bit()) != 0
    }

    /// Whether `action` transitioned downward-to-upward during this tick's
    /// sampling window.
    pub const fn is_released(&self, action: ActionId) -> bool {
        (self.released & action.bit()) != 0
    }

    /// Whether `action` was active during this tick (`is_pressed || is_held`),
    /// including rapid sub-tick taps that already released before the tick ran.
    pub const fn was_active(&self, action: ActionId) -> bool {
        ((self.pressed | self.held) & action.bit()) != 0
    }

    /// Sampled value of `axis` at this tick.
    pub fn axis(&self, axis: AxisId) -> f32 {
        self.axes
            .get(usize::from(axis.index()))
            .copied()
            .unwrap_or(0.0)
    }

    /// Raw bitmask of actions pressed during this tick's window.
    pub const fn pressed_mask(&self) -> u64 {
        self.pressed
    }

    /// Raw bitmask of actions held at this tick boundary.
    pub const fn held_mask(&self) -> u64 {
        self.held
    }

    /// Raw bitmask of actions released during this tick's window.
    pub const fn released_mask(&self) -> u64 {
        self.released
    }

    /// Raw array of all 16 axis slots.
    pub const fn axes(&self) -> &[f32; MAX_AXES] {
        &self.axes
    }

    /// Validates this command against `rules`.
    ///
    /// Rejects:
    /// - any action bit set outside `rules`'s allowed action mask;
    /// - impossible per-action transition combinations (`pressed && !held && !released`
    ///   or `!pressed && held && released`);
    /// - `NaN` or `±inf` in any axis slot;
    /// - any non-zero value in an axis slot outside `rules`'s allowed axis mask;
    /// - any axis value outside its configured `[min, max]` range or exceeding
    ///   `max_unclamped_magnitude`.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidCommand`] (`INPT-0006`) on the first
    /// violated rule.
    #[track_caller]
    pub fn validate(&self, rules: &CommandValidation) -> Result<(), InputError> {
        let active_bits = self.pressed | self.held | self.released;
        let disallowed_actions = active_bits & !rules.allowed_actions;
        if disallowed_actions != 0 {
            let first = disallowed_actions.trailing_zeros();
            return Err(
                InputError::new(InputErrorKind::InvalidCommand).with_detail(format!(
                    "unregistered action bit {first} is set (mask 0x{disallowed_actions:016x})"
                )),
            );
        }

        // An action pressed during the window cannot end unheld unless it was
        // also released; an action released during the window cannot end held
        // unless it was also pressed.
        let impossible = (self.pressed & !self.held & !self.released)
            | (!self.pressed & self.held & self.released);
        if impossible != 0 {
            let first = impossible.trailing_zeros();
            return Err(
                InputError::new(InputErrorKind::InvalidCommand).with_detail(format!(
                    "action {first} has inconsistent transition flags (pressed={}, held={}, released={})",
                    (self.pressed >> first) & 1,
                    (self.held >> first) & 1,
                    (self.released >> first) & 1,
                )),
            );
        }

        for (idx, &value) in self.axes.iter().enumerate() {
            if !value.is_finite() {
                return Err(InputError::new(InputErrorKind::InvalidCommand)
                    .with_detail(format!("axis {idx} is non-finite ({value})")));
            }
            let allowed = (rules.allowed_axes & (1_u16 << idx)) != 0;
            if !allowed {
                if value != 0.0 {
                    return Err(InputError::new(InputErrorKind::InvalidCommand).with_detail(
                        format!("unregistered axis {idx} has non-zero value {value}"),
                    ));
                }
                continue;
            }
            if let Some(Some((min, max))) = rules.axis_ranges.get(idx).copied() {
                if value < min || value > max {
                    return Err(InputError::new(InputErrorKind::InvalidCommand).with_detail(
                        format!("axis {idx} value {value} is outside allowed range [{min}, {max}]"),
                    ));
                }
            } else if value.abs() > rules.max_unclamped_magnitude {
                return Err(
                    InputError::new(InputErrorKind::InvalidCommand).with_detail(format!(
                        "axis {idx} magnitude {} exceeds maximum {}",
                        value.abs(),
                        rules.max_unclamped_magnitude
                    )),
                );
            }
        }

        Ok(())
    }
}

/// Validation schema for untrusted [`InputCommand`] values (for example on an
/// authoritative server or replay loader).
///
/// Can be constructed directly without any platform keybindings via
/// [`Self::strict_empty`] / [`Self::permissive`], or derived from a client's
/// [`crate::InputMap::validation_rules`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandValidation {
    allowed_actions: u64,
    allowed_axes: u16,
    axis_ranges: [Option<(f32, f32)>; MAX_AXES],
    max_unclamped_magnitude: f32,
}

impl Default for CommandValidation {
    fn default() -> Self {
        Self::permissive()
    }
}

impl CommandValidation {
    /// Creates a schema allowing all 64 action slots and all 16 finite axis
    /// slots while still rejecting `NaN`, `±inf`, and impossible transition
    /// flag combinations.
    pub const fn permissive() -> Self {
        Self {
            allowed_actions: u64::MAX,
            allowed_axes: u16::MAX,
            axis_ranges: [None; MAX_AXES],
            max_unclamped_magnitude: f32::MAX,
        }
    }

    /// Creates an empty allowlist schema that rejects every non-zero action bit
    /// and non-zero axis until explicitly permitted.
    pub const fn strict_empty() -> Self {
        Self {
            allowed_actions: 0,
            allowed_axes: 0,
            axis_ranges: [None; MAX_AXES],
            max_unclamped_magnitude: f32::MAX,
        }
    }

    pub(crate) const fn from_schema(
        allowed_actions: u64,
        allowed_axes: u16,
        axis_ranges: [Option<(f32, f32)>; MAX_AXES],
    ) -> Self {
        Self {
            allowed_actions,
            allowed_axes,
            axis_ranges,
            max_unclamped_magnitude: f32::MAX,
        }
    }

    /// Permits `action` in validated commands.
    pub const fn allow_action(mut self, action: ActionId) -> Self {
        self.allowed_actions |= action.bit();
        self
    }

    /// Permits `axis` (unclamped up to [`Self::with_max_unclamped_magnitude`])
    /// in validated commands.
    pub const fn allow_axis(mut self, axis: AxisId) -> Self {
        self.allowed_axes |= axis.bit();
        self
    }

    /// Permits `axis` and requires its value to lie within `[min, max]`, which
    /// must include neutral `0.0` (`min <= 0.0 <= max`).
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidAxisRange`] (`INPT-0003`) if `min` or
    /// `max` is non-finite, `min > 0.0`, or `max < 0.0`.
    #[track_caller]
    pub fn allow_axis_with_range(
        mut self,
        axis: AxisId,
        min: f32,
        max: f32,
    ) -> Result<Self, InputError> {
        if !min.is_finite() || !max.is_finite() || min > 0.0 || max < 0.0 {
            return Err(InputError::new(InputErrorKind::InvalidAxisRange)
                .with_detail(format!("range = [{min}, {max}]")));
        }
        let clean_min = if min == 0.0 { 0.0 } else { min };
        let clean_max = if max == 0.0 { 0.0 } else { max };
        self.allowed_axes |= axis.bit();
        if let Some(slot) = self.axis_ranges.get_mut(usize::from(axis.index())) {
            *slot = Some((clean_min, clean_max));
        }
        Ok(self)
    }

    /// Caps the absolute magnitude permitted on unclamped axes (for example
    /// bounding per-tick look deltas received from network clients).
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidAxisRange`] (`INPT-0003`) if `limit` is
    /// non-finite or `<= 0.0`.
    #[track_caller]
    pub fn with_max_unclamped_magnitude(mut self, limit: f32) -> Result<Self, InputError> {
        if !limit.is_finite() || limit <= 0.0 {
            return Err(InputError::new(InputErrorKind::InvalidAxisRange)
                .with_detail(format!("max_unclamped_magnitude = {limit}")));
        }
        self.max_unclamped_magnitude = limit;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_copy_and_compact() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<InputCommand>();
        assert_eq!(std::mem::size_of::<InputCommand>(), 96);
    }

    #[test]
    fn untrusted_command_validation_catches_nan_inf_ranges_and_impossible_bits()
    -> Result<(), InputError> {
        let act0 = ActionId::new(0)?;
        let ax0 = AxisId::new(0)?;
        let ax1 = AxisId::new(1)?;
        let ax3 = AxisId::new(3)?;
        let ax4 = AxisId::new(4)?;

        // Zero-excluding ranges are rejected on `CommandValidation` too.
        for (min, max) in [(0.5, 1.0), (-1.0, -0.5), (1.0, -1.0), (f32::NAN, 1.0)] {
            let err = CommandValidation::strict_empty()
                .allow_axis_with_range(ax0, min, max)
                .unwrap_err();
            assert_eq!(err.kind(), InputErrorKind::InvalidAxisRange);
            assert_eq!(err.code(), "INPT-0003");
        }

        let rules = CommandValidation::strict_empty()
            .allow_action(act0)
            .allow_axis_with_range(ax0, -1.0, 1.0)?
            .allow_axis(ax1)
            .allow_axis_with_range(ax3, 0.0, 1.0)?
            .allow_axis_with_range(ax4, -1.0, 0.0)?
            .with_max_unclamped_magnitude(500.0)?;

        // Valid empty and valid active commands pass.
        InputCommand::empty(Tick::ZERO).validate(&rules)?;
        let mut axes = [0.0_f32; MAX_AXES];
        axes[0] = -0.75;
        axes[1] = 250.0;
        let valid = InputCommand::from_raw_parts(Tick::new(5), 1, 1, 0, axes);
        valid.validate(&rules)?;

        // Unregistered action bit -> INPT-0006.
        let bad_action = InputCommand::from_raw_parts(Tick::ZERO, 0b10, 0b10, 0, [0.0; MAX_AXES]);
        assert_eq!(bad_action.validate(&rules).unwrap_err().code(), "INPT-0006");

        // Impossible transitions: pressed=1, held=0, released=0 -> INPT-0006.
        let impossible_press = InputCommand::from_raw_parts(Tick::ZERO, 1, 0, 0, [0.0; MAX_AXES]);
        assert_eq!(
            impossible_press.validate(&rules).unwrap_err().kind(),
            InputErrorKind::InvalidCommand
        );

        // Impossible transitions: pressed=0, held=1, released=1 -> INPT-0006.
        let impossible_release = InputCommand::from_raw_parts(Tick::ZERO, 0, 1, 1, [0.0; MAX_AXES]);
        assert_eq!(
            impossible_release.validate(&rules).unwrap_err().kind(),
            InputErrorKind::InvalidCommand
        );

        // Rapid tap (pressed=1, held=0, released=1) and release+repress
        // (pressed=1, held=1, released=1) are valid.
        InputCommand::from_raw_parts(Tick::ZERO, 1, 0, 1, [0.0; MAX_AXES]).validate(&rules)?;
        InputCommand::from_raw_parts(Tick::ZERO, 1, 1, 1, [0.0; MAX_AXES]).validate(&rules)?;

        // NaN, +inf, -inf in axis -> INPT-0006.
        for non_finite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bad_axes = [0.0_f32; MAX_AXES];
            bad_axes[0] = non_finite;
            let cmd = InputCommand::from_raw_parts(Tick::ZERO, 0, 0, 0, bad_axes);
            assert_eq!(
                cmd.validate(&rules).unwrap_err().kind(),
                InputErrorKind::InvalidCommand
            );
        }

        // Clamped axis out of range -> INPT-0006.
        let mut out_of_range = [0.0_f32; MAX_AXES];
        out_of_range[0] = 1.01;
        assert_eq!(
            InputCommand::from_raw_parts(Tick::ZERO, 0, 0, 0, out_of_range)
                .validate(&rules)
                .unwrap_err()
                .kind(),
            InputErrorKind::InvalidCommand
        );

        // Unclamped axis exceeding max_unclamped_magnitude -> INPT-0006.
        let mut huge_delta = [0.0_f32; MAX_AXES];
        huge_delta[1] = -500.1;
        assert_eq!(
            InputCommand::from_raw_parts(Tick::ZERO, 0, 0, 0, huge_delta)
                .validate(&rules)
                .unwrap_err()
                .kind(),
            InputErrorKind::InvalidCommand
        );

        // Non-zero value in unregistered axis slot -> INPT-0006.
        let mut unreg_axis = [0.0_f32; MAX_AXES];
        unreg_axis[2] = 0.25;
        assert_eq!(
            InputCommand::from_raw_parts(Tick::ZERO, 0, 0, 0, unreg_axis)
                .validate(&rules)
                .unwrap_err()
                .kind(),
            InputErrorKind::InvalidCommand
        );

        Ok(())
    }
}
