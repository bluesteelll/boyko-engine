//! Slice-0 step 0a — the validation-layer oracle.
//!
//! Wires `VK_LAYER_KHRONOS_validation` + a `VK_EXT_debug_utils` messenger whose
//! callback **counts** every validation message of severity WARNING or ERROR.
//! Because raw FFI cannot be checked by Miri (VRAM mapping, GPU↔CPU buffers,
//! driver-internal state), these counters are the soundness oracle that
//! substitutes for Miri (plan §6). They are read at two scopes:
//!
//! * **Per context** — [`DebugMessengerState`], asserted to be zero by the ~30
//!   RHI-level GPU tests in this crate. Those tests skip under
//!   `BOYKO_DISABLE_VALIDATION`, and a context's state dies with the context, so
//!   it cannot see a message delivered while the device or instance is destroyed.
//! * **Per process** — the [`validation_ledger`], which every callback increments
//!   before and independently of the per-context state, and which is never
//!   dropped. It is what the full-engine boot gate
//!   (`boyko_app/tests/boot_validation_clean.rs`) reads after `App::run` returns,
//!   so the teardown window (`vkDestroyDevice` leak reports, `vkDestroyInstance`)
//!   is inside its verdict.
//!
//! # The callback / user-data contract
//!
//! A Vulkan debug callback is a plain `extern "system" fn` and therefore cannot
//! capture state; it receives an opaque `p_user_data` pointer. We heap-allocate
//! a [`DebugMessengerState`] (atomic counters) and hand the persistent messenger a
//! stable raw pointer to it. The [`crate::device::VulkanContext`] owns the `Box`,
//! so the state outlives the messenger (the messenger is destroyed in `Drop`
//! *before* the `Box` is dropped). The create-time messenger chained into
//! `VkInstanceCreateInfo::pNext` (covering `vkCreateInstance` /
//! `vkDestroyInstance`) passes a NULL `p_user_data`: its messages reach the
//! process ledger and the log, never a per-context counter. The counters are
//! atomic because the loader may invoke the callback from any thread that
//! triggers a validation message.

use core::ffi::{CStr, c_char, c_void};
use core::sync::atomic::{AtomicU32, Ordering};
use std::borrow::Cow;

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

/// The process-wide validation ledger: seven independent monotonic counters.
///
/// FFI oracle instrumentation of the same class as [`DebugMessengerState`], not a
/// data store. It is a `static`, so it is never dropped: a message the layer
/// delivers inside `vkDestroyDevice` is still countable after the context that
/// received it is gone. Cold (touched only when a message fires or a messenger
/// is created/destroyed), so no alignment or padding is spent on it.
struct ValidationLedger {
    /// +1 after `vkCreateDebugUtilsMessengerEXT` succeeds — the ARMING witness.
    messengers_created: AtomicU32,
    /// +1 after `vkDestroyDebugUtilsMessengerEXT` returns.
    messengers_destroyed: AtomicU32,
    /// ERROR severity whose type is not GENERAL-only: a validation finding.
    errors: AtomicU32,
    /// ERROR severity, GENERAL type only: the loader's or a layer's own failure (a stale
    /// manifest, a library that did not load) — gated, but not a verdict on the engine's calls.
    general_errors: AtomicU32,
    /// WARNING severity that is not GENERAL-only (VALIDATION or PERFORMANCE).
    validation_warnings: AtomicU32,
    /// WARNING severity, GENERAL type only (loader / environment; reported, not gated).
    general_warnings: AtomicU32,
    /// Messages of any counted class delivered to the create-time instance messenger (NULL
    /// user data), i.e. inside `vkCreateInstance` / `vkDestroyInstance`. A breakdown of the
    /// four counters above by window, not a fifth class.
    instance_window: AtomicU32,
}

impl ValidationLedger {
    /// Increments the counter `class` selects, and [`Self::instance_window`] too when the
    /// message came from the create-time instance messenger. [`MessageClass::Ignored`] touches
    /// none.
    #[inline]
    fn record(&self, class: MessageClass, instance_window: bool) {
        let counter = match class {
            MessageClass::Error => &self.errors,
            MessageClass::GeneralError => &self.general_errors,
            MessageClass::ValidationWarning => &self.validation_warnings,
            MessageClass::GeneralWarning => &self.general_warnings,
            MessageClass::Ignored => return,
        };
        // Release: pairs with the Acquire loads in `validation_ledger`, so a reader
        // on another thread observes every increment a callback thread published.
        counter.fetch_add(1, Ordering::Release);
        if instance_window {
            self.instance_window.fetch_add(1, Ordering::Release);
        }
    }
}

static LEDGER: ValidationLedger = ValidationLedger {
    messengers_created: AtomicU32::new(0),
    messengers_destroyed: AtomicU32::new(0),
    errors: AtomicU32::new(0),
    general_errors: AtomicU32::new(0),
    validation_warnings: AtomicU32::new(0),
    general_warnings: AtomicU32::new(0),
    instance_window: AtomicU32::new(0),
};

/// A copy of the process-wide validation ledger at one instant (see [`validation_ledger`]).
///
/// Every field is a monotonic count since process start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidationLedgerSnapshot {
    /// Debug-utils messengers successfully created in this process. A value of `0` after a
    /// validated boot means no layer ever listened: the oracle was never armed.
    pub messengers_created: u32,
    /// Debug-utils messengers destroyed in this process. The persistent messenger is destroyed
    /// after `vkDestroyDevice`, so `destroyed == created` proves the device-destroy window was
    /// listened to.
    pub messengers_destroyed: u32,
    /// ERROR-severity messages whose type is not GENERAL alone: validation findings.
    pub errors: u32,
    /// ERROR-severity messages carrying only the GENERAL type: the loader's or a layer's own
    /// failure (an environment fault, not a finding about the engine's API use).
    pub general_errors: u32,
    /// WARNING-severity messages carrying the VALIDATION or PERFORMANCE type.
    pub validation_warnings: u32,
    /// WARNING-severity messages carrying only the GENERAL type (loader / environment).
    pub general_warnings: u32,
    /// How many of the messages counted above arrived inside `vkCreateInstance` /
    /// `vkDestroyInstance` (the create-time instance messenger).
    pub instance_window: u32,
}

/// Reads the process-wide validation ledger.
///
/// The seven fields are seven separate loads, not one atomic read. That is sound for a verdict
/// taken when no Vulkan object is alive (nothing can call back), and for a delta of two
/// snapshots on monotonic counters; it is not a consistent cut while messages are in flight.
#[inline]
pub fn validation_ledger() -> ValidationLedgerSnapshot {
    // Acquire (each load): pairs with the Release `fetch_add`s in `ValidationLedger::record`,
    // `note_messenger_created` and `note_messenger_destroyed`.
    ValidationLedgerSnapshot {
        messengers_created: LEDGER.messengers_created.load(Ordering::Acquire),
        messengers_destroyed: LEDGER.messengers_destroyed.load(Ordering::Acquire),
        errors: LEDGER.errors.load(Ordering::Acquire),
        general_errors: LEDGER.general_errors.load(Ordering::Acquire),
        validation_warnings: LEDGER.validation_warnings.load(Ordering::Acquire),
        general_warnings: LEDGER.general_warnings.load(Ordering::Acquire),
        instance_window: LEDGER.instance_window.load(Ordering::Acquire),
    }
}

/// Records one debug-utils messenger created. Called only after
/// `vkCreateDebugUtilsMessengerEXT` returned success.
#[inline]
pub(crate) fn note_messenger_created() {
    // Release: pairs with the Acquire loads in `validation_ledger` and `note_messenger_destroyed`.
    LEDGER.messengers_created.fetch_add(1, Ordering::Release);
}

/// Records one debug-utils messenger destroyed. Called right after
/// `vkDestroyDebugUtilsMessengerEXT` returned.
#[inline]
pub(crate) fn note_messenger_destroyed() {
    // AcqRel, not Release: the Acquire half synchronises with every earlier destroy's increment
    // this RMW reads past, and each of those happened after its own messenger's creation was
    // counted — so the `created` load below observes at least `prior + 1` even when several
    // contexts in one test binary tear down concurrently. Release publishes to the ledger reader.
    let prior = LEDGER.messengers_destroyed.fetch_add(1, Ordering::AcqRel);
    debug_assert!(
        prior < LEDGER.messengers_created.load(Ordering::Acquire),
        "invariant: a destroyed debug messenger was counted at create"
    );
}

/// What one message counts as in the process ledger — a pure function of its severity and type
/// bits (see [`classify_message`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageClass {
    /// ERROR severity whose type is not GENERAL-only — a validation finding, gated.
    Error,
    /// ERROR severity with the GENERAL type alone — the loader's or a layer's own failure. Gated
    /// (an error is never let through), but reported as an environment fault, not a finding.
    GeneralError,
    /// WARNING severity that is not GENERAL-only — gated.
    ValidationWarning,
    /// WARNING severity with the GENERAL type alone — loader / environment, reported only.
    GeneralWarning,
    /// INFO / VERBOSE — outside the messengers' severity mask; counted nowhere.
    Ignored,
}

/// Classifies one message for the process ledger.
///
/// Severity decides first: ERROR over WARNING over everything else. Within a severity, GENERAL
/// and nothing else is the loader's and the environment's channel; any other type set —
/// including one with no known bit — is a validation finding, so an unknown type fails closed.
const fn classify_message(severity: VkFlags, types: VkFlags) -> MessageClass {
    let validation_types =
        VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
    let general_only = types & VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT != 0 && types & validation_types == 0;
    if severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT != 0 {
        return if general_only { MessageClass::GeneralError } else { MessageClass::Error };
    }
    if severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT == 0 {
        return MessageClass::Ignored;
    }
    if general_only { MessageClass::GeneralWarning } else { MessageClass::ValidationWarning }
}

/// `p`'s text (borrowed when it is valid UTF-8), or `-` when the pointer is null.
///
/// # Safety
///
/// `p` is null or points to a NUL-terminated C string that stays live for `'a`.
unsafe fn c_str_or_dash<'a>(p: *const c_char) -> Cow<'a, str> {
    if p.is_null() {
        return Cow::Borrowed("-");
    }
    // SAFETY: `p` is non-null (checked above) and, per this fn's contract, points to a
    // NUL-terminated C string live for `'a`; `CStr::from_ptr` reads up to the NUL.
    unsafe { CStr::from_ptr(p) }.to_string_lossy()
}

/// The `VK_EXT_debug_utils` callback. Invoked by the loader for each message
/// whose severity/type intersects the messenger's configured masks. It records
/// the message into the process ledger first — for every messenger, including
/// the create-time one whose `p_user_data` is NULL — then, when `p_user_data`
/// is non-null, into that context's [`DebugMessengerState`], then logs
/// `[vk-validation] <message-id name>: <message>`. It always returns `VK_FALSE`
/// (the spec mandates `VK_FALSE` from application callbacks; `VK_TRUE` is
/// reserved for layer development and aborts the call).
///
/// # Safety
///
/// This is an FFI callback. The loader guarantees `p_callback_data` (when
/// non-null) points to a valid `VkDebugUtilsMessengerCallbackDataEXT` for the
/// duration of the call, and `p_user_data` is exactly the pointer supplied at
/// messenger creation: NULL for the create-time instance messenger, or a live
/// `*const DebugMessengerState` owned by the context, which outlives the
/// persistent messenger.
pub(crate) unsafe extern "system" fn debug_callback(
    message_severity: VkFlags,
    message_types: VkFlags,
    p_callback_data: *const VkDebugUtilsMessengerCallbackDataExt,
    p_user_data: *mut c_void,
) -> VkBool32 {
    // The ledger is counted before anything that can be skipped, so no message the loader
    // delivers is missing from the process verdict. Only the create-time instance messenger
    // passes NULL user data, so a NULL pointer IS the instance window.
    LEDGER.record(classify_message(message_severity, message_types), p_user_data.is_null());

    if !p_user_data.is_null() {
        // SAFETY: a non-null `p_user_data` is the `*const DebugMessengerState` we passed to
        // `vkCreateDebugUtilsMessengerEXT`; the context owns the `Box<...>` and destroys the
        // messenger before dropping it, so the pointee is live for every callback invocation.
        // We only ever take `&` (atomic RMWs) — never `&mut` — so concurrent callbacks do not
        // alias mutably.
        let state = unsafe { &*(p_user_data as *const DebugMessengerState) };
        if (message_severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT) != 0 {
            // Release: publishes the increment to the test thread's Acquire load.
            state.errors.fetch_add(1, Ordering::Release);
        } else if (message_severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT) != 0 {
            state.warnings.fetch_add(1, Ordering::Release);
        }
    }

    // Diagnostic only — no verdict reads this text. `pMessageIdName` carries the VUID, which the
    // SDK 1.4.350 `pMessage` text does not.
    let (id_name, message) = if p_callback_data.is_null() {
        (Cow::Borrowed("-"), Cow::Borrowed("-"))
    } else {
        // SAFETY: the loader guarantees a non-null `p_callback_data` points to a valid
        // callback-data struct for the duration of this call.
        let data = unsafe { &*p_callback_data };
        // SAFETY: `p_message_id_name` and `p_message` are each null or a NUL-terminated C string
        // owned by the loader for the duration of this call (the debug-utils spec); `data`
        // borrows that call-scoped struct, so the text does not outlive it.
        unsafe { (c_str_or_dash(data.p_message_id_name), c_str_or_dash(data.p_message)) }
    };
    eprintln!("[vk-validation] {id_name}: {message}");

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

#[cfg(test)]
mod tests {
    use super::*;

    const SEVERITIES: [VkFlags; 4] = [
        VK_DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT,
        VK_DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT,
        VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT,
        VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT,
    ];

    /// The oracle `classify_message` must agree with, stated per D7's policy rather than per the
    /// implementation's branch order.
    fn expected(severity: VkFlags, types: VkFlags) -> MessageClass {
        let general = types & VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT != 0;
        let validation = types & VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT != 0;
        let performance = types & VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT != 0;
        let general_only = general && !validation && !performance;
        match severity {
            VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT if general_only => MessageClass::GeneralError,
            VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT => MessageClass::Error,
            VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT if general_only => MessageClass::GeneralWarning,
            VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT => MessageClass::ValidationWarning,
            _ => MessageClass::Ignored,
        }
    }

    /// Exhaustive: the 4 single severity bits × the 8 subsets of the 3 message-type bits.
    #[test]
    fn classify_message_is_exhaustively_the_d7_policy() {
        let mut cases = 0;
        for severity in SEVERITIES {
            for subset in 0u32..8 {
                let mut types = 0;
                if subset & 1 != 0 {
                    types |= VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT;
                }
                if subset & 2 != 0 {
                    types |= VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT;
                }
                if subset & 4 != 0 {
                    types |= VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
                }
                assert_eq!(
                    classify_message(severity, types),
                    expected(severity, types),
                    "severity {severity:#x}, types {types:#x}"
                );
                cases += 1;
            }
        }
        assert_eq!(cases, 32, "the sweep covers 4 severities x 8 type subsets");
    }

    /// The named anchors of D7 (and of W2's GENERAL-error class), spelled out so a reader does
    /// not have to run the sweep.
    #[test]
    fn classify_message_anchors() {
        let e = VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;
        let w = VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT;
        let g = VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT;
        let v = VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT;
        let p = VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
        assert_eq!(
            classify_message(e, g),
            MessageClass::GeneralError,
            "a GENERAL-only error is its own (gated) class"
        );
        assert_eq!(classify_message(e, v), MessageClass::Error);
        assert_eq!(classify_message(e, g | v), MessageClass::Error);
        assert_eq!(classify_message(e, 0), MessageClass::Error, "an error with no known type fails closed");
        assert_eq!(classify_message(w, g), MessageClass::GeneralWarning);
        assert_eq!(classify_message(w, v), MessageClass::ValidationWarning);
        assert_eq!(classify_message(w, p), MessageClass::ValidationWarning);
        assert_eq!(classify_message(w, g | p), MessageClass::ValidationWarning);
        assert_eq!(classify_message(w, 0), MessageClass::ValidationWarning, "fails closed");
        assert_eq!(
            classify_message(VK_DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT, v),
            MessageClass::Ignored
        );
    }
}
