//! GPU selection and swapchain configuration choices (pure, no Vulkan types).
//!
//! The Vulkan backend queries each GPU and surface, converts the answers into
//! the plain data below and applies the decisions. Requirements follow
//! ADR-0004: Vulkan 1.3 with dynamic rendering, synchronization2, timeline
//! semaphores and descriptor indexing (bindless), plus `VK_KHR_swapchain` and
//! one queue family that supports both graphics and presentation.

use crate::types::{GpuKind, PresentModePreference, SurfaceSize};

/// Required Vulkan version (major, minor).
pub(crate) const REQUIRED_API_VERSION: (u32, u32) = (1, 3);

/// Decodes a packed Vulkan version (`VK_MAKE_API_VERSION`).
pub(crate) const fn decode_version(packed: u32) -> (u32, u32, u32) {
    (
        (packed >> 22) & 0x7f,
        (packed >> 12) & 0x3ff,
        packed & 0xfff,
    )
}

/// Device features required by ADR-0004, as reported by the driver.
///
/// The descriptor-indexing set is the subset a bindless sampled-image and
/// storage-buffer table needs; it is enabled at device creation even though
/// the clear frame does not use it yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // One flag per Vulkan feature bit.
pub(crate) struct RequiredFeatures {
    pub dynamic_rendering: bool,
    pub synchronization2: bool,
    pub timeline_semaphore: bool,
    pub descriptor_indexing: bool,
    pub runtime_descriptor_array: bool,
    pub descriptor_binding_partially_bound: bool,
    pub descriptor_binding_variable_descriptor_count: bool,
    pub shader_sampled_image_array_non_uniform_indexing: bool,
    pub descriptor_binding_sampled_image_update_after_bind: bool,
    pub descriptor_binding_storage_buffer_update_after_bind: bool,
}

impl RequiredFeatures {
    /// Every required feature enabled (what the device is created with).
    pub(crate) const ALL: Self = Self {
        dynamic_rendering: true,
        synchronization2: true,
        timeline_semaphore: true,
        descriptor_indexing: true,
        runtime_descriptor_array: true,
        descriptor_binding_partially_bound: true,
        descriptor_binding_variable_descriptor_count: true,
        shader_sampled_image_array_non_uniform_indexing: true,
        descriptor_binding_sampled_image_update_after_bind: true,
        descriptor_binding_storage_buffer_update_after_bind: true,
    };

    /// Vulkan names of the features that are missing.
    pub(crate) fn missing(&self) -> Vec<&'static str> {
        [
            (self.dynamic_rendering, "dynamicRendering"),
            (self.synchronization2, "synchronization2"),
            (self.timeline_semaphore, "timelineSemaphore"),
            (self.descriptor_indexing, "descriptorIndexing"),
            (self.runtime_descriptor_array, "runtimeDescriptorArray"),
            (
                self.descriptor_binding_partially_bound,
                "descriptorBindingPartiallyBound",
            ),
            (
                self.descriptor_binding_variable_descriptor_count,
                "descriptorBindingVariableDescriptorCount",
            ),
            (
                self.shader_sampled_image_array_non_uniform_indexing,
                "shaderSampledImageArrayNonUniformIndexing",
            ),
            (
                self.descriptor_binding_sampled_image_update_after_bind,
                "descriptorBindingSampledImageUpdateAfterBind",
            ),
            (
                self.descriptor_binding_storage_buffer_update_after_bind,
                "descriptorBindingStorageBufferUpdateAfterBind",
            ),
        ]
        .into_iter()
        .filter_map(|(present, name)| (!present).then_some(name))
        .collect()
    }
}

/// One queue family of a GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QueueFamily {
    pub graphics: bool,
    pub present: bool,
}

/// What the backend learned about one GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceCandidate {
    pub name: String,
    pub kind: GpuKind,
    /// Packed Vulkan version supported by the device.
    pub api_version: u32,
    pub has_swapchain_extension: bool,
    /// `None` when the device is below Vulkan 1.3 (features not queried).
    pub features: Option<RequiredFeatures>,
    pub queue_families: Vec<QueueFamily>,
    pub surface_formats: usize,
    pub present_modes: usize,
}

/// The chosen GPU and queue family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeviceChoice {
    /// Index into the candidate list.
    pub candidate: usize,
    /// Queue family used for graphics and presentation.
    pub queue_family: u32,
}

/// The queue family to use, or one line explaining every reason the
/// candidate cannot be used.
fn evaluate(candidate: &DeviceCandidate) -> Result<u32, String> {
    let (major, minor, patch) = decode_version(candidate.api_version);
    let mut problems = Vec::new();
    if (major, minor) < REQUIRED_API_VERSION {
        problems.push(format!(
            "Vulkan {major}.{minor}.{patch} < {}.{}",
            REQUIRED_API_VERSION.0, REQUIRED_API_VERSION.1
        ));
    }
    if !candidate.has_swapchain_extension {
        problems.push("missing extension `VK_KHR_swapchain`".to_owned());
    }
    match &candidate.features {
        Some(features) => {
            let missing = features.missing();
            if !missing.is_empty() {
                problems.push(format!("missing features {}", missing.join(", ")));
            }
        }
        None if problems.is_empty() => {
            problems.push("device features could not be queried".to_owned())
        }
        None => {}
    }
    let family = candidate
        .queue_families
        .iter()
        .position(|family| family.graphics && family.present)
        .and_then(|index| u32::try_from(index).ok());
    if family.is_none() {
        problems.push(
            "no queue family supports both graphics and presentation to this surface".to_owned(),
        );
    }
    if candidate.surface_formats == 0 {
        problems.push("no surface formats".to_owned());
    }
    if candidate.present_modes == 0 {
        problems.push("no present modes".to_owned());
    }
    match family {
        Some(family) if problems.is_empty() => Ok(family),
        _ => Err(format!("{}: {}", candidate.name, problems.join("; "))),
    }
}

fn kind_rank(kind: GpuKind) -> u8 {
    match kind {
        GpuKind::Discrete => 4,
        GpuKind::Integrated => 3,
        GpuKind::Virtual => 2,
        GpuKind::Cpu => 1,
        GpuKind::Other => 0,
    }
}

/// Picks the best usable GPU: discrete before integrated before virtual before
/// software; among equals, the first enumerated. No vendor is preferred.
/// On failure returns one line per GPU explaining the rejection.
pub(crate) fn select_device(candidates: &[DeviceCandidate]) -> Result<DeviceChoice, Vec<String>> {
    let mut rejections = Vec::new();
    let mut best: Option<(u8, DeviceChoice)> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        match evaluate(candidate) {
            Ok(queue_family) => {
                let rank = kind_rank(candidate.kind);
                if best.is_none_or(|(best_rank, _)| rank > best_rank) {
                    best = Some((
                        rank,
                        DeviceChoice {
                            candidate: index,
                            queue_family,
                        },
                    ));
                }
            }
            Err(reason) => rejections.push(reason),
        }
    }
    match best {
        Some((_, choice)) => Ok(choice),
        None if candidates.is_empty() => Err(vec!["no Vulkan GPU was enumerated".to_owned()]),
        None => Err(rejections),
    }
}

/// Color formats the renderer knows how to clear correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatKind {
    Bgra8Srgb,
    Rgba8Srgb,
    Bgra8Unorm,
    Rgba8Unorm,
    Other,
}

/// One surface format offered by the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FormatOption {
    pub kind: FormatKind,
    /// Color space is `SRGB_NONLINEAR` (the standard desktop color space).
    pub srgb_nonlinear: bool,
}

/// The chosen surface format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FormatChoice {
    /// Index into the offered formats.
    pub index: usize,
    /// The clear color must be sRGB-encoded on the CPU (UNORM format).
    pub encode_srgb: bool,
}

/// Prefers 8-bit sRGB formats in the sRGB color space (the GPU encodes linear
/// colors), then 8-bit UNORM in that color space (the CPU encodes), then the
/// first offered format as is.
pub(crate) fn choose_format(options: &[FormatOption]) -> Option<FormatChoice> {
    let find = |kinds: &[FormatKind]| {
        options
            .iter()
            .position(|option| option.srgb_nonlinear && kinds.contains(&option.kind))
    };
    if let Some(index) = find(&[FormatKind::Bgra8Srgb, FormatKind::Rgba8Srgb]) {
        return Some(FormatChoice {
            index,
            encode_srgb: false,
        });
    }
    if let Some(index) = find(&[FormatKind::Bgra8Unorm, FormatKind::Rgba8Unorm]) {
        return Some(FormatChoice {
            index,
            encode_srgb: true,
        });
    }
    (!options.is_empty()).then_some(FormatChoice {
        index: 0,
        encode_srgb: false,
    })
}

/// Presentation modes the renderer distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentModeKind {
    Fifo,
    Mailbox,
    Immediate,
    Other,
}

/// The preferred mode when supported, otherwise FIFO (always supported).
pub(crate) fn choose_present_mode(
    preference: PresentModePreference,
    supported: &[PresentModeKind],
) -> PresentModeKind {
    let wanted = match preference {
        PresentModePreference::Fifo => PresentModeKind::Fifo,
        PresentModePreference::Mailbox => PresentModeKind::Mailbox,
        PresentModePreference::Immediate => PresentModeKind::Immediate,
    };
    if supported.contains(&wanted) {
        wanted
    } else {
        PresentModeKind::Fifo
    }
}

/// One more image than the minimum (so acquire rarely blocks), within the
/// maximum (`0` means no maximum).
pub(crate) fn choose_image_count(min: u32, max: u32) -> u32 {
    let wanted = min.saturating_add(1);
    if max == 0 { wanted } else { wanted.min(max) }
}

/// Swapchain extent: the surface's current extent when it dictates one
/// (`current`), otherwise the requested size clamped to the surface limits.
pub(crate) fn choose_extent(
    current: Option<SurfaceSize>,
    min: SurfaceSize,
    max: SurfaceSize,
    requested: SurfaceSize,
) -> SurfaceSize {
    if let Some(current) = current {
        return current;
    }
    let clamp = |value: u32, low: u32, high: u32| value.max(low).min(high.max(low));
    SurfaceSize::new(
        clamp(requested.width, min.width, max.width),
        clamp(requested.height, min.height, max.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn version(major: u32, minor: u32) -> u32 {
        (major << 22) | (minor << 12)
    }

    fn good(name: &str, kind: GpuKind) -> DeviceCandidate {
        DeviceCandidate {
            name: name.to_owned(),
            kind,
            api_version: version(1, 4),
            has_swapchain_extension: true,
            features: Some(RequiredFeatures::ALL),
            queue_families: vec![
                QueueFamily {
                    graphics: false,
                    present: true,
                },
                QueueFamily {
                    graphics: true,
                    present: true,
                },
            ],
            surface_formats: 2,
            present_modes: 1,
        }
    }

    #[test]
    fn decodes_packed_versions() {
        assert_eq!(decode_version(version(1, 3) | 281), (1, 3, 281));
    }

    #[test]
    fn selects_the_graphics_and_present_family() {
        let choice = select_device(&[good("gpu", GpuKind::Discrete)]);
        assert_eq!(
            choice,
            Ok(DeviceChoice {
                candidate: 0,
                queue_family: 1
            })
        );
    }

    #[test]
    fn prefers_discrete_then_first_enumerated_without_vendor_bias() {
        let candidates = [
            good("software", GpuKind::Cpu),
            good("integrated", GpuKind::Integrated),
            good("discrete a", GpuKind::Discrete),
            good("discrete b", GpuKind::Discrete),
        ];
        assert_eq!(
            select_device(&candidates).map(|choice| choice.candidate),
            Ok(2)
        );
    }

    #[test]
    fn falls_back_to_a_lesser_but_usable_gpu() {
        let mut broken = good("old discrete", GpuKind::Discrete);
        broken.api_version = version(1, 2);
        broken.features = None;
        let candidates = [broken, good("integrated", GpuKind::Integrated)];
        assert_eq!(
            select_device(&candidates).map(|choice| choice.candidate),
            Ok(1)
        );
    }

    #[test]
    fn rejections_name_every_gpu_and_every_problem() {
        let mut old = good("old", GpuKind::Discrete);
        old.api_version = version(1, 2);
        old.features = None;
        let mut no_sync2 = good("no sync2", GpuKind::Integrated);
        no_sync2.features = Some(RequiredFeatures {
            synchronization2: false,
            runtime_descriptor_array: false,
            ..RequiredFeatures::ALL
        });
        no_sync2.has_swapchain_extension = false;
        let mut no_present = good("headless only", GpuKind::Virtual);
        no_present.queue_families = vec![QueueFamily {
            graphics: true,
            present: false,
        }];
        no_present.surface_formats = 0;
        no_present.present_modes = 0;

        let rejections = select_device(&[old, no_sync2, no_present])
            .err()
            .unwrap_or_default();
        assert_eq!(
            rejections,
            [
                "old: Vulkan 1.2.0 < 1.3",
                "no sync2: missing extension `VK_KHR_swapchain`; missing features synchronization2, runtimeDescriptorArray",
                "headless only: no queue family supports both graphics and presentation to this surface; no surface formats; no present modes",
            ]
        );
    }

    #[test]
    fn no_devices_is_reported() {
        assert_eq!(
            select_device(&[]),
            Err(vec!["no Vulkan GPU was enumerated".to_owned()])
        );
    }

    #[test]
    fn every_required_feature_is_named_when_missing() {
        assert_eq!(RequiredFeatures::default().missing().len(), 10);
        assert!(RequiredFeatures::ALL.missing().is_empty());
    }

    #[test]
    fn format_prefers_srgb_then_unorm_then_first() {
        let option = |kind, srgb_nonlinear| FormatOption {
            kind,
            srgb_nonlinear,
        };
        let offered = [
            option(FormatKind::Other, true),
            option(FormatKind::Bgra8Unorm, true),
            option(FormatKind::Bgra8Srgb, false),
            option(FormatKind::Rgba8Srgb, true),
        ];
        assert_eq!(
            choose_format(&offered),
            Some(FormatChoice {
                index: 3,
                encode_srgb: false
            })
        );
        assert_eq!(
            choose_format(&offered[..3]),
            Some(FormatChoice {
                index: 1,
                encode_srgb: true
            })
        );
        assert_eq!(
            choose_format(&[option(FormatKind::Other, false)]),
            Some(FormatChoice {
                index: 0,
                encode_srgb: false
            })
        );
        assert_eq!(choose_format(&[]), None);
    }

    #[test]
    fn present_mode_falls_back_to_fifo() {
        use PresentModeKind as M;
        let supported = [M::Fifo, M::Mailbox];
        assert_eq!(
            choose_present_mode(PresentModePreference::Mailbox, &supported),
            M::Mailbox
        );
        assert_eq!(
            choose_present_mode(PresentModePreference::Immediate, &supported),
            M::Fifo
        );
        assert_eq!(
            choose_present_mode(PresentModePreference::Fifo, &[M::Fifo, M::Immediate]),
            M::Fifo
        );
    }

    #[test]
    fn image_count_is_min_plus_one_within_max() {
        assert_eq!(choose_image_count(2, 0), 3);
        assert_eq!(choose_image_count(3, 8), 4);
        assert_eq!(choose_image_count(3, 3), 3);
        assert_eq!(choose_image_count(u32::MAX, 0), u32::MAX);
    }

    #[test]
    fn extent_follows_surface_or_clamps_request() {
        let size = SurfaceSize::new;
        let (min, max) = (size(1, 1), size(4096, 4096));
        assert_eq!(
            choose_extent(Some(size(800, 600)), min, max, size(10, 10)),
            size(800, 600)
        );
        assert_eq!(
            choose_extent(Some(size(0, 0)), min, max, size(10, 10)),
            size(0, 0)
        );
        assert_eq!(
            choose_extent(None, min, max, size(1280, 720)),
            size(1280, 720)
        );
        assert_eq!(
            choose_extent(None, min, max, size(10_000, 0)),
            size(4096, 1)
        );
        // Inverted limits from a buggy driver never panic.
        assert_eq!(
            choose_extent(None, size(100, 100), size(50, 50), size(10, 10)),
            size(100, 100)
        );
    }
}
