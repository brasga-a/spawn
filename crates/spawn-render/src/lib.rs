//! `spawn-render`: GPU abstraction, resources, shaders, materials, frame execution.
//!
//! Always available (no GPU code):
//!
//! - [`SurfaceSize`], [`ClearColor`], [`RendererDescriptor`],
//!   [`PresentModePreference`], [`Validation`], [`FrameOutcome`],
//!   [`PresentationState`], [`RendererInfo`], [`ValidationMessage`].
//! - Cooked-shader binary contract and untrusted layout validator (ADR-0014):
//!   [`CookedShader`], [`ShaderStage`], [`ShaderStageFlags`], [`VertexFormat`],
//!   [`VertexInputAttribute`], [`DescriptorKind`], [`DescriptorBinding`],
//!   [`PushConstantRange`], [`ShaderLayoutExpectation`], [`PipelineLayoutDesc`],
//!   [`validate_graphics_pipeline_shaders`].
//! - The located, coded [`RenderError`] (ADR-0009).
//!
//! With the `vulkan` Cargo feature, the Vulkan 1.3 backend (ADR-0004) adds
//! `Renderer`: it presents a clear color to a window given through the
//! `raw-window-handle` traits. Vulkan types never appear in the public API;
//! the in-house RHI is the private `vulkan` module.
//!
//! Ownership, boundaries and rules are in this crate's `AGENTS.md`.

mod error;
#[cfg(feature = "vulkan")]
mod renderer;
mod shader;
#[cfg(feature = "vulkan")]
mod vulkan;
// The policy modules are used by the backend and compiled for their tests.
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
mod presentation;
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
mod selection;
mod types;

pub use error::{RenderError, RenderErrorKind};
#[cfg(feature = "vulkan")]
pub use renderer::Renderer;
pub use shader::{
    CookedShader, DescriptorBinding, DescriptorKind, MAX_COOKED_SHADER_BYTES,
    MAX_DESCRIPTOR_BINDINGS, MAX_ENTRY_POINT_BYTES, MAX_PUSH_CONSTANT_BYTES, MAX_SPIRV_WORDS,
    MAX_VERTEX_ATTRIBUTES, PipelineLayoutDesc, PushConstantRange, RUNTIME_DESCRIPTOR_COUNT,
    SHADER_FORMAT_VERSION, SHADER_MAGIC, SPIRV_MAGIC, ShaderLayoutExpectation, ShaderStage,
    ShaderStageFlags, VertexFormat, VertexInputAttribute, validate_graphics_pipeline_shaders,
};
pub use types::{
    ClearColor, FrameOutcome, GpuKind, PresentModePreference, PresentationState,
    RendererDescriptor, RendererInfo, SurfaceSize, Validation, ValidationMessage,
    ValidationSeverity,
};
