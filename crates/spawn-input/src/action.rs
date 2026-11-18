//! Semantic action, axis, context, and mapping definitions.
//!
//! `spawn-input` defines no genre-specific actions (`SPN-INV-003`,
//! `FND-INV-13`). Callers allocate [`ActionId`], [`AxisId`], and [`ContextId`]
//! values as data and bind raw [`ButtonTrigger`] or [`AxisSource`] inputs to
//! them inside an [`InputMap`].

#![deny(clippy::indexing_slicing)]

use spawn_platform::{KeyCode, MouseButton};

use crate::command::{CommandValidation, InputCommand};
use crate::device::{DeviceState, key_index};
use crate::error::{InputError, InputErrorKind};

/// Maximum number of distinct digital semantic actions representable in one
/// [`InputCommand`] (`u64` bitmask).
pub const MAX_ACTIONS: usize = 64;

/// Maximum number of distinct 1D semantic axes representable in one
/// [`InputCommand`].
pub const MAX_AXES: usize = 16;

/// Maximum number of input contexts tracked simultaneously (`u16` bitmask).
pub const MAX_CONTEXTS: usize = 16;

/// Maximum number of digital action bindings stored in one [`InputMap`].
pub const MAX_ACTION_BINDINGS: usize = 256;

/// Maximum number of axis bindings stored in one [`InputMap`].
pub const MAX_AXIS_BINDINGS: usize = 128;

/// Caller-defined identifier of a digital semantic action (`0..64`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionId(u8);

impl ActionId {
    /// Creates an action identifier in `0..MAX_ACTIONS`.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::IdOutOfRange`] (`INPT-0005`) if
    /// `usize::from(index) >= MAX_ACTIONS`.
    #[track_caller]
    pub fn new(index: u8) -> Result<Self, InputError> {
        if usize::from(index) >= MAX_ACTIONS {
            return Err(
                InputError::new(InputErrorKind::IdOutOfRange).with_detail(format!(
                    "action index {index} exceeds maximum {}",
                    MAX_ACTIONS - 1
                )),
            );
        }
        Ok(Self(index))
    }

    /// Numeric index of this action (`0..64`).
    pub const fn index(self) -> u8 {
        self.0
    }

    pub(crate) const fn bit(self) -> u64 {
        1_u64 << self.0
    }
}

/// Caller-defined identifier of a 1D semantic axis (`0..16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AxisId(u8);

impl AxisId {
    /// Creates an axis identifier in `0..MAX_AXES`.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::IdOutOfRange`] (`INPT-0005`) if
    /// `usize::from(index) >= MAX_AXES`.
    #[track_caller]
    pub fn new(index: u8) -> Result<Self, InputError> {
        if usize::from(index) >= MAX_AXES {
            return Err(
                InputError::new(InputErrorKind::IdOutOfRange).with_detail(format!(
                    "axis index {index} exceeds maximum {}",
                    MAX_AXES - 1
                )),
            );
        }
        Ok(Self(index))
    }

    /// Numeric index of this axis (`0..16`).
    pub const fn index(self) -> u8 {
        self.0
    }

    pub(crate) const fn bit(self) -> u16 {
        1_u16 << self.0
    }
}

/// Caller-defined identifier of an input mapping context (`0..16`).
///
/// [`ContextId::GLOBAL`] (`0`) is enabled by default on a fresh
/// [`InputState`](crate::InputState). Additional contexts let applications
/// enable or disable groups of bindings (for example menu vs. world controls)
/// cleanly without mutating the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextId(u8);

impl Default for ContextId {
    fn default() -> Self {
        Self::GLOBAL
    }
}

impl ContextId {
    /// Default context (`0`), active on every fresh [`InputState`](crate::InputState).
    pub const GLOBAL: Self = Self(0);

    /// Creates a context identifier in `0..MAX_CONTEXTS`.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::IdOutOfRange`] (`INPT-0005`) if
    /// `usize::from(index) >= MAX_CONTEXTS`.
    #[track_caller]
    pub fn new(index: u8) -> Result<Self, InputError> {
        if usize::from(index) >= MAX_CONTEXTS {
            return Err(
                InputError::new(InputErrorKind::IdOutOfRange).with_detail(format!(
                    "context index {index} exceeds maximum {}",
                    MAX_CONTEXTS - 1
                )),
            );
        }
        Ok(Self(index))
    }

    /// Numeric index of this context (`0..16`).
    pub const fn index(self) -> u8 {
        self.0
    }

    pub(crate) const fn bit(self) -> u16 {
        1_u16 << self.0
    }
}

/// Raw digital button trigger that can drive an [`ActionId`] or contribute to
/// an [`AxisId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ButtonTrigger {
    /// Named layout-independent physical key. [`KeyCode::Unidentified`] is
    /// rejected at bind time; bind unidentified keys via [`Self::NativeKey`].
    Key(KeyCode),
    /// Platform-specific native key scancode (`KeyEvent::native_code`).
    NativeKey(u32),
    /// Pointer / mouse button.
    MouseButton(MouseButton),
}

impl ButtonTrigger {
    #[track_caller]
    pub(crate) fn validate(self) -> Result<(), InputError> {
        match self {
            Self::Key(key) if key_index(key).is_none() => Err(InputError::new(
                InputErrorKind::InvalidBinding,
            )
            .with_detail(format!(
                "key {key:?} cannot be bound by KeyCode; use ButtonTrigger::NativeKey"
            ))),
            Self::Key(_) | Self::NativeKey(_) | Self::MouseButton(_) => Ok(()),
        }
    }

    pub(crate) fn is_down(self, devices: &DeviceState) -> bool {
        match self {
            Self::Key(key) => devices.is_key_down(key),
            Self::NativeKey(code) => devices.is_native_key_down(code),
            Self::MouseButton(button) => devices.is_mouse_button_down(button),
        }
    }
}

/// Component selector for 2D pointer motion or scroll vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotionAxis {
    /// Horizontal component (`dx` / `x`).
    X,
    /// Vertical component (`dy` / `y`).
    Y,
}

/// Raw source feeding a 1D semantic [`AxisId`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum AxisSource {
    /// Digital button contributing `scale` while held down.
    Button {
        /// Digital trigger to poll.
        trigger: ButtonTrigger,
        /// Signed contribution while `trigger` is held; must be finite,
        /// non-zero, and within `±AxisConfig::MAX_SENSITIVITY`.
        scale: f32,
    },
    /// Effective tick pointer motion delta (prefers raw relative motion when
    /// received during the tick, falling back to window cursor displacement).
    PointerMotion(MotionAxis),
    /// Strictly raw unaccelerated relative pointer motion delta.
    PointerRelative(MotionAxis),
    /// Strictly window-surface cursor displacement delta.
    CursorMotion(MotionAxis),
    /// Discrete scroll-wheel notches accumulated during the tick.
    ScrollLines(MotionAxis),
    /// Smooth/pixel scroll delta accumulated during the tick.
    ScrollPixels(MotionAxis),
}

impl AxisSource {
    #[track_caller]
    pub(crate) fn validate(self) -> Result<(), InputError> {
        match self {
            Self::Button { trigger, scale } => {
                trigger.validate()?;
                if !scale.is_finite() || scale == 0.0 || scale.abs() > AxisConfig::MAX_SENSITIVITY {
                    return Err(
                        InputError::new(InputErrorKind::InvalidBinding).with_detail(format!(
                            "axis button scale must be finite, non-zero, and within ±{}, got {scale}",
                            AxisConfig::MAX_SENSITIVITY
                        )),
                    );
                }
                Ok(())
            }
            Self::PointerMotion(_)
            | Self::PointerRelative(_)
            | Self::CursorMotion(_)
            | Self::ScrollLines(_)
            | Self::ScrollPixels(_) => Ok(()),
        }
    }

    pub(crate) fn sample(self, devices: &DeviceState) -> f32 {
        match self {
            Self::Button { trigger, scale } => {
                if trigger.is_down(devices) {
                    scale
                } else {
                    0.0
                }
            }
            Self::PointerMotion(axis) => {
                let delta = devices.pointer_delta();
                select_axis_f64(delta.dx, delta.dy, axis)
            }
            Self::PointerRelative(axis) => {
                let delta = devices.relative_delta();
                select_axis_f64(delta.dx, delta.dy, axis)
            }
            Self::CursorMotion(axis) => {
                let delta = devices.cursor_delta();
                select_axis_f64(delta.dx, delta.dy, axis)
            }
            Self::ScrollLines(axis) => {
                let scroll = devices.scroll_lines();
                select_axis_f64(scroll.x, scroll.y, axis)
            }
            Self::ScrollPixels(axis) => {
                let scroll = devices.scroll_pixels();
                select_axis_f64(scroll.x, scroll.y, axis)
            }
        }
    }
}

/// Response parameters applied to a 1D semantic [`AxisId`]: axial dead zone,
/// sensitivity multiplier, optional sign inversion, and optional output clamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisConfig {
    dead_zone: f32,
    sensitivity: f32,
    inverted: bool,
    range: Option<(f32, f32)>,
}

impl Default for AxisConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl AxisConfig {
    /// Upper bound on [`Self::sensitivity`] and digital axis button scales.
    pub const MAX_SENSITIVITY: f32 = 10_000.0;

    /// Unclamped default configuration (`dead_zone = 0.0`, `sensitivity = 1.0`,
    /// `inverted = false`, `range = None`), suited to delta-based axes such as
    /// pointer motion or scroll.
    pub const fn new() -> Self {
        Self {
            dead_zone: 0.0,
            sensitivity: 1.0,
            inverted: false,
            range: None,
        }
    }

    /// Unit-clamped configuration (`range = Some((-1.0, 1.0))`), suited to
    /// directional movement or stick axes where multiple held bindings must not
    /// exceed unit magnitude.
    pub const fn unit() -> Self {
        Self {
            dead_zone: 0.0,
            sensitivity: 1.0,
            inverted: false,
            range: Some((-1.0, 1.0)),
        }
    }

    /// Sets the axial dead zone in `[0.0, 1.0)`.
    ///
    /// Magnitudes at or below `dead_zone` map to `0.0`; magnitudes above it are
    /// linearly rescaled by `(abs - dead_zone) / (1.0 - dead_zone)` before
    /// applying sensitivity.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidDeadZone`] (`INPT-0001`) if `dead_zone`
    /// is `NaN`, infinite, negative, or `>= 1.0`.
    #[track_caller]
    pub fn with_dead_zone(mut self, dead_zone: f32) -> Result<Self, InputError> {
        if !dead_zone.is_finite() || !(0.0..1.0).contains(&dead_zone) {
            return Err(InputError::new(InputErrorKind::InvalidDeadZone)
                .with_detail(format!("dead_zone = {dead_zone}")));
        }
        self.dead_zone = if dead_zone == 0.0 { 0.0 } else { dead_zone };
        Ok(self)
    }

    /// Sets the positive sensitivity multiplier in `(0.0, Self::MAX_SENSITIVITY]`.
    ///
    /// Use [`Self::with_inverted`] rather than a negative multiplier to flip an
    /// axis direction.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidSensitivity`] (`INPT-0002`) if
    /// `sensitivity` is `NaN`, infinite, `<= 0.0`, or above
    /// [`Self::MAX_SENSITIVITY`].
    #[track_caller]
    pub fn with_sensitivity(mut self, sensitivity: f32) -> Result<Self, InputError> {
        if !sensitivity.is_finite() || sensitivity <= 0.0 || sensitivity > Self::MAX_SENSITIVITY {
            return Err(InputError::new(InputErrorKind::InvalidSensitivity)
                .with_detail(format!("sensitivity = {sensitivity}")));
        }
        self.sensitivity = sensitivity;
        Ok(self)
    }

    /// Flips the output sign when `inverted` is `true`.
    pub const fn with_inverted(mut self, inverted: bool) -> Self {
        self.inverted = inverted;
        self
    }

    /// Clamps the processed axis output to `[min, max]`, which must contain
    /// neutral `0.0` (`min <= 0.0 <= max`).
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidAxisRange`] (`INPT-0003`) if `min` or
    /// `max` is non-finite, `min > 0.0`, or `max < 0.0`.
    #[track_caller]
    pub fn with_range(mut self, min: f32, max: f32) -> Result<Self, InputError> {
        if !min.is_finite() || !max.is_finite() || min > 0.0 || max < 0.0 {
            return Err(InputError::new(InputErrorKind::InvalidAxisRange)
                .with_detail(format!("range = [{min}, {max}]")));
        }
        let clean_min = if min == 0.0 { 0.0 } else { min };
        let clean_max = if max == 0.0 { 0.0 } else { max };
        self.range = Some((clean_min, clean_max));
        Ok(self)
    }

    /// Removes any output clamp from this axis.
    pub const fn without_range(mut self) -> Self {
        self.range = None;
        self
    }

    /// Configured axial dead zone (`[0.0, 1.0)`).
    pub const fn dead_zone(&self) -> f32 {
        self.dead_zone
    }

    /// Configured sensitivity multiplier (`(0.0, Self::MAX_SENSITIVITY]`).
    pub const fn sensitivity(&self) -> f32 {
        self.sensitivity
    }

    /// Whether the axis sign is inverted.
    pub const fn is_inverted(&self) -> bool {
        self.inverted
    }

    /// Optional `[min, max]` output clamp.
    pub const fn range(&self) -> Option<(f32, f32)> {
        self.range
    }

    /// Applies dead-zone rescaling, sign inversion, sensitivity, and clamping
    /// to a raw summed input value. Non-finite inputs evaluate to `0.0`, and
    /// `-0.0` is normalized to `0.0`.
    pub fn apply(&self, raw: f32) -> f32 {
        if !raw.is_finite() {
            return 0.0;
        }
        let magnitude = raw.abs();
        if magnitude <= self.dead_zone {
            return 0.0;
        }
        let rescaled = (magnitude - self.dead_zone) / (1.0 - self.dead_zone);
        let sign = if self.inverted {
            -raw.signum()
        } else {
            raw.signum()
        };
        let scaled = clamp_finite_f32(sign * rescaled * self.sensitivity);
        let bounded = match self.range {
            Some((min, max)) => scaled.clamp(min, max),
            None => scaled,
        };
        canonical_zero_f32(bounded)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActionBinding {
    pub(crate) context: ContextId,
    pub(crate) action: ActionId,
    pub(crate) trigger: ButtonTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AxisBinding {
    pub(crate) context: ContextId,
    pub(crate) axis: AxisId,
    pub(crate) source: AxisSource,
}

/// Caller-defined bindings between raw device inputs and semantic [`ActionId`] /
/// [`AxisId`] intents.
///
/// Heap allocation occurs only when bindings are added during configuration or
/// rebinding, never during per-event processing or per-tick sampling.
#[derive(Debug, Clone)]
pub struct InputMap {
    action_bindings: Vec<ActionBinding>,
    axis_bindings: Vec<AxisBinding>,
    axis_configs: [AxisConfig; MAX_AXES],
    registered_actions: u64,
    registered_axes: u16,
}

impl Default for InputMap {
    fn default() -> Self {
        Self::new()
    }
}

impl InputMap {
    /// Creates an empty input map with no actions or axes bound.
    pub fn new() -> Self {
        Self {
            action_bindings: Vec::new(),
            axis_bindings: Vec::new(),
            axis_configs: [AxisConfig::new(); MAX_AXES],
            registered_actions: 0,
            registered_axes: 0,
        }
    }

    /// Declares `action` as part of this map's schema even when it currently
    /// has no physical bindings, so [`Self::validation_rules`] accepts it.
    pub fn register_action(&mut self, action: ActionId) {
        self.registered_actions |= action.bit();
    }

    /// Declares `axis` as part of this map's schema even when it currently has
    /// no physical bindings, so [`Self::validation_rules`] accepts it.
    pub fn register_axis(&mut self, axis: AxisId) {
        self.registered_axes |= axis.bit();
    }

    /// Binds `trigger` to `action` in [`ContextId::GLOBAL`]. Duplicate bindings
    /// are ignored idempotently.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if `trigger` is
    /// invalid or [`MAX_ACTION_BINDINGS`] would be exceeded.
    #[track_caller]
    pub fn bind_action(
        &mut self,
        action: ActionId,
        trigger: ButtonTrigger,
    ) -> Result<(), InputError> {
        self.bind_action_in(ContextId::GLOBAL, action, trigger)
    }

    /// Binds `trigger` to `action` in `context`. Duplicate bindings are ignored
    /// idempotently.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if `trigger` is
    /// invalid or [`MAX_ACTION_BINDINGS`] would be exceeded.
    #[track_caller]
    pub fn bind_action_in(
        &mut self,
        context: ContextId,
        action: ActionId,
        trigger: ButtonTrigger,
    ) -> Result<(), InputError> {
        trigger.validate()?;
        let candidate = ActionBinding {
            context,
            action,
            trigger,
        };
        if !self.action_bindings.contains(&candidate) {
            if self.action_bindings.len() >= MAX_ACTION_BINDINGS {
                return Err(
                    InputError::new(InputErrorKind::InvalidBinding).with_detail(format!(
                        "action binding limit ({MAX_ACTION_BINDINGS}) reached"
                    )),
                );
            }
            self.action_bindings.push(candidate);
        }
        self.registered_actions |= action.bit();
        Ok(())
    }

    /// Replaces all bindings for `action` in [`ContextId::GLOBAL`] with
    /// `trigger`. If `trigger` fails validation, existing bindings remain
    /// untouched.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if `trigger` is
    /// invalid or [`MAX_ACTION_BINDINGS`] would be exceeded.
    #[track_caller]
    pub fn rebind_action(
        &mut self,
        action: ActionId,
        trigger: ButtonTrigger,
    ) -> Result<(), InputError> {
        self.rebind_action_in(ContextId::GLOBAL, action, trigger)
    }

    /// Replaces all bindings for `action` in `context` with `trigger`. If
    /// `trigger` fails validation, existing bindings remain untouched.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if `trigger` is
    /// invalid or [`MAX_ACTION_BINDINGS`] would be exceeded.
    #[track_caller]
    pub fn rebind_action_in(
        &mut self,
        context: ContextId,
        action: ActionId,
        trigger: ButtonTrigger,
    ) -> Result<(), InputError> {
        trigger.validate()?;
        let removing_any = self
            .action_bindings
            .iter()
            .any(|b| b.context == context && b.action == action);
        if !removing_any && self.action_bindings.len() >= MAX_ACTION_BINDINGS {
            return Err(
                InputError::new(InputErrorKind::InvalidBinding).with_detail(format!(
                    "action binding limit ({MAX_ACTION_BINDINGS}) reached"
                )),
            );
        }
        self.action_bindings
            .retain(|b| !(b.context == context && b.action == action));
        self.action_bindings.push(ActionBinding {
            context,
            action,
            trigger,
        });
        self.registered_actions |= action.bit();
        Ok(())
    }

    /// Removes a specific `trigger` binding from `action` in
    /// [`ContextId::GLOBAL`], returning `true` if it was present.
    pub fn unbind_action(&mut self, action: ActionId, trigger: ButtonTrigger) -> bool {
        self.unbind_action_in(ContextId::GLOBAL, action, trigger)
    }

    /// Removes a specific `trigger` binding from `action` in `context`,
    /// returning `true` if it was present.
    pub fn unbind_action_in(
        &mut self,
        context: ContextId,
        action: ActionId,
        trigger: ButtonTrigger,
    ) -> bool {
        let before = self.action_bindings.len();
        self.action_bindings
            .retain(|b| !(b.context == context && b.action == action && b.trigger == trigger));
        self.action_bindings.len() != before
    }

    /// Removes every binding for `action` across all contexts.
    pub fn clear_action(&mut self, action: ActionId) {
        self.action_bindings.retain(|b| b.action != action);
    }

    /// Removes every binding for `action` within `context`.
    pub fn clear_action_in(&mut self, context: ContextId, action: ActionId) {
        self.action_bindings
            .retain(|b| !(b.context == context && b.action == action));
    }

    /// Sets the [`AxisConfig`] (dead zone, sensitivity, inversion, range) for
    /// `axis` and registers `axis` in the map schema.
    pub fn set_axis_config(&mut self, axis: AxisId, config: AxisConfig) {
        if let Some(slot) = self.axis_configs.get_mut(usize::from(axis.index())) {
            *slot = config;
        }
        self.registered_axes |= axis.bit();
    }

    /// Current [`AxisConfig`] of `axis`.
    pub fn axis_config(&self, axis: AxisId) -> AxisConfig {
        self.axis_configs
            .get(usize::from(axis.index()))
            .copied()
            .unwrap_or_default()
    }

    /// Binds `source` to `axis` in [`ContextId::GLOBAL`].
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if `source` is
    /// invalid or [`MAX_AXIS_BINDINGS`] would be exceeded.
    #[track_caller]
    pub fn bind_axis(&mut self, axis: AxisId, source: AxisSource) -> Result<(), InputError> {
        self.bind_axis_in(ContextId::GLOBAL, axis, source)
    }

    /// Binds `source` to `axis` in `context`.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if `source` is
    /// invalid or [`MAX_AXIS_BINDINGS`] would be exceeded.
    #[track_caller]
    pub fn bind_axis_in(
        &mut self,
        context: ContextId,
        axis: AxisId,
        source: AxisSource,
    ) -> Result<(), InputError> {
        source.validate()?;
        let candidate = AxisBinding {
            context,
            axis,
            source,
        };
        if !self.axis_bindings.contains(&candidate) {
            if self.axis_bindings.len() >= MAX_AXIS_BINDINGS {
                return Err(InputError::new(InputErrorKind::InvalidBinding)
                    .with_detail(format!("axis binding limit ({MAX_AXIS_BINDINGS}) reached")));
            }
            self.axis_bindings.push(candidate);
        }
        self.registered_axes |= axis.bit();
        Ok(())
    }

    /// Binds a negative (`-1.0`) and positive (`+1.0`) digital button pair to
    /// `axis` in [`ContextId::GLOBAL`].
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if either
    /// trigger is invalid, `negative == positive`, or [`MAX_AXIS_BINDINGS`]
    /// would be exceeded.
    #[track_caller]
    pub fn bind_axis_digital_pair(
        &mut self,
        axis: AxisId,
        negative: ButtonTrigger,
        positive: ButtonTrigger,
    ) -> Result<(), InputError> {
        self.bind_axis_digital_pair_in(ContextId::GLOBAL, axis, negative, positive)
    }

    /// Binds a negative (`-1.0`) and positive (`+1.0`) digital button pair to
    /// `axis` in `context`.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if either
    /// trigger is invalid, `negative == positive`, or [`MAX_AXIS_BINDINGS`]
    /// would be exceeded.
    #[track_caller]
    pub fn bind_axis_digital_pair_in(
        &mut self,
        context: ContextId,
        axis: AxisId,
        negative: ButtonTrigger,
        positive: ButtonTrigger,
    ) -> Result<(), InputError> {
        negative.validate()?;
        positive.validate()?;
        if negative == positive {
            return Err(InputError::new(InputErrorKind::InvalidBinding).with_detail(
                "negative and positive triggers of a digital axis pair must be distinct",
            ));
        }
        let neg_binding = AxisBinding {
            context,
            axis,
            source: AxisSource::Button {
                trigger: negative,
                scale: -1.0,
            },
        };
        let pos_binding = AxisBinding {
            context,
            axis,
            source: AxisSource::Button {
                trigger: positive,
                scale: 1.0,
            },
        };
        let needed = usize::from(!self.axis_bindings.contains(&neg_binding))
            + usize::from(!self.axis_bindings.contains(&pos_binding));
        if self.axis_bindings.len() + needed > MAX_AXIS_BINDINGS {
            return Err(InputError::new(InputErrorKind::InvalidBinding)
                .with_detail(format!("axis binding limit ({MAX_AXIS_BINDINGS}) reached")));
        }
        if !self.axis_bindings.contains(&neg_binding) {
            self.axis_bindings.push(neg_binding);
        }
        if !self.axis_bindings.contains(&pos_binding) {
            self.axis_bindings.push(pos_binding);
        }
        self.registered_axes |= axis.bit();
        Ok(())
    }

    /// Replaces all bindings for `axis` in [`ContextId::GLOBAL`] with a new
    /// `(-1.0, +1.0)` digital pair atomically.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if either
    /// trigger is invalid, `negative == positive`, or [`MAX_AXIS_BINDINGS`]
    /// would be exceeded.
    #[track_caller]
    pub fn rebind_axis_digital_pair(
        &mut self,
        axis: AxisId,
        negative: ButtonTrigger,
        positive: ButtonTrigger,
    ) -> Result<(), InputError> {
        self.rebind_axis_digital_pair_in(ContextId::GLOBAL, axis, negative, positive)
    }

    /// Replaces all bindings for `axis` in `context` with a new `(-1.0, +1.0)`
    /// digital pair atomically.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidBinding`] (`INPT-0004`) if either
    /// trigger is invalid, `negative == positive`, or [`MAX_AXIS_BINDINGS`]
    /// would be exceeded.
    #[track_caller]
    pub fn rebind_axis_digital_pair_in(
        &mut self,
        context: ContextId,
        axis: AxisId,
        negative: ButtonTrigger,
        positive: ButtonTrigger,
    ) -> Result<(), InputError> {
        negative.validate()?;
        positive.validate()?;
        if negative == positive {
            return Err(InputError::new(InputErrorKind::InvalidBinding).with_detail(
                "negative and positive triggers of a digital axis pair must be distinct",
            ));
        }
        let existing_for_slot = self
            .axis_bindings
            .iter()
            .filter(|b| b.context == context && b.axis == axis)
            .count();
        if self.axis_bindings.len() - existing_for_slot + 2 > MAX_AXIS_BINDINGS {
            return Err(InputError::new(InputErrorKind::InvalidBinding)
                .with_detail(format!("axis binding limit ({MAX_AXIS_BINDINGS}) reached")));
        }
        self.axis_bindings
            .retain(|b| !(b.context == context && b.axis == axis));
        self.axis_bindings.push(AxisBinding {
            context,
            axis,
            source: AxisSource::Button {
                trigger: negative,
                scale: -1.0,
            },
        });
        self.axis_bindings.push(AxisBinding {
            context,
            axis,
            source: AxisSource::Button {
                trigger: positive,
                scale: 1.0,
            },
        });
        self.registered_axes |= axis.bit();
        Ok(())
    }

    /// Removes every binding for `axis` across all contexts.
    pub fn clear_axis(&mut self, axis: AxisId) {
        self.axis_bindings.retain(|b| b.axis != axis);
    }

    /// Removes every binding for `axis` within `context`.
    pub fn clear_axis_in(&mut self, context: ContextId, axis: AxisId) {
        self.axis_bindings
            .retain(|b| !(b.context == context && b.axis == axis));
    }

    /// Builds a [`CommandValidation`] schema matching this map's registered
    /// actions, registered axes, and configured axis clamp ranges.
    pub fn validation_rules(&self) -> CommandValidation {
        let mut ranges = [None; MAX_AXES];
        for (dst, cfg) in ranges.iter_mut().zip(self.axis_configs.iter()) {
            *dst = cfg.range();
        }
        CommandValidation::from_schema(self.registered_actions, self.registered_axes, ranges)
    }

    /// Validates an untrusted [`InputCommand`] against this map's schema.
    ///
    /// # Errors
    ///
    /// Returns [`InputErrorKind::InvalidCommand`] (`INPT-0006`) if `command`
    /// contains non-finite values, out-of-range axis values, active bits for
    /// unregistered actions/axes, or impossible button transition flags.
    #[track_caller]
    pub fn validate_command(&self, command: &InputCommand) -> Result<(), InputError> {
        command.validate(&self.validation_rules())
    }

    pub(crate) const fn registered_actions_mask(&self) -> u64 {
        self.registered_actions
    }

    /// Evaluates the bitmask of actions currently satisfied by at least one
    /// held binding in an active context (`active_contexts` bitmask).
    pub(crate) fn active_action_mask(&self, devices: &DeviceState, active_contexts: u16) -> u64 {
        let mut mask = 0_u64;
        for binding in &self.action_bindings {
            if (active_contexts & binding.context.bit()) != 0 && binding.trigger.is_down(devices) {
                mask |= binding.action.bit();
            }
        }
        mask
    }

    /// Evaluates all 16 semantic axes for the current sampled tick.
    pub(crate) fn sample_axes(
        &self,
        devices: &DeviceState,
        active_contexts: u16,
    ) -> [f32; MAX_AXES] {
        let mut raw = [0.0_f32; MAX_AXES];
        let mut touched = 0_u16;
        for binding in &self.axis_bindings {
            if (active_contexts & binding.context.bit()) == 0 {
                continue;
            }
            let idx = usize::from(binding.axis.index());
            if let Some(acc) = raw.get_mut(idx) {
                *acc = clamp_finite_f32(*acc + binding.source.sample(devices));
                touched |= binding.axis.bit();
            }
        }

        let mut output = [0.0_f32; MAX_AXES];
        for idx in 0..MAX_AXES {
            if (touched & (1_u16 << idx)) == 0 {
                continue;
            }
            if let (Some(dst), Some(sum), Some(cfg)) = (
                output.get_mut(idx),
                raw.get(idx).copied(),
                self.axis_configs.get(idx),
            ) {
                *dst = cfg.apply(sum);
            }
        }
        output
    }
}

const fn canonical_zero_f32(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn clamp_finite_f32(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        canonical_zero_f32(value.clamp(f32::MIN, f32::MAX))
    }
}

fn select_axis_f64(x: f64, y: f64, axis: MotionAxis) -> f32 {
    let component = match axis {
        MotionAxis::X => x,
        MotionAxis::Y => y,
    };
    if !component.is_finite() {
        0.0
    } else {
        let clamped = component.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32;
        canonical_zero_f32(clamped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_bounds_reject_out_of_range_indices() {
        assert!(ActionId::new(0).is_ok());
        assert!(ActionId::new(63).is_ok());
        let err = ActionId::new(64).unwrap_err();
        assert_eq!(err.kind(), InputErrorKind::IdOutOfRange);
        assert_eq!(err.code(), "INPT-0005");

        assert!(AxisId::new(15).is_ok());
        assert_eq!(
            AxisId::new(16).unwrap_err().kind(),
            InputErrorKind::IdOutOfRange
        );

        assert!(ContextId::new(15).is_ok());
        assert_eq!(
            ContextId::new(16).unwrap_err().kind(),
            InputErrorKind::IdOutOfRange
        );
    }

    #[test]
    fn axis_config_rejects_nan_infinity_and_invalid_bounds() {
        for bad_dz in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.0, 1.5] {
            let err = AxisConfig::new().with_dead_zone(bad_dz).unwrap_err();
            assert_eq!(err.kind(), InputErrorKind::InvalidDeadZone);
            assert_eq!(err.code(), "INPT-0001");
        }

        for bad_sens in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            -1.0,
            AxisConfig::MAX_SENSITIVITY + 1.0,
        ] {
            let err = AxisConfig::new().with_sensitivity(bad_sens).unwrap_err();
            assert_eq!(err.kind(), InputErrorKind::InvalidSensitivity);
            assert_eq!(err.code(), "INPT-0002");
        }

        // Valid zero-inclusive ranges (unilateral and degenerate zero) succeed.
        assert_eq!(
            AxisConfig::new().with_range(0.0, 1.0).unwrap().range(),
            Some((0.0, 1.0))
        );
        assert_eq!(
            AxisConfig::new().with_range(-1.0, 0.0).unwrap().range(),
            Some((-1.0, 0.0))
        );
        assert_eq!(
            AxisConfig::new().with_range(-0.0, 0.0).unwrap().range(),
            Some((0.0, 0.0))
        );

        for (min, max) in [
            (f32::NAN, 1.0),
            (-1.0, f32::NAN),
            (f32::NEG_INFINITY, 1.0),
            (-1.0, f32::INFINITY),
            (0.5, -0.5),
            (0.5, 1.0),
            (-1.0, -0.5),
        ] {
            let err = AxisConfig::new().with_range(min, max).unwrap_err();
            assert_eq!(err.kind(), InputErrorKind::InvalidAxisRange);
            assert_eq!(err.code(), "INPT-0003");
        }
    }

    #[test]
    fn axis_config_applies_dead_zone_rescaling_inversion_and_clamping() -> Result<(), InputError> {
        let cfg = AxisConfig::unit()
            .with_dead_zone(0.2)?
            .with_sensitivity(1.5)?
            .with_inverted(true);

        // Inside or on the dead-zone boundary -> exact +0.0 (not -0.0).
        assert_eq!(cfg.apply(0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(cfg.apply(0.2).to_bits(), 0.0_f32.to_bits());
        assert_eq!(cfg.apply(-0.19).to_bits(), 0.0_f32.to_bits());

        // Non-finite raw value -> sanitized to 0.0.
        assert_eq!(cfg.apply(f32::NAN).to_bits(), 0.0_f32.to_bits());

        // Above dead zone: (0.6 - 0.2) / 0.8 = 0.5; * 1.5 = 0.75; inverted -> -0.75.
        let val = cfg.apply(0.6);
        assert!((val - (-0.75)).abs() < 1e-6, "got {val}");

        // Saturated and clamped to [-1.0, 1.0].
        assert_eq!(cfg.apply(-1.0), 1.0);
        assert_eq!(cfg.apply(1.0), -1.0);
        Ok(())
    }

    #[test]
    fn binding_validation_rejects_unidentified_key_bad_scale_and_identical_pairs()
    -> Result<(), InputError> {
        let mut map = InputMap::new();
        let action = ActionId::new(0)?;
        let axis = AxisId::new(0)?;

        let err = map
            .bind_action(action, ButtonTrigger::Key(KeyCode::Unidentified))
            .unwrap_err();
        assert_eq!(err.kind(), InputErrorKind::InvalidBinding);
        assert_eq!(err.code(), "INPT-0004");

        // NativeKey works for unidentified physical keys.
        map.bind_action(action, ButtonTrigger::NativeKey(240))?;

        for bad_scale in [
            0.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            AxisConfig::MAX_SENSITIVITY + 1.0,
        ] {
            let err = map
                .bind_axis(
                    axis,
                    AxisSource::Button {
                        trigger: ButtonTrigger::Key(KeyCode::W),
                        scale: bad_scale,
                    },
                )
                .unwrap_err();
            assert_eq!(err.kind(), InputErrorKind::InvalidBinding);
        }

        let err = map
            .bind_axis_digital_pair(
                axis,
                ButtonTrigger::Key(KeyCode::A),
                ButtonTrigger::Key(KeyCode::A),
            )
            .unwrap_err();
        assert_eq!(err.kind(), InputErrorKind::InvalidBinding);

        // Failed rebind preserves prior valid binding.
        map.rebind_action(action, ButtonTrigger::Key(KeyCode::Space))?;
        let _ = map.rebind_action(action, ButtonTrigger::Key(KeyCode::Unidentified));
        assert_eq!(map.action_bindings.len(), 1);
        assert_eq!(
            map.action_bindings.first().map(|b| b.trigger),
            Some(ButtonTrigger::Key(KeyCode::Space))
        );
        Ok(())
    }
}
