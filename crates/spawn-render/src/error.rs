//! Error types of `spawn-render` (ADR-0009: hand-written, located, coded).

#![forbid(unsafe_code)]

use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::Location;

/// What went wrong in a [`RenderError`].
///
/// Each kind has a stable error code (see [`RenderErrorKind::code`]); codes
/// are never reused or renumbered. Kinds marked *fatal* stop the renderer:
/// later calls return [`RenderErrorKind::RendererFailed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RenderErrorKind {
    /// `REND-0001`: no usable Vulkan loader, or it does not support
    /// Vulkan 1.3.
    VulkanUnavailable,
    /// `REND-0002`: the window or display handle is of a kind the renderer
    /// cannot present to, or the Vulkan loader lacks its surface extension.
    UnsupportedWindowSystem,
    /// `REND-0003`: no GPU meets the requirements of ADR-0004; the detail
    /// names every GPU and why it was rejected.
    NoSuitableDevice,
    /// `REND-0004`: a Vulkan call failed unexpectedly (for example out of
    /// memory). *Fatal* after initialization.
    VulkanCallFailed,
    /// `REND-0005`: the GPU device was lost (driver reset or hang). *Fatal*;
    /// device recovery is not supported.
    DeviceLost,
    /// `REND-0006`: the surface could not be restored within the bounded
    /// number of recovery attempts. *Fatal*.
    SurfaceRecoveryFailed,
    /// `REND-0007`: GPU work did not complete within the time limit. *Fatal*.
    GpuTimeout,
    /// `REND-0008`: validation was required but the Khronos validation layer
    /// is not installed.
    ValidationUnavailable,
    /// `REND-0009`: the renderer stopped after an earlier fatal error.
    RendererFailed,
    /// `REND-0010`: a cooked shader artifact is corrupt, truncated, oversized,
    /// or uses an unsupported format version.
    InvalidShaderArtifact,
    /// `REND-0011`: a cooked shader's reflected interface does not match the
    /// expected pipeline layout.
    ShaderLayoutMismatch,
    /// `REND-0012`: GPU memory suballocation or block allocation failed or
    /// exceeded the configured memory budget.
    GpuMemoryExhausted,
    /// `REND-0013`: a resource handle is stale, destroyed, or forged.
    InvalidResourceHandle,
    /// `REND-0014`: a pipeline or render-target descriptor has invalid
    /// parameters or target compatibility.
    InvalidResourceDescriptor,
}

impl RenderErrorKind {
    /// Stable, searchable error code of this kind.
    pub const fn code(self) -> &'static str {
        match self {
            Self::VulkanUnavailable => "REND-0001",
            Self::UnsupportedWindowSystem => "REND-0002",
            Self::NoSuitableDevice => "REND-0003",
            Self::VulkanCallFailed => "REND-0004",
            Self::DeviceLost => "REND-0005",
            Self::SurfaceRecoveryFailed => "REND-0006",
            Self::GpuTimeout => "REND-0007",
            Self::ValidationUnavailable => "REND-0008",
            Self::RendererFailed => "REND-0009",
            Self::InvalidShaderArtifact => "REND-0010",
            Self::ShaderLayoutMismatch => "REND-0011",
            Self::GpuMemoryExhausted => "REND-0012",
            Self::InvalidResourceHandle => "REND-0013",
            Self::InvalidResourceDescriptor => "REND-0014",
        }
    }

    /// One-line description of this kind.
    pub const fn message(self) -> &'static str {
        match self {
            Self::VulkanUnavailable => "Vulkan 1.3 is not available",
            Self::UnsupportedWindowSystem => "the renderer cannot present to this window system",
            Self::NoSuitableDevice => "no GPU meets the renderer's requirements",
            Self::VulkanCallFailed => "a Vulkan call failed",
            Self::DeviceLost => "the GPU device was lost",
            Self::SurfaceRecoveryFailed => "the presentation surface could not be recovered",
            Self::GpuTimeout => "GPU work did not complete in time",
            Self::ValidationUnavailable => "the Vulkan validation layer is not installed",
            Self::RendererFailed => "the renderer stopped after an earlier fatal error",
            Self::InvalidShaderArtifact => "invalid cooked shader artifact",
            Self::ShaderLayoutMismatch => "shader interface does not match pipeline layout",
            Self::GpuMemoryExhausted => "GPU memory allocation failed or exceeded budget",
            Self::InvalidResourceHandle => "invalid or stale render resource handle",
            Self::InvalidResourceDescriptor => "invalid render resource or target descriptor",
        }
    }
}

/// Error returned by `spawn-render`.
///
/// Carries the kind, the source location inside `spawn-render` where the
/// failure was detected, an optional detail naming what was involved, the
/// underlying cause as opaque text (backend error types never cross the public
/// API) and, when enabled through `RUST_BACKTRACE`, a backtrace. `Display`
/// renders a compiler-like report:
///
/// ```text
/// error[REND-0003]: no GPU meets the renderer's requirements
///   --> crates/spawn-render/src/vulkan/device.rs:120:17
///    = detail: AMD Radeon RX 460 Graphics (RADV POLARIS11): missing feature `synchronization2`
/// ```
#[derive(Debug)]
pub struct RenderError {
    kind: RenderErrorKind,
    location: &'static Location<'static>,
    detail: Option<String>,
    cause: Option<Cause>,
    backtrace: Backtrace,
}

impl RenderError {
    /// Creates an error of `kind`, recording the caller's source location.
    #[track_caller]
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(crate) fn new(kind: RenderErrorKind) -> Self {
        Self {
            kind,
            location: Location::caller(),
            detail: None,
            cause: None,
            backtrace: Backtrace::capture(),
        }
    }

    /// Adds a detail line naming what was involved.
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Records `cause` (and its whole `source()` chain) as opaque text, so no
    /// backend error type escapes through [`std::error::Error::source`].
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(crate) fn with_cause(mut self, cause: &(dyn std::error::Error + 'static)) -> Self {
        let mut message = cause.to_string();
        let mut next = cause.source();
        while let Some(inner) = next {
            message.push_str(": ");
            message.push_str(&inner.to_string());
            next = inner.source();
        }
        self.cause = Some(Cause(message));
        self
    }

    /// What went wrong.
    pub fn kind(&self) -> RenderErrorKind {
        self.kind
    }

    /// Stable error code (e.g. `REND-0001`).
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Source location where the error was created.
    pub fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// What was involved (a Vulkan call, a GPU name), if recorded.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Backtrace captured when the error was created; empty unless enabled
    /// through `RUST_BACKTRACE` / `RUST_LIB_BACKTRACE`.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "error[{}]: {}\n  --> {}:{}:{}",
            self.kind.code(),
            self.kind.message(),
            self.location.file(),
            self.location.line(),
            self.location.column()
        )?;
        if let Some(detail) = &self.detail {
            write!(f, "\n   = detail: {detail}")?;
        }
        if let Some(cause) = &self.cause {
            write!(f, "\ncaused by: {cause}")?;
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\nbacktrace:\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

/// Underlying cause of a [`RenderError`], flattened to text.
#[derive(Debug)]
struct Cause(String);

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Cause {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[derive(Debug)]
    struct Cause;
    impl fmt::Display for Cause {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("ERROR_OUT_OF_DEVICE_MEMORY")
        }
    }
    impl std::error::Error for Cause {}

    #[test]
    fn codes_are_stable_and_distinct() {
        use RenderErrorKind as K;
        let kinds = [
            K::VulkanUnavailable,
            K::UnsupportedWindowSystem,
            K::NoSuitableDevice,
            K::VulkanCallFailed,
            K::DeviceLost,
            K::SurfaceRecoveryFailed,
            K::GpuTimeout,
            K::ValidationUnavailable,
            K::RendererFailed,
            K::InvalidShaderArtifact,
            K::ShaderLayoutMismatch,
            K::GpuMemoryExhausted,
            K::InvalidResourceHandle,
            K::InvalidResourceDescriptor,
        ];
        let codes: Vec<_> = kinds.iter().map(|kind| kind.code()).collect();
        let expected: Vec<_> = (1..=14).map(|n| format!("REND-{n:04}")).collect();
        assert_eq!(codes, expected);
    }

    #[test]
    fn report_has_code_location_detail_and_cause() {
        let error = RenderError::new(RenderErrorKind::VulkanCallFailed)
            .with_detail("vkCreateSwapchainKHR")
            .with_cause(&Cause);
        let line = line!() - 3;
        let report = error.to_string();
        let expected = format!(
            "error[REND-0004]: a Vulkan call failed\n  --> {}:{line}:21\n   = detail: vkCreateSwapchainKHR\ncaused by: ERROR_OUT_OF_DEVICE_MEMORY",
            file!()
        );
        assert!(report.starts_with(&expected), "report was:\n{report}");
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("ERROR_OUT_OF_DEVICE_MEMORY")
        );
    }
}
