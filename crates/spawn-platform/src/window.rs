//! Window creation parameters (backend-independent).

use crate::error::{PlatformError, PlatformErrorKind};
use crate::event::LogicalSize;

/// Parameters for creating a window. Validated on construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDescriptor {
    title: String,
    app_id: Option<String>,
    size: LogicalSize,
}

impl WindowDescriptor {
    /// Largest accepted width or height in logical units.
    pub const MAX_EXTENT: u32 = 16_384;

    /// Creates a descriptor for a window titled `title` with an initial
    /// logical `size`. The window system may choose a different size; the
    /// actual size is reported by the window and by resize events.
    ///
    /// # Errors
    ///
    /// [`PlatformErrorKind::InvalidWindowSize`] if a dimension is zero or
    /// above [`Self::MAX_EXTENT`].
    #[track_caller]
    pub fn new(title: impl Into<String>, size: LogicalSize) -> Result<Self, PlatformError> {
        let valid = |extent: u32| (1..=Self::MAX_EXTENT).contains(&extent);
        if !valid(size.width) || !valid(size.height) {
            return Err(PlatformError::new(PlatformErrorKind::InvalidWindowSize)
                .with_detail(format!("requested {}x{}", size.width, size.height)));
        }
        Ok(Self {
            title: title.into(),
            app_id: None,
            size,
        })
    }

    /// Sets the application identifier the window system uses to group
    /// windows and apply rules (the Wayland `app_id`). Without it the window
    /// system's default applies.
    #[must_use]
    pub fn with_app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }

    /// Window title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Application identifier, if set.
    pub fn app_id(&self) -> Option<&str> {
        self.app_id.as_deref()
    }

    /// Requested initial size in logical units.
    pub fn size(&self) -> LogicalSize {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_sizes_within_bounds() {
        let max = WindowDescriptor::MAX_EXTENT;
        for size in [
            LogicalSize::new(1, 1),
            LogicalSize::new(1280, 720),
            LogicalSize::new(max, max),
        ] {
            let descriptor = WindowDescriptor::new("t", size);
            assert_eq!(descriptor.map(|d| d.size()).ok(), Some(size));
        }
    }

    #[test]
    fn rejects_zero_and_oversized_dimensions() {
        let max = WindowDescriptor::MAX_EXTENT;
        for size in [
            LogicalSize::new(0, 720),
            LogicalSize::new(1280, 0),
            LogicalSize::new(max + 1, 720),
            LogicalSize::new(1280, u32::MAX),
        ] {
            let error = WindowDescriptor::new("t", size).err();
            assert_eq!(
                error.map(|e| e.kind()),
                Some(PlatformErrorKind::InvalidWindowSize)
            );
        }
    }

    #[test]
    fn keeps_title_and_app_id() {
        let descriptor = WindowDescriptor::new("Spawn", LogicalSize::new(640, 480))
            .map(|d| d.with_app_id("dev.spawn.test"));
        let descriptor = descriptor.ok();
        assert_eq!(
            descriptor.as_ref().map(WindowDescriptor::title),
            Some("Spawn")
        );
        assert_eq!(
            descriptor.as_ref().and_then(WindowDescriptor::app_id),
            Some("dev.spawn.test")
        );
    }
}
