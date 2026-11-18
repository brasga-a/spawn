//! Deterministic in-memory procedural 3D test scene (`DESIGN.md` §5.2,
//! `F3D-FR-11`, `SPN-INV-003`, `SPN-INV-050`..`SPN-INV-053`).
//!
//! Generates indexed counter-clockwise (`VK_FRONT_FACE_COUNTER_CLOCKWISE`) 3D
//! reference geometry in Rust memory at startup — zero external glTF or texture
//! files — and extracts camera-relative `f32` [`MeshDraw`] lists per frame from
//! `f64` [`WorldPos`] placements without steady-state heap allocation.

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use spawn_math::{Quat, Vec3, Vec4, WorldPos, det};
use spawn_render::{MeshDescriptor, MeshDraw, MeshHandle, MeshVertex};

use crate::camera::CameraPresentation;
use crate::error::{EngineError, EngineErrorKind};

/// Cooked vertex-shader artifact filename emitted by `spawn-cli shaders build`
/// for `shaders/mesh_3d.slang`.
pub const MESH_3D_VS_ARTIFACT: &str = "mesh_3d.vs_main.spnshdr";

/// Cooked fragment-shader artifact filename emitted by `spawn-cli shaders build`
/// for `shaders/mesh_3d.slang`.
pub const MESH_3D_FS_ARTIFACT: &str = "mesh_3d.fs_main.spnshdr";

/// Owned CPU-side indexed `TRIANGLE_LIST` mesh template ready for staged GPU upload.
#[derive(Debug, Clone, PartialEq)]
pub struct ProceduralMesh {
    label: &'static str,
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
}

impl ProceduralMesh {
    /// Creates a procedural mesh template from raw components.
    #[must_use]
    pub fn new(label: &'static str, vertices: Vec<MeshVertex>, indices: Vec<u32>) -> Self {
        Self {
            label,
            vertices,
            indices,
        }
    }

    /// Debug label of this mesh template.
    pub const fn label(&self) -> &'static str {
        self.label
    }

    /// Vertex slice (`40` bytes per [`MeshVertex`]).
    pub fn vertices(&self) -> &[MeshVertex] {
        &self.vertices
    }

    /// Triangle-list index slice (`u32`).
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Borrows this template as a `spawn-render` [`MeshDescriptor`].
    #[must_use]
    pub fn descriptor(&self) -> MeshDescriptor<'_> {
        MeshDescriptor::new(self.label, &self.vertices, &self.indices)
    }

    /// Validates vertex/index bounds and finiteness via [`MeshDescriptor::validate`].
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Scene`] (`ENGN-0010`) on invalid geometry.
    #[track_caller]
    pub fn validate(&self) -> Result<(), EngineError> {
        self.descriptor().validate().map(|_| ()).map_err(|err| {
            EngineError::new(EngineErrorKind::Scene)
                .with_detail(format!(
                    "procedural mesh `{}` failed validation",
                    self.label
                ))
                .with_cause(&err)
        })
    }
}

/// Genre-free 3D object placement in `f64` world coordinates (`SPN-INV-051`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneInstance {
    /// Index into [`ProceduralScene::meshes`].
    pub mesh_index: usize,
    /// Absolute `f64` world-space position in meters.
    pub world_pos: WorldPos,
    /// Local-to-world unit quaternion rotation.
    pub rotation: Quat,
    /// Local non-uniform scale (`m`).
    pub scale: Vec3,
    /// Linear RGBA multiplier applied in `shaders/mesh_3d.slang`.
    pub base_color: Vec4,
    /// Blend between flat unlit color (`0.0`) and directional shading (`1.0`).
    pub shade_weight: f32,
}

impl SceneInstance {
    /// Creates a shaded (`shade_weight = 1.0`), unrotated, unit-scale instance.
    #[must_use]
    pub const fn new(mesh_index: usize, world_pos: WorldPos, base_color: Vec4) -> Self {
        Self {
            mesh_index,
            world_pos,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            base_color,
            shade_weight: 1.0,
        }
    }

    /// Sets local rotation.
    #[must_use]
    pub const fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets local non-uniform scale.
    #[must_use]
    pub const fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Sets `shade_weight` (`0.0..=1.0`).
    #[must_use]
    pub const fn with_shade_weight(mut self, shade_weight: f32) -> Self {
        self.shade_weight = shade_weight;
        self
    }
}

/// Deterministic procedural 3D scene composed of reusable [`ProceduralMesh`]
/// templates and `f64` [`SceneInstance`] placements.
#[derive(Debug, Clone, PartialEq)]
pub struct ProceduralScene {
    meshes: Vec<ProceduralMesh>,
    instances: Vec<SceneInstance>,
}

impl ProceduralScene {
    /// Constructs a custom procedural scene and validates all meshes and instances.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Scene`] (`ENGN-0010`) if `meshes` or
    /// `instances` is empty, any mesh fails [`MeshDescriptor::validate`], any
    /// `mesh_index` is out of bounds, or any placement contains non-finite
    /// values.
    #[track_caller]
    pub fn new(
        meshes: Vec<ProceduralMesh>,
        instances: Vec<SceneInstance>,
    ) -> Result<Self, EngineError> {
        let scene = Self { meshes, instances };
        scene.validate()?;
        Ok(scene)
    }

    /// Builds the Milestone 2 reference 3D scene (`DESIGN.md` §5.2):
    /// - 3 procedural mesh templates (`unit_cube`, `unit_octahedron`, `ground_grid`),
    /// - reference ground checkerboard grid at `y = 0.0 m`,
    /// - markers at `-2.0 m`, `-5.0 m` (ordered after `-2.0 m` to exercise
    ///   reversed-Z depth occlusion continuously), `-25.0 m`, and `-200.0 m`.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Scene`] (`ENGN-0010`) if validation fails.
    #[track_caller]
    pub fn default_scene() -> Result<Self, EngineError> {
        let cube = build_unit_cube_mesh();
        let octahedron = build_unit_octahedron_mesh();
        let ground_grid = build_ground_grid_mesh(16, 4.0);

        let cube_idx = 0;
        let octa_idx = 1;
        let grid_idx = 2;

        let instances = vec![
            // Reference ground plane (`y = 0.0`) spanning near to deep midground.
            SceneInstance::new(
                grid_idx,
                WorldPos::new(0.0, 0.0, -28.0),
                Vec4::new(1.0, 1.0, 1.0, 1.0),
            )
            .with_shade_weight(0.65),
            // Foreground marker at -2.0 m (partially overlaps the -5.0 m center cube).
            SceneInstance::new(
                cube_idx,
                WorldPos::new(-0.28, 1.35, -2.0),
                Vec4::new(0.94, 0.34, 0.24, 1.0),
            )
            .with_scale(Vec3::splat(0.65))
            .with_rotation(det::quat_from_yaw_pitch(0.32, 0.14)),
            // Midground center cube at -5.0 m (submitted AFTER -2.0 m so reversed-Z
            // `GREATER_OR_EQUAL` occlusion resolves the -2.0 m marker in front).
            SceneInstance::new(
                cube_idx,
                WorldPos::new(0.0, 1.45, -5.0),
                Vec4::new(0.18, 0.72, 0.88, 1.0),
            )
            .with_scale(Vec3::splat(1.35))
            .with_rotation(det::quat_from_yaw_pitch(-0.42, 0.0)),
            // Midground +X / +Y orientation octahedron at -5.0 m.
            SceneInstance::new(
                octa_idx,
                WorldPos::new(2.25, 2.15, -5.0),
                Vec4::new(1.0, 1.0, 1.0, 1.0),
            )
            .with_scale(Vec3::splat(1.25)),
            // Midground -X column at -5.0 m.
            SceneInstance::new(
                cube_idx,
                WorldPos::new(-2.25, 1.0, -5.0),
                Vec4::new(0.32, 0.82, 0.48, 1.0),
            )
            .with_scale(Vec3::new(0.85, 2.0, 0.85)),
            // Deep midground gateway and octahedron at -25.0 m.
            SceneInstance::new(
                cube_idx,
                WorldPos::new(-4.5, 3.0, -25.0),
                Vec4::new(0.76, 0.56, 0.28, 1.0),
            )
            .with_scale(Vec3::new(1.2, 6.0, 1.2)),
            SceneInstance::new(
                cube_idx,
                WorldPos::new(4.5, 3.0, -25.0),
                Vec4::new(0.76, 0.56, 0.28, 1.0),
            )
            .with_scale(Vec3::new(1.2, 6.0, 1.2)),
            SceneInstance::new(
                cube_idx,
                WorldPos::new(0.0, 6.45, -25.0),
                Vec4::new(0.86, 0.68, 0.36, 1.0),
            )
            .with_scale(Vec3::new(10.4, 0.9, 1.4)),
            SceneInstance::new(
                octa_idx,
                WorldPos::new(0.0, 3.1, -25.0),
                Vec4::new(0.95, 0.95, 1.0, 1.0),
            )
            .with_scale(Vec3::splat(2.4)),
            // Far horizon landmarks at -200.0 m.
            SceneInstance::new(
                cube_idx,
                WorldPos::new(0.0, 18.0, -200.0),
                Vec4::new(0.46, 0.58, 0.92, 1.0),
            )
            .with_scale(Vec3::new(14.0, 36.0, 14.0))
            .with_rotation(det::quat_from_yaw_pitch(0.25, 0.0)),
            SceneInstance::new(
                octa_idx,
                WorldPos::new(-32.0, 14.0, -200.0),
                Vec4::new(0.92, 0.62, 0.84, 1.0),
            )
            .with_scale(Vec3::new(12.0, 20.0, 12.0)),
            SceneInstance::new(
                octa_idx,
                WorldPos::new(32.0, 14.0, -200.0),
                Vec4::new(0.56, 0.88, 0.78, 1.0),
            )
            .with_scale(Vec3::new(12.0, 20.0, 12.0)),
        ];

        Self::new(vec![cube, octahedron, ground_grid], instances)
    }

    /// Mesh templates to upload once at session startup.
    pub fn meshes(&self) -> &[ProceduralMesh] {
        &self.meshes
    }

    /// World-space object placements rendered each frame.
    pub fn instances(&self) -> &[SceneInstance] {
        &self.instances
    }

    /// Validates all procedural meshes and world placements.
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Scene`] (`ENGN-0010`) on invalid geometry or
    /// placement fields.
    #[track_caller]
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.meshes.is_empty() {
            return Err(EngineError::new(EngineErrorKind::Scene)
                .with_detail("procedural scene must contain at least one mesh template"));
        }
        if self.instances.is_empty() {
            return Err(EngineError::new(EngineErrorKind::Scene)
                .with_detail("procedural scene must contain at least one placed instance"));
        }
        for mesh in &self.meshes {
            mesh.validate()?;
        }
        for (idx, instance) in self.instances.iter().enumerate() {
            if instance.mesh_index >= self.meshes.len() {
                return Err(
                    EngineError::new(EngineErrorKind::Scene).with_detail(format!(
                        "scene instance[{idx}] references mesh_index {}, but only {} meshes exist",
                        instance.mesh_index,
                        self.meshes.len()
                    )),
                );
            }
            if !instance.world_pos.is_finite()
                || !instance.rotation.is_finite()
                || !instance.scale.is_finite()
                || !instance.base_color.is_finite()
                || !instance.shade_weight.is_finite()
            {
                return Err(
                    EngineError::new(EngineErrorKind::Scene).with_detail(format!(
                        "scene instance[{idx}] contains non-finite transform or color values"
                    )),
                );
            }
            if instance.scale.x <= 0.0 || instance.scale.y <= 0.0 || instance.scale.z <= 0.0 {
                return Err(
                    EngineError::new(EngineErrorKind::Scene).with_detail(format!(
                        "scene instance[{idx}] scale must be strictly positive on all axes, got {:?}",
                        instance.scale
                    )),
                );
            }
        }
        Ok(())
    }

    /// Clears `out` and populates camera-relative `f32` [`MeshDraw`] items for
    /// all scene instances using `presentation.world_pos` (`SPN-INV-052`).
    ///
    /// Once `out` has capacity `>= self.instances.len()`, subsequent calls perform
    /// zero heap allocations (`F3D-NFR-05`).
    ///
    /// # Errors
    ///
    /// Returns [`EngineErrorKind::Scene`] (`ENGN-0010`) if `mesh_handles.len()`
    /// does not match `self.meshes.len()`.
    #[track_caller]
    pub fn extract_draws(
        &self,
        mesh_handles: &[MeshHandle],
        presentation: &CameraPresentation,
        out: &mut Vec<MeshDraw>,
    ) -> Result<(), EngineError> {
        if mesh_handles.len() != self.meshes.len() {
            return Err(
                EngineError::new(EngineErrorKind::Scene).with_detail(format!(
                    "expected {} uploaded MeshHandles for procedural scene, got {}",
                    self.meshes.len(),
                    mesh_handles.len()
                )),
            );
        }
        out.clear();
        out.reserve(self.instances.len());
        for (idx, instance) in self.instances.iter().enumerate() {
            let handle = *mesh_handles.get(instance.mesh_index).ok_or_else(|| {
                EngineError::new(EngineErrorKind::Scene).with_detail(format!(
                    "scene instance[{idx}] mesh_index {} out of bounds",
                    instance.mesh_index
                ))
            })?;
            let draw = presentation
                .extract_draw(handle, instance.world_pos)
                .with_rotation(instance.rotation)
                .with_scale(instance.scale)
                .with_base_color(instance.base_color)
                .with_shade_weight(instance.shade_weight);
            out.push(draw);
        }
        Ok(())
    }
}

/// Builds a 24-vertex, 36-index unit cube (`[-0.5, +0.5]^3`) with per-face
/// outward normals and subtle orientation tinting (`+Y` top brightest), wound
/// counter-clockwise from outside (`VK_FRONT_FACE_COUNTER_CLOCKWISE`).
#[must_use]
pub fn build_unit_cube_mesh() -> ProceduralMesh {
    let h = 0.5_f32;
    let faces: [([Vec3; 4], Vec3, Vec4); 6] = [
        // +Z front face
        (
            [
                Vec3::new(-h, -h, h),
                Vec3::new(h, -h, h),
                Vec3::new(h, h, h),
                Vec3::new(-h, h, h),
            ],
            Vec3::Z,
            Vec4::new(0.95, 0.95, 0.98, 1.0),
        ),
        // -Z back face
        (
            [
                Vec3::new(h, -h, -h),
                Vec3::new(-h, -h, -h),
                Vec3::new(-h, h, -h),
                Vec3::new(h, h, -h),
            ],
            Vec3::NEG_Z,
            Vec4::new(0.78, 0.82, 0.90, 1.0),
        ),
        // +X right face
        (
            [
                Vec3::new(h, -h, h),
                Vec3::new(h, -h, -h),
                Vec3::new(h, h, -h),
                Vec3::new(h, h, h),
            ],
            Vec3::X,
            Vec4::new(0.92, 0.88, 0.84, 1.0),
        ),
        // -X left face
        (
            [
                Vec3::new(-h, -h, -h),
                Vec3::new(-h, -h, h),
                Vec3::new(-h, h, h),
                Vec3::new(-h, h, -h),
            ],
            Vec3::NEG_X,
            Vec4::new(0.82, 0.88, 0.92, 1.0),
        ),
        // +Y top face (brightest to reinforce +Y world-up visually)
        (
            [
                Vec3::new(-h, h, h),
                Vec3::new(h, h, h),
                Vec3::new(h, h, -h),
                Vec3::new(-h, h, -h),
            ],
            Vec3::Y,
            Vec4::new(1.0, 1.0, 0.94, 1.0),
        ),
        // -Y bottom face
        (
            [
                Vec3::new(-h, -h, -h),
                Vec3::new(h, -h, -h),
                Vec3::new(h, -h, h),
                Vec3::new(-h, -h, h),
            ],
            Vec3::NEG_Y,
            Vec4::new(0.55, 0.58, 0.64, 1.0),
        ),
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (quad, normal, color) in faces {
        let base = vertices.len() as u32;
        for pos in quad {
            vertices.push(MeshVertex::new(pos, normal, color));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    ProceduralMesh::new("unit_cube", vertices, indices)
}

/// Builds a 24-vertex, 24-index flat-shaded unit octahedron (radius `0.5`) with
/// gold `+Y` upper faces and teal `-Y` lower faces, wound counter-clockwise from
/// outside.
#[must_use]
pub fn build_unit_octahedron_mesh() -> ProceduralMesh {
    let r = 0.5_f32;
    let inv_sqrt3 = 1.0_f32 / det::sqrt_f32(3.0);
    let signs = [-1.0_f32, 1.0_f32];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(24);

    for sx in signs {
        for sy in signs {
            for sz in signs {
                let px = Vec3::new(sx * r, 0.0, 0.0);
                let py = Vec3::new(0.0, sy * r, 0.0);
                let pz = Vec3::new(0.0, 0.0, sz * r);
                let normal = Vec3::new(sx * inv_sqrt3, sy * inv_sqrt3, sz * inv_sqrt3);
                let color = if sy > 0.0 {
                    Vec4::new(1.0, 0.86, 0.36, 1.0)
                } else {
                    Vec4::new(0.22, 0.60, 0.72, 1.0)
                };

                let tri = if sx * sy * sz > 0.0 {
                    [px, py, pz]
                } else {
                    [px, pz, py]
                };

                let base = vertices.len() as u32;
                for pos in tri {
                    vertices.push(MeshVertex::new(pos, normal, color));
                }
                indices.extend_from_slice(&[base, base + 1, base + 2]);
            }
        }
    }

    ProceduralMesh::new("unit_octahedron", vertices, indices)
}

/// Builds an upward-facing (`+Y` normal) checkerboard ground grid on `y = 0.0`
/// with `tiles_per_side × tiles_per_side` tiles of `tile_size_meters`.
#[must_use]
pub fn build_ground_grid_mesh(tiles_per_side: u32, tile_size_meters: f32) -> ProceduralMesh {
    let side = tiles_per_side.max(1);
    let step = if tile_size_meters.is_finite() && tile_size_meters > 0.0 {
        tile_size_meters
    } else {
        1.0
    };
    let half_span = (side as f32) * step * 0.5;
    let tile_count = (side as usize).saturating_mul(side as usize);

    let mut vertices = Vec::with_capacity(tile_count.saturating_mul(4));
    let mut indices = Vec::with_capacity(tile_count.saturating_mul(6));

    let center = side / 2;
    for gz in 0..side {
        for gx in 0..side {
            let x0 = -half_span + (gx as f32) * step;
            let x1 = x0 + step;
            let z0 = -half_span + (gz as f32) * step;
            let z1 = z0 + step;

            let is_even = ((gx + gz) & 1) == 0;
            let color = if gz == center && gx >= center {
                // +X world-right reference strip
                Vec4::new(0.42, 0.16, 0.16, 1.0)
            } else if gx == center && gz <= center {
                // -Z world-forward reference strip
                Vec4::new(0.14, 0.36, 0.38, 1.0)
            } else if is_even {
                Vec4::new(0.16, 0.19, 0.24, 1.0)
            } else {
                Vec4::new(0.09, 0.11, 0.15, 1.0)
            };

            // On `y = 0` with outward normal `+Y = (0, 1, 0)`:
            // `p0 = (x0, 0, z1)`, `p1 = (x1, 0, z1)`, `p2 = (x1, 0, z0)`, `p3 = (x0, 0, z0)`
            // `(p1 - p0) × (p2 - p0) = (step, 0, 0) × (step, 0, -step) = (0, +step^2, 0)` -> CCW!
            let base = vertices.len() as u32;
            vertices.push(MeshVertex::new(Vec3::new(x0, 0.0, z1), Vec3::Y, color));
            vertices.push(MeshVertex::new(Vec3::new(x1, 0.0, z1), Vec3::Y, color));
            vertices.push(MeshVertex::new(Vec3::new(x1, 0.0, z0), Vec3::Y, color));
            vertices.push(MeshVertex::new(Vec3::new(x0, 0.0, z0), Vec3::Y, color));
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    ProceduralMesh::new("ground_grid", vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{FreeCamera, FreeCameraConfig};

    #[test]
    fn default_scene_meshes_are_valid_and_strictly_ccw_outward() {
        let scene = ProceduralScene::default_scene().unwrap();
        assert_eq!(scene.meshes().len(), 3);

        for mesh in scene.meshes() {
            mesh.validate().unwrap();
            let verts = mesh.vertices();
            for &[i0, i1, i2] in mesh.indices().as_chunks::<3>().0 {
                let p0 = verts.get(i0 as usize).unwrap().position;
                let p1 = verts.get(i1 as usize).unwrap().position;
                let p2 = verts.get(i2 as usize).unwrap().position;
                let n0 = verts.get(i0 as usize).unwrap().normal;
                let geometric_normal = (p1 - p0).cross(p2 - p0);
                assert!(
                    geometric_normal.dot(n0) > 1.0e-5,
                    "mesh `{}` triangle [{i0}, {i1}, {i2}] is not counter-clockwise relative to its outward normal",
                    mesh.label(),
                );
            }
        }

        // Verify required reference depths (-2 m, -5 m, -25 m, -200 m) and far-after-near ordering.
        let z_positions: Vec<f64> = scene.instances().iter().map(|i| i.world_pos.z).collect();
        for expected_z in [-2.0, -5.0, -25.0, -200.0] {
            assert!(
                z_positions.contains(&expected_z),
                "expected scene marker at z = {expected_z}, got {z_positions:?}"
            );
        }
        let near_pos = z_positions.iter().position(|&z| z == -2.0).unwrap();
        let mid_pos = z_positions.iter().position(|&z| z == -5.0).unwrap();
        assert!(
            near_pos < mid_pos,
            "-2.0 m marker should precede -5.0 m marker so reversed-Z depth test is exercised"
        );
    }

    #[test]
    fn extract_draws_converts_world_positions_and_reuses_buffer_capacity() {
        let scene = ProceduralScene::default_scene().unwrap();
        let cam = FreeCamera::new(FreeCameraConfig::default()).unwrap();
        let pres = cam.sample_presentation(0.0, 16.0 / 9.0).unwrap();
        let handles = [
            MeshHandle::from_raw(0, 1),
            MeshHandle::from_raw(1, 1),
            MeshHandle::from_raw(2, 1),
        ];

        let mut draws = Vec::new();
        scene.extract_draws(&handles, &pres, &mut draws).unwrap();
        assert_eq!(draws.len(), scene.instances().len());
        let cap_after_first = draws.capacity();

        scene.extract_draws(&handles, &pres, &mut draws).unwrap();
        assert_eq!(draws.capacity(), cap_after_first);

        // Wrong handle count fails cleanly with ENGN-0010.
        let err = scene
            .extract_draws(&handles[..2], &pres, &mut draws)
            .unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Scene);
        assert_eq!(err.code(), "ENGN-0010");
    }
}
