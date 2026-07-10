//! [`GpuUpload`] — the trait a GPU-resident [`Asset`] implements to turn its
//! decoded CPU intermediate into a resident record, plus the generic
//! [`upload_assets`] drain that drives it (asset-system rung A3a: the upload
//! SKELETON only — no concrete `GpuUpload` impl exists yet; the first one
//! lands at rung A3b, together with the actual per-asset-type upload work).

use boyko_ecs::ecs::core::asset::{Asset, Assets, AssetStaging};
use boyko_rhi_vulkan::device::VulkanContext;

/// A GPU-resident [`Asset`] that can turn its decoded
/// [`Asset::Cpu`](boyko_ecs::ecs::core::asset::Asset::Cpu) intermediate into
/// a resident record on the device.
///
/// `Aux` is the per-asset-type mutable context an upload needs beyond the
/// device itself (e.g. a bindless-slot allocator, a staging-ring cursor) —
/// left an associated type because rung A3a has no concrete implementor yet;
/// A3b fills one in per asset type (mesh / material / texture) and picks a
/// concrete `Aux`.
pub trait GpuUpload: Asset {
    /// Extra per-asset-type mutable state [`upload`](Self::upload) needs
    /// beyond the device context.
    type Aux;

    /// Turns a decoded CPU intermediate into a resident GPU asset.
    fn upload(cpu: <Self as Asset>::Cpu, ctx: &VulkanContext, aux: &mut Self::Aux) -> Self;
}

/// Drains every entry queued in `staging`, uploads each via
/// [`GpuUpload::upload`], and [`fill`](Assets::fill)s the corresponding
/// `Reserved` row in `assets`.
///
/// This is the A3a SKELETON: it compiles and is behavior-complete for any
/// concrete `A: GpuUpload`, but no such impl exists in the engine yet (rung
/// A3b adds the first one) — so this function has no production caller until
/// then. The empty-queue check keeps a per-frame call site (once A3b wires
/// one) cheap when nothing is in flight, without touching `assets` at all.
pub fn upload_assets<A: GpuUpload>(
    assets: &mut Assets<A>,
    staging: &mut AssetStaging<A>,
    ctx: &VulkanContext,
    aux: &mut A::Aux,
) {
    if staging.is_empty() {
        return;
    }
    for staged in staging.drain() {
        let gpu = A::upload(staged.cpu, ctx, aux);
        let filled = assets.fill(staged.handle, gpu);
        debug_assert!(
            filled.is_ok(),
            "invariant: a handle staged by AssetServer::load must still be Reserved \
             when its upload drains here (fill failed — stale handle or double-fill bug)"
        );
    }
}
