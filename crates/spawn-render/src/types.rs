//! Backend-independent renderer types: sizes, colors, options and outcomes.

#![forbid(unsafe_code)]

use crate::handle::ShaderHandle;
use crate::shader::{DescriptorBinding, PushConstantRange, VertexInputAttribute};

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

/// Color target family a graphics pipeline renders into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PipelineTarget {
    /// Presentation swapchain color image paired with `D32_SFLOAT` reversed-Z depth.
    #[default]
    Swapchain,
    /// Offscreen `R8G8B8A8_UNORM` color image paired with `D32_SFLOAT` reversed-Z depth.
    OffscreenRgba8Unorm,
}

/// Polygon front-face screen-space winding rule (`SPN-INV-050`, `SPN-INV-053`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FrontFace {
    /// Standard right-handed counter-clockwise front-face winding (`VK_FRONT_FACE_COUNTER_CLOCKWISE`).
    #[default]
    CounterClockwise,
    /// Clockwise front-face winding (`VK_FRONT_FACE_CLOCKWISE`).
    Clockwise,
}

/// Polygon face-culling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CullMode {
    /// Cull back-facing triangles (`VK_CULL_MODE_BACK_BIT`).
    #[default]
    Back,
    /// Cull front-facing triangles (`VK_CULL_MODE_FRONT_BIT`).
    Front,
    /// Disable polygon culling (`VK_CULL_MODE_NONE`).
    None,
}

/// Backend-independent descriptor for compiling a `dynamicRendering` graphics pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphicsPipelineDescriptor<'a> {
    /// Debug name attached through `VK_EXT_debug_utils` (`spawn.pipeline.<label>`).
    pub label: &'a str,
    /// Validated vertex shader handle.
    pub vertex_shader: ShaderHandle,
    /// Validated fragment shader handle.
    pub fragment_shader: ShaderHandle,
    /// Target color attachment family (`Swapchain` by default).
    pub target: PipelineTarget,
    /// Vertex buffer attributes (`&[]` when the vertex shader uses `SV_VulkanVertexID`).
    pub vertex_inputs: &'a [VertexInputAttribute],
    /// Vertex buffer byte stride when `vertex_inputs` is non-empty (`0` when empty).
    pub vertex_stride: u32,
    /// Push-constant block range (`PushConstantRange::NONE` when unused).
    pub push_constants: PushConstantRange,
    /// Descriptor bindings (`&[]` when unused).
    pub descriptor_bindings: &'a [DescriptorBinding],
    /// Front-face winding (`FrontFace::CounterClockwise` by default).
    pub front_face: FrontFace,
    /// Polygon culling mode (`CullMode::Back` by default).
    pub cull_mode: CullMode,
    /// Enable reversed-Z depth test (`GREATER_OR_EQUAL`) and depth write (`true` by default).
    pub depth_test_enable: bool,
}

impl<'a> GraphicsPipelineDescriptor<'a> {
    /// Creates a graphics pipeline descriptor with Spawn's reversed-Z,
    /// counter-clockwise front-face, back-face culling defaults.
    pub const fn new(
        label: &'a str,
        vertex_shader: ShaderHandle,
        fragment_shader: ShaderHandle,
    ) -> Self {
        Self {
            label,
            vertex_shader,
            fragment_shader,
            target: PipelineTarget::Swapchain,
            vertex_inputs: &[],
            vertex_stride: 0,
            push_constants: PushConstantRange::NONE,
            descriptor_bindings: &[],
            front_face: FrontFace::CounterClockwise,
            cull_mode: CullMode::Back,
            depth_test_enable: true,
        }
    }

    /// Sets the target color attachment family (`Swapchain` or `OffscreenRgba8Unorm`).
    #[must_use]
    pub const fn with_target(mut self, target: PipelineTarget) -> Self {
        self.target = target;
        self
    }

    /// Sets the single-binding vertex buffer byte stride and attribute declarations.
    #[must_use]
    pub const fn with_vertex_inputs(
        mut self,
        vertex_stride: u32,
        vertex_inputs: &'a [VertexInputAttribute],
    ) -> Self {
        self.vertex_stride = vertex_stride;
        self.vertex_inputs = vertex_inputs;
        self
    }

    /// Sets the pipeline push-constant range.
    #[must_use]
    pub const fn with_push_constants(mut self, push_constants: PushConstantRange) -> Self {
        self.push_constants = push_constants;
        self
    }

    /// Sets the pipeline descriptor bindings.
    #[must_use]
    pub const fn with_descriptor_bindings(
        mut self,
        descriptor_bindings: &'a [DescriptorBinding],
    ) -> Self {
        self.descriptor_bindings = descriptor_bindings;
        self
    }

    /// Sets the front-face polygon winding rule.
    #[must_use]
    pub const fn with_front_face(mut self, front_face: FrontFace) -> Self {
        self.front_face = front_face;
        self
    }

    /// Sets the polygon face-culling mode.
    #[must_use]
    pub const fn with_cull_mode(mut self, cull_mode: CullMode) -> Self {
        self.cull_mode = cull_mode;
        self
    }

    /// Enables or disables reversed-Z depth testing and depth writing.
    #[must_use]
    pub const fn with_depth_test(mut self, enable: bool) -> Self {
        self.depth_test_enable = enable;
        self
    }
}

/// Host-side RGBA8 (`[r, g, b, a]` per pixel, row-major top-to-bottom) readback
/// captured from an offscreen render target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffscreenReadback {
    size: SurfaceSize,
    rgba8: Vec<u8>,
}

impl OffscreenReadback {
    #[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
    pub(crate) fn new(size: SurfaceSize, rgba8: Vec<u8>) -> Self {
        Self { size, rgba8 }
    }

    /// Physical pixel dimensions of the captured image.
    pub const fn size(&self) -> SurfaceSize {
        self.size
    }

    /// Raw tightly-packed row-major RGBA8 bytes (`width * height * 4` bytes).
    pub fn rgba8_bytes(&self) -> &[u8] {
        &self.rgba8
    }

    /// Reads the `[r, g, b, a]` pixel at `(x, y)` (`(0, 0)` is top-left),
    /// returning `None` if `(x, y)` is outside `self.size()`.
    pub fn pixel_rgba(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.size.width || y >= self.size.height {
            return None;
        }
        let pixel_index = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.size.width).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?;
        let byte_offset = pixel_index.checked_mul(4)?;
        let end = byte_offset.checked_add(4)?;
        let slice = self.rgba8.get(byte_offset..end)?;
        slice.try_into().ok()
    }
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
