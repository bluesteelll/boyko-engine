//! Slice-0 step 0a — the validation-layer oracle.
//!
//! Wires `VK_LAYER_KHRONOS_validation` + a `VK_EXT_debug_utils` messenger whose
//! callback **counts** every validation message of severity WARNING or ERROR.
//! Because raw FFI cannot be checked by Miri (VRAM mapping, GPU↔CPU buffers,
//! driver-internal state), this counter — asserted to be zero after every GPU
//! test — is the soundness oracle that substitutes for Miri (plan §6).
//!
//! # The callback / user-data contract
//!
//! A Vulkan debug callback is a plain `extern "system" fn` and therefore cannot
//! capture state; it receives an opaque `p_user_data` pointer. We heap-allocate
//! a [`DebugMessengerState`] (atomic counters) and hand the messenger a stable
//! raw pointer to it. The [`crate::device::VulkanContext`] owns the `Box`, so
//! the state outlives the messenger (the messenger is destroyed in `Drop`
//! *before* the `Box` is dropped). The counters are atomic because the loader
//! may invoke the callback from any thread that triggers a validation message.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::ffi::*;

/// Shared state the debug callback writes into. Heap-pinned and pointed-to by
/// the messenger's `p_user_data`; owned by the [`crate::device::VulkanContext`].
///
/// The counters are `AtomicU32` because the validation layer may call the
/// callback from a worker thread (the load/store ordering only needs to make a
/// later same-thread `count()` observe its own callback writes, but `Relaxed`
/// is insufficient for the *cross-thread* read in a test, so `AcqRel`/`Acquire`
/// pair the increment with the test's read — see [`Self::total`]).
#[derive(Default)]
pub struct DebugMessengerState {
    warnings: AtomicU32,
    errors: AtomicU32,
}

impl DebugMessengerState {
    /// A fresh state with zero recorded messages.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of WARNING-severity validation messages recorded so far.
    #[inline]
    pub fn warning_count(&self) -> u32 {
        // Acquire: matches the Release `fetch_add` in `debug_callback` so a
        // reader thread observes every increment a callback thread published.
        self.warnings.load(Ordering::Acquire)
    }

    /// Number of ERROR-severity validation messages recorded so far.
    #[inline]
    pub fn error_count(&self) -> u32 {
        // Acquire: pairs with the Release `fetch_add` in `debug_callback`.
        self.errors.load(Ordering::Acquire)
    }

    /// Total WARNING + ERROR messages — the test's clean-run assertion target.
    #[inline]
    pub fn total(&self) -> u32 {
        self.warning_count() + self.error_count()
    }
}

/// The `VK_EXT_debug_utils` callback. Invoked by the loader for each message
/// whose severity/type intersects the messenger's configured masks; it records
/// WARNING/ERROR counts into the `p_user_data` [`DebugMessengerState`] and
/// always returns `VK_FALSE` (the spec mandates `VK_FALSE` from application
/// callbacks; `VK_TRUE` is reserved for layer development and aborts the call).
///
/// # Safety
///
/// This is an FFI callback. The loader guarantees `p_callback_data` (when
/// non-null) points to a valid `VkDebugUtilsMessengerCallbackDataEXT` for the
/// duration of the call, and `p_user_data` is exactly the pointer supplied at
/// messenger creation (a live `*const DebugMessengerState` owned by the
/// context, which outlives the messenger).
pub(crate) unsafe extern "system" fn debug_callback(
    message_severity: VkFlags,
    _message_types: VkFlags,
    p_callback_data: *const VkDebugUtilsMessengerCallbackDataExt,
    p_user_data: *mut c_void,
) -> VkBool32 {
    if p_user_data.is_null() {
        return VK_FALSE;
    }
    // SAFETY: `p_user_data` is the `*const DebugMessengerState` we passed to
    // `vkCreateDebugUtilsMessengerEXT`; the context owns the `Box<...>` and
    // destroys the messenger before dropping it, so the pointee is live for
    // every callback invocation. We only ever take `&` (atomic loads/stores) —
    // never `&mut` — so concurrent callbacks do not alias mutably.
    let state = unsafe { &*(p_user_data as *const DebugMessengerState) };

    if (message_severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT) != 0 {
        // Release: publishes the increment to the test thread's Acquire load.
        state.errors.fetch_add(1, Ordering::Release);
    } else if (message_severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT) != 0 {
        state.warnings.fetch_add(1, Ordering::Release);
    }

    // Best-effort: surface the message text to the test log so a non-zero
    // count is diagnosable. The pointer is valid per the loader contract.
    if !p_callback_data.is_null() {
        // SAFETY: the loader guarantees `p_callback_data` points to a valid
        // callback-data struct for this call; `p_message` is a NUL-terminated
        // C string (it may be null only if the message is empty, which we
        // guard).
        let data = unsafe { &*p_callback_data };
        if !data.p_message.is_null() {
            // SAFETY: `p_message` is a NUL-terminated UTF-8 C string per the
            // debug-utils spec; `CStr::from_ptr` reads up to the NUL.
            let msg = unsafe { core::ffi::CStr::from_ptr(data.p_message) };
            eprintln!("[vk-validation] {}", msg.to_string_lossy());
        }
    }

    // Application callbacks MUST return VK_FALSE.
    VK_FALSE
}

/// The configured severity/type masks for the messenger (WARNING + ERROR,
/// across all message types). Verbose/info are excluded so the callback only
/// counts the messages the oracle cares about.
pub(crate) const MESSENGER_SEVERITY: VkFlags =
    VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;

pub(crate) const MESSENGER_TYPE: VkFlags = VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT
    | VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT
    | VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
