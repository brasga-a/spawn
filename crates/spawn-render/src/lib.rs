//! `spawn-render`: GPU abstraction, resources, shaders, materials, frame execution.
//!
//! Always available (no GPU code):
//!
//! - [`SurfaceSize`], [`ClearColor`], [`RendererDescriptor`],
//!   [`PresentModePreference`], [`Validation`], [`FrameOutcome`],
//!   [`PresentationState`], [`RendererInfo`], [`ValidationMessage`].
//! - GPU memory budget and diagnostics: [`GpuMemoryBudget`], [`GpuAllocatorStats`],
//!   [`REVERSED_Z_CLEAR_DEPTH`].
//! - Generational resource handles and pipeline/offscreen descriptors (`SPN-INV-035`):
//!   [`ShaderHandle`], [`PipelineHandle`], [`OffscreenTargetHandle`],
//!   [`MeshHandle`], [`PipelineTarget`], [`FrontFace`], [`CullMode`],
//!   [`GraphicsPipelineDescriptor`], [`OffscreenReadback`].
//! - Static 3D mesh descriptors and camera-relative draw contracts (`SPN-INV-052`,
//!   `SPN-INV-053`): [`MeshVertex`], [`MeshDescriptor`], [`CameraRelativeView`],
//!   [`MeshDraw`], [`PackedMeshPushConstants`], [`MESH_VERTEX_STRIDE`],
//!   [`MESH_VERTEX_ATTRIBUTES`], [`MESH_PUSH_CONSTANT_BYTES`],
//!   [`MESH_PUSH_CONSTANT_RANGE`].
//! - Cooked-shader binary contract and untrusted layout validator (ADR-0014):
//!   [`CookedShader`], [`ShaderStage`], [`ShaderStageFlags`], [`VertexFormat`],
//!   [`VertexInputAttribute`], [`DescriptorKind`], [`DescriptorBinding`],
//!   [`PushConstantRange`], [`ShaderLayoutExpectation`], [`PipelineLayoutDesc`],
//!   [`validate_graphics_pipeline_shaders`].
//! - The located, coded [`RenderError`] (ADR-0009).
//!
//! With the `vulkan` Cargo feature, the Vulkan 1.3 backend (ADR-0004) adds
//! `Renderer`: it manages in-house GPU block suballocation, swapchain-matched
//! `D32_SFLOAT` reversed-Z depth attachments, `dynamicRendering` graphics
//! pipelines, static 3D vertex/index staging uploads, camera-relative indexed
//! mesh draws, offscreen RGBA8 readback targets, and presentation to a window
//! given through the `raw-window-handle` traits (or a headless surface). Vulkan
//! and `bytemuck` types never appear in the public API; the in-house RHI is the
//! private `vulkan` module.
//!
//! Ownership, boundaries and rules are in this crate's `AGENTS.md`.

#![cfg_attr(not(feature = "vulkan"), forbid(unsafe_code))]

mod allocator;
mod depth;
mod error;
mod handle;
mod mesh;
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

pub use allocator::{
    DEFAULT_DEVICE_LOCAL_BLOCK_BYTES, DEFAULT_DEVICE_LOCAL_BUDGET_BYTES,
    DEFAULT_HOST_VISIBLE_BLOCK_BYTES, DEFAULT_HOST_VISIBLE_BUDGET_BYTES, GpuAllocatorStats,
    GpuMemoryBudget,
};
pub use depth::REVERSED_Z_CLEAR_DEPTH;
pub use error::{RenderError, RenderErrorKind};
pub use handle::{MeshHandle, OffscreenTargetHandle, PipelineHandle, ShaderHandle};
pub use mesh::{
    CameraRelativeView, MAX_MESH_INDICES, MAX_MESH_VERTICES, MESH_PUSH_CONSTANT_BYTES,
    MESH_PUSH_CONSTANT_RANGE, MESH_VERTEX_ATTRIBUTES, MESH_VERTEX_STRIDE, MeshDescriptor, MeshDraw,
    MeshVertex, PackedMeshPushConstants,
};
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
    ClearColor, CullMode, FrameOutcome, FrontFace, GpuKind, GraphicsPipelineDescriptor,
    OffscreenReadback, PipelineTarget, PresentModePreference, PresentationState,
    RendererDescriptor, RendererInfo, SurfaceSize, Validation, ValidationMessage,
    ValidationSeverity,
};
