//! Backend-independent renderer types: sizes, colors, options and outcomes.

/// Size of the drawable surface in physical pixels.
///
/// Zero in either dimension means "not drawable" (for example minimized):
/// the renderer suspends presentation until a non-zero size is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SurfaceSize {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
}

impl SurfaceSize {
    /// Creates a surface size.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Whether either dimension is zero (nothing can be presented).
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A clear color in **linear** RGB with straight alpha, components in `0..=1`.
///
/// The renderer encodes it for the swapchain format (sRGB formats encode on
/// the GPU; for UNORM formats the renderer applies the sRGB transfer function
/// itself), so the same value looks the same on every format. Non-finite
/// components are treated as `0`, and values are clamped to `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClearColor {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Alpha (opacity); swapchains are presented opaque where supported.
    pub a: f32,
}

impl ClearColor {
    /// Creates a clear color from linear components.
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Components with non-finite values replaced by 0 and clamped to `0..=1`.
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(crate) fn sanitized(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a].map(|c| {
            if c.is_finite() {
                c.clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
    }

    /// Components to write into a UNORM image so it shows this color: the
    /// color channels go through the sRGB transfer function, alpha does not.
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(crate) fn srgb_encoded(self) -> [f32; 4] {
        let [r, g, b, a] = self.sanitized();
        let encode = |c: f32| {
            if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }
        };
        [encode(r), encode(g), encode(b), a]
    }
}

/// Which presentation mode the renderer should prefer.
///
/// FIFO (vsync) is the only mode every Vulkan driver must support; any other
/// preference falls back to it when unsupported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PresentModePreference {
    /// Wait for vertical blank; never tears, bounds CPU and GPU work to the
    /// display rate. Always available.
    #[default]
    Fifo,
    /// Replace the queued image at each vertical blank; no tearing, lower
    /// latency than FIFO, renders frames that may never be shown.
    Mailbox,
    /// Present immediately; may tear.
    Immediate,
}

/// Whether to enable the Khronos validation layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Validation {
    /// Never enable it.
    Disabled,
    /// Enable it when the layer is installed; continue without it otherwise
    /// ([`crate::RendererInfo::validation_enabled`] reports which).
    IfAvailable,
    /// Fail with `REND-0008` when the layer is not installed.
    Required,
}

impl Default for Validation {
    /// [`Validation::IfAvailable`] in builds with debug assertions,
    /// [`Validation::Disabled`] otherwise.
    fn default() -> Self {
        if cfg!(debug_assertions) {
            Self::IfAvailable
        } else {
            Self::Disabled
        }
    }
}

/// Renderer creation options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RendererDescriptor {
    /// Preferred presentation mode.
    pub present_mode: PresentModePreference,
    /// Validation layer policy.
    pub validation: Validation,
}

/// What one call to render a frame did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub enum FrameOutcome {
    /// The frame was submitted and queued for presentation.
    Presented,
    /// Nothing was done because the surface is not drawable (zero size).
    /// The caller should wait for window events instead of rendering again
    /// immediately.
    Suspended,
    /// No frame was shown this time: the swapchain became outdated, the
    /// surface was lost, or no image was available within the acquire timeout.
    /// Recovery is scheduled for the next call, within bounded attempts.
    Skipped,
}

/// Coarse presentation state (foundation DESIGN, presentation state machine).
///
/// "Uninitialized" and "Stopped" are not states of a live renderer: a renderer
/// exists only after successful initialization, and dropping it stops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationState {
    /// The swapchain matches the requested size; the next frame acquires an
    /// image.
    Ready,
    /// The surface is not drawable (zero size); no image is acquired or
    /// presented.
    Suspended,
    /// The swapchain (and possibly the surface) is recreated before the next
    /// acquire.
    Recovering,
    /// A fatal error stopped the renderer; every frame call fails.
    Fatal,
}

/// Severity of a validation-layer message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValidationSeverity {
    /// Diagnostic detail.
    Verbose,
    /// Informational.
    Info,
    /// Possible misuse or inefficiency.
    Warning,
    /// Invalid API usage.
    Error,
}

/// One message from the Vulkan validation layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationMessage {
    /// How severe the message is.
    pub severity: ValidationSeverity,
    /// The layer's message text.
    pub text: String,
}

/// Kind of GPU, as reported by the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuKind {
    /// A discrete GPU.
    Discrete,
    /// A GPU integrated with the CPU.
    Integrated,
    /// A virtualized GPU.
    Virtual,
    /// A software rasterizer running on the CPU.
    Cpu,
    /// Anything else.
    Other,
}

/// Diagnostic information about the renderer's GPU and configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererInfo {
    /// GPU name as reported by the driver.
    pub gpu_name: String,
    /// GPU kind.
    pub gpu_kind: GpuKind,
    /// Vulkan version supported by the device (major, minor, patch).
    pub api_version: (u32, u32, u32),
    /// Driver name and version text as reported by the driver.
    pub driver: String,
    /// Whether the Khronos validation layer is active.
    pub validation_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_size_emptiness() {
        assert!(SurfaceSize::new(0, 1).is_empty());
        assert!(SurfaceSize::new(1, 0).is_empty());
        assert!(!SurfaceSize::new(1, 1).is_empty());
    }

    #[test]
    fn clear_color_is_sanitized() {
        let color = ClearColor::new(f32::NAN, 2.0, -1.0, f32::INFINITY);
        assert_eq!(color.sanitized(), [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn srgb_encoding_matches_reference_values() {
        let [black, white, mid, alpha] = ClearColor::new(0.0, 1.0, 0.5, 0.5).srgb_encoded();
        assert_eq!(black, 0.0);
        assert!((white - 1.0).abs() < 1e-6);
        // Linear 0.5 is sRGB ~0.7354.
        assert!((mid - 0.735_356_9).abs() < 1e-4, "{mid}");
        // Alpha is not encoded.
        assert_eq!(alpha, 0.5);
        // Linear segment below the threshold.
        let [low, ..] = ClearColor::new(0.002, 0.0, 0.0, 1.0).srgb_encoded();
        assert!((low - 0.002 * 12.92).abs() < 1e-7);
    }

    #[test]
    fn defaults_are_fifo_and_debug_dependent_validation() {
        let descriptor = RendererDescriptor::default();
        assert_eq!(descriptor.present_mode, PresentModePreference::Fifo);
        let expected = if cfg!(debug_assertions) {
            Validation::IfAvailable
        } else {
            Validation::Disabled
        };
        assert_eq!(descriptor.validation, expected);
    }
}
