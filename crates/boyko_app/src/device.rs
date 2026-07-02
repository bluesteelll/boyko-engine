//! The [`GpuDevice`] NonSend newtype — the world-resident face of the pinned
//! device singleton (host plan D2 step 4).

use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_rhi_vulkan::device::VulkanContext;

/// The pinned process-singleton device handle, world-resident so setup-stage
/// systems (startup one-shots, mesh registration in R3+) can reach the device
/// without touching the host structs.
///
/// # Containment contract (review P1-2)
///
/// The `&'static` NEVER escapes to safe user code: the field is crate-private
/// and [`get`](Self::get) reborrows at the caller's `&self` lifetime — a user
/// system can use the device for the duration of its borrow of this resource,
/// but cannot stash a `'static` copy (in its own resource, a `static`, …) that
/// would survive the runner's eviction and dangle after
/// `VulkanContext::destroy_singleton`. Only the runner (this crate) holds the
/// raw `'static`, and it upholds the lifecycle.
///
/// Inserted by the windowed runner BEFORE `App::finish()` (so startup systems
/// see it) and REMOVED by the runner's teardown BEFORE
/// [`VulkanContext::destroy_singleton`] runs — no dangling `&'static` may
/// remain in a live structure once the singleton's lifecycle ends (the
/// `'static` is a documented fiction that call ends).
pub struct GpuDevice(pub(crate) &'static VulkanContext);

impl GpuDevice {
    /// Borrows the device for the duration of the `&self` borrow. The
    /// `'static` deliberately does not escape — see the containment contract
    /// on the type.
    #[inline]
    pub fn get(&self) -> &VulkanContext {
        self.0
    }
}

// SAFETY (no `unsafe`): `GpuDevice` is `!Send + !Sync` automatically —
// `VulkanContext` holds raw `*const DeviceFns` pointers, so it is `!Sync`, and
// `&T` is `Send`/`Sync` only if `T: Sync`; the property propagates. The
// `NonSendResource` contract ("touched only on the owning thread") is upheld by
// the runner: all Vulkan queue access is runner-thread-only.
impl NonSendResource for GpuDevice {}
