//! Private Vulkan POD upload structs (`bytemuck`), static vertex/index staging
//! preparation, and prepared indexed draw records (`ADR-0004`, `SPN-INV-011`).

use ash::vk;
use bytemuck::{Pod, Zeroable};

use crate::allocator::MemoryDomain;
use crate::error::{RenderError, RenderErrorKind};
use crate::handle::{GenerationalPool, MeshHandle};
use crate::mesh::{
    CameraRelativeView, MESH_PUSH_CONSTANT_BYTES, MESH_VERTEX_STRIDE, MeshDescriptor, MeshDraw,
    MeshVertex, PackedMeshPushConstants,
};
use crate::vulkan::device::DeviceOwner;
use crate::vulkan::memory::{VulkanBuffer, VulkanMemoryAllocator};

/// Private POD mirror of [`MeshVertex`] (`40` bytes; `bytemuck` stays strictly
/// inside `src/vulkan/` per `SPN-INV-011`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct GpuMeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuMeshVertex>() == MESH_VERTEX_STRIDE as usize);

impl GpuMeshVertex {
    #[inline]
    pub(crate) const fn from_vertex(v: &MeshVertex) -> Self {
        Self {
            position: v.position.to_array(),
            normal: v.normal.to_array(),
            color: v.color.to_array(),
        }
    }
}

/// Private POD mirror of [`PackedMeshPushConstants`] (`128` bytes) matching
/// `MeshPushConstants` in `shaders/mesh_3d.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct GpuMeshPushConstants {
    pub mvp_camera_relative: [[f32; 4]; 4],
    pub normal_col0: [f32; 4],
    pub normal_col1: [f32; 4],
    pub normal_col2: [f32; 4],
    pub base_color: [f32; 4],
}

const _: () =
    assert!(std::mem::size_of::<GpuMeshPushConstants>() == MESH_PUSH_CONSTANT_BYTES as usize);

impl GpuMeshPushConstants {
    #[inline]
    pub(crate) const fn from_packed(p: &PackedMeshPushConstants) -> Self {
        Self {
            mvp_camera_relative: p.mvp_camera_relative.to_cols_array_2d(),
            normal_col0: p.normal_col0.to_array(),
            normal_col1: p.normal_col1.to_array(),
            normal_col2: p.normal_col2.to_array(),
            base_color: p.base_color.to_array(),
        }
    }
}

/// Resident `DEVICE_LOCAL` vertex + index buffer pair stored in the renderer's
/// generational mesh pool.
pub(crate) struct UploadedMesh {
    pub vertex_buffer: VulkanBuffer,
    pub index_buffer: VulkanBuffer,
    pub index_count: u32,
}

/// Staging + destination buffers prepared for `vkCmdCopyBuffer2`.
pub(crate) struct StagedMeshUpload {
    pub staging_buffer: VulkanBuffer,
    pub mesh: UploadedMesh,
    pub vertex_bytes: u64,
    pub index_bytes: u64,
}

/// Validated GPU draw command ready for command-buffer recording.
#[derive(Clone, Copy)]
pub(crate) struct PreparedMeshDraw {
    pub vertex_buffer: vk::Buffer,
    pub index_buffer: vk::Buffer,
    pub index_count: u32,
    pub push_constants: GpuMeshPushConstants,
}

/// Validates `descriptor`, populates a `HOST_VISIBLE | HOST_COHERENT` staging
/// buffer (`[0..vertex_bytes)` vertices, `[vertex_bytes..total)` indices), and
/// allocates `DEVICE_LOCAL` vertex and index buffers with `VK_EXT_debug_utils`
/// labels. Unwinds in reverse order on any allocation failure.
#[track_caller]
pub(crate) fn prepare_mesh_staging_upload(
    device: &DeviceOwner,
    allocator: &mut VulkanMemoryAllocator,
    descriptor: &MeshDescriptor<'_>,
) -> Result<StagedMeshUpload, RenderError> {
    let (vertex_bytes, index_bytes) = descriptor.validate()?;
    let index_count = u32::try_from(descriptor.indices.len()).map_err(|_| {
        RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
            .with_detail("mesh index count overflows u32")
    })?;
    let staging_total_bytes = vertex_bytes.checked_add(index_bytes).ok_or_else(|| {
        RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
            .with_detail("mesh combined staging byte size overflow")
    })?;

    let mut gpu_vertices = Vec::with_capacity(descriptor.vertices.len());
    for v in descriptor.vertices {
        gpu_vertices.push(GpuMeshVertex::from_vertex(v));
    }
    let vertex_raw: &[u8] = bytemuck::cast_slice(&gpu_vertices);
    let index_raw: &[u8] = bytemuck::cast_slice(descriptor.indices);

    let staging_buffer = VulkanBuffer::create(
        device,
        allocator,
        staging_total_bytes,
        vk::BufferUsageFlags::TRANSFER_SRC,
        MemoryDomain::HostVisible,
        &format!("spawn.mesh.{}.staging", descriptor.label),
    )?;

    if let Err(error) = allocator
        .write_host_bytes(&staging_buffer.suballocation, 0, vertex_raw)
        .and_then(|()| {
            allocator.write_host_bytes(&staging_buffer.suballocation, vertex_bytes, index_raw)
        })
    {
        let _ = allocator.retire_buffer(device, staging_buffer, u64::MAX, 0);
        return Err(error);
    }

    let vertex_buffer = match VulkanBuffer::create(
        device,
        allocator,
        vertex_bytes,
        vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        MemoryDomain::DeviceLocal,
        &format!("spawn.mesh.{}.vb", descriptor.label),
    ) {
        Ok(buf) => buf,
        Err(error) => {
            let _ = allocator.retire_buffer(device, staging_buffer, u64::MAX, 0);
            return Err(error);
        }
    };

    let index_buffer = match VulkanBuffer::create(
        device,
        allocator,
        index_bytes,
        vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        MemoryDomain::DeviceLocal,
        &format!("spawn.mesh.{}.ib", descriptor.label),
    ) {
        Ok(buf) => buf,
        Err(error) => {
            let _ = allocator.retire_buffer(device, vertex_buffer, u64::MAX, 0);
            let _ = allocator.retire_buffer(device, staging_buffer, u64::MAX, 0);
            return Err(error);
        }
    };

    Ok(StagedMeshUpload {
        staging_buffer,
        mesh: UploadedMesh {
            vertex_buffer,
            index_buffer,
            index_count,
        },
        vertex_bytes,
        index_bytes,
    })
}

/// Validates `view` and `draws` and packs draw records into `out`, reusing `out`'s
/// existing heap allocation across frames so steady-state rendering performs zero
/// heap allocations after warm-up.
#[track_caller]
pub(crate) fn prepare_mesh_draws_into(
    meshes: &GenerationalPool<MeshHandle, UploadedMesh>,
    out: &mut Vec<PreparedMeshDraw>,
    view: &CameraRelativeView,
    draws: &[MeshDraw],
) -> Result<(), RenderError> {
    out.clear();
    if !view.projection.is_finite() || !view.camera_rotation.is_finite() {
        return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
            .with_detail("CameraRelativeView contains NaN or infinite values"));
    }
    out.reserve(draws.len());
    for draw in draws {
        let uploaded = meshes.get(draw.mesh)?;
        let packed = draw.pack_push_constants(view)?;
        out.push(PreparedMeshDraw {
            vertex_buffer: uploaded.vertex_buffer.handle,
            index_buffer: uploaded.index_buffer.handle,
            index_count: uploaded.index_count,
            push_constants: GpuMeshPushConstants::from_packed(&packed),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::{AllocationId, BlockId, Suballocation, SuballocationKind};
    use crate::handle::MeshHandle;
    use crate::mesh::{CameraRelativeView, MeshDraw};
    use ash::vk::Handle as _;
    use spawn_math::{Mat4, Quat, Vec3, Vec4};

    fn dummy_buffer(raw_handle: u64) -> VulkanBuffer {
        VulkanBuffer {
            handle: vk::Buffer::from_raw(raw_handle),
            suballocation: Suballocation {
                id: AllocationId(raw_handle),
                block_id: BlockId(1),
                domain: MemoryDomain::DeviceLocal,
                memory_type_index: 0,
                offset: 0,
                size: 64,
                alignment: 16,
                kind: SuballocationKind::Linear,
                dedicated: false,
            },
        }
    }

    #[test]
    fn bytemuck_pod_push_constants_match_headless_le_bytes_exact() {
        let proj = Mat4::perspective_reversed_z_rh(1.2, 1.5, 0.1, 200.0).unwrap();
        let view = CameraRelativeView::new(proj, Quat::from_rotation_y(0.5));
        let draw = MeshDraw::new(MeshHandle::from_raw(0, 1), Vec3::new(1.5, -2.5, -6.0))
            .with_rotation(Quat::from_rotation_z(-0.3))
            .with_scale(Vec3::new(1.0, 2.0, 3.0))
            .with_base_color(Vec4::new(0.25, 0.50, 0.75, 1.0))
            .with_shade_weight(0.65);

        let packed = draw.pack_push_constants(&view).unwrap();
        let pod = GpuMeshPushConstants::from_packed(&packed);
        assert_eq!(bytemuck::bytes_of(&pod), &packed.to_le_bytes());
    }

    #[test]
    fn prepare_mesh_draws_reuses_capacity_without_reallocating_after_warmup() {
        let mut meshes = GenerationalPool::new();
        let mesh_a = meshes
            .insert(UploadedMesh {
                vertex_buffer: dummy_buffer(1),
                index_buffer: dummy_buffer(2),
                index_count: 36,
            })
            .unwrap();
        let mesh_b = meshes
            .insert(UploadedMesh {
                vertex_buffer: dummy_buffer(3),
                index_buffer: dummy_buffer(4),
                index_count: 6,
            })
            .unwrap();

        let proj = Mat4::perspective_reversed_z_rh(1.2, 1.5, 0.1, 200.0).unwrap();
        let view = CameraRelativeView::new(proj, Quat::IDENTITY);
        let draws_8: Vec<MeshDraw> = (0..8)
            .map(|i| {
                let handle = if i % 2 == 0 { mesh_a } else { mesh_b };
                MeshDraw::new(handle, Vec3::new(i as f32, 0.0, -4.0))
            })
            .collect();

        let mut scratch = Vec::new();
        prepare_mesh_draws_into(&meshes, &mut scratch, &view, &draws_8).unwrap();
        assert_eq!(scratch.len(), 8);
        let cap_after_warmup = scratch.capacity();
        let ptr_after_warmup = scratch.as_ptr();
        assert!(cap_after_warmup >= 8);

        // Smaller draw list, empty draw list, and peak-size draw list must all
        // reuse the exact backing allocation with zero reallocation.
        prepare_mesh_draws_into(&meshes, &mut scratch, &view, &draws_8[..3]).unwrap();
        assert_eq!(scratch.len(), 3);
        assert_eq!(scratch.capacity(), cap_after_warmup);
        assert_eq!(scratch.as_ptr(), ptr_after_warmup);

        prepare_mesh_draws_into(&meshes, &mut scratch, &view, &[]).unwrap();
        assert!(scratch.is_empty());
        assert_eq!(scratch.capacity(), cap_after_warmup);
        assert_eq!(scratch.as_ptr(), ptr_after_warmup);

        prepare_mesh_draws_into(&meshes, &mut scratch, &view, &draws_8).unwrap();
        assert_eq!(scratch.len(), 8);
        assert_eq!(scratch.capacity(), cap_after_warmup);
        assert_eq!(scratch.as_ptr(), ptr_after_warmup);

        // An invalid draw clears stale entries without dropping capacity.
        let bad_draws = [MeshDraw::new(MeshHandle::from_raw(99, 1), Vec3::ZERO)];
        let err = prepare_mesh_draws_into(&meshes, &mut scratch, &view, &bad_draws).unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidResourceHandle);
        assert!(scratch.is_empty());
        assert_eq!(scratch.capacity(), cap_after_warmup);
        assert_eq!(scratch.as_ptr(), ptr_after_warmup);

        // Subsequent valid steady-state frame still performs zero reallocations.
        prepare_mesh_draws_into(&meshes, &mut scratch, &view, &draws_8).unwrap();
        assert_eq!(scratch.len(), 8);
        assert_eq!(scratch.capacity(), cap_after_warmup);
        assert_eq!(scratch.as_ptr(), ptr_after_warmup);
    }
}
