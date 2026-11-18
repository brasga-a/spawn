//! Vulkan loader, instance and validation-layer messages.

use std::collections::VecDeque;
use std::ffi::{CStr, c_char, c_void};
use std::mem::ManuallyDrop;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use ash::{ext, vk};

use crate::error::{RenderError, RenderErrorKind};
use crate::selection::{REQUIRED_API_VERSION, decode_version};
use crate::types::{Validation, ValidationMessage, ValidationSeverity};
use crate::vulkan::vk_error;

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// Serializes Vulkan loader instance setup and teardown (`Entry::load`,
/// `vkEnumerateInstance*`, `vkCreateInstance`, `vkDestroyInstance`). When zero
/// `VkInstance`s are live, the Vulkan loader `dlopen`s/`dlclose`s installed
/// ICDs on each pre-instance query; some Linux ICDs (`libGLX_nvidia.so.0`)
/// race in `vk_icdNegotiateLoaderICDInterfaceVersion` if two threads probe the
/// loader simultaneously (for example in parallel GPU tests). Cold startup and
/// shutdown path only; never acquired per frame.
static LOADER_LOCK: Mutex<()> = Mutex::new(());

/// Messages kept between drains; older ones are counted as dropped.
const VALIDATION_CAPACITY: usize = 256;

/// Receives validation-layer messages. The layer may call back from any
/// thread, hence the lock; it is used only with validation enabled
/// (development), never on a release hot path.
#[derive(Debug, Default)]
pub(crate) struct ValidationSink {
    messages: Mutex<VecDeque<ValidationMessage>>,
    dropped: AtomicU64,
    errors: AtomicU64,
}

impl ValidationSink {
    fn push(&self, message: ValidationMessage) {
        if message.severity == ValidationSeverity::Error {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        // A poisoned lock only means another callback panicked mid-push; the
        // queue is still a valid VecDeque.
        let mut messages = self
            .messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if messages.len() == VALIDATION_CAPACITY {
            messages.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        messages.push_back(message);
    }

    /// Moves queued messages to `out`; returns how many were dropped since
    /// the last drain because the queue was full.
    pub(crate) fn drain_into(&self, out: &mut Vec<ValidationMessage>) -> u64 {
        let mut messages = self
            .messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        out.extend(messages.drain(..));
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// Error-severity messages received since creation.
    pub(crate) fn error_count(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }
}

/// Validation-layer callback. Never unwinds into the driver.
unsafe extern "system" fn on_validation_message(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _types: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    user_data: *mut c_void,
) -> vk::Bool32 {
    let severity = if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        ValidationSeverity::Error
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        ValidationSeverity::Warning
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        ValidationSeverity::Info
    } else {
        ValidationSeverity::Verbose
    };
    // SAFETY: the layer passes a valid callback-data pointer (or null) for
    // the duration of the call; its message is a NUL-terminated string.
    let text = unsafe { data.as_ref().and_then(|data| data.message_as_c_str()) }.map_or_else(
        || "<no message>".to_owned(),
        |text| text.to_string_lossy().into_owned(),
    );
    // SAFETY: `user_data` is the `ValidationSink` boxed in `InstanceOwner`,
    // which destroys the messenger before freeing the box.
    if let Some(sink) = unsafe { user_data.cast::<ValidationSink>().as_ref() } {
        sink.push(ValidationMessage { severity, text });
    }
    vk::FALSE
}

/// Where the surface comes from; decides the instance surface extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceExtension {
    Wayland,
    /// `VK_EXT_headless_surface`, for headless rendering and GPU tests without a window.
    Headless,
}

impl SurfaceExtension {
    fn name(self) -> &'static CStr {
        match self {
            Self::Wayland => ash::khr::wayland_surface::NAME,
            Self::Headless => ext::headless_surface::NAME,
        }
    }
}

struct Messenger {
    loader: ext::debug_utils::Instance,
    handle: vk::DebugUtilsMessengerEXT,
}

/// Owns the loader, the instance and the validation messenger.
/// Dropped last: everything else is created from it.
pub(crate) struct InstanceOwner {
    pub entry: ManuallyDrop<ash::Entry>,
    pub instance: ash::Instance,
    messenger: Option<Messenger>,
    /// Debug-utils is enabled (object names, validation messages).
    pub debug_utils: bool,
    /// `VK_EXT_validation_features` synchronization validation was requested.
    sync_validation: bool,
    sink: Box<ValidationSink>,
}

impl InstanceOwner {
    pub(crate) fn new(
        surface: SurfaceExtension,
        validation: Validation,
    ) -> Result<Self, RenderError> {
        let _guard = LOADER_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: loading the system Vulkan loader runs its initialization;
        // serialized by `LOADER_LOCK` so concurrent callers cannot race inside
        // loader/ICD global initialization.
        let entry = unsafe { ash::Entry::load() }.map_err(|error| {
            RenderError::new(RenderErrorKind::VulkanUnavailable)
                .with_detail("loading the Vulkan loader (libvulkan.so.1)")
                .with_cause(&error)
        })?;

        // SAFETY: `entry` holds valid global function pointers.
        let version = unsafe { entry.try_enumerate_instance_version() }
            .map_err(|result| vk_error("vkEnumerateInstanceVersion", result))?
            .unwrap_or(vk::API_VERSION_1_0);
        let (major, minor, patch) = decode_version(version);
        if (major, minor) < REQUIRED_API_VERSION {
            return Err(
                RenderError::new(RenderErrorKind::VulkanUnavailable).with_detail(format!(
                    "the Vulkan loader supports {major}.{minor}.{patch}; 1.3 is required"
                )),
            );
        }

        // SAFETY: as above.
        let available = unsafe { entry.enumerate_instance_extension_properties(None) }
            .map_err(|result| vk_error("vkEnumerateInstanceExtensionProperties", result))?;
        let has_extension = |name: &CStr| {
            available
                .iter()
                .any(|properties| properties.extension_name_as_c_str() == Ok(name))
        };
        // Surface support comes from the drivers (ICDs): without
        // `VK_KHR_surface` no driver is installed or loadable at all.
        if !has_extension(ash::khr::surface::NAME) {
            return Err(RenderError::new(RenderErrorKind::VulkanUnavailable).with_detail(
                "no Vulkan driver provides `VK_KHR_surface`; is a Vulkan driver (ICD) installed, \
                 and are VK_DRIVER_FILES / VK_ICD_FILENAMES unset or correct?",
            ));
        }
        if !has_extension(surface.name()) {
            return Err(
                RenderError::new(RenderErrorKind::UnsupportedWindowSystem).with_detail(format!(
                    "no Vulkan driver provides `{}`",
                    surface.name().to_string_lossy()
                )),
            );
        }

        let layer_available =
            validation != Validation::Disabled && validation_layer_available(&entry)?;
        if validation == Validation::Required && !layer_available {
            return Err(
                RenderError::new(RenderErrorKind::ValidationUnavailable).with_detail(
                    "install the Khronos validation layer (Arch: `vulkan-validation-layers`)",
                ),
            );
        }
        let debug_utils = layer_available && has_extension(ext::debug_utils::NAME);

        let sync_validation = layer_available && validation_features_available(&entry);
        // `Validation::Required` means errors are *observable* (debug-utils messenger)
        // and synchronization validation is active (ADR-0004, SPN-INV-092), not
        // merely that the layer is loaded.
        if validation == Validation::Required && !(debug_utils && sync_validation) {
            return Err(
                RenderError::new(RenderErrorKind::ValidationUnavailable).with_detail(format!(
                    "the Khronos validation layer is missing required support \
                     (VK_EXT_debug_utils: {debug_utils}, VK_EXT_validation_features: {sync_validation})"
                )),
            );
        }
        let mut extensions: Vec<*const c_char> =
            vec![ash::khr::surface::NAME.as_ptr(), surface.name().as_ptr()];
        if debug_utils {
            extensions.push(ext::debug_utils::NAME.as_ptr());
        }
        if sync_validation {
            extensions.push(ext::validation_features::NAME.as_ptr());
        }
        let layers: Vec<*const c_char> = if layer_available {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };
        let application = vk::ApplicationInfo::default()
            .application_name(c"Spawn")
            .engine_name(c"Spawn")
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);
        // Synchronization validation (ADR-0004) when the layer offers it.
        let enabled_features = [vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION];
        let mut validation_features =
            vk::ValidationFeaturesEXT::default().enabled_validation_features(&enabled_features);
        let mut info = vk::InstanceCreateInfo::default()
            .application_info(&application)
            .enabled_extension_names(&extensions)
            .enabled_layer_names(&layers);
        if sync_validation {
            info = info.push_next(&mut validation_features);
        }

        // SAFETY: every pointer in `info` refers to locals that outlive the
        // call; the extensions and layers were checked to exist.
        let instance = unsafe { entry.create_instance(&info, None) }.map_err(|result| {
            let kind = if result == vk::Result::ERROR_INCOMPATIBLE_DRIVER {
                RenderErrorKind::VulkanUnavailable
            } else {
                RenderErrorKind::VulkanCallFailed
            };
            RenderError::new(kind)
                .with_detail("vkCreateInstance")
                .with_cause(&result)
        })?;
        // Release before constructing `Self` so an error in `create_messenger`
        // can drop `Self` without re-locking `LOADER_LOCK` on this thread.
        drop(_guard);

        let mut owner = Self {
            entry: ManuallyDrop::new(entry),
            instance,
            messenger: None,
            debug_utils,
            sync_validation,
            sink: Box::default(),
        };
        if debug_utils {
            owner.create_messenger()?;
        }
        Ok(owner)
    }

    fn create_messenger(&mut self) -> Result<(), RenderError> {
        let loader = ext::debug_utils::Instance::new(&self.entry, &self.instance);
        let sink: *const ValidationSink = &*self.sink;
        let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(on_validation_message))
            .user_data(sink.cast_mut().cast());
        // SAFETY: the instance was created with debug-utils enabled; the sink
        // outlives the messenger (destroyed first in `Drop`).
        let handle = unsafe { loader.create_debug_utils_messenger(&info, None) }
            .map_err(|result| vk_error("vkCreateDebugUtilsMessengerEXT", result))?;
        self.messenger = Some(Messenger { loader, handle });
        Ok(())
    }

    /// Whether the validation layer is active.
    pub(crate) fn validation_enabled(&self) -> bool {
        self.messenger.is_some()
    }

    /// Whether synchronization validation is active alongside the validation layer.
    pub(crate) fn sync_validation_enabled(&self) -> bool {
        self.messenger.is_some() && self.sync_validation
    }

    pub(crate) fn sink(&self) -> &ValidationSink {
        &self.sink
    }
}

impl Drop for InstanceOwner {
    fn drop(&mut self) {
        let _guard = LOADER_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(messenger) = self.messenger.take() {
            // SAFETY: the messenger belongs to this instance and is destroyed
            // once, before the instance.
            unsafe {
                messenger
                    .loader
                    .destroy_debug_utils_messenger(messenger.handle, None)
            };
        }
        // SAFETY: every child object (surface, device) is owned by values that
        // are dropped before this one (field order of `VulkanRenderer`, or
        // reverse local order during construction).
        unsafe { self.instance.destroy_instance(None) };
        // SAFETY: the instance and messenger were destroyed above; `entry` is
        // dropped once here while `_guard` is held so `dlclose` cannot race
        // with concurrent `Entry::load` / pre-instance ICD queries.
        unsafe { ManuallyDrop::drop(&mut self.entry) };
    }
}

fn validation_layer_available(entry: &ash::Entry) -> Result<bool, RenderError> {
    // SAFETY: `entry` holds valid global function pointers.
    let layers = unsafe { entry.enumerate_instance_layer_properties() }
        .map_err(|result| vk_error("vkEnumerateInstanceLayerProperties", result))?;
    Ok(layers
        .iter()
        .any(|layer| layer.layer_name_as_c_str() == Ok(VALIDATION_LAYER)))
}

fn validation_features_available(entry: &ash::Entry) -> bool {
    // SAFETY: `entry` holds valid global function pointers; the layer name is
    // NUL-terminated.
    unsafe { entry.enumerate_instance_extension_properties(Some(VALIDATION_LAYER)) }
        .map(|extensions| {
            extensions.iter().any(|properties| {
                properties.extension_name_as_c_str() == Ok(ext::validation_features::NAME)
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(severity: ValidationSeverity, n: usize) -> ValidationMessage {
        ValidationMessage {
            severity,
            text: format!("message {n}"),
        }
    }

    #[test]
    fn sink_is_bounded_and_counts_errors_and_drops() {
        let sink = ValidationSink::default();
        for n in 0..VALIDATION_CAPACITY + 10 {
            let severity = if n % 2 == 0 {
                ValidationSeverity::Error
            } else {
                ValidationSeverity::Warning
            };
            sink.push(message(severity, n));
        }
        let mut out = Vec::new();
        assert_eq!(sink.drain_into(&mut out), 10);
        assert_eq!(out.len(), VALIDATION_CAPACITY);
        assert_eq!(out.first().map(|m| m.text.as_str()), Some("message 10"));
        assert_eq!(sink.error_count(), (VALIDATION_CAPACITY as u64 + 10) / 2);
        out.clear();
        assert_eq!(sink.drain_into(&mut out), 0);
        assert!(out.is_empty());
    }
}
