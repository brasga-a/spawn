//! Vulkan backing memory blocks, suballocated buffers/images, and timeline-safe
//! deferred destruction (`ADR-0004` decision 3, `F3D-FR-06`, `F3D-INV-13`).

use std::collections::BTreeMap;
use std::ptr::NonNull;

use ash::vk;

use crate::allocator::{
    AllocationRequest, BlockId, GpuAllocatorStats, GpuMemoryBudget, MemoryDomain, PureGpuAllocator,
    Suballocation, SuballocationKind,
};
use crate::error::{RenderError, RenderErrorKind};
use crate::types::SurfaceSize;
use crate::vulkan::device::DeviceOwner;
use crate::vulkan::vk_error;

struct BackingBlock {
    memory: vk::DeviceMemory,
    size: u64,
    mapped: Option<NonNull<u8>>,
}

enum DeferredObject {
    Buffer(vk::Buffer),
    Image {
        image: vk::Image,
        view: vk::ImageView,
    },
    Pipeline {
        pipeline: vk::Pipeline,
        layout: vk::PipelineLayout,
    },
    ShaderModule(vk::ShaderModule),
}

struct RetiredVulkanObject {
    object: DeferredObject,
    last_used_timeline: u64,
}

/// Suballocated `VkBuffer` bound to a [`VulkanMemoryAllocator`] block.
pub(crate) struct VulkanBuffer {
    pub handle: vk::Buffer,
    pub suballocation: Suballocation,
}

impl VulkanBuffer {
    pub(crate) fn create(
        device: &DeviceOwner,
        allocator: &mut VulkanMemoryAllocator,
        size: u64,
        usage: vk::BufferUsageFlags,
        domain: MemoryDomain,
        debug_name: &str,
    ) -> Result<Self, RenderError> {
        if size == 0 {
            return Err(RenderError::new(RenderErrorKind::GpuMemoryExhausted)
                .with_detail("VkBuffer byte size must be greater than 0"));
        }
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `device.device` is valid and `info` has `size > 0` with valid flags.
        let handle = unsafe { device.device.create_buffer(&info, None) }
            .map_err(|result| vk_alloc_or_call_error("vkCreateBuffer", result))?;
        device.name_str(handle, debug_name);

        // SAFETY: `handle` was just created on `device.device`.
        let reqs = unsafe { device.device.get_buffer_memory_requirements(handle) };
        let (suballocation, memory) = match allocator.allocate(
            device,
            reqs.memory_type_bits,
            reqs.size.max(size),
            reqs.alignment.max(1),
            domain,
            SuballocationKind::Linear,
        ) {
            Ok(pair) => pair,
            Err(error) => {
                // SAFETY: `handle` has not been submitted or bound; destroyed once on unwind.
                unsafe { device.device.destroy_buffer(handle, None) };
                return Err(error);
            }
        };

        // SAFETY: `memory` belongs to `device.device`, `suballocation.offset` is aligned to
        // `reqs.alignment` and separated by `bufferImageGranularity` from optimal images, and
        // `suballocation.size >= reqs.size`.
        if let Err(result) = unsafe {
            device
                .device
                .bind_buffer_memory(handle, memory, suballocation.offset)
        } {
            // SAFETY: `handle` was never submitted; destroyed before freeing the suballocation.
            unsafe { device.device.destroy_buffer(handle, None) };
            let _ = allocator.free_now(device, suballocation);
            return Err(vk_alloc_or_call_error("vkBindBufferMemory", result));
        }

        Ok(Self {
            handle,
            suballocation,
        })
    }
}

/// Suballocated 2D optimal-tiling `VkImage` and its `VkImageView`.
pub(crate) struct VulkanImage {
    pub image: vk::Image,
    pub view: vk::ImageView,
    #[cfg(test)]
    pub extent: SurfaceSize,
    pub suballocation: Suballocation,
}

impl VulkanImage {
    pub(crate) fn create_2d(
        device: &DeviceOwner,
        allocator: &mut VulkanMemoryAllocator,
        extent: SurfaceSize,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        aspect: vk::ImageAspectFlags,
        debug_name: &str,
    ) -> Result<Self, RenderError> {
        if extent.is_empty() {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "cannot create 2D image `{debug_name}` with empty extent {}x{}",
                    extent.width, extent.height
                )),
            );
        }
        if extent.width > device.max_image_dimension_2d
            || extent.height > device.max_image_dimension_2d
        {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "2D image `{debug_name}` extent {}x{} exceeds maxImageDimension2D ({})",
                    extent.width, extent.height, device.max_image_dimension_2d
                )),
            );
        }

        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: `device.device` is valid and `extent` is non-zero within device limits.
        let image = unsafe { device.device.create_image(&image_info, None) }
            .map_err(|result| vk_alloc_or_call_error("vkCreateImage", result))?;
        device.name_str(image, debug_name);

        // SAFETY: `image` was just created on `device.device`.
        let reqs = unsafe { device.device.get_image_memory_requirements(image) };
        let (suballocation, memory) = match allocator.allocate(
            device,
            reqs.memory_type_bits,
            reqs.size.max(1),
            reqs.alignment.max(1),
            MemoryDomain::DeviceLocal,
            SuballocationKind::Optimal,
        ) {
            Ok(pair) => pair,
            Err(error) => {
                // SAFETY: `image` was never bound or submitted; destroyed once on unwind.
                unsafe { device.device.destroy_image(image, None) };
                return Err(error);
            }
        };

        // SAFETY: `memory` belongs to `device.device`, `suballocation.offset` honors both
        // `reqs.alignment` and `bufferImageGranularity`, and `suballocation.size >= reqs.size`.
        if let Err(result) = unsafe {
            device
                .device
                .bind_image_memory(image, memory, suballocation.offset)
        } {
            // SAFETY: `image` was never submitted; destroyed before releasing suballocation.
            unsafe { device.device.destroy_image(image, None) };
            let _ = allocator.free_now(device, suballocation);
            return Err(vk_alloc_or_call_error("vkBindImageMemory", result));
        }

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(aspect)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        // SAFETY: `image` is bound to valid device memory on `device.device`.
        let view = match unsafe { device.device.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(result) => {
                // SAFETY: `image` has no live view and was never submitted; destroyed before freeing memory.
                unsafe { device.device.destroy_image(image, None) };
                let _ = allocator.free_now(device, suballocation);
                return Err(vk_alloc_or_call_error("vkCreateImageView", result));
            }
        };
        device.name_str(view, &format!("{debug_name}.view"));

        Ok(Self {
            image,
            view,
            #[cfg(test)]
            extent,
            suballocation,
        })
    }
}

/// Pairs the pure [`PureGpuAllocator`] with `VkDeviceMemory` blocks, persistent
/// `HOST_VISIBLE | HOST_COHERENT` mappings, and timeline-gated Vulkan handle destruction.
pub(crate) struct VulkanMemoryAllocator {
    pure: PureGpuAllocator,
    blocks: BTreeMap<BlockId, BackingBlock>,
    retired_objects: Vec<RetiredVulkanObject>,
}

impl VulkanMemoryAllocator {
    #[track_caller]
    pub(crate) fn new(
        budget: GpuMemoryBudget,
        buffer_image_granularity: u64,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            pure: PureGpuAllocator::new(budget, buffer_image_granularity)?,
            blocks: BTreeMap::new(),
            retired_objects: Vec::new(),
        })
    }

    pub(crate) fn stats(&self) -> GpuAllocatorStats {
        self.pure.stats()
    }

    #[track_caller]
    pub(crate) fn allocate(
        &mut self,
        device: &DeviceOwner,
        memory_type_bits: u32,
        size: u64,
        alignment: u64,
        domain: MemoryDomain,
        kind: SuballocationKind,
    ) -> Result<(Suballocation, vk::DeviceMemory), RenderError> {
        let memory_type_index =
            select_memory_type(&device.memory_properties, memory_type_bits, domain)?;
        let req = AllocationRequest {
            domain,
            memory_type_index,
            size,
            alignment,
            kind,
        };

        if let Some(sub) = self.pure.try_suballocate_existing(req)? {
            let backing = self.blocks.get(&sub.block_id).ok_or_else(|| {
                RenderError::new(RenderErrorKind::VulkanCallFailed)
                    .with_detail(format!("missing Vulkan backing block {}", sub.block_id.0))
            })?;
            return Ok((sub, backing.memory));
        }

        let plan = self.pure.plan_new_block(req)?;
        for evicted_id in self.pure.commit_evictions(&plan.evict_blocks) {
            if let Some(old_block) = self.blocks.remove(&evicted_id) {
                free_backing_block(device, old_block);
            }
        }

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(plan.size)
            .memory_type_index(plan.memory_type_index);
        // SAFETY: `plan.memory_type_index` is valid for `device` and `plan.size > 0`.
        let memory = unsafe { device.device.allocate_memory(&alloc_info, None) }
            .map_err(|result| vk_alloc_or_call_error("vkAllocateMemory", result))?;
        device.name_str(
            memory,
            &format!(
                "spawn.memory.{}.{}",
                domain.as_str().to_ascii_lowercase(),
                plan.id.0
            ),
        );

        let mapped = if domain == MemoryDomain::HostVisible {
            // SAFETY: `memory` was just allocated from a `HOST_VISIBLE | HOST_COHERENT`
            // memory type and is not currently mapped; mapped once for the block's lifetime.
            match unsafe {
                device
                    .device
                    .map_memory(memory, 0, plan.size, vk::MemoryMapFlags::empty())
            } {
                Ok(raw_ptr) => match NonNull::new(raw_ptr.cast::<u8>()) {
                    Some(ptr) => Some(ptr),
                    None => {
                        // SAFETY: unmapping and freeing the unreferenced block on null map pointer.
                        unsafe {
                            device.device.unmap_memory(memory);
                            device.device.free_memory(memory, None);
                        }
                        return Err(RenderError::new(RenderErrorKind::GpuMemoryExhausted)
                            .with_detail("vkMapMemory returned a null pointer"));
                    }
                },
                Err(result) => {
                    // SAFETY: `memory` has no bound objects and is freed once on map failure.
                    unsafe { device.device.free_memory(memory, None) };
                    return Err(vk_alloc_or_call_error("vkMapMemory", result));
                }
            }
        } else {
            None
        };

        let sub = match self.pure.register_block_and_allocate(&plan, req) {
            Ok(sub) => sub,
            Err(error) => {
                free_backing_block(
                    device,
                    BackingBlock {
                        memory,
                        size: plan.size,
                        mapped,
                    },
                );
                return Err(error);
            }
        };

        self.blocks.insert(
            plan.id,
            BackingBlock {
                memory,
                size: plan.size,
                mapped,
            },
        );
        Ok((sub, memory))
    }

    /// Reads `byte_len` bytes from a persistently mapped `HOST_VISIBLE | HOST_COHERENT`
    /// suballocation after the caller has synchronized GPU writes (`COPY -> HOST_READ`
    /// barrier + timeline semaphore wait).
    #[track_caller]
    pub(crate) fn read_host_bytes(
        &self,
        sub: &Suballocation,
        byte_len: usize,
    ) -> Result<Vec<u8>, RenderError> {
        if sub.domain != MemoryDomain::HostVisible {
            return Err(RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("cannot read host bytes from a non-HOST_VISIBLE suballocation"));
        }
        let len_u64 = u64::try_from(byte_len).map_err(|_| {
            RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("readback byte length overflows u64")
        })?;
        if len_u64 > sub.size {
            return Err(
                RenderError::new(RenderErrorKind::InvalidResourceDescriptor).with_detail(format!(
                    "readback length {len_u64} B exceeds suballocation size {} B",
                    sub.size
                )),
            );
        }
        let backing = self.blocks.get(&sub.block_id).ok_or_else(|| {
            RenderError::new(RenderErrorKind::VulkanCallFailed)
                .with_detail(format!("missing mapped backing block {}", sub.block_id.0))
        })?;
        let end = sub.offset.checked_add(len_u64).ok_or_else(|| {
            RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("readback offset + length overflow")
        })?;
        if end > backing.size {
            return Err(RenderError::new(RenderErrorKind::VulkanCallFailed)
                .with_detail("suballocation range exceeds backing block size"));
        }
        let base_ptr = backing.mapped.ok_or_else(|| {
            RenderError::new(RenderErrorKind::VulkanCallFailed)
                .with_detail("HOST_VISIBLE backing block is missing persistent mapping")
        })?;
        let offset_usize = usize::try_from(sub.offset).map_err(|_| {
            RenderError::new(RenderErrorKind::InvalidResourceDescriptor)
                .with_detail("suballocation offset overflows usize")
        })?;

        // SAFETY: `backing.mapped` points to a valid `HOST_VISIBLE | HOST_COHERENT`
        // mapping of `backing.size` bytes, `offset_usize + byte_len <= backing.size`,
        // and the caller completed the `COPY -> HOST_READ` barrier + timeline wait so no
        // GPU write races with this read.
        let slice =
            unsafe { std::slice::from_raw_parts(base_ptr.as_ptr().add(offset_usize), byte_len) };
        Ok(slice.to_vec())
    }

    #[track_caller]
    pub(crate) fn free_now(
        &mut self,
        device: &DeviceOwner,
        sub: Suballocation,
    ) -> Result<(), RenderError> {
        if let Some(freed_block_id) = self.pure.free_now(sub)?
            && let Some(backing) = self.blocks.remove(&freed_block_id)
        {
            free_backing_block(device, backing);
        }
        Ok(())
    }

    /// Retires `buffer` and its suballocation after `last_used_timeline` (or frees
    /// immediately if `completed_timeline >= last_used_timeline`).
    #[track_caller]
    pub(crate) fn retire_buffer(
        &mut self,
        device: &DeviceOwner,
        buffer: VulkanBuffer,
        completed_timeline: u64,
        last_used_timeline: u64,
    ) -> Result<(), RenderError> {
        if completed_timeline >= last_used_timeline {
            // SAFETY: all GPU work up to `last_used_timeline` has completed; destroyed before memory free.
            unsafe { device.device.destroy_buffer(buffer.handle, None) };
            return self.free_now(device, buffer.suballocation);
        }
        self.pure.retire(buffer.suballocation, last_used_timeline)?;
        self.retired_objects.push(RetiredVulkanObject {
            object: DeferredObject::Buffer(buffer.handle),
            last_used_timeline,
        });
        Ok(())
    }

    /// Retires `image` (plus its `VkImageView` and suballocation) after `last_used_timeline`
    /// (or frees immediately if `completed_timeline >= last_used_timeline`).
    #[track_caller]
    pub(crate) fn retire_image(
        &mut self,
        device: &DeviceOwner,
        image: VulkanImage,
        completed_timeline: u64,
        last_used_timeline: u64,
    ) -> Result<(), RenderError> {
        if completed_timeline >= last_used_timeline {
            // SAFETY: all GPU work up to `last_used_timeline` has completed; view destroyed
            // before image, and image destroyed before freeing backing memory (`VUID-00677`).
            unsafe {
                device.device.destroy_image_view(image.view, None);
                device.device.destroy_image(image.image, None);
            }
            return self.free_now(device, image.suballocation);
        }
        self.pure.retire(image.suballocation, last_used_timeline)?;
        self.retired_objects.push(RetiredVulkanObject {
            object: DeferredObject::Image {
                image: image.image,
                view: image.view,
            },
            last_used_timeline,
        });
        Ok(())
    }

    /// Retires a `VkPipeline` and its `VkPipelineLayout` until `last_used_timeline` completes.
    pub(crate) fn retire_pipeline(
        &mut self,
        device: &DeviceOwner,
        pipeline: vk::Pipeline,
        layout: vk::PipelineLayout,
        completed_timeline: u64,
        last_used_timeline: u64,
    ) {
        if completed_timeline >= last_used_timeline {
            destroy_deferred_object(device, DeferredObject::Pipeline { pipeline, layout });
        } else {
            self.retired_objects.push(RetiredVulkanObject {
                object: DeferredObject::Pipeline { pipeline, layout },
                last_used_timeline,
            });
        }
    }

    /// Retires a `VkShaderModule` until `last_used_timeline` completes.
    pub(crate) fn retire_shader_module(
        &mut self,
        device: &DeviceOwner,
        module: vk::ShaderModule,
        completed_timeline: u64,
        last_used_timeline: u64,
    ) {
        if completed_timeline >= last_used_timeline {
            destroy_deferred_object(device, DeferredObject::ShaderModule(module));
        } else {
            self.retired_objects.push(RetiredVulkanObject {
                object: DeferredObject::ShaderModule(module),
                last_used_timeline,
            });
        }
    }

    /// Reclaims all retired Vulkan handles and memory suballocations whose
    /// `last_used_timeline <= completed_timeline`, destroying child handles
    /// strictly before freeing any dedicated `VkDeviceMemory` blocks.
    pub(crate) fn reclaim_completed(&mut self, device: &DeviceOwner, completed_timeline: u64) {
        let mut remaining = Vec::with_capacity(self.retired_objects.len());
        for entry in self.retired_objects.drain(..) {
            if completed_timeline >= entry.last_used_timeline {
                destroy_deferred_object(device, entry.object);
            } else {
                remaining.push(entry);
            }
        }
        self.retired_objects = remaining;

        for freed_block_id in self.pure.reclaim_completed(completed_timeline) {
            if let Some(backing) = self.blocks.remove(&freed_block_id) {
                free_backing_block(device, backing);
            }
        }
    }

    /// Drains all retired handles and frees all `VkDeviceMemory` blocks on shutdown.
    /// Caller must ensure `device.wait_idle()` has completed first.
    pub(crate) fn shutdown(&mut self, device: &DeviceOwner) -> usize {
        for entry in self.retired_objects.drain(..) {
            destroy_deferred_object(device, entry.object);
        }
        let (blocks_to_free, leaked) = self.pure.drain_all_blocks_for_shutdown();
        for block_id in blocks_to_free {
            if let Some(backing) = self.blocks.remove(&block_id) {
                free_backing_block(device, backing);
            }
        }
        while let Some((_id, backing)) = self.blocks.pop_first() {
            free_backing_block(device, backing);
        }
        debug_assert_eq!(
            leaked, 0,
            "VulkanMemoryAllocator::shutdown detected {leaked} leaked suballocation(s)"
        );
        leaked
    }
}

fn destroy_deferred_object(device: &DeviceOwner, object: DeferredObject) {
    // SAFETY: called only after GPU timeline completion (or `vkDeviceWaitIdle`) and
    // once per owned handle.
    unsafe {
        match object {
            DeferredObject::Buffer(buffer) => {
                device.device.destroy_buffer(buffer, None);
            }
            DeferredObject::Image { image, view } => {
                device.device.destroy_image_view(view, None);
                device.device.destroy_image(image, None);
            }
            DeferredObject::Pipeline { pipeline, layout } => {
                device.device.destroy_pipeline(pipeline, None);
                device.device.destroy_pipeline_layout(layout, None);
            }
            DeferredObject::ShaderModule(module) => {
                device.device.destroy_shader_module(module, None);
            }
        }
    }
}

fn free_backing_block(device: &DeviceOwner, backing: BackingBlock) {
    // SAFETY: all child buffers/images bound to `backing.memory` have been destroyed,
    // the GPU is no longer accessing the block, and each `BackingBlock` is freed once.
    unsafe {
        if backing.mapped.is_some() {
            device.device.unmap_memory(backing.memory);
        }
        device.device.free_memory(backing.memory, None);
    }
}

#[track_caller]
fn select_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    domain: MemoryDomain,
) -> Result<u32, RenderError> {
    let count = usize::try_from(props.memory_type_count)
        .unwrap_or(0)
        .min(props.memory_types.len());

    let find =
        |required: vk::MemoryPropertyFlags, excluded: vk::MemoryPropertyFlags| -> Option<u32> {
            for (idx, mem_type) in props.memory_types.iter().take(count).enumerate() {
                let idx_u32 = u32::try_from(idx).ok()?;
                if (type_bits & (1_u32 << idx_u32)) == 0 {
                    continue;
                }
                if mem_type.property_flags.contains(required)
                    && !mem_type.property_flags.intersects(excluded)
                {
                    return Some(idx_u32);
                }
            }
            None
        };

    let selected = match domain {
        MemoryDomain::DeviceLocal => {
            // Prefer pure `DEVICE_LOCAL` (saving host-visible BAR aperture on discrete GPUs),
            // then fall back to any `DEVICE_LOCAL` heap (e.g. unified memory / ReBAR).
            find(
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                vk::MemoryPropertyFlags::HOST_VISIBLE,
            )
            .or_else(|| {
                find(
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                    vk::MemoryPropertyFlags::empty(),
                )
            })
        }
        MemoryDomain::HostVisible => {
            let req =
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            // Prefer `HOST_CACHED` for fast readbacks when coherent, then any `HOST_VISIBLE | HOST_COHERENT`.
            find(
                req | vk::MemoryPropertyFlags::HOST_CACHED,
                vk::MemoryPropertyFlags::empty(),
            )
            .or_else(|| find(req, vk::MemoryPropertyFlags::empty()))
        }
    };

    selected.ok_or_else(|| {
        RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(format!(
            "no Vulkan memory type in mask {type_bits:#x} satisfies `{}`",
            domain.as_str()
        ))
    })
}

#[track_caller]
fn vk_alloc_or_call_error(call: &'static str, result: vk::Result) -> RenderError {
    match result {
        vk::Result::ERROR_OUT_OF_DEVICE_MEMORY | vk::Result::ERROR_OUT_OF_HOST_MEMORY => {
            RenderError::new(RenderErrorKind::GpuMemoryExhausted)
                .with_detail(call)
                .with_cause(&result)
        }
        _ => vk_error(call, result),
    }
}
