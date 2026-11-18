//! Validated startup options for the `spawn-engine` runtime composition root.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use spawn_core::FixedStepConfig;
use spawn_platform::{CursorGrabMode, LogicalSize};
use spawn_render::{ClearColor, PresentModePreference, Validation};

use crate::camera::FreeCameraConfig;
use crate::error::{EngineError, EngineErrorKind};

/// Longest bounded smoke duration accepted from `SPAWN_SMOKE_SECONDS` (24 h).
const MAX_SMOKE_SECONDS: f64 = 86_400.0;

/// Default directory holding offline-cooked `.spnshdr` artifacts (`ADR-0014`).
pub const DEFAULT_SHADER_DIR: &str = "target/shaders";

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
    /// Linear RGBA clear color presented behind the 3D scene each drawable frame.
    pub clear_color: ClearColor,
    /// Preferred presentation mode (`Fifo` by default).
    pub present_mode: PresentModePreference,
    /// Graphics validation-layer policy (`IfAvailable` in debug builds).
    pub validation: Validation,
    /// Directory containing offline-cooked `.spnshdr` shader artifacts
    /// (`SPAWN_SHADER_DIR`, defaulting to [`DEFAULT_SHADER_DIR`]).
    pub shader_dir: PathBuf,
    /// Genre-free 60 Hz `f64` free-fly camera parameters (`ADR-0010`).
    pub camera: FreeCameraConfig,
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
            clear_color: ClearColor::new(0.02, 0.04, 0.08, 1.0),
            present_mode: PresentModePreference::Fifo,
            validation: Validation::default(),
            shader_dir: PathBuf::from(DEFAULT_SHADER_DIR),
            camera: FreeCameraConfig::default(),
            auto_close_after: None,
            smoke_cursor_grab: None,
        }
    }
}

impl RuntimeOptions {
    /// Reads and validates optional environment overrides
    /// (`SPAWN_SMOKE_SECONDS`, `SPAWN_SMOKE_GRAB`, `SPAWN_SHADER_DIR`) onto
    /// [`Self::default`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::InvalidOption`] (`ENGN-0001`) if any
    /// environment variable is non-UTF-8, empty, malformed, non-finite,
    /// negative, or names an unknown cursor grab mode.
    pub fn from_env() -> Result<Self, EngineError> {
        let smoke_seconds = read_optional_env("SPAWN_SMOKE_SECONDS")?;
        let smoke_grab = read_optional_env("SPAWN_SMOKE_GRAB")?;
        let shader_dir = read_optional_env("SPAWN_SHADER_DIR")?;
        Self::from_env_values_with_shader_dir(
            smoke_seconds.as_deref(),
            smoke_grab.as_deref(),
            shader_dir.as_deref(),
        )
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
        Self::from_env_values_with_shader_dir(smoke_seconds, smoke_grab, None)
    }

    /// Validates raw string values for `SPAWN_SMOKE_SECONDS`,
    /// `SPAWN_SMOKE_GRAB`, and `SPAWN_SHADER_DIR` without reading process
    /// environment state.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::InvalidOption`] (`ENGN-0001`) when any value
    /// fails validation.
    pub fn from_env_values_with_shader_dir(
        smoke_seconds: Option<&str>,
        smoke_grab: Option<&str>,
        shader_dir: Option<&str>,
    ) -> Result<Self, EngineError> {
        let mut options = Self::default();
        if let Some(raw) = smoke_seconds {
            options.auto_close_after = Some(parse_smoke_seconds(raw)?);
        }
        if let Some(raw) = smoke_grab {
            options.smoke_cursor_grab = Some(parse_smoke_grab(raw)?);
        }
        if let Some(raw) = shader_dir {
            options.shader_dir = parse_shader_dir(raw)?;
        }
        Ok(options)
    }
}

/// Validates that `shader_dir` is non-empty and free of NUL bytes.
#[track_caller]
pub(crate) fn validate_shader_dir_path(shader_dir: &Path) -> Result<(), EngineError> {
    let raw = shader_dir.as_os_str();
    if raw.is_empty() {
        return Err(EngineError::new(EngineErrorKind::InvalidOption)
            .with_detail("field `shader_dir` cannot be empty"));
    }
    if shader_dir.to_string_lossy().contains('\0') {
        return Err(EngineError::new(EngineErrorKind::InvalidOption)
            .with_detail("field `shader_dir` cannot contain NUL bytes"));
    }
    Ok(())
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

fn parse_shader_dir(raw: &str) -> Result<PathBuf, EngineError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || raw.contains('\0') {
        return Err(EngineError::new(EngineErrorKind::InvalidOption)
            .with_detail(format!("SPAWN_SHADER_DIR={raw:?} must be a non-empty path")));
    }
    Ok(PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_smoke_and_shader_dir_options() {
        let defaults = RuntimeOptions::from_env_values(None, None).unwrap();
        assert_eq!(defaults.auto_close_after, None);
        assert_eq!(defaults.smoke_cursor_grab, None);
        assert_eq!(defaults.shader_dir, PathBuf::from(DEFAULT_SHADER_DIR));

        let configured = RuntimeOptions::from_env_values_with_shader_dir(
            Some("2.5"),
            Some("locked"),
            Some("build/cooked-shaders"),
        )
        .unwrap();
        assert_eq!(
            configured.auto_close_after,
            Some(Duration::from_millis(2500))
        );
        assert_eq!(configured.smoke_cursor_grab, Some(CursorGrabMode::Locked));
        assert_eq!(configured.shader_dir, PathBuf::from("build/cooked-shaders"));

        let confined = RuntimeOptions::from_env_values(Some("0"), Some("confined")).unwrap();
        assert_eq!(confined.auto_close_after, Some(Duration::ZERO));
        assert_eq!(confined.smoke_cursor_grab, Some(CursorGrabMode::Confined));

        let none = RuntimeOptions::from_env_values(None, Some("none")).unwrap();
        assert_eq!(none.smoke_cursor_grab, Some(CursorGrabMode::None));
    }

    #[test]
    fn rejects_invalid_smoke_and_shader_dir_options() {
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
        for bad_dir in ["", "   ", "bad\0path"] {
            let err = RuntimeOptions::from_env_values_with_shader_dir(None, None, Some(bad_dir))
                .unwrap_err();
            assert_eq!(err.kind(), EngineErrorKind::InvalidOption);
            assert_eq!(err.code(), "ENGN-0001");
        }
    }
}
