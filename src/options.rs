//! Minimal validated startup options for the foundation runtime slice.

use std::time::Duration;

use spawn_core::FixedStepConfig;
use spawn_platform::{CursorGrabMode, LogicalSize};
use spawn_render::{ClearColor, PresentModePreference, Validation};

use crate::error::{EngineError, EngineErrorKind};

/// Longest bounded smoke duration accepted from `SPAWN_SMOKE_SECONDS` (24 h).
const MAX_SMOKE_SECONDS: f64 = 86_400.0;

/// Startup configuration for one [`run_session`](crate::run_session) execution.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeOptions {
    /// Native top-level window title.
    pub title: String,
    /// Platform application identifier (Wayland `app_id`).
    pub app_id: String,
    /// Initial requested logical window size.
    pub window_size: LogicalSize,
    /// Fixed-timestep simulation clock configuration (ADR-0003).
    pub fixed_step: FixedStepConfig,
    /// Linear RGBA clear color presented each drawable frame.
    pub clear_color: ClearColor,
    /// Preferred presentation mode (`Fifo` by default).
    pub present_mode: PresentModePreference,
    /// Graphics validation-layer policy (`IfAvailable` in debug builds).
    pub validation: Validation,
    /// Optional bounded runtime duration for automated smoke runs
    /// (`SPAWN_SMOKE_SECONDS`).
    pub auto_close_after: Option<Duration>,
    /// Optional initial cursor constraint for smoke runs (`SPAWN_SMOKE_GRAB`).
    pub smoke_cursor_grab: Option<CursorGrabMode>,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            title: "Spawn Engine".to_owned(),
            app_id: "dev.spawn.engine".to_owned(),
            window_size: LogicalSize::new(960, 540),
            fixed_step: FixedStepConfig::default(),
            clear_color: ClearColor::new(0.02, 0.25, 0.30, 1.0),
            present_mode: PresentModePreference::Fifo,
            validation: Validation::default(),
            auto_close_after: None,
            smoke_cursor_grab: None,
        }
    }
}

impl RuntimeOptions {
    /// Reads and validates optional smoke-test environment overrides
    /// (`SPAWN_SMOKE_SECONDS`, `SPAWN_SMOKE_GRAB`) onto [`Self::default`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::InvalidOption`] (`ENGN-0001`) if either
    /// environment variable is non-UTF-8, malformed, non-finite, negative, or
    /// names an unknown cursor grab mode.
    pub fn from_env() -> Result<Self, EngineError> {
        let smoke_seconds = read_optional_env("SPAWN_SMOKE_SECONDS")?;
        let smoke_grab = read_optional_env("SPAWN_SMOKE_GRAB")?;
        Self::from_env_values(smoke_seconds.as_deref(), smoke_grab.as_deref())
    }

    /// Validates raw string values for `SPAWN_SMOKE_SECONDS` and
    /// `SPAWN_SMOKE_GRAB` without reading process environment state.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::InvalidOption`] (`ENGN-0001`) when either
    /// value fails validation.
    pub fn from_env_values(
        smoke_seconds: Option<&str>,
        smoke_grab: Option<&str>,
    ) -> Result<Self, EngineError> {
        let mut options = Self::default();
        if let Some(raw) = smoke_seconds {
            options.auto_close_after = Some(parse_smoke_seconds(raw)?);
        }
        if let Some(raw) = smoke_grab {
            options.smoke_cursor_grab = Some(parse_smoke_grab(raw)?);
        }
        Ok(options)
    }
}

fn read_optional_env(name: &'static str) -> Result<Option<String>, EngineError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(EngineError::new(EngineErrorKind::InvalidOption)
                .with_detail(format!("{name} contains non-UTF-8 bytes")))
        }
    }
}

fn parse_smoke_seconds(raw: &str) -> Result<Duration, EngineError> {
    let trimmed = raw.trim();
    let seconds: f64 = trimmed.parse().map_err(|err| {
        EngineError::new(EngineErrorKind::InvalidOption)
            .with_detail(format!(
                "SPAWN_SMOKE_SECONDS={raw:?} is not a valid number of seconds"
            ))
            .with_cause(&err)
    })?;
    if !seconds.is_finite() || !(0.0..=MAX_SMOKE_SECONDS).contains(&seconds) {
        return Err(
            EngineError::new(EngineErrorKind::InvalidOption).with_detail(format!(
                "SPAWN_SMOKE_SECONDS={raw:?} must be finite and in 0..={MAX_SMOKE_SECONDS}"
            )),
        );
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn parse_smoke_grab(raw: &str) -> Result<CursorGrabMode, EngineError> {
    match raw.trim() {
        "none" => Ok(CursorGrabMode::None),
        "confined" => Ok(CursorGrabMode::Confined),
        "locked" => Ok(CursorGrabMode::Locked),
        _ => Err(
            EngineError::new(EngineErrorKind::InvalidOption).with_detail(format!(
                "SPAWN_SMOKE_GRAB={raw:?} is invalid; expected `none`, `confined`, or `locked`"
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_smoke_options() {
        let defaults = RuntimeOptions::from_env_values(None, None).unwrap();
        assert_eq!(defaults.auto_close_after, None);
        assert_eq!(defaults.smoke_cursor_grab, None);

        let configured = RuntimeOptions::from_env_values(Some("2.5"), Some("locked")).unwrap();
        assert_eq!(
            configured.auto_close_after,
            Some(Duration::from_millis(2500))
        );
        assert_eq!(configured.smoke_cursor_grab, Some(CursorGrabMode::Locked));

        let confined = RuntimeOptions::from_env_values(Some("0"), Some("confined")).unwrap();
        assert_eq!(confined.auto_close_after, Some(Duration::ZERO));
        assert_eq!(confined.smoke_cursor_grab, Some(CursorGrabMode::Confined));

        let none = RuntimeOptions::from_env_values(None, Some("none")).unwrap();
        assert_eq!(none.smoke_cursor_grab, Some(CursorGrabMode::None));
    }

    #[test]
    fn rejects_invalid_smoke_options() {
        for bad_seconds in ["", "abc", "-1", "NaN", "inf", "100000"] {
            let err = RuntimeOptions::from_env_values(Some(bad_seconds), None).unwrap_err();
            assert_eq!(err.kind(), EngineErrorKind::InvalidOption);
            assert_eq!(err.code(), "ENGN-0001");
        }
        for bad_grab in ["", "grabbed", "LOCKED_NOW"] {
            let err = RuntimeOptions::from_env_values(None, Some(bad_grab)).unwrap_err();
            assert_eq!(err.kind(), EngineErrorKind::InvalidOption);
            assert_eq!(err.code(), "ENGN-0001");
        }
    }
}
