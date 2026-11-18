//! Versioned Spawn cooked-shader binary contract, untrusted decoder, and
//! pipeline-layout compatibility validator (ADR-0014, `SPN-INV-034`, `SPN-INV-083`).
//!
//! Always compiled (`default-features = false`), GPU-free, `#![forbid(unsafe_code)]`,
//! and `#![deny(clippy::indexing_slicing)]`. Both `spawn-cli` (when emitting
//! offline-cooked shader bundles) and `spawn-render` (before creating GPU shader
//! modules and pipelines) share this module so header framing and layout checks
//! live in one place.

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use std::collections::BTreeSet;
use std::fmt;
use std::ops::{BitOr, BitOrAssign};

use crate::error::{RenderError, RenderErrorKind};

/// 8-byte file magic identifying a Spawn cooked-shader artifact (`"SPNSHDR\0"`).
pub const SHADER_MAGIC: [u8; 8] = *b"SPNSHDR\0";

/// Current cooked-shader binary format version (`SPN-INV-080`).
pub const SHADER_FORMAT_VERSION: u32 = 1;

/// Standard Khronos SPIR-V little-endian magic number (`0x0723_0203`).
pub const SPIRV_MAGIC: u32 = 0x0723_0203;

/// Reversed-endian SPIR-V magic number, used only to produce a clear diagnostic.
const SPIRV_MAGIC_SWAPPED: u32 = 0x0302_2307;

/// Maximum byte size accepted for an untrusted cooked-shader payload (4 MiB).
pub const MAX_COOKED_SHADER_BYTES: usize = 4 * 1024 * 1024;

/// Maximum UTF-8 byte length of a shader entry-point identifier.
pub const MAX_ENTRY_POINT_BYTES: usize = 256;

/// Maximum number of reflected descriptor bindings in a single shader artifact.
pub const MAX_DESCRIPTOR_BINDINGS: usize = 128;

/// Maximum number of reflected stage vertex input attributes in a vertex shader.
pub const MAX_VERTEX_ATTRIBUTES: usize = 32;

/// Maximum push-constant block byte size accepted in a cooked shader.
pub const MAX_PUSH_CONSTANT_BYTES: u32 = 256;

/// Maximum SPIR-V 32-bit word count accepted in a cooked shader.
pub const MAX_SPIRV_WORDS: usize = MAX_COOKED_SHADER_BYTES / 4;

/// Sentinel descriptor count representing an unbounded runtime descriptor array (`OpTypeRuntimeArray`).
pub const RUNTIME_DESCRIPTOR_COUNT: u32 = u32::MAX;

/// Fixed binary header size in bytes before variable-length sections.
const HEADER_BYTES: usize = 40;
/// Encoded byte size of one [`DescriptorBinding`] record (`5 * u32`).
const DESCRIPTOR_RECORD_BYTES: usize = 20;
/// Encoded byte size of one [`VertexInputAttribute`] record (`2 * u32`).
const VERTEX_ATTR_RECORD_BYTES: usize = 8;
/// Minimum SPIR-V module length in 32-bit words (5-word header).
const MIN_SPIRV_HEADER_WORDS: usize = 5;
/// Upper sanity bound on SPIR-V `<id>` bound header field.
const MAX_SPIRV_ID_BOUND: u32 = 1_000_000;
/// SPIR-V `OpEntryPoint` opcode.
const SPIRV_OP_ENTRY_POINT: u16 = 15;

/// Programmable shader stage represented by a [`CookedShader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShaderStage {
    /// Vertex stage (`[shader("vertex")]`).
    Vertex,
    /// Fragment / pixel stage (`[shader("fragment")]`).
    Fragment,
    /// Compute stage (`[shader("compute")]`).
    Compute,
}

impl ShaderStage {
    /// Stable wire tag used in cooked-shader headers.
    const fn to_wire(self) -> u32 {
        match self {
            Self::Vertex => 1,
            Self::Fragment => 2,
            Self::Compute => 3,
        }
    }

    /// Decodes a wire tag into a [`ShaderStage`].
    const fn from_wire(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::Vertex),
            2 => Some(Self::Fragment),
            3 => Some(Self::Compute),
            _ => None,
        }
    }

    /// SPIR-V `ExecutionModel` word corresponding to this stage.
    const fn spirv_execution_model(self) -> u32 {
        match self {
            Self::Vertex => 0,
            Self::Fragment => 4,
            Self::Compute => 5,
        }
    }

    /// Lowercase canonical stage name (`"vertex"`, `"fragment"`, `"compute"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Fragment => "fragment",
            Self::Compute => "compute",
        }
    }

    /// Single-bit [`ShaderStageFlags`] mask for this stage.
    pub const fn stage_flag(self) -> ShaderStageFlags {
        match self {
            Self::Vertex => ShaderStageFlags::VERTEX,
            Self::Fragment => ShaderStageFlags::FRAGMENT,
            Self::Compute => ShaderStageFlags::COMPUTE,
        }
    }
}

impl fmt::Display for ShaderStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bitmask of [`ShaderStage`]s accessing a push-constant block or descriptor binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ShaderStageFlags(u32);

impl ShaderStageFlags {
    /// Empty stage mask (`0`).
    pub const NONE: Self = Self(0);
    /// Vertex stage bit (`1 << 0`).
    pub const VERTEX: Self = Self(1 << 0);
    /// Fragment stage bit (`1 << 1`).
    pub const FRAGMENT: Self = Self(1 << 1);
    /// Compute stage bit (`1 << 2`).
    pub const COMPUTE: Self = Self(1 << 2);
    /// Vertex and fragment stages combined.
    pub const ALL_GRAPHICS: Self = Self(Self::VERTEX.0 | Self::FRAGMENT.0);
    /// All supported programmable shader stages.
    pub const ALL: Self = Self(Self::VERTEX.0 | Self::FRAGMENT.0 | Self::COMPUTE.0);

    /// Empty stage mask (`0`).
    pub const fn empty() -> Self {
        Self::NONE
    }

    /// Raw bit representation.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Constructs flags from raw bits, returning `None` if unknown bits are set.
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if (bits & !Self::ALL.0) == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// Returns `true` when no stage bit is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns `true` when every stage bit in `other` is present in `self`.
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Bitwise union of two stage masks.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOr for ShaderStageFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for ShaderStageFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Scalar or vector element format of a vertex shader stage input attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VertexFormat {
    /// 32-bit IEEE float (`f32`, 4 bytes).
    Float32,
    /// 2-component 32-bit IEEE float vector (`[f32; 2]`, 8 bytes).
    Float32x2,
    /// 3-component 32-bit IEEE float vector (`[f32; 3]`, 12 bytes).
    Float32x3,
    /// 4-component 32-bit IEEE float vector (`[f32; 4]`, 16 bytes).
    Float32x4,
    /// 32-bit signed integer (`i32`, 4 bytes).
    Sint32,
    /// 2-component 32-bit signed integer vector (`[i32; 2]`, 8 bytes).
    Sint32x2,
    /// 3-component 32-bit signed integer vector (`[i32; 3]`, 12 bytes).
    Sint32x3,
    /// 4-component 32-bit signed integer vector (`[i32; 4]`, 16 bytes).
    Sint32x4,
    /// 32-bit unsigned integer (`u32`, 4 bytes).
    Uint32,
    /// 2-component 32-bit unsigned integer vector (`[u32; 2]`, 8 bytes).
    Uint32x2,
    /// 3-component 32-bit unsigned integer vector (`[u32; 3]`, 12 bytes).
    Uint32x3,
    /// 4-component 32-bit unsigned integer vector (`[u32; 4]`, 16 bytes).
    Uint32x4,
}

impl VertexFormat {
    const fn to_wire(self) -> u32 {
        match self {
            Self::Float32 => 1,
            Self::Float32x2 => 2,
            Self::Float32x3 => 3,
            Self::Float32x4 => 4,
            Self::Sint32 => 5,
            Self::Sint32x2 => 6,
            Self::Sint32x3 => 7,
            Self::Sint32x4 => 8,
            Self::Uint32 => 9,
            Self::Uint32x2 => 10,
            Self::Uint32x3 => 11,
            Self::Uint32x4 => 12,
        }
    }

    const fn from_wire(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::Float32),
            2 => Some(Self::Float32x2),
            3 => Some(Self::Float32x3),
            4 => Some(Self::Float32x4),
            5 => Some(Self::Sint32),
            6 => Some(Self::Sint32x2),
            7 => Some(Self::Sint32x3),
            8 => Some(Self::Sint32x4),
            9 => Some(Self::Uint32),
            10 => Some(Self::Uint32x2),
            11 => Some(Self::Uint32x3),
            12 => Some(Self::Uint32x4),
            _ => None,
        }
    }

    /// Size of a single attribute value in bytes.
    pub const fn byte_size(self) -> u32 {
        match self {
            Self::Float32 | Self::Sint32 | Self::Uint32 => 4,
            Self::Float32x2 | Self::Sint32x2 | Self::Uint32x2 => 8,
            Self::Float32x3 | Self::Sint32x3 | Self::Uint32x3 => 12,
            Self::Float32x4 | Self::Sint32x4 | Self::Uint32x4 => 16,
        }
    }

    /// Human-readable format name for diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Float32 => "Float32",
            Self::Float32x2 => "Float32x2",
            Self::Float32x3 => "Float32x3",
            Self::Float32x4 => "Float32x4",
            Self::Sint32 => "Sint32",
            Self::Sint32x2 => "Sint32x2",
            Self::Sint32x3 => "Sint32x3",
            Self::Sint32x4 => "Sint32x4",
            Self::Uint32 => "Uint32",
            Self::Uint32x2 => "Uint32x2",
            Self::Uint32x3 => "Uint32x3",
            Self::Uint32x4 => "Uint32x4",
        }
    }
}

impl fmt::Display for VertexFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reflected or expected vertex input attribute `(location, format)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VertexInputAttribute {
    /// Shader input location index (`[[vk::location(N)]]`).
    pub location: u32,
    /// Attribute component format.
    pub format: VertexFormat,
}

impl VertexInputAttribute {
    /// Creates a vertex input attribute declaration.
    pub const fn new(location: u32, format: VertexFormat) -> Self {
        Self { location, format }
    }
}

/// Kind of resource bound at a shader descriptor slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DescriptorKind {
    /// Constant / uniform buffer (`ConstantBuffer<T>`).
    UniformBuffer,
    /// Read-only or read-write storage buffer (`StructuredBuffer<T>`, `RWStructuredBuffer<T>`).
    StorageBuffer,
    /// Sampled texture without an embedded sampler (`Texture2D<T>`, etc.).
    SampledImage,
    /// Read/write storage image (`RWTexture2D<T>`, etc.).
    StorageImage,
    /// Standalone texture sampler (`SamplerState`, `SamplerComparisonState`).
    Sampler,
    /// Combined texture and sampler (`Sampler2D<T>`, etc.).
    CombinedImageSampler,
}

impl DescriptorKind {
    const fn to_wire(self) -> u32 {
        match self {
            Self::UniformBuffer => 1,
            Self::StorageBuffer => 2,
            Self::SampledImage => 3,
            Self::StorageImage => 4,
            Self::Sampler => 5,
            Self::CombinedImageSampler => 6,
        }
    }

    const fn from_wire(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::UniformBuffer),
            2 => Some(Self::StorageBuffer),
            3 => Some(Self::SampledImage),
            4 => Some(Self::StorageImage),
            5 => Some(Self::Sampler),
            6 => Some(Self::CombinedImageSampler),
            _ => None,
        }
    }

    /// Human-readable descriptor kind name for diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UniformBuffer => "UniformBuffer",
            Self::StorageBuffer => "StorageBuffer",
            Self::SampledImage => "SampledImage",
            Self::StorageImage => "StorageImage",
            Self::Sampler => "Sampler",
            Self::CombinedImageSampler => "CombinedImageSampler",
        }
    }
}

impl fmt::Display for DescriptorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reflected or expected descriptor binding `(set, binding, kind, count, stages)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DescriptorBinding {
    /// Descriptor set index.
    pub set: u32,
    /// Binding index within `set`.
    pub binding: u32,
    /// Kind of bound resource.
    pub kind: DescriptorKind,
    /// Element count (`1` for non-arrays, `N` for fixed arrays, [`RUNTIME_DESCRIPTOR_COUNT`] for runtime arrays).
    pub count: u32,
    /// Shader stages accessing or declaring this binding.
    pub stages: ShaderStageFlags,
}

/// Push-constant byte size and stage visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PushConstantRange {
    /// Size of the push-constant block in bytes (`0` if unused; multiple of 4 when `> 0`).
    pub size: u32,
    /// Shader stages accessing the push-constant block (`ShaderStageFlags::NONE` when `size == 0`).
    pub stages: ShaderStageFlags,
}

impl PushConstantRange {
    /// Empty push-constant range (`size = 0`, `stages = NONE`).
    pub const NONE: Self = Self {
        size: 0,
        stages: ShaderStageFlags::NONE,
    };

    /// Creates a push-constant range.
    pub const fn new(size: u32, stages: ShaderStageFlags) -> Self {
        Self { size, stages }
    }
}

/// Versioned cooked shader artifact combining reflection metadata and validated SPIR-V words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookedShader {
    /// Binary format version (`SHADER_FORMAT_VERSION`).
    pub version: u32,
    /// Programmable stage compiled into `spirv_words`.
    pub stage: ShaderStage,
    /// Entry-point function name inside the SPIR-V module.
    pub entry_point: String,
    /// Reflected push-constant block requirement (`size == 0` when unused).
    pub push_constants: PushConstantRange,
    /// Reflected descriptor bindings, strictly sorted by `(set, binding)`.
    pub descriptor_bindings: Vec<DescriptorBinding>,
    /// Reflected stage vertex inputs, strictly sorted by `location` (empty for non-vertex stages).
    pub vertex_inputs: Vec<VertexInputAttribute>,
    /// 4-byte-aligned little-endian SPIR-V 32-bit word stream.
    pub spirv_words: Vec<u32>,
}

/// Expected interface for validating a single [`CookedShader`] stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderLayoutExpectation<'a> {
    /// Expected programmable shader stage.
    pub stage: ShaderStage,
    /// Expected vertex input attributes (must be empty when `stage != ShaderStage::Vertex`).
    pub vertex_inputs: &'a [VertexInputAttribute],
    /// Expected push-constant range for this stage.
    pub push_constants: PushConstantRange,
    /// Expected descriptor bindings available to this stage.
    pub descriptor_bindings: &'a [DescriptorBinding],
}

/// Expected pipeline layout interface for a vertex + fragment graphics pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineLayoutDesc<'a> {
    /// Vertex attributes supplied by the pipeline's vertex input bindings.
    pub vertex_inputs: &'a [VertexInputAttribute],
    /// Push-constant range declared on the pipeline layout.
    pub push_constants: PushConstantRange,
    /// Descriptor bindings declared on the pipeline layout.
    pub descriptor_bindings: &'a [DescriptorBinding],
}

impl CookedShader {
    /// Validates all structural, reflection, size-cap, and SPIR-V framing/entry-point
    /// invariants on `self`.
    #[track_caller]
    pub fn validate(&self) -> Result<(), RenderError> {
        self.validate_with_context(None)
    }

    /// Encodes `self` into the canonical versioned Spawn cooked-shader binary format
    /// after validating all invariants.
    #[track_caller]
    pub fn encode(&self) -> Result<Vec<u8>, RenderError> {
        self.validate_with_context(None)?;

        let total_bytes = compute_encoded_byte_len(
            self.entry_point.len(),
            self.descriptor_bindings.len(),
            self.vertex_inputs.len(),
            self.spirv_words.len(),
            None,
        )?;

        let mut out = Vec::with_capacity(total_bytes);
        out.extend_from_slice(&SHADER_MAGIC);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.stage.to_wire().to_le_bytes());
        out.extend_from_slice(&(self.entry_point.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.push_constants.size.to_le_bytes());
        out.extend_from_slice(&self.push_constants.stages.bits().to_le_bytes());
        out.extend_from_slice(&(self.descriptor_bindings.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.vertex_inputs.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.spirv_words.len() as u32).to_le_bytes());

        let entry_bytes = self.entry_point.as_bytes();
        out.extend_from_slice(entry_bytes);
        let pad_len = padded_4(entry_bytes.len()).saturating_sub(entry_bytes.len());
        out.extend(std::iter::repeat_n(0, pad_len));

        for binding in &self.descriptor_bindings {
            out.extend_from_slice(&binding.set.to_le_bytes());
            out.extend_from_slice(&binding.binding.to_le_bytes());
            out.extend_from_slice(&binding.kind.to_wire().to_le_bytes());
            out.extend_from_slice(&binding.count.to_le_bytes());
            out.extend_from_slice(&binding.stages.bits().to_le_bytes());
        }

        for attr in &self.vertex_inputs {
            out.extend_from_slice(&attr.location.to_le_bytes());
            out.extend_from_slice(&attr.format.to_wire().to_le_bytes());
        }

        for &word in &self.spirv_words {
            out.extend_from_slice(&word.to_le_bytes());
        }

        debug_assert_eq!(out.len(), total_bytes);
        Ok(out)
    }

    /// Decodes and validates an untrusted cooked-shader byte slice (`SPN-INV-034`).
    ///
    /// Size caps, 4-byte alignment, and exact section byte lengths are verified
    /// before allocating any variable-length collections.
    #[track_caller]
    pub fn decode(bytes: &[u8]) -> Result<Self, RenderError> {
        Self::decode_inner(bytes, None)
    }

    /// Decodes and validates an untrusted cooked-shader byte slice, prefixing
    /// `label` (such as an artifact path) onto any error detail (`SPN-INV-081`).
    #[track_caller]
    pub fn decode_named(label: &str, bytes: &[u8]) -> Result<Self, RenderError> {
        Self::decode_inner(bytes, Some(label))
    }

    #[track_caller]
    fn decode_inner(bytes: &[u8], label: Option<&str>) -> Result<Self, RenderError> {
        if bytes.len() > MAX_COOKED_SHADER_BYTES {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `payload`: size {} bytes exceeds maximum {} bytes",
                    bytes.len(),
                    MAX_COOKED_SHADER_BYTES
                ),
            ));
        }
        if bytes.len() < HEADER_BYTES {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `header`: truncated artifact of {} bytes (minimum {} bytes)",
                    bytes.len(),
                    HEADER_BYTES
                ),
            ));
        }
        if !bytes.len().is_multiple_of(4) {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `payload`: byte length {} is not 4-byte aligned",
                    bytes.len()
                ),
            ));
        }

        let magic = bytes
            .get(0..8)
            .ok_or_else(|| invalid_artifact(label, "field `magic`: missing header magic"))?;
        if magic != SHADER_MAGIC {
            return Err(invalid_artifact(
                label,
                format!("field `magic`: expected `SPNSHDR\\0`, found {:02x?}", magic),
            ));
        }

        let version = read_u32_le(bytes, 8, label, "version")?;
        if version != SHADER_FORMAT_VERSION {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `version`: unsupported cooked-shader version {version} (expected {SHADER_FORMAT_VERSION})"
                ),
            ));
        }

        let stage_raw = read_u32_le(bytes, 12, label, "stage")?;
        let stage = ShaderStage::from_wire(stage_raw).ok_or_else(|| {
            invalid_artifact(
                label,
                format!("field `stage`: invalid stage discriminant {stage_raw}"),
            )
        })?;

        let entry_len_u32 = read_u32_le(bytes, 16, label, "entry_point_len")?;
        let entry_len = usize::try_from(entry_len_u32).map_err(|_| {
            invalid_artifact(
                label,
                format!("field `entry_point_len`: length {entry_len_u32} overflows usize"),
            )
        })?;
        if entry_len == 0 || entry_len > MAX_ENTRY_POINT_BYTES {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `entry_point_len`: length {entry_len} outside 1..={MAX_ENTRY_POINT_BYTES}"
                ),
            ));
        }

        let push_size = read_u32_le(bytes, 20, label, "push_constant_size")?;
        let push_stages_raw = read_u32_le(bytes, 24, label, "push_constant_stages")?;
        let push_stages = ShaderStageFlags::from_bits(push_stages_raw).ok_or_else(|| {
            invalid_artifact(
                label,
                format!("field `push_constant_stages`: invalid stage flags {push_stages_raw:#x}"),
            )
        })?;

        let desc_count = read_bounded_count(
            bytes,
            28,
            MAX_DESCRIPTOR_BINDINGS,
            label,
            "descriptor_binding_count",
        )?;
        let attr_count = read_bounded_count(
            bytes,
            32,
            MAX_VERTEX_ATTRIBUTES,
            label,
            "vertex_input_count",
        )?;
        let spirv_word_count =
            read_bounded_count(bytes, 36, MAX_SPIRV_WORDS, label, "spirv_word_count")?;
        if spirv_word_count < MIN_SPIRV_HEADER_WORDS {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `spirv_word_count`: count {spirv_word_count} below minimum SPIR-V header ({MIN_SPIRV_HEADER_WORDS} words)"
                ),
            ));
        }

        // Verify exact payload framing BEFORE allocating any String or Vec (SPN-INV-034).
        let expected_total_bytes =
            compute_encoded_byte_len(entry_len, desc_count, attr_count, spirv_word_count, label)?;
        if bytes.len() < expected_total_bytes {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `payload`: truncated artifact of {} bytes (header requires {} bytes)",
                    bytes.len(),
                    expected_total_bytes
                ),
            ));
        }
        if bytes.len() > expected_total_bytes {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `payload`: unexpected trailing bytes ({} bytes found, expected {} bytes)",
                    bytes.len(),
                    expected_total_bytes
                ),
            ));
        }

        let mut cursor = HEADER_BYTES;
        let padded_entry_len = padded_4(entry_len);
        let entry_end = cursor
            .checked_add(entry_len)
            .ok_or_else(|| invalid_artifact(label, "field `entry_point`: byte offset overflow"))?;
        let padded_entry_end = cursor.checked_add(padded_entry_len).ok_or_else(|| {
            invalid_artifact(label, "field `entry_point`: padded offset overflow")
        })?;
        let entry_raw = bytes
            .get(cursor..entry_end)
            .ok_or_else(|| invalid_artifact(label, "field `entry_point`: slice out of bounds"))?;
        let entry_pad = bytes.get(entry_end..padded_entry_end).ok_or_else(|| {
            invalid_artifact(label, "field `entry_point`: padding slice out of bounds")
        })?;
        if entry_pad.iter().any(|&b| b != 0) {
            return Err(invalid_artifact(
                label,
                "field `entry_point`: non-zero alignment padding bytes",
            ));
        }
        let entry_str = std::str::from_utf8(entry_raw)
            .map_err(|_| invalid_artifact(label, "field `entry_point`: invalid UTF-8 sequence"))?;
        let entry_point = entry_str.to_owned();
        cursor = padded_entry_end;

        let desc_bytes_len = desc_count
            .checked_mul(DESCRIPTOR_RECORD_BYTES)
            .ok_or_else(|| invalid_artifact(label, "field `descriptor_bindings`: size overflow"))?;
        let desc_end = cursor.checked_add(desc_bytes_len).ok_or_else(|| {
            invalid_artifact(label, "field `descriptor_bindings`: offset overflow")
        })?;
        let desc_slice = bytes.get(cursor..desc_end).ok_or_else(|| {
            invalid_artifact(label, "field `descriptor_bindings`: slice out of bounds")
        })?;
        let mut descriptor_bindings = Vec::with_capacity(desc_count);
        for (idx, chunk) in desc_slice
            .as_chunks::<DESCRIPTOR_RECORD_BYTES>()
            .0
            .iter()
            .enumerate()
        {
            let set = read_u32_le(chunk, 0, label, "descriptor_binding.set")?;
            let binding = read_u32_le(chunk, 4, label, "descriptor_binding.binding")?;
            let kind_raw = read_u32_le(chunk, 8, label, "descriptor_binding.kind")?;
            let kind = DescriptorKind::from_wire(kind_raw).ok_or_else(|| {
                invalid_artifact(
                    label,
                    format!(
                        "field `descriptor_bindings[{idx}].kind`: invalid kind discriminant {kind_raw}"
                    ),
                )
            })?;
            let count = read_u32_le(chunk, 12, label, "descriptor_binding.count")?;
            let stages_raw = read_u32_le(chunk, 16, label, "descriptor_binding.stages")?;
            let stages = ShaderStageFlags::from_bits(stages_raw).ok_or_else(|| {
                invalid_artifact(
                    label,
                    format!(
                        "field `descriptor_bindings[{idx}].stages`: invalid stage flags {stages_raw:#x}"
                    ),
                )
            })?;
            descriptor_bindings.push(DescriptorBinding {
                set,
                binding,
                kind,
                count,
                stages,
            });
        }
        cursor = desc_end;

        let attr_bytes_len = attr_count
            .checked_mul(VERTEX_ATTR_RECORD_BYTES)
            .ok_or_else(|| invalid_artifact(label, "field `vertex_inputs`: size overflow"))?;
        let attr_end = cursor
            .checked_add(attr_bytes_len)
            .ok_or_else(|| invalid_artifact(label, "field `vertex_inputs`: offset overflow"))?;
        let attr_slice = bytes
            .get(cursor..attr_end)
            .ok_or_else(|| invalid_artifact(label, "field `vertex_inputs`: slice out of bounds"))?;
        let mut vertex_inputs = Vec::with_capacity(attr_count);
        for (idx, chunk) in attr_slice
            .as_chunks::<VERTEX_ATTR_RECORD_BYTES>()
            .0
            .iter()
            .enumerate()
        {
            let location = read_u32_le(chunk, 0, label, "vertex_input.location")?;
            let format_raw = read_u32_le(chunk, 4, label, "vertex_input.format")?;
            let format = VertexFormat::from_wire(format_raw).ok_or_else(|| {
                invalid_artifact(
                    label,
                    format!(
                        "field `vertex_inputs[{idx}].format`: invalid format discriminant {format_raw}"
                    ),
                )
            })?;
            vertex_inputs.push(VertexInputAttribute { location, format });
        }
        cursor = attr_end;

        let spirv_bytes_len = spirv_word_count
            .checked_mul(4)
            .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: size overflow"))?;
        let spirv_end = cursor
            .checked_add(spirv_bytes_len)
            .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: offset overflow"))?;
        let spirv_slice = bytes
            .get(cursor..spirv_end)
            .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: slice out of bounds"))?;
        let mut spirv_words = Vec::with_capacity(spirv_word_count);
        for &chunk in spirv_slice.as_chunks::<4>().0 {
            spirv_words.push(u32::from_le_bytes(chunk));
        }

        let shader = Self {
            version,
            stage,
            entry_point,
            push_constants: PushConstantRange {
                size: push_size,
                stages: push_stages,
            },
            descriptor_bindings,
            vertex_inputs,
            spirv_words,
        };
        shader.validate_with_context(label)?;
        Ok(shader)
    }

    /// Validates that `self`'s reflected stage, vertex inputs, push-constant range,
    /// and descriptor bindings are compatible with `expected` before GPU object creation.
    ///
    /// Compatibility rules match Vulkan 1.3 pipeline layout rules (`ADR-0014`):
    /// - Stage and vertex attributes (`location`, `format`) must match `expected` exactly.
    /// - Push constants follow a compatible-superset rule (`VUID-VkGraphicsPipelineCreateInfo-layout-07987`):
    ///   if `self.push_constants.size == 0`, any valid `expected.push_constants` range is accepted;
    ///   if `self.push_constants.size > 0`, `expected.push_constants.size` must be `>= self.push_constants.size`
    ///   (allowing std430 / `#[repr(C)]` struct tail padding and shared multi-shader layouts) and
    ///   `expected.push_constants.stages` must include `self.stage`.
    /// - Descriptor bindings follow a compatible-superset rule: every `(set, binding)` statically
    ///   used by `self` must exist in `expected.descriptor_bindings` with matching kind, count,
    ///   and stage visibility including `self.stage`.
    #[track_caller]
    pub fn validate_against(
        &self,
        expected: &ShaderLayoutExpectation<'_>,
    ) -> Result<(), RenderError> {
        self.validate_with_context(None)?;

        if self.stage != expected.stage {
            return Err(layout_mismatch(format!(
                "entry `{}`: shader stage is `{}`, expected `{}`",
                self.entry_point, self.stage, expected.stage
            )));
        }

        validate_expected_push_constants_def(expected.push_constants)?;
        if expected.stage != ShaderStage::Vertex && !expected.vertex_inputs.is_empty() {
            return Err(layout_mismatch(format!(
                "stage `{}`: non-vertex expectation cannot declare {} vertex input(s)",
                expected.stage,
                expected.vertex_inputs.len()
            )));
        }

        let expected_attrs = canonicalize_vertex_inputs(expected.vertex_inputs)?;
        if self.stage == ShaderStage::Vertex {
            validate_vertex_inputs_equal(&self.entry_point, &self.vertex_inputs, &expected_attrs)?;
        }

        validate_shader_push_constants_against_layout(self, expected.push_constants)?;

        let expected_bindings = canonicalize_descriptor_bindings(expected.descriptor_bindings)?;
        validate_shader_bindings_against_layout(
            &self.entry_point,
            self.stage,
            &self.descriptor_bindings,
            &expected_bindings,
        )?;
        Ok(())
    }

    #[track_caller]
    fn validate_with_context(&self, label: Option<&str>) -> Result<(), RenderError> {
        if self.version != SHADER_FORMAT_VERSION {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `version`: unsupported cooked-shader version {} (expected {SHADER_FORMAT_VERSION})",
                    self.version
                ),
            ));
        }

        let entry_bytes = self.entry_point.as_bytes();
        if entry_bytes.is_empty() || entry_bytes.len() > MAX_ENTRY_POINT_BYTES {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `entry_point`: length {} bytes outside 1..={MAX_ENTRY_POINT_BYTES}",
                    entry_bytes.len()
                ),
            ));
        }
        if entry_bytes.iter().any(|&b| b == 0 || b.is_ascii_control()) {
            return Err(invalid_artifact(
                label,
                "field `entry_point`: contains NUL or ASCII control character",
            ));
        }

        if ShaderStageFlags::from_bits(self.push_constants.stages.bits()).is_none() {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `push_constants.stages`: invalid stage flags {:#x}",
                    self.push_constants.stages.bits()
                ),
            ));
        }
        if self.push_constants.size == 0 {
            if !self.push_constants.stages.is_empty() {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `push_constants`: size is 0 but stages mask is non-empty ({:#x})",
                        self.push_constants.stages.bits()
                    ),
                ));
            }
        } else {
            if self.push_constants.size > MAX_PUSH_CONSTANT_BYTES {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `push_constants.size`: {} bytes exceeds maximum {MAX_PUSH_CONSTANT_BYTES} bytes",
                        self.push_constants.size
                    ),
                ));
            }
            if !self.push_constants.size.is_multiple_of(4) {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `push_constants.size`: {} bytes is not a multiple of 4",
                        self.push_constants.size
                    ),
                ));
            }
            if !self.push_constants.stages.contains(self.stage.stage_flag()) {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `push_constants.stages`: mask {:#x} does not include shader stage `{}`",
                        self.push_constants.stages.bits(),
                        self.stage
                    ),
                ));
            }
        }

        if self.descriptor_bindings.len() > MAX_DESCRIPTOR_BINDINGS {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `descriptor_bindings`: count {} exceeds maximum {MAX_DESCRIPTOR_BINDINGS}",
                    self.descriptor_bindings.len()
                ),
            ));
        }
        let mut prev_slot: Option<(u32, u32)> = None;
        for (idx, binding) in self.descriptor_bindings.iter().enumerate() {
            if binding.count == 0 {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `descriptor_bindings[{idx}]`: descriptor `(set={}, binding={})` has invalid count 0",
                        binding.set, binding.binding
                    ),
                ));
            }
            if ShaderStageFlags::from_bits(binding.stages.bits()).is_none()
                || !binding.stages.contains(self.stage.stage_flag())
            {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `descriptor_bindings[{idx}]`: descriptor `(set={}, binding={})` stages {:#x} invalid for stage `{}`",
                        binding.set,
                        binding.binding,
                        binding.stages.bits(),
                        self.stage
                    ),
                ));
            }
            let slot = (binding.set, binding.binding);
            if let Some(prev) = prev_slot
                && slot <= prev
            {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `descriptor_bindings`: duplicate or unsorted `(set, binding)` {:?} after {:?}",
                        slot, prev
                    ),
                ));
            }
            prev_slot = Some(slot);
        }

        if self.stage != ShaderStage::Vertex && !self.vertex_inputs.is_empty() {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `vertex_inputs`: stage `{}` must have 0 vertex inputs, found {}",
                    self.stage,
                    self.vertex_inputs.len()
                ),
            ));
        }
        if self.vertex_inputs.len() > MAX_VERTEX_ATTRIBUTES {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `vertex_inputs`: count {} exceeds maximum {MAX_VERTEX_ATTRIBUTES}",
                    self.vertex_inputs.len()
                ),
            ));
        }
        let mut prev_location: Option<u32> = None;
        for attr in &self.vertex_inputs {
            if let Some(prev) = prev_location
                && attr.location <= prev
            {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `vertex_inputs`: duplicate or unsorted location {} after {}",
                        attr.location, prev
                    ),
                ));
            }
            prev_location = Some(attr.location);
        }

        let _ = compute_encoded_byte_len(
            self.entry_point.len(),
            self.descriptor_bindings.len(),
            self.vertex_inputs.len(),
            self.spirv_words.len(),
            label,
        )?;

        validate_spirv_stream(&self.spirv_words, self.stage, &self.entry_point, label)?;
        Ok(())
    }
}

/// Validates a cooked vertex shader and fragment shader pair against a
/// [`PipelineLayoutDesc`] before creating graphics pipeline objects (`ADR-0014`).
///
/// Vertex inputs must match `vertex` exactly, while push constants and descriptor
/// bindings follow Vulkan 1.3 compatible-superset rules (identical to calling
/// [`CookedShader::validate_against`] on each stage): stages with `push_constants.size == 0`
/// accept any valid pipeline push-constant range, and stages with `push_constants.size > 0`
/// require `layout.push_constants.size >= shader.push_constants.size` (accommodating
/// std430 / `#[repr(C)]` tail padding) with `layout.push_constants.stages` including
/// that shader's stage flag.
#[track_caller]
pub fn validate_graphics_pipeline_shaders(
    vertex: &CookedShader,
    fragment: &CookedShader,
    layout: &PipelineLayoutDesc<'_>,
) -> Result<(), RenderError> {
    vertex.validate()?;
    fragment.validate()?;

    if vertex.stage != ShaderStage::Vertex {
        return Err(layout_mismatch(format!(
            "vertex shader slot: got stage `{}` (entry `{}`), expected `vertex`",
            vertex.stage, vertex.entry_point
        )));
    }
    if fragment.stage != ShaderStage::Fragment {
        return Err(layout_mismatch(format!(
            "fragment shader slot: got stage `{}` (entry `{}`), expected `fragment`",
            fragment.stage, fragment.entry_point
        )));
    }

    validate_expected_push_constants_def(layout.push_constants)?;
    let expected_attrs = canonicalize_vertex_inputs(layout.vertex_inputs)?;
    validate_vertex_inputs_equal(&vertex.entry_point, &vertex.vertex_inputs, &expected_attrs)?;

    validate_shader_push_constants_against_layout(vertex, layout.push_constants)?;
    validate_shader_push_constants_against_layout(fragment, layout.push_constants)?;

    let expected_bindings = canonicalize_descriptor_bindings(layout.descriptor_bindings)?;
    validate_shader_bindings_against_layout(
        &vertex.entry_point,
        vertex.stage,
        &vertex.descriptor_bindings,
        &expected_bindings,
    )?;
    validate_shader_bindings_against_layout(
        &fragment.entry_point,
        fragment.stage,
        &fragment.descriptor_bindings,
        &expected_bindings,
    )?;
    Ok(())
}

#[track_caller]
fn validate_spirv_stream(
    words: &[u32],
    expected_stage: ShaderStage,
    expected_entry: &str,
    label: Option<&str>,
) -> Result<(), RenderError> {
    if words.len() < MIN_SPIRV_HEADER_WORDS || words.len() > MAX_SPIRV_WORDS {
        return Err(invalid_artifact(
            label,
            format!(
                "field `spirv_words`: word count {} outside {}..={MAX_SPIRV_WORDS}",
                words.len(),
                MIN_SPIRV_HEADER_WORDS
            ),
        ));
    }

    let magic = words
        .first()
        .copied()
        .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: missing magic word"))?;
    if magic == SPIRV_MAGIC_SWAPPED {
        return Err(invalid_artifact(
            label,
            "field `spirv_words[0]`: byte-swapped SPIR-V magic (big-endian stream is unsupported)",
        ));
    }
    if magic != SPIRV_MAGIC {
        return Err(invalid_artifact(
            label,
            format!(
                "field `spirv_words[0]`: invalid SPIR-V magic {magic:#010x} (expected {SPIRV_MAGIC:#010x})"
            ),
        ));
    }

    let version_word = words
        .get(1)
        .copied()
        .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: missing version word"))?;
    let major = (version_word >> 16) & 0xFF;
    let minor = (version_word >> 8) & 0xFF;
    if (version_word & 0xFF00_00FF) != 0 || major != 1 || minor > 6 {
        return Err(invalid_artifact(
            label,
            format!("field `spirv_words[1]`: unsupported SPIR-V version word {version_word:#010x}"),
        ));
    }

    let bound = words
        .get(3)
        .copied()
        .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: missing bound word"))?;
    if bound == 0 || bound > MAX_SPIRV_ID_BOUND {
        return Err(invalid_artifact(
            label,
            format!("field `spirv_words[3]`: invalid SPIR-V ID bound {bound}"),
        ));
    }

    let schema = words
        .get(4)
        .copied()
        .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: missing schema word"))?;
    if schema != 0 {
        return Err(invalid_artifact(
            label,
            format!("field `spirv_words[4]`: unsupported non-zero SPIR-V schema {schema}"),
        ));
    }

    let mut cursor = MIN_SPIRV_HEADER_WORDS;
    let mut found_matching_entry = false;
    let mut seen_entry_points: BTreeSet<(u32, String)> = BTreeSet::new();

    while cursor < words.len() {
        let first_word = words.get(cursor).copied().ok_or_else(|| {
            invalid_artifact(
                label,
                "field `spirv_words`: instruction cursor out of range",
            )
        })?;
        let word_count = usize::from((first_word >> 16) as u16);
        let opcode = (first_word & 0xFFFF) as u16;
        if word_count == 0 {
            return Err(invalid_artifact(
                label,
                format!(
                    "field `spirv_words[{cursor}]`: malformed SPIR-V instruction with word count 0"
                ),
            ));
        }
        let end = cursor.checked_add(word_count).ok_or_else(|| {
            invalid_artifact(
                label,
                "field `spirv_words`: instruction word count overflow",
            )
        })?;
        let inst_words = words.get(cursor..end).ok_or_else(|| {
            invalid_artifact(
                label,
                format!(
                    "field `spirv_words[{cursor}]`: truncated SPIR-V instruction (needs {word_count} words, {} remaining)",
                    words.len().saturating_sub(cursor)
                ),
            )
        })?;

        if opcode == SPIRV_OP_ENTRY_POINT {
            let (exec_model, name) = parse_spirv_entry_point(inst_words, bound, label)?;
            if !seen_entry_points.insert((exec_model, name.clone())) {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `spirv_words`: duplicate OpEntryPoint `(execution_model={exec_model}, name={name})` in SPIR-V module"
                    ),
                ));
            }
            if exec_model == expected_stage.spirv_execution_model() && name == expected_entry {
                found_matching_entry = true;
            }
        }
        cursor = end;
    }

    if !found_matching_entry {
        return Err(invalid_artifact(
            label,
            format!(
                "field `entry_point`: SPIR-V module has no `{}` OpEntryPoint named `{}`",
                expected_stage, expected_entry
            ),
        ));
    }

    Ok(())
}

#[track_caller]
fn parse_spirv_entry_point(
    inst_words: &[u32],
    bound: u32,
    label: Option<&str>,
) -> Result<(u32, String), RenderError> {
    if inst_words.len() < 4 {
        return Err(invalid_artifact(
            label,
            format!(
                "field `spirv_words`: malformed OpEntryPoint with {} words (minimum 4)",
                inst_words.len()
            ),
        ));
    }
    let exec_model = inst_words.get(1).copied().ok_or_else(|| {
        invalid_artifact(label, "field `spirv_words`: missing OpEntryPoint model")
    })?;
    let target_id = inst_words
        .get(2)
        .copied()
        .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: missing OpEntryPoint id"))?;
    if target_id == 0 || target_id >= bound {
        return Err(invalid_artifact(
            label,
            format!("field `spirv_words`: OpEntryPoint target id {target_id} outside 1..{bound}"),
        ));
    }

    let operand_words = inst_words
        .get(3..)
        .ok_or_else(|| invalid_artifact(label, "field `spirv_words`: missing OpEntryPoint name"))?;

    let mut name_bytes = Vec::new();
    let mut interface_start: Option<usize> = None;

    for (idx, &word) in operand_words.iter().enumerate() {
        let le = word.to_le_bytes();
        let mut nul_seen_in_word = false;
        for &byte in &le {
            if nul_seen_in_word {
                if byte != 0 {
                    return Err(invalid_artifact(
                        label,
                        "field `spirv_words`: non-zero byte after NUL terminator in OpEntryPoint name",
                    ));
                }
            } else if byte == 0 {
                nul_seen_in_word = true;
            } else {
                if name_bytes.len() >= MAX_ENTRY_POINT_BYTES {
                    return Err(invalid_artifact(
                        label,
                        format!(
                            "field `spirv_words`: OpEntryPoint name exceeds {MAX_ENTRY_POINT_BYTES} bytes"
                        ),
                    ));
                }
                name_bytes.push(byte);
            }
        }
        if nul_seen_in_word {
            interface_start = Some(idx.saturating_add(1));
            break;
        }
    }

    let iface_idx = interface_start.ok_or_else(|| {
        invalid_artifact(
            label,
            "field `spirv_words`: unterminated literal string in OpEntryPoint",
        )
    })?;

    let name = std::str::from_utf8(&name_bytes).map_err(|_| {
        invalid_artifact(
            label,
            "field `spirv_words`: invalid UTF-8 in OpEntryPoint name",
        )
    })?;

    if let Some(interface_words) = operand_words.get(iface_idx..) {
        for &iface_id in interface_words {
            if iface_id == 0 || iface_id >= bound {
                return Err(invalid_artifact(
                    label,
                    format!(
                        "field `spirv_words`: OpEntryPoint interface id {iface_id} outside 1..{bound}"
                    ),
                ));
            }
        }
    }

    Ok((exec_model, name.to_owned()))
}

#[track_caller]
fn validate_shader_push_constants_against_layout(
    shader: &CookedShader,
    expected: PushConstantRange,
) -> Result<(), RenderError> {
    if shader.push_constants.size == 0 {
        return Ok(());
    }
    if expected.size < shader.push_constants.size {
        return Err(layout_mismatch(format!(
            "entry `{}` ({}): shader requires {} bytes of push constants, but pipeline declares {} bytes",
            shader.entry_point, shader.stage, shader.push_constants.size, expected.size
        )));
    }
    if !expected.stages.contains(shader.stage.stage_flag()) {
        return Err(layout_mismatch(format!(
            "entry `{}` ({}): pipeline push-constant stages ({:#x}) do not include `{}`",
            shader.entry_point,
            shader.stage,
            expected.stages.bits(),
            shader.stage
        )));
    }
    Ok(())
}

#[track_caller]
fn validate_expected_push_constants_def(range: PushConstantRange) -> Result<(), RenderError> {
    if ShaderStageFlags::from_bits(range.stages.bits()).is_none() {
        return Err(layout_mismatch(format!(
            "pipeline push-constant stages ({:#x}) contain unsupported bits",
            range.stages.bits()
        )));
    }
    if range.size == 0 {
        if !range.stages.is_empty() {
            return Err(layout_mismatch(format!(
                "pipeline push-constant size is 0, but stage mask is non-empty ({:#x})",
                range.stages.bits()
            )));
        }
    } else {
        if range.size > MAX_PUSH_CONSTANT_BYTES || !range.size.is_multiple_of(4) {
            return Err(layout_mismatch(format!(
                "pipeline push-constant size {} is invalid (must be a multiple of 4 in 4..={MAX_PUSH_CONSTANT_BYTES})",
                range.size
            )));
        }
        if range.stages.is_empty() {
            return Err(layout_mismatch(format!(
                "pipeline push-constant size is {} bytes, but stage mask is empty",
                range.size
            )));
        }
    }
    Ok(())
}

#[track_caller]
fn canonicalize_vertex_inputs(
    attrs: &[VertexInputAttribute],
) -> Result<Vec<VertexInputAttribute>, RenderError> {
    if attrs.len() > MAX_VERTEX_ATTRIBUTES {
        return Err(layout_mismatch(format!(
            "pipeline declares {} vertex inputs (maximum {MAX_VERTEX_ATTRIBUTES})",
            attrs.len()
        )));
    }
    let mut sorted = attrs.to_vec();
    sorted.sort_by_key(|attr| attr.location);
    let mut prev: Option<u32> = None;
    for attr in &sorted {
        if let Some(last) = prev
            && attr.location == last
        {
            return Err(layout_mismatch(format!(
                "pipeline declares duplicate vertex input location {}",
                attr.location
            )));
        }
        prev = Some(attr.location);
    }
    Ok(sorted)
}

#[track_caller]
fn validate_vertex_inputs_equal(
    entry_point: &str,
    shader_attrs: &[VertexInputAttribute],
    expected_attrs: &[VertexInputAttribute],
) -> Result<(), RenderError> {
    for shader_attr in shader_attrs {
        match expected_attrs
            .iter()
            .find(|cand| cand.location == shader_attr.location)
        {
            None => {
                return Err(layout_mismatch(format!(
                    "entry `{entry_point}`: shader requires vertex input location {} (`{}`), which is missing from pipeline vertex inputs",
                    shader_attr.location, shader_attr.format
                )));
            }
            Some(expected_attr) if expected_attr.format != shader_attr.format => {
                return Err(layout_mismatch(format!(
                    "entry `{entry_point}`: vertex input location {} has format `{}` in shader, but pipeline provides `{}`",
                    shader_attr.location, shader_attr.format, expected_attr.format
                )));
            }
            Some(_) => {}
        }
    }
    for expected_attr in expected_attrs {
        if !shader_attrs
            .iter()
            .any(|cand| cand.location == expected_attr.location)
        {
            return Err(layout_mismatch(format!(
                "entry `{entry_point}`: pipeline provides unused vertex input location {} (`{}`) not declared by vertex shader",
                expected_attr.location, expected_attr.format
            )));
        }
    }
    Ok(())
}

#[track_caller]
fn canonicalize_descriptor_bindings(
    bindings: &[DescriptorBinding],
) -> Result<Vec<DescriptorBinding>, RenderError> {
    if bindings.len() > MAX_DESCRIPTOR_BINDINGS {
        return Err(layout_mismatch(format!(
            "pipeline declares {} descriptor bindings (maximum {MAX_DESCRIPTOR_BINDINGS})",
            bindings.len()
        )));
    }
    let mut sorted = bindings.to_vec();
    sorted.sort_by_key(|b| (b.set, b.binding));
    let mut prev: Option<(u32, u32)> = None;
    for binding in &sorted {
        if binding.count == 0 {
            return Err(layout_mismatch(format!(
                "pipeline descriptor `(set={}, binding={})` has invalid count 0",
                binding.set, binding.binding
            )));
        }
        if ShaderStageFlags::from_bits(binding.stages.bits()).is_none() || binding.stages.is_empty()
        {
            return Err(layout_mismatch(format!(
                "pipeline descriptor `(set={}, binding={})` has invalid stage flags {:#x}",
                binding.set,
                binding.binding,
                binding.stages.bits()
            )));
        }
        let slot = (binding.set, binding.binding);
        if let Some(last) = prev
            && slot == last
        {
            return Err(layout_mismatch(format!(
                "pipeline declares duplicate descriptor `(set={}, binding={})`",
                binding.set, binding.binding
            )));
        }
        prev = Some(slot);
    }
    Ok(sorted)
}

#[track_caller]
fn validate_shader_bindings_against_layout(
    entry_point: &str,
    stage: ShaderStage,
    shader_bindings: &[DescriptorBinding],
    expected_bindings: &[DescriptorBinding],
) -> Result<(), RenderError> {
    for sb in shader_bindings {
        let Some(eb) = expected_bindings
            .iter()
            .find(|cand| cand.set == sb.set && cand.binding == sb.binding)
        else {
            return Err(layout_mismatch(format!(
                "entry `{entry_point}` ({stage}): shader requires descriptor `(set={}, binding={}, kind={})`, which is missing from pipeline layout",
                sb.set, sb.binding, sb.kind
            )));
        };
        if eb.kind != sb.kind {
            return Err(layout_mismatch(format!(
                "entry `{entry_point}` ({stage}): descriptor `(set={}, binding={})` has kind `{}` in shader, but pipeline declares `{}`",
                sb.set, sb.binding, sb.kind, eb.kind
            )));
        }
        if eb.count != sb.count {
            return Err(layout_mismatch(format!(
                "entry `{entry_point}` ({stage}): descriptor `(set={}, binding={})` has count {} in shader, but pipeline declares {}",
                sb.set, sb.binding, sb.count, eb.count
            )));
        }
        if !eb.stages.contains(stage.stage_flag()) {
            return Err(layout_mismatch(format!(
                "entry `{entry_point}` ({stage}): pipeline descriptor `(set={}, binding={})` stages ({:#x}) do not include `{stage}`",
                sb.set,
                sb.binding,
                eb.stages.bits()
            )));
        }
    }
    Ok(())
}

const fn padded_4(len: usize) -> usize {
    (len.saturating_add(3)) & !3
}

#[track_caller]
fn compute_encoded_byte_len(
    entry_len: usize,
    desc_count: usize,
    attr_count: usize,
    spirv_words: usize,
    label: Option<&str>,
) -> Result<usize, RenderError> {
    let entry_padded = entry_len
        .checked_add(3)
        .map(|n| n & !3)
        .ok_or_else(|| invalid_artifact(label, "field `entry_point_len`: byte size overflow"))?;
    let desc_bytes = desc_count
        .checked_mul(DESCRIPTOR_RECORD_BYTES)
        .ok_or_else(|| {
            invalid_artifact(
                label,
                "field `descriptor_binding_count`: byte size overflow",
            )
        })?;
    let attr_bytes = attr_count
        .checked_mul(VERTEX_ATTR_RECORD_BYTES)
        .ok_or_else(|| invalid_artifact(label, "field `vertex_input_count`: byte size overflow"))?;
    let spirv_bytes = spirv_words
        .checked_mul(4)
        .ok_or_else(|| invalid_artifact(label, "field `spirv_word_count`: byte size overflow"))?;

    let total = HEADER_BYTES
        .checked_add(entry_padded)
        .and_then(|n| n.checked_add(desc_bytes))
        .and_then(|n| n.checked_add(attr_bytes))
        .and_then(|n| n.checked_add(spirv_bytes))
        .ok_or_else(|| invalid_artifact(label, "field `payload`: total byte size overflow"))?;

    if total > MAX_COOKED_SHADER_BYTES {
        return Err(invalid_artifact(
            label,
            format!(
                "field `payload`: encoded size {total} bytes exceeds maximum {MAX_COOKED_SHADER_BYTES} bytes"
            ),
        ));
    }
    Ok(total)
}

#[track_caller]
fn read_u32_le(
    bytes: &[u8],
    offset: usize,
    label: Option<&str>,
    field: &str,
) -> Result<u32, RenderError> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| invalid_artifact(label, format!("field `{field}`: offset overflow")))?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| invalid_artifact(label, format!("field `{field}`: unexpected EOF")))?;
    let array: [u8; 4] = slice
        .try_into()
        .map_err(|_| invalid_artifact(label, format!("field `{field}`: expected 4 bytes")))?;
    Ok(u32::from_le_bytes(array))
}

#[track_caller]
fn read_bounded_count(
    bytes: &[u8],
    offset: usize,
    max: usize,
    label: Option<&str>,
    field: &str,
) -> Result<usize, RenderError> {
    let raw = read_u32_le(bytes, offset, label, field)?;
    let count = usize::try_from(raw).map_err(|_| {
        invalid_artifact(
            label,
            format!("field `{field}`: count {raw} overflows usize"),
        )
    })?;
    if count > max {
        return Err(invalid_artifact(
            label,
            format!("field `{field}`: count {count} exceeds maximum {max}"),
        ));
    }
    Ok(count)
}

#[track_caller]
fn invalid_artifact(label: Option<&str>, reason: impl Into<String>) -> RenderError {
    let reason = reason.into();
    let detail = match label {
        Some(name) => format!("artifact `{name}`: {reason}"),
        None => reason,
    };
    RenderError::new(RenderErrorKind::InvalidShaderArtifact).with_detail(detail)
}

#[track_caller]
fn layout_mismatch(detail: impl Into<String>) -> RenderError {
    RenderError::new(RenderErrorKind::ShaderLayoutMismatch).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_spirv_string(s: &str) -> Vec<u32> {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_le_bytes)
            .collect()
    }

    fn minimal_spirv(stage: ShaderStage, entry: &str) -> Vec<u32> {
        let name_words = pack_spirv_string(entry);
        let entry_len = (3 + name_words.len()) as u32;
        let mut words = vec![
            SPIRV_MAGIC,
            0x0001_0600, // SPIR-V 1.6
            0,
            8,              // bound
            0,              // schema
            (2 << 16) | 17, // OpCapability Shader
            1,
            (3 << 16) | 14, // OpMemoryModel Logical GLSL450
            0,
            1,
            (entry_len << 16) | u32::from(SPIRV_OP_ENTRY_POINT),
            stage.spirv_execution_model(),
            4, // %main id
        ];
        words.extend_from_slice(&name_words);
        words
    }

    #[test]
    fn round_trip_encode_and_decode_preserves_cooked_shader() {
        let shader = CookedShader {
            version: SHADER_FORMAT_VERSION,
            stage: ShaderStage::Vertex,
            entry_point: "vs_main".to_owned(),
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: vec![DescriptorBinding {
                set: 0,
                binding: 0,
                kind: DescriptorKind::UniformBuffer,
                count: 1,
                stages: ShaderStageFlags::VERTEX,
            }],
            vertex_inputs: vec![
                VertexInputAttribute::new(0, VertexFormat::Float32x3),
                VertexInputAttribute::new(1, VertexFormat::Float32x4),
            ],
            spirv_words: minimal_spirv(ShaderStage::Vertex, "vs_main"),
        };

        let bytes = shader.encode().unwrap();
        assert_eq!(bytes.len() % 4, 0);
        let decoded = CookedShader::decode(&bytes).unwrap();
        assert_eq!(decoded, shader);
    }

    #[test]
    fn stage_flags_reject_unknown_bits() {
        assert_eq!(
            ShaderStageFlags::from_bits(0b111),
            Some(ShaderStageFlags::ALL)
        );
        assert_eq!(ShaderStageFlags::from_bits(0b1000), None);
    }
}
