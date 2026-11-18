//! GPU enumeration, selection and logical device creation.

use std::ffi::CStr;

use ash::{ext, khr, vk};

use crate::error::{RenderError, RenderErrorKind};
use crate::selection::{
    DeviceCandidate, QueueFamily, RequiredFeatures, decode_version, select_device,
};
use crate::types::{GpuKind, RendererInfo};
use crate::vulkan::instance::InstanceOwner;
use crate::vulkan::surface::SurfaceOwner;
use crate::vulkan::vk_error;

/// Owns the logical device and its single graphics + present queue.
pub(crate) struct DeviceOwner {
    pub device: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub queue_family: u32,
    pub swapchain: khr::swapchain::Device,
    debug_utils: Option<ext::debug_utils::Device>,
}

impl DeviceOwner {
    /// Selects a GPU that can present to `surface` and creates the device.
    pub(crate) fn new(
        instance: &InstanceOwner,
        surface: &SurfaceOwner,
    ) -> Result<(Self, RendererInfo), RenderError> {
        // SAFETY: the instance is valid for the duration of the call.
        let physical_devices = unsafe { instance.instance.enumerate_physical_devices() }
            .map_err(|result| vk_error("vkEnumeratePhysicalDevices", result))?;
        let candidates = physical_devices
            .iter()
            .map(|&physical| query_candidate(instance, surface, physical))
            .collect::<Result<Vec<_>, _>>()?;
        let choice = select_device(&candidates).map_err(|rejections| {
            RenderError::new(RenderErrorKind::NoSuitableDevice).with_detail(rejections.join(" | "))
        })?;
        let physical = physical_devices
            .get(choice.candidate)
            .copied()
            .ok_or_else(|| RenderError::new(RenderErrorKind::NoSuitableDevice))?;

        let priorities = [1.0_f32];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(choice.queue_family)
            .queue_priorities(&priorities)];
        let extensions = [khr::swapchain::NAME.as_ptr()];
        let required = RequiredFeatures::ALL;
        let mut features12 = vk::PhysicalDeviceVulkan12Features::default()
            .timeline_semaphore(required.timeline_semaphore)
            .descriptor_indexing(required.descriptor_indexing)
            .runtime_descriptor_array(required.runtime_descriptor_array)
            .descriptor_binding_partially_bound(required.descriptor_binding_partially_bound)
            .descriptor_binding_variable_descriptor_count(
                required.descriptor_binding_variable_descriptor_count,
            )
            .shader_sampled_image_array_non_uniform_indexing(
                required.shader_sampled_image_array_non_uniform_indexing,
            )
            .descriptor_binding_sampled_image_update_after_bind(
                required.descriptor_binding_sampled_image_update_after_bind,
            )
            .descriptor_binding_storage_buffer_update_after_bind(
                required.descriptor_binding_storage_buffer_update_after_bind,
            );
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(required.dynamic_rendering)
            .synchronization2(required.synchronization2);
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&extensions)
            .push_next(&mut features12)
            .push_next(&mut features13);
        // SAFETY: `physical` was enumerated from this instance and supports
        // every enabled feature and extension (checked by `select_device`).
        let device = unsafe { instance.instance.create_device(physical, &info, None) }
            .map_err(|result| vk_error("vkCreateDevice", result))?;
        // SAFETY: the queue family and index 0 were requested above.
        let queue = unsafe { device.get_device_queue(choice.queue_family, 0) };
        let owner = Self {
            swapchain: khr::swapchain::Device::new(&instance.instance, &device),
            debug_utils: instance
                .debug_utils
                .then(|| ext::debug_utils::Device::new(&instance.instance, &device)),
            device,
            physical,
            queue,
            queue_family: choice.queue_family,
        };
        let info = renderer_info(instance, physical, &candidates[choice.candidate]);
        Ok((owner, info))
    }

    /// Gives a Vulkan object a debug name (RenderDoc, validation messages).
    /// Does nothing without debug-utils.
    pub(crate) fn name<H: vk::Handle>(&self, handle: H, name: &CStr) {
        if let Some(debug_utils) = &self.debug_utils {
            let info = vk::DebugUtilsObjectNameInfoEXT::default()
                .object_handle(handle)
                .object_name(name);
            // SAFETY: the handle belongs to this device; a failed naming call
            // only loses a debug label.
            let _ = unsafe { debug_utils.set_debug_utils_object_name(&info) };
        }
    }

    /// Waits until the GPU is idle.
    pub(crate) fn wait_idle(&self) -> Result<(), RenderError> {
        // SAFETY: the device is valid; no queue is used concurrently
        // (single-threaded renderer).
        unsafe { self.device.device_wait_idle() }
            .map_err(|result| vk_error("vkDeviceWaitIdle", result))
    }
}

impl Drop for DeviceOwner {
    fn drop(&mut self) {
        // SAFETY: every object created from the device (swapchain, frames) is
        // destroyed before this value is dropped (`VulkanRenderer::drop`); a
        // failed wait (device lost) still allows destruction.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
        }
    }
}

fn gpu_kind(kind: vk::PhysicalDeviceType) -> GpuKind {
    match kind {
        vk::PhysicalDeviceType::DISCRETE_GPU => GpuKind::Discrete,
        vk::PhysicalDeviceType::INTEGRATED_GPU => GpuKind::Integrated,
        vk::PhysicalDeviceType::VIRTUAL_GPU => GpuKind::Virtual,
        vk::PhysicalDeviceType::CPU => GpuKind::Cpu,
        _ => GpuKind::Other,
    }
}

fn query_candidate(
    instance: &InstanceOwner,
    surface: &SurfaceOwner,
    physical: vk::PhysicalDevice,
) -> Result<DeviceCandidate, RenderError> {
    let vk_instance = &instance.instance;
    // SAFETY: `physical` was enumerated from this instance and `surface`
    // belongs to it (as for every query in this function).
    let properties = unsafe { vk_instance.get_physical_device_properties(physical) };
    let name = properties.device_name_as_c_str().map_or_else(
        |_| "<unnamed GPU>".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    // SAFETY: see above.
    let extensions = unsafe { vk_instance.enumerate_device_extension_properties(physical) }
        .map_err(|result| vk_error("vkEnumerateDeviceExtensionProperties", result))?;
    let has_swapchain_extension = extensions
        .iter()
        .any(|properties| properties.extension_name_as_c_str() == Ok(khr::swapchain::NAME));

    let (major, minor, _) = decode_version(properties.api_version);
    let features = ((major, minor) >= (1, 3)).then(|| {
        let mut features12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default();
        {
            let mut features = vk::PhysicalDeviceFeatures2::default()
                .push_next(&mut features12)
                .push_next(&mut features13);
            // SAFETY: see above; the 1.2/1.3 structures are valid because the
            // device supports Vulkan 1.3.
            unsafe { vk_instance.get_physical_device_features2(physical, &mut features) };
        }
        let on = |bit: vk::Bool32| bit == vk::TRUE;
        RequiredFeatures {
            dynamic_rendering: on(features13.dynamic_rendering),
            synchronization2: on(features13.synchronization2),
            timeline_semaphore: on(features12.timeline_semaphore),
            descriptor_indexing: on(features12.descriptor_indexing),
            runtime_descriptor_array: on(features12.runtime_descriptor_array),
            descriptor_binding_partially_bound: on(features12.descriptor_binding_partially_bound),
            descriptor_binding_variable_descriptor_count: on(
                features12.descriptor_binding_variable_descriptor_count
            ),
            shader_sampled_image_array_non_uniform_indexing: on(
                features12.shader_sampled_image_array_non_uniform_indexing
            ),
            descriptor_binding_sampled_image_update_after_bind: on(
                features12.descriptor_binding_sampled_image_update_after_bind
            ),
            descriptor_binding_storage_buffer_update_after_bind: on(
                features12.descriptor_binding_storage_buffer_update_after_bind
            ),
        }
    });

    // SAFETY: see above.
    let families = unsafe { vk_instance.get_physical_device_queue_family_properties(physical) };
    let queue_families = families
        .iter()
        .enumerate()
        .map(|(index, family)| {
            let present = u32::try_from(index).is_ok_and(|index| {
                // SAFETY: see above; `index` is a valid family index.
                unsafe {
                    surface.loader.get_physical_device_surface_support(
                        physical,
                        index,
                        surface.handle,
                    )
                }
                .unwrap_or(false)
            });
            QueueFamily {
                graphics: family.queue_flags.contains(vk::QueueFlags::GRAPHICS),
                present,
            }
        })
        .collect();
    // SAFETY: see above.
    let surface_formats = unsafe {
        surface
            .loader
            .get_physical_device_surface_formats(physical, surface.handle)
    }
    .map_or(0, |formats| formats.len());
    // SAFETY: see above.
    let present_modes = unsafe {
        surface
            .loader
            .get_physical_device_surface_present_modes(physical, surface.handle)
    }
    .map_or(0, |modes| modes.len());

    Ok(DeviceCandidate {
        name,
        kind: gpu_kind(properties.device_type),
        api_version: properties.api_version,
        has_swapchain_extension,
        features,
        queue_families,
        surface_formats,
        present_modes,
    })
}

fn renderer_info(
    instance: &InstanceOwner,
    physical: vk::PhysicalDevice,
    candidate: &DeviceCandidate,
) -> RendererInfo {
    let mut properties12 = vk::PhysicalDeviceVulkan12Properties::default();
    {
        let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut properties12);
        // SAFETY: the device supports Vulkan 1.3 (selected), so the 1.2
        // properties structure is valid.
        unsafe {
            instance
                .instance
                .get_physical_device_properties2(physical, &mut properties);
        }
    }
    let text = |result: Result<&CStr, _>| {
        result.map_or_else(
            |_| String::new(),
            |text: &CStr| text.to_string_lossy().into_owned(),
        )
    };
    let driver = format!(
        "{} {}",
        text(properties12.driver_name_as_c_str()),
        text(properties12.driver_info_as_c_str())
    );
    RendererInfo {
        gpu_name: candidate.name.clone(),
        gpu_kind: candidate.kind,
        api_version: decode_version(candidate.api_version),
        driver: driver.trim().to_owned(),
        validation_enabled: instance.validation_enabled(),
    }
}
