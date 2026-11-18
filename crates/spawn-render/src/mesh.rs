//! Plain-data 3D mesh descriptors, camera-relative draw contracts, and 128-byte
//! push-constant packing matching `shaders/mesh_3d.slang` (`ADR-0004`, `ADR-0010`,
//! `SPN-INV-050..053`, `F3D-FR-02`, `F3D-FR-08`, `F3D-FR-10`).
//!
//! Always compiled (`default-features = false`), GPU-free, `#![forbid(unsafe_code)]`,
//! and `#![deny(clippy::indexing_slicing)]`. No `bytemuck` or Vulkan type appears
//! in the public API (`SPN-INV-011`), and `f64` [`WorldPos`] coordinates are
//! subtracted in `f64` on the CPU via [`camera_relative_f32`] before entering
//! [`MeshDraw`] (`SPN-INV-052`).

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use spawn_math::{Mat3, Mat4, Quat, Transform, Vec3, Vec4, WorldPos, camera_relative_f32};

use crate::error::{RenderError, RenderErrorKind};
use crate::handle::MeshHandle;
use crate::shader::{PushConstantRange, ShaderStageFlags, VertexFormat, VertexInputAttribute};

/// Byte stride of [`MeshVertex`] (`3 + 3 + 4` IEEE `f32` scalars = `40` bytes).
pub const MESH_VERTEX_STRIDE: u32 = 40;

/// Canonical vertex input attribute layout of [`MeshVertex`] matching `shaders/mesh_3d.slang`.
pub const MESH_VERTEX_ATTRIBUTES: [VertexInputAttribute; 3] = [
    VertexInputAttribute::new(0, VertexFormat::Float32x3),
    VertexInputAttribute::new(1, VertexFormat::Float32x3),
    VertexInputAttribute::new(2, VertexFormat::Float32x4),
];

/// Exact byte size of the `MeshPushConstants` block in `shaders/mesh_3d.slang` (`128` bytes).
pub const MESH_PUSH_CONSTANT_BYTES: u32 = 128;

/// Pipeline push-constant range for `shaders/mesh_3d.slang` (`128` bytes in vertex stage).
pub const MESH_PUSH_CONSTANT_RANGE: PushConstantRange =
    PushConstantRange::new(MESH_PUSH_CONSTANT_BYTES, ShaderStageFlags::VERTEX);

/// Upper sanity cap on vertices per static mesh upload (`4,194,304` vertices = `160 MiB`; `SPN-INV-034`).
pub const MAX_MESH_VERTICES: usize = 4 * 1024 * 1024;

/// Upper sanity cap on `u32` indices per static mesh upload (`16,777,216` indices = `64 MiB`; `SPN-INV-034`).
pub const MAX_MESH_INDICES: usize = 16 * 1024 * 1024;

/// Single 3D mesh vertex (`40` bytes: local position, local normal, linear RGBA color)
/// matching `VertexInput` in `shaders/mesh_3d.slang`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshVertex {
    /// Local-space 3D position (`location 0`, `R32G32B32_SFLOAT`, byte offset `0`).
    pub position: Vec3,
    /// Local-space surface normal (`location 1`, `R32G32B32_SFLOAT`, byte offset `12`).
    pub normal: Vec3,
    /// Linear RGBA vertex color (`location 2`, `R32G32B32A32_SFLOAT`, byte offset `24`).
    pub color: Vec4,
}

impl MeshVertex {
    /// Creates a 3D mesh vertex.
    #[inline]
    pub const fn new(position: Vec3, normal: Vec3, color: Vec4) -> Self {
        Self {
            position,
            normal,
            color,
        }
    }

    /// Returns `true` when `position`, `normal`, and `color` contain only finite scalars.
    #[inline]
    pub fn is_finite(&self) -> bool {
        self.position.is_finite() && self.normal.is_finite() && self.color.is_finite()
    }
}

/// CPU-side indexed `TRIANGLE_LIST` mesh upload descriptor (`u32` indices).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshDescriptor<'a> {
    /// Debug label attached via `VK_EXT_debug_utils` (`spawn.mesh.<label>.{vb,ib}`).
    pub label: &'a str,
    /// Non-empty vertex slice (`1..=MAX_MESH_VERTICES`, all finite).
    pub vertices: &'a [MeshVertex],
    /// Non-empty triangle index slice (`len % 3 == 0`, `3..=MAX_MESH_INDICES`, each `< vertices.len()`).
    pub indices: &'a [u32],
}

impl<'a> MeshDescriptor<'a> {
    /// Creates an indexed 3D mesh upload descriptor.
    pub const fn new(label: &'a str, vertices: &'a [MeshVertex], indices: &'a [u32]) -> Self {
        Self {
            label,
            vertices,
            indices,
        }
    }

    /// Validates vertex/index counts, triangle-list multiplicity, finite vertex values,
    /// index bounds, and byte-size limits before allocating any CPU or GPU staging memory
    /// (`SPN-INV-034`). Returns `(vertex_bytes, index_bytes)` on success.
    #[track_caller]
    pub fn validate(&self) -> Result<(u64, u64), RenderError> {
        if self.vertices.is_empty() {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "mesh `{}`: vertex slice cannot be empty",
                    self.label
                )),
            );
        }
        if self.vertices.len() > MAX_MESH_VERTICES {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "mesh `{}`: vertex count {} exceeds maximum {MAX_MESH_VERTICES}",
                    self.label,
                    self.vertices.len()
                )),
            );
        }
        let vertex_count_u32 = u32::try_from(self.vertices.len()).map_err(|_| {
            RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail(format!("mesh `{}`: vertex count overflows u32", self.label))
        })?;

        if self.indices.is_empty() {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "mesh `{}`: index slice cannot be empty",
                    self.label
                )),
            );
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "mesh `{}`: TRIANGLE_LIST index count {} is not a multiple of 3",
                    self.label,
                    self.indices.len()
                )),
            );
        }
        if self.indices.len() > MAX_MESH_INDICES {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "mesh `{}`: index count {} exceeds maximum {MAX_MESH_INDICES}",
                    self.label,
                    self.indices.len()
                )),
            );
        }
        let _index_count_u32 = u32::try_from(self.indices.len()).map_err(|_| {
            RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail(format!("mesh `{}`: index count overflows u32", self.label))
        })?;

        for (v_idx, vertex) in self.vertices.iter().enumerate() {
            if !vertex.is_finite() {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(format!(
                        "mesh `{}`: vertex[{v_idx}] contains NaN or infinite component",
                        self.label
                    )));
            }
        }

        for (i_pos, &index) in self.indices.iter().enumerate() {
            if index >= vertex_count_u32 {
                return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(format!(
                        "mesh `{}`: indices[{i_pos}] = {index} is out of bounds for {} vertices",
                        self.label, vertex_count_u32
                    )));
            }
        }

        let vertex_bytes = u64::try_from(self.vertices.len())
            .ok()
            .and_then(|n| n.checked_mul(u64::from(MESH_VERTEX_STRIDE)))
            .ok_or_else(|| {
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(format!("mesh `{}`: vertex byte size overflow", self.label))
            })?;
        let index_bytes = u64::try_from(self.indices.len())
            .ok()
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| {
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                    .with_detail(format!("mesh `{}`: index byte size overflow", self.label))
            })?;

        Ok((vertex_bytes, index_bytes))
    }
}

/// Camera-relative presentation view (`SPN-INV-052`, `SPN-INV-053`).
///
/// In camera-relative space the camera sits at `(0, 0, 0)` because each mesh's
/// translation is computed as `(mesh_world_f64 - camera_world_f64)` on the CPU
/// before conversion to `f32`, so the view transform is pure inverse camera
/// rotation (`camera_rotation.conjugate()`) followed by `projection`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraRelativeView {
    /// Reversed-Z Vulkan perspective projection (`Mat4::perspective_reversed_z_rh`
    /// or `Mat4::perspective_infinite_reversed_z_rh`), containing the single-site
    /// Vulkan NDC Y-flip (`SPN-INV-053`).
    pub projection: Mat4,
    /// Camera world orientation (`Quat::IDENTITY` looks down `-Z` with `+Y` up).
    pub camera_rotation: Quat,
}

impl CameraRelativeView {
    /// Creates a camera-relative view from a reversed-Z `projection` and `camera_rotation`.
    #[inline]
    pub const fn new(projection: Mat4, camera_rotation: Quat) -> Self {
        Self {
            projection,
            camera_rotation,
        }
    }

    /// Computes `projection * Mat4::from_quat(camera_rotation.conjugate())`.
    #[inline]
    pub fn view_projection(&self) -> Mat4 {
        let inv_cam_rot = self.camera_rotation.normalize_or_identity().conjugate();
        self.projection * Mat4::from_quat(inv_cam_rot)
    }
}

/// Single camera-relative indexed 3D mesh draw item (`SPN-INV-052`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshDraw {
    /// Generational handle of the uploaded mesh (`DEVICE_LOCAL` vertex + index buffers).
    pub mesh: MeshHandle,
    /// Position of the mesh relative to the camera in meters (`f32`).
    pub camera_relative_position: Vec3,
    /// Local-to-world orientation of the mesh (`Quat::IDENTITY` by default).
    pub rotation: Quat,
    /// Local non-uniform scale (`Vec3::ONE` by default).
    pub scale: Vec3,
    /// Linear RGBA color multiplier (`Vec4::ONE` by default).
    pub base_color: Vec4,
    /// Blend between deterministic flat unlit color (`0.0`) and directional
    /// shading (`1.0` by default), clamped to `0.0..=1.0`.
    pub shade_weight: f32,
}

impl MeshDraw {
    /// Creates a draw item from a precomputed camera-relative `f32` offset.
    #[inline]
    pub const fn new(mesh: MeshHandle, camera_relative_position: Vec3) -> Self {
        Self {
            mesh,
            camera_relative_position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            base_color: Vec4::ONE,
            shade_weight: 1.0,
        }
    }

    /// Creates a draw item from `f64` [`WorldPos`] coordinates by subtracting
    /// `camera_world_pos` from `mesh_world_pos` in `f64` on the CPU via
    /// [`camera_relative_f32`] before narrowing to `f32` (`SPN-INV-052`).
    #[inline]
    pub const fn from_world(
        mesh: MeshHandle,
        mesh_world_pos: WorldPos,
        camera_world_pos: WorldPos,
    ) -> Self {
        Self::new(mesh, camera_relative_f32(mesh_world_pos, camera_world_pos))
    }

    /// Sets the mesh rotation.
    #[must_use]
    #[inline]
    pub const fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets the mesh scale.
    #[must_use]
    #[inline]
    pub const fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Applies a local camera-relative [`Transform`] (`translation`, `rotation`, `scale`).
    #[must_use]
    #[inline]
    pub const fn with_transform(mut self, transform: Transform) -> Self {
        self.camera_relative_position = transform.translation;
        self.rotation = transform.rotation;
        self.scale = transform.scale;
        self
    }

    /// Sets the linear RGBA `base_color` multiplier.
    #[must_use]
    #[inline]
    pub const fn with_base_color(mut self, base_color: Vec4) -> Self {
        self.base_color = base_color;
        self
    }

    /// Sets `shade_weight` (`0.0` = unlit flat color; `1.0` = directional shading).
    #[must_use]
    #[inline]
    pub const fn with_shade_weight(mut self, shade_weight: f32) -> Self {
        self.shade_weight = shade_weight;
        self
    }

    /// Validates finiteness and packs the exact `128`-byte `MeshPushConstants`
    /// structure expected by `shaders/mesh_3d.slang`.
    #[track_caller]
    pub fn pack_push_constants(
        &self,
        view: &CameraRelativeView,
    ) -> Result<PackedMeshPushConstants, RenderError> {
        if !view.projection.is_finite() || !view.camera_rotation.is_finite() {
            return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("CameraRelativeView contains NaN or infinite values"));
        }
        if !self.camera_relative_position.is_finite()
            || !self.rotation.is_finite()
            || !self.scale.is_finite()
            || !self.base_color.is_finite()
        {
            return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("MeshDraw contains NaN or infinite transform/color values"));
        }

        let model = Mat4::from_scale_rotation_translation(
            self.scale,
            self.rotation,
            self.camera_relative_position,
        );
        let mvp_camera_relative = view.view_projection() * model;
        if !mvp_camera_relative.is_finite() {
            return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("camera-relative MVP matrix overflowed to non-finite values"));
        }

        let normal_basis = Mat3::normal_matrix(self.scale, self.rotation)
            .unwrap_or_else(|| Mat3::from_quat(self.rotation));
        let clamped_shade = if self.shade_weight.is_finite() {
            self.shade_weight.clamp(0.0, 1.0)
        } else {
            0.0
        };

        Ok(PackedMeshPushConstants {
            mvp_camera_relative,
            normal_col0: normal_basis.x_axis.extend(clamped_shade),
            normal_col1: normal_basis.y_axis.extend(0.0),
            normal_col2: normal_basis.z_axis.extend(0.0),
            base_color: self.base_color,
        })
    }
}

/// Validated, CPU-packed `128`-byte push-constant payload matching `MeshPushConstants`
/// in `shaders/mesh_3d.slang`:
/// - `0..64`: `mvp_camera_relative` (`Mat4`, column-major)
/// - `64..80`: `normal_col0` (`xyz` = column 0 of normal matrix, `w` = `shade_weight`)
/// - `80..96`: `normal_col1` (`xyz` = column 1 of normal matrix, `w` = `0.0`)
/// - `96..112`: `normal_col2` (`xyz` = column 2 of normal matrix, `w` = `0.0`)
/// - `112..128`: `base_color` (linear RGBA multiplier)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PackedMeshPushConstants {
    /// Camera-relative model-view-projection matrix (`0..64`).
    pub mvp_camera_relative: Mat4,
    /// Normal basis column 0 in `.xyz` and `shade_weight` in `.w` (`64..80`).
    pub normal_col0: Vec4,
    /// Normal basis column 1 in `.xyz` (`80..96`).
    pub normal_col1: Vec4,
    /// Normal basis column 2 in `.xyz` (`96..112`).
    pub normal_col2: Vec4,
    /// Linear RGBA color multiplier (`112..128`).
    pub base_color: Vec4,
}

impl PackedMeshPushConstants {
    /// Serializes all `32` `f32` fields into canonical `128`-byte little-endian layout.
    pub fn to_le_bytes(&self) -> [u8; 128] {
        let mut out = [0_u8; 128];
        let mvp = self.mvp_camera_relative.to_le_bytes();
        if let Some(dst) = out.get_mut(0..64) {
            dst.copy_from_slice(&mvp);
        }
        for (slot_idx, vec) in [
            self.normal_col0,
            self.normal_col1,
            self.normal_col2,
            self.base_color,
        ]
        .into_iter()
        .enumerate()
        {
            let base = 64 + slot_idx * 16;
            for (comp_idx, scalar) in vec.to_array().into_iter().enumerate() {
                let offset = base + comp_idx * 4;
                if let Some(dst) = out.get_mut(offset..offset + 4) {
                    dst.copy_from_slice(&scalar.to_le_bytes());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::ShaderHandle;
    use crate::shader::{
        CookedShader, PipelineLayoutDesc, SHADER_FORMAT_VERSION, SPIRV_MAGIC, ShaderStage,
        validate_graphics_pipeline_shaders,
    };
    use crate::types::GraphicsPipelineDescriptor;
    use spawn_math::DVec3;

    fn sample_quad_vertices() -> [MeshVertex; 4] {
        [
            MeshVertex::new(Vec3::new(-1.0, -1.0, 0.0), Vec3::Z, Vec4::ONE),
            MeshVertex::new(Vec3::new(1.0, -1.0, 0.0), Vec3::Z, Vec4::ONE),
            MeshVertex::new(Vec3::new(1.0, 1.0, 0.0), Vec3::Z, Vec4::ONE),
            MeshVertex::new(Vec3::new(-1.0, 1.0, 0.0), Vec3::Z, Vec4::ONE),
        ]
    }

    #[test]
    fn mesh_descriptor_validates_counts_finiteness_and_index_bounds() {
        let verts = sample_quad_vertices();
        let indices = [0_u32, 1, 2, 0, 2, 3];
        let valid = MeshDescriptor::new("quad", &verts, &indices);
        assert_eq!(valid.validate().unwrap(), (4 * 40, 6 * 4));

        // Empty vertices -> REND-0014.
        assert_eq!(
            MeshDescriptor::new("empty_v", &[], &[0, 0, 0])
                .validate()
                .unwrap_err()
                .kind(),
            RenderErrorKind::InvalidResourceDescriptor
        );

        // Empty indices and non-multiple-of-3 indices -> REND-0014.
        assert_eq!(
            MeshDescriptor::new("empty_i", &verts, &[])
                .validate()
                .unwrap_err()
                .kind(),
            RenderErrorKind::InvalidResourceDescriptor
        );
        assert_eq!(
            MeshDescriptor::new("non_tri", &verts, &[0, 1, 2, 3])
                .validate()
                .unwrap_err()
                .kind(),
            RenderErrorKind::InvalidResourceDescriptor
        );

        // Out-of-bounds index -> REND-0014.
        let oob = MeshDescriptor::new("oob", &verts, &[0, 1, 4])
            .validate()
            .unwrap_err();
        assert_eq!(oob.kind(), RenderErrorKind::InvalidResourceDescriptor);
        assert!(oob.detail().unwrap_or("").contains("indices[2] = 4"));

        // NaN / Inf vertex attribute -> REND-0014.
        let mut nan_verts = verts;
        nan_verts[1].normal.y = f32::NAN;
        let nan_err = MeshDescriptor::new("nan_v", &nan_verts, &indices)
            .validate()
            .unwrap_err();
        assert_eq!(nan_err.kind(), RenderErrorKind::InvalidResourceDescriptor);
        assert!(nan_err.detail().unwrap_or("").contains("vertex[1]"));
    }

    #[test]
    fn pack_push_constants_matches_128_byte_layout_and_preserves_50km_world_precision() {
        let proj =
            Mat4::perspective_reversed_z_rh(std::f32::consts::FRAC_PI_2, 16.0 / 9.0, 0.1, 500.0)
                .unwrap();
        let cam_rot = Quat::from_rotation_y(0.35);
        let view = CameraRelativeView::new(proj, cam_rot);

        let dummy_mesh = MeshHandle::from_raw(0, 1);
        let mesh_offset = DVec3::new(1.25, -0.75, -3.5);

        let draw_origin =
            MeshDraw::from_world(dummy_mesh, WorldPos::ORIGIN + mesh_offset, WorldPos::ORIGIN)
                .with_rotation(Quat::from_rotation_x(0.2))
                .with_scale(Vec3::new(2.0, 0.5, 1.0))
                .with_base_color(Vec4::new(0.8, 0.4, 0.2, 1.0))
                .with_shade_weight(0.0);

        let cam_far = WorldPos::new(50_000.0, -50_000.0, 50_000.0);
        let draw_50km = MeshDraw::from_world(dummy_mesh, cam_far + mesh_offset, cam_far)
            .with_rotation(Quat::from_rotation_x(0.2))
            .with_scale(Vec3::new(2.0, 0.5, 1.0))
            .with_base_color(Vec4::new(0.8, 0.4, 0.2, 1.0))
            .with_shade_weight(0.0);

        assert!(
            draw_origin
                .camera_relative_position
                .abs_diff_eq(draw_50km.camera_relative_position, 1.0e-5),
            "50 km camera-relative offset must avoid ~4 mm f32 world quantization"
        );

        let packed_origin = draw_origin.pack_push_constants(&view).unwrap();
        let packed_50km = draw_50km.pack_push_constants(&view).unwrap();
        assert!(
            packed_origin
                .mvp_camera_relative
                .abs_diff_eq(&packed_50km.mvp_camera_relative, 1.0e-5)
        );
        assert_eq!(packed_origin.normal_col0.w, 0.0);
        assert_eq!(packed_origin.normal_col1.w, 0.0);
        assert_eq!(packed_origin.normal_col2.w, 0.0);
        assert_eq!(packed_origin.base_color, Vec4::new(0.8, 0.4, 0.2, 1.0));

        let bytes = packed_origin.to_le_bytes();
        assert_eq!(bytes.len(), 128);
        assert_eq!(
            Mat4::from_le_bytes(bytes[0..64].try_into().unwrap()),
            packed_origin.mvp_camera_relative
        );
        assert_eq!(
            f32::from_le_bytes(bytes[76..80].try_into().unwrap()),
            0.0_f32
        );
        assert_eq!(
            f32::from_le_bytes(bytes[112..116].try_into().unwrap()),
            0.8_f32
        );

        // Zero scale (`singular` normal matrix) falls back safely without panicking;
        // out-of-range `shade_weight` clamps to `[0.0, 1.0]`.
        let zero_scale_draw = MeshDraw::new(dummy_mesh, Vec3::new(0.0, 0.0, -2.0))
            .with_scale(Vec3::new(0.0, 1.0, 1.0))
            .with_shade_weight(5.0);
        let fallback_packed = zero_scale_draw.pack_push_constants(&view).unwrap();
        assert!(fallback_packed.normal_col0.is_finite());
        assert_eq!(fallback_packed.normal_col0.w, 1.0);

        // Non-finite inputs return REND-0014.
        let bad_draw = MeshDraw::new(dummy_mesh, Vec3::NAN);
        assert_eq!(
            bad_draw.pack_push_constants(&view).unwrap_err().kind(),
            RenderErrorKind::InvalidResourceDescriptor
        );
    }

    #[test]
    fn mesh_3d_pipeline_descriptor_matches_shader_validator_contract() {
        let vs_handle = ShaderHandle::from_raw(0, 1);
        let fs_handle = ShaderHandle::from_raw(1, 1);
        let pipe_desc = GraphicsPipelineDescriptor::new_mesh_3d("mesh_3d", vs_handle, fs_handle);
        assert_eq!(pipe_desc.vertex_stride, MESH_VERTEX_STRIDE);
        assert_eq!(pipe_desc.vertex_inputs, &MESH_VERTEX_ATTRIBUTES);
        assert_eq!(pipe_desc.push_constants, MESH_PUSH_CONSTANT_RANGE);

        let minimal_vs_spirv = vec![
            SPIRV_MAGIC,
            0x0001_0600,
            0,
            8,
            0,
            (2 << 16) | 17,
            1,
            (3 << 16) | 14,
            0,
            1,
            (5 << 16) | 15,
            0,
            4,
            u32::from_le_bytes(*b"vs_m"),
            u32::from_le_bytes(*b"ain\0"),
        ];
        let minimal_fs_spirv = vec![
            SPIRV_MAGIC,
            0x0001_0600,
            0,
            8,
            0,
            (2 << 16) | 17,
            1,
            (3 << 16) | 14,
            0,
            1,
            (5 << 16) | 15,
            4,
            4,
            u32::from_le_bytes(*b"fs_m"),
            u32::from_le_bytes(*b"ain\0"),
        ];
        let vs = CookedShader {
            version: SHADER_FORMAT_VERSION,
            stage: ShaderStage::Vertex,
            entry_point: "vs_main".to_owned(),
            push_constants: MESH_PUSH_CONSTANT_RANGE,
            descriptor_bindings: Vec::new(),
            vertex_inputs: MESH_VERTEX_ATTRIBUTES.to_vec(),
            spirv_words: minimal_vs_spirv,
        };
        let fs = CookedShader {
            version: SHADER_FORMAT_VERSION,
            stage: ShaderStage::Fragment,
            entry_point: "fs_main".to_owned(),
            push_constants: PushConstantRange::NONE,
            descriptor_bindings: Vec::new(),
            vertex_inputs: Vec::new(),
            spirv_words: minimal_fs_spirv,
        };
        validate_graphics_pipeline_shaders(
            &vs,
            &fs,
            &PipelineLayoutDesc {
                vertex_inputs: pipe_desc.vertex_inputs,
                push_constants: pipe_desc.push_constants,
                descriptor_bindings: pipe_desc.descriptor_bindings,
            },
        )
        .unwrap();
    }
}
