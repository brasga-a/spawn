//! Swapchain, its image views and per-image present semaphores.

use ash::vk;

use crate::error::RenderError;
use crate::selection::{
    FormatKind, FormatOption, PresentModeKind, choose_extent, choose_format, choose_image_count,
    choose_present_mode,
};
use crate::types::{PresentModePreference, SurfaceSize};
use crate::vulkan::device::DeviceOwner;
use crate::vulkan::surface::SurfaceOwner;
use crate::vulkan::{VkFailure, vk_error};

/// A swapchain and everything created per swapchain image.
///
/// The present semaphores are per image, not per frame in flight: an image's
/// semaphore is reused only after that image is acquired again, which
/// guarantees its previous presentation has consumed the semaphore.
pub(crate) struct Swapchain {
    pub handle: vk::SwapchainKHR,
    pub extent: SurfaceSize,
    pub format: vk::Format,
    /// The clear color must be sRGB-encoded on the CPU (UNORM format).
    pub encode_srgb: bool,
    pub images: Vec<vk::Image>,
    pub views: Vec<vk::ImageView>,
    pub present_semaphores: Vec<vk::Semaphore>,
}

/// Result of creating a swapchain.
pub(crate) enum SwapchainBuild {
    Created(Swapchain),
    /// The surface reports a zero extent: nothing to create.
    ZeroExtent,
}

impl Swapchain {
    /// Creates a swapchain for `requested`, retiring `old` (which the caller
    /// destroys afterwards whether or not this succeeds, as the spec requires).
    pub(crate) fn create(
        device: &DeviceOwner,
        surface: &SurfaceOwner,
        requested: SurfaceSize,
        preference: PresentModePreference,
        old: vk::SwapchainKHR,
    ) -> Result<SwapchainBuild, VkFailure> {
        let loader = &surface.loader;
        // SAFETY: the physical device and surface are valid and belong to the
        // same instance (as for every query below).
        let caps = unsafe {
            loader.get_physical_device_surface_capabilities(device.physical, surface.handle)
        }
        .map_err(|result| {
            VkFailure::from_call("vkGetPhysicalDeviceSurfaceCapabilitiesKHR", result)
        })?;
        let current = (caps.current_extent.width != u32::MAX)
            .then(|| SurfaceSize::new(caps.current_extent.width, caps.current_extent.height));
        let extent = choose_extent(
            current,
            SurfaceSize::new(caps.min_image_extent.width, caps.min_image_extent.height),
            SurfaceSize::new(caps.max_image_extent.width, caps.max_image_extent.height),
            requested,
        );
        if extent.is_empty() {
            return Ok(SwapchainBuild::ZeroExtent);
        }

        // SAFETY: see above.
        let formats =
            unsafe { loader.get_physical_device_surface_formats(device.physical, surface.handle) }
                .map_err(|result| {
                    VkFailure::from_call("vkGetPhysicalDeviceSurfaceFormatsKHR", result)
                })?;
        let options: Vec<_> = formats
            .iter()
            .map(|format| FormatOption {
                kind: format_kind(format.format),
                srgb_nonlinear: format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR,
            })
            .collect();
        let format_choice = choose_format(&options).ok_or_else(|| {
            VkFailure::from_call(
                "vkGetPhysicalDeviceSurfaceFormatsKHR",
                vk::Result::ERROR_FORMAT_NOT_SUPPORTED,
            )
        })?;
        let surface_format = formats.get(format_choice.index).copied().ok_or_else(|| {
            VkFailure::from_call(
                "surface format selection",
                vk::Result::ERROR_FORMAT_NOT_SUPPORTED,
            )
        })?;

        // SAFETY: see above.
        let modes = unsafe {
            loader.get_physical_device_surface_present_modes(device.physical, surface.handle)
        }
        .map_err(|result| {
            VkFailure::from_call("vkGetPhysicalDeviceSurfacePresentModesKHR", result)
        })?;
        let kinds: Vec<_> = modes.iter().map(|&mode| present_mode_kind(mode)).collect();
        let present_mode = match choose_present_mode(preference, &kinds) {
            PresentModeKind::Mailbox => vk::PresentModeKHR::MAILBOX,
            PresentModeKind::Immediate => vk::PresentModeKHR::IMMEDIATE,
            PresentModeKind::Fifo | PresentModeKind::Other => vk::PresentModeKHR::FIFO,
        };

        let composite_alpha = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::INHERIT,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        ]
        .into_iter()
        .find(|&mode| caps.supported_composite_alpha.contains(mode))
        .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);

        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface.handle)
            .min_image_count(choose_image_count(
                caps.min_image_count,
                caps.max_image_count,
            ))
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(vk::Extent2D {
                width: extent.width,
                height: extent.height,
            })
            .image_array_layers(1)
            // Color attachment usage is guaranteed for every swapchain.
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(composite_alpha)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old);
        // SAFETY: the surface is supported by the device's queue family
        // (checked at selection); `old`, if not null, is a swapchain of this
        // surface that the caller destroys after this call.
        let handle = unsafe { device.swapchain.create_swapchain(&info, None) }
            .map_err(|result| VkFailure::from_call("vkCreateSwapchainKHR", result))?;
        device.name(handle, c"spawn.swapchain");

        let mut swapchain = Self {
            handle,
            extent,
            format: surface_format.format,
            encode_srgb: format_choice.encode_srgb,
            images: Vec::new(),
            views: Vec::new(),
            present_semaphores: Vec::new(),
        };
        // On error the partially built swapchain is destroyed here.
        match swapchain.create_image_resources(device) {
            Ok(()) => Ok(SwapchainBuild::Created(swapchain)),
            Err(error) => {
                swapchain.destroy(device);
                Err(VkFailure::Fatal(error))
            }
        }
    }

    fn create_image_resources(&mut self, device: &DeviceOwner) -> Result<(), RenderError> {
        // SAFETY: the swapchain was just created on this device.
        self.images = unsafe { device.swapchain.get_swapchain_images(self.handle) }
            .map_err(|result| vk_error("vkGetSwapchainImagesKHR", result))?;
        for &image in &self.images {
            let info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(self.format)
                .subresource_range(color_range());
            // SAFETY: `image` belongs to this swapchain and device.
            let view = unsafe { device.device.create_image_view(&info, None) }
                .map_err(|result| vk_error("vkCreateImageView", result))?;
            self.views.push(view);
            // SAFETY: plain binary semaphore on a valid device.
            let semaphore = unsafe {
                device
                    .device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
            }
            .map_err(|result| vk_error("vkCreateSemaphore", result))?;
            device.name(semaphore, c"spawn.swapchain.present");
            self.present_semaphores.push(semaphore);
        }
        Ok(())
    }

    /// Destroys the swapchain and its per-image objects. The caller ensures
    /// the GPU no longer uses them (device idle).
    pub(crate) fn destroy(self, device: &DeviceOwner) {
        // SAFETY: the caller waited for the device to be idle, so no command
        // buffer or queue operation references these objects; each is
        // destroyed exactly once because `self` is consumed.
        unsafe {
            for semaphore in self.present_semaphores {
                device.device.destroy_semaphore(semaphore, None);
            }
            for view in self.views {
                device.device.destroy_image_view(view, None);
            }
            device.swapchain.destroy_swapchain(self.handle, None);
        }
    }
}

/// The single color mip level and array layer of a 2D color attachment.
pub(crate) fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}

/// The single depth mip level and array layer of a `D32_SFLOAT` depth attachment.
pub(crate) fn depth_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::DEPTH)
        .level_count(1)
        .layer_count(1)
}

/// Queries the surface color format that [`Swapchain::create`] chooses, even
/// when the window is currently suspended at `0x0` extent.
pub(crate) fn query_chosen_surface_format(
    device: &DeviceOwner,
    surface: &SurfaceOwner,
) -> Result<vk::Format, RenderError> {
    // SAFETY: `device.physical` and `surface.handle` belong to the same valid instance.
    let formats = unsafe {
        surface
            .loader
            .get_physical_device_surface_formats(device.physical, surface.handle)
    }
    .map_err(|result| vk_error("vkGetPhysicalDeviceSurfaceFormatsKHR", result))?;
    let options: Vec<_> = formats
        .iter()
        .map(|format| FormatOption {
            kind: format_kind(format.format),
            srgb_nonlinear: format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR,
        })
        .collect();
    let choice = choose_format(&options).ok_or_else(|| {
        vk_error(
            "vkGetPhysicalDeviceSurfaceFormatsKHR",
            vk::Result::ERROR_FORMAT_NOT_SUPPORTED,
        )
    })?;
    formats.get(choice.index).map(|f| f.format).ok_or_else(|| {
        vk_error(
            "surface format selection",
            vk::Result::ERROR_FORMAT_NOT_SUPPORTED,
        )
    })
}

fn format_kind(format: vk::Format) -> FormatKind {
    match format {
        vk::Format::B8G8R8A8_SRGB => FormatKind::Bgra8Srgb,
        vk::Format::R8G8B8A8_SRGB => FormatKind::Rgba8Srgb,
        vk::Format::B8G8R8A8_UNORM => FormatKind::Bgra8Unorm,
        vk::Format::R8G8B8A8_UNORM => FormatKind::Rgba8Unorm,
        _ => FormatKind::Other,
    }
}

fn present_mode_kind(mode: vk::PresentModeKHR) -> PresentModeKind {
    match mode {
        vk::PresentModeKHR::FIFO => PresentModeKind::Fifo,
        vk::PresentModeKHR::MAILBOX => PresentModeKind::Mailbox,
        vk::PresentModeKHR::IMMEDIATE => PresentModeKind::Immediate,
        _ => PresentModeKind::Other,
    }
}
