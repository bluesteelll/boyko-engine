//! [`GpuUpload`] — the trait a GPU-resident [`Asset`] implements to turn its
//! decoded CPU intermediate into a resident record, plus the generic
//! [`upload_assets`] drain that drives it, the concrete [`MeshGpu`] /
//! [`Material`] / [`TextureGpu`] impls, and the three boot one-shot drains
//! ([`upload_material_assets`] / [`upload_mesh_assets`] / [`upload_texture_assets`])
//! the runner calls between `finish()` and `MaterialTable::boot_seed` (asset-system
//! rung A3b; [`upload_texture_assets`] added textured-PBR rung T6b).

use boyko_ecs::ecs::core::asset::{Asset, AssetBacking, Assets, AssetStaging};
use boyko_ecs::ecs::core::system::{NonSendRes, NonSendResMut, ResMut};
use boyko_rhi_vulkan::device::VulkanContext;

use crate::bindless::BindlessTextureTable;
use crate::gpu_column::RhiContext;
use crate::material::Material;
use crate::mesh::MeshGpu;
use crate::mesh_assets::build_mesh_gpu;
use crate::mesh_data::MeshData;
use crate::texture::{TextureGpu, build_texture_gpu};
use crate::texture_data::TextureData;

/// A GPU-resident [`Asset`] that can turn its decoded
/// [`Asset::Cpu`](boyko_ecs::ecs::core::asset::Asset::Cpu) intermediate into
/// a resident record on the device.
///
/// `Aux` is the per-asset-type mutable context an upload needs beyond the
/// device itself (e.g. a bindless-slot allocator, a staging-ring cursor) —
/// left an associated type so a future asset type (texture) can carry one;
/// both current implementors ([`MeshGpu`], [`Material`]) need none (`()`).
pub trait GpuUpload: Asset {
    /// Extra per-asset-type mutable state [`upload`](Self::upload) needs
    /// beyond the device context.
    type Aux;

    /// Turns a decoded CPU intermediate into a resident GPU asset.
    fn upload(cpu: <Self as Asset>::Cpu, ctx: &VulkanContext, aux: &mut Self::Aux) -> Self;
}

impl GpuUpload for MeshGpu {
    /// `build_mesh_gpu` needs only the device context — no extra per-type state.
    type Aux = ();

    /// Builds the resident mesh through the EXACT SAME device path
    /// [`MeshAssetsExt::register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh)
    /// uses for a host-authored mesh: create + fill the vertex/index buffers
    /// (index width chosen by O3), and build the BLAS eagerly on an RT device.
    #[inline]
    fn upload(cpu: MeshData, ctx: &VulkanContext, _aux: &mut Self::Aux) -> Self {
        build_mesh_gpu(ctx, &cpu.vertices, &cpu.indices)
    }
}

impl GpuUpload for Material {
    /// Material upload is identity — no device work; no extra state needed.
    type Aux = ();

    /// Materials need no device work: the GPU layout IS the decoded CPU form
    /// (see [`Asset::Cpu`] on [`Material`]'s own `Asset` impl). The GPU
    /// mirror ([`MaterialTable`](crate::material_table::MaterialTable)) reads
    /// the filled [`Assets<Material>`] row's `gpu` field separately — it holds no
    /// authority of its own.
    #[inline]
    fn upload(cpu: Material, _ctx: &VulkanContext, _aux: &mut Self::Aux) -> Self {
        cpu
    }
}

impl GpuUpload for TextureGpu {
    /// A texture upload registers a bindless slot, so it needs the world's
    /// [`BindlessTextureTable`] (textured-PBR T2) beyond the device context.
    type Aux = BindlessTextureTable;

    /// Builds the resident, mip-chained, bindless-registered texture through the
    /// EXACT SAME device path
    /// [`TextureAssetsExt::register_texture`](crate::texture::TextureAssetsExt::register_texture)
    /// uses for a host-authored texture.
    #[inline]
    fn upload(cpu: TextureData, ctx: &VulkanContext, aux: &mut Self::Aux) -> Self {
        build_texture_gpu(ctx, aux, &cpu)
    }
}

/// Drains every entry queued in `staging`, uploads each via
/// [`GpuUpload::upload`], and [`fill`](Assets::fill)s the corresponding
/// `Reserved` row in `assets`.
///
/// The empty-queue check keeps a call site cheap when nothing is in flight,
/// without touching `assets` at all — the common case at boot (no scene loads
/// from disk yet) and, later, on a per-frame call site with nothing newly
/// decoded this frame.
///
/// `A: AssetBacking` (asset-streaming plan F1): `assets: &mut Assets<A>`
/// requires it — `Assets<T>`'s own generic bound.
pub fn upload_assets<A: GpuUpload + AssetBacking>(
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
        // A handle that no longer resolves to `Reserved` (removed between decode
        // and a later rung's unload, or already filled by a re-entrant drain) is a
        // no-op here, not a bug this drain can rule out in general — so the `Err`
        // is ignored rather than asserted.
        let _ = assets.fill(staged.handle, gpu);
    }
}

/// Boot one-shot (asset-system rung A3b): drains `AssetStaging<Material>`
/// into the world's `Assets<Material>` [`Resource`](boyko_ecs::ecs::core::resources::resource::Resource)
/// table.
///
/// A distinct, non-generic wrapper — not a single `upload_assets::<A, _>`
/// closure generic over the asset type — because `Assets<Material>` is
/// registered as a `Resource` (`Material: Send + Sync`) while
/// `Assets<MeshGpu>` (see [`upload_mesh_assets`]) is registered as a
/// [`NonSendResource`](boyko_ecs::ecs::core::resources::resource::NonSendResource):
/// `ResMut<Assets<Material>>` and `NonSendResMut<Assets<MeshGpu>>` are two
/// different `SystemParam` wrapper types no single generic-over-`A` system
/// signature can express uniformly.
///
/// Called explicitly at boot, BEFORE [`MaterialTable::boot_seed`](crate::material_table::MaterialTable::boot_seed)
/// (which hard-sizes the device SSBO from whatever this drain just filled) —
/// never on the per-frame path.
pub fn upload_material_assets(
    mut assets: ResMut<Assets<Material>>,
    mut staging: NonSendResMut<AssetStaging<Material>>,
    ctx: NonSendRes<RhiContext>,
) {
    upload_assets(&mut assets, &mut staging, ctx.context(), &mut ());
}

/// Boot one-shot (asset-system rung A3b): drains `AssetStaging<MeshGpu>` into
/// the world's NonSend `Assets<MeshGpu>` table (the mesh records own their GPU
/// buffers, so this table itself is the GPU-resident mesh table — no separate
/// mirror). See [`upload_material_assets`] for why this is a distinct,
/// non-generic wrapper rather than one generic-over-`A` system.
pub fn upload_mesh_assets(
    mut assets: NonSendResMut<Assets<MeshGpu>>,
    mut staging: NonSendResMut<AssetStaging<MeshGpu>>,
    ctx: NonSendRes<RhiContext>,
) {
    upload_assets(&mut assets, &mut staging, ctx.context(), &mut ());
}

/// Boot one-shot (textured-PBR rung T6b): drains `AssetStaging<TextureGpu>` into
/// the world's NonSend `Assets<TextureGpu>` table (mirrors [`upload_mesh_assets`] —
/// `TextureGpu` owns its GPU image directly, so this table itself is the
/// GPU-resident texture table, no separate mirror). Unlike the mesh/material
/// wrappers, `TextureGpu::Aux = BindlessTextureTable` (T4) is threaded through as
/// the world's NonSend bindless-slot allocator, so a drained texture is
/// bindless-registered by the SAME call that fills its `Assets<TextureGpu>` row.
/// See [`upload_material_assets`] for why this is a distinct, non-generic wrapper
/// rather than one generic-over-`A` system.
pub fn upload_texture_assets(
    mut assets: NonSendResMut<Assets<TextureGpu>>,
    mut staging: NonSendResMut<AssetStaging<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    ctx: NonSendRes<RhiContext>,
) {
    upload_assets(&mut assets, &mut staging, ctx.context(), &mut bindless);
}
