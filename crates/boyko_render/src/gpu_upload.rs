//! [`GpuUpload`] — the trait a GPU-resident
//! [`Asset`](boyko_ecs::ecs::core::asset::Asset) implements to turn its
//! decoded CPU intermediate into a resident record, plus the generic
//! [`upload_assets`] drain that drives it, the concrete [`MeshGpu`] /
//! [`Material`] / [`TextureGpu`] impls, and the three boot one-shot drains
//! ([`upload_material_assets`] / [`upload_mesh_assets`] / [`upload_texture_assets`])
//! the runner calls between `finish()` and `MaterialTable::boot_seed` (asset-system
//! rung A3b; [`upload_texture_assets`] added textured-PBR rung T6b).

use boyko_ecs::ecs::core::asset::{Asset, AssetBacking, Assets, AssetStaging, Handle};
use boyko_ecs::ecs::core::system::{NonSendRes, NonSendResMut, ResMut};
use boyko_rhi_vulkan::device::VulkanContext;

use crate::bindless::BindlessTextureTable;
use crate::gpu_column::RhiContext;
use crate::material::Material;
use crate::mesh::MeshGpu;
use crate::mesh_assets::build_mesh_gpu;
use crate::mesh_data::MeshData;
use crate::mesh_geometry_table::{MeshGeometryTableSlot, VB_GEOMETRY_RESERVED_SLOT};
use crate::texture::{TextureGpu, build_texture_gpu};
use crate::texture_data::TextureData;

/// A GPU-resident [`Asset`] that can turn its decoded
/// [`Asset::Cpu`] intermediate into
/// a resident record on the device.
///
/// `Aux` is the per-asset-type mutable context an upload needs beyond the
/// device itself — an associated type because the implementors genuinely differ:
/// [`MeshGpu`] threads the VB [`MeshGeometryTableSlot`], [`TextureGpu`] threads the
/// [`BindlessTextureTable`], and only [`Material`] (a pure host-side identity upload)
/// needs none (`()`).
pub trait GpuUpload: Asset {
    /// Extra per-asset-type mutable state [`upload`](Self::upload) needs
    /// beyond the device context.
    type Aux;

    /// Turns a decoded CPU intermediate into a resident GPU asset.
    fn upload(cpu: <Self as Asset>::Cpu, ctx: &VulkanContext, aux: &mut Self::Aux) -> Self;
}

impl GpuUpload for MeshGpu {
    /// Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / Rev-5 streaming
    /// invariant): the always-present [`MeshGeometryTableSlot`] wrapper resource
    /// (`None` unless `ResolvedRenderPath.vb_geometry_table` was armed at boot — see
    /// that type's doc) — the STREAMED mesh path (this impl) is the ONE registration
    /// site that threads `Option<&mut MeshGeometryTable>` all the way through
    /// `build_mesh_gpu`, satisfying "every runtime-streamed mesh claims a slot when
    /// armed" without widening `MeshAssetsExt::register_mesh`'s public signature (see
    /// `build_mesh_gpu`'s doc for the host-authored-path scope cut).
    type Aux = MeshGeometryTableSlot;

    /// Builds the resident mesh through the EXACT SAME device path
    /// [`MeshAssetsExt::register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh)
    /// uses for a host-authored mesh: create + fill the vertex/index buffers
    /// (index width chosen by O3), build the BLAS eagerly on an RT device, and — when
    /// `aux.0` holds a live table — claim its geometry-table slot.
    #[inline]
    fn upload(cpu: MeshData, ctx: &VulkanContext, aux: &mut Self::Aux) -> Self {
        build_mesh_gpu(ctx, &cpu.vertices, &cpu.indices, aux.0.as_mut())
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
///
/// Multi-paradigm render-path plan, rung R-VBGEO: also threads the world's
/// [`MeshGeometryTableSlot`] (mirrors [`upload_texture_assets`]'s
/// `NonSendResMut<BindlessTextureTable>` thread one level down) so every mesh this
/// drain uploads claims a geometry-table slot when armed. `boyko_app::runner` inserts
/// `MeshGeometryTableSlot` right after `resolve_render_path`, BEFORE this system's
/// first call — the Rev-5 "flag reaches the registration site before the first mesh
/// upload" gate.
pub fn upload_mesh_assets(
    mut assets: NonSendResMut<Assets<MeshGpu>>,
    mut staging: NonSendResMut<AssetStaging<MeshGpu>>,
    ctx: NonSendRes<RhiContext>,
    mut geometry_table: NonSendResMut<MeshGeometryTableSlot>,
) {
    upload_assets(&mut assets, &mut staging, ctx.context(), &mut geometry_table);
}

/// Boot one-shot (Multi-paradigm render-path plan): back-fill a VB geometry-table slot for every
/// HOST-AUTHORED mesh that does not have one yet, so a `VisibilityBuffer` boot renders ANY scene's
/// meshes — not only those a scene happened to register through the VB-aware
/// [`MeshAssetsVbExt::register_mesh_vb`](crate::mesh_assets::MeshAssetsVbExt::register_mesh_vb).
///
/// # Why this exists
///
/// [`MeshAssetsExt::register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh)/`cube`/`plane`
/// (what nearly every scene calls) already create STORAGE-usage-capable vertex/index buffers when
/// the table is armed ([`build_mesh_gpu`] reads `ctx.vb_geometry_table_armed()` for the usage bits,
/// INDEPENDENTLY of the threaded table) — they simply pass `None` for the table and so leave
/// `geometry_slot == VB_GEOMETRY_RESERVED_SLOT`. Under a VB boot those meshes therefore had
/// STORAGE-ready buffers but no bindless slot, so `vb_resolve` re-fetched the degenerate zero-count
/// slot 0 and drew nothing. This drain claims the missing slots ONCE at boot, from the buffers the
/// registration already built — the same `MeshGeometryTable::register` call the streamed path and
/// `register_mesh_vb` use, just applied after the fact.
///
/// # Idempotent / no-op unless armed
///
/// A `None` table slot (every non-VB boot) returns immediately — so Deferred / Forward / ForwardPlus
/// are byte-identical (this system never runs its body). Meshes that already hold a real slot
/// (streamed, or `register_mesh_vb`) are skipped (their slot is `>= 1`), so re-running is harmless
/// and the existing VB goldens (`vb_mesh`/`vb_both`/`vb_sdf_only`, which use `register_mesh_vb`)
/// stay byte-identical. `boyko_app::runner` calls this right after [`upload_mesh_assets`], after
/// `finish()` has drained every startup `register_mesh`.
///
/// # Scope
///
/// Boot-static, exactly like the SDF-edit gather (`collect_sdf_edits`): a mesh registered at
/// RUNTIME (post-boot) under VB would need this re-run — no scene does that today, and the whole
/// host mesh/SDF assembly is boot-static this rung.
pub fn backfill_vb_geometry_slots(
    mut assets: NonSendResMut<Assets<MeshGpu>>,
    ctx: NonSendRes<RhiContext>,
    mut geometry_table: NonSendResMut<MeshGeometryTableSlot>,
) {
    // Not a VB boot (`MeshGeometryTableSlot(None)`) → nothing to back-fill (the 0%-gate — keeps
    // every non-VB path byte-identical).
    let Some(table) = geometry_table.0.as_mut() else {
        return;
    };
    let ctx = ctx.context();

    // `iter()` borrows `assets` immutably; collect the handles of the still-reserved (host-authored)
    // meshes first, then re-borrow mutably via `get_mut` to claim + stamp each slot. One small boot
    // allocation, pre-sized to the live count — never on the per-frame path.
    let mut pending: Vec<Handle<MeshGpu>> = Vec::with_capacity(assets.len());
    for (handle, mesh) in assets.iter() {
        if mesh.geometry_slot == VB_GEOMETRY_RESERVED_SLOT {
            pending.push(handle);
        }
    }
    for handle in pending {
        if let Some(mesh) = assets.get_mut(handle) {
            // Virtual-geometry ladder, rung R2d-1: a back-filled mesh gets its
            // `gMeshBounds[]` row from the AABB `build_mesh_gpu` already folded onto the
            // record - the back-fill reaches the same rows the streamed path writes, so a
            // host-authored mesh is not left on the "bounds unknown" sentinel.
            let slot = table.register(
                ctx,
                &mesh.vertex_buffer,
                mesh.vertex_count,
                &mesh.index_buffer,
                mesh.index_count,
                mesh.index_type,
                mesh.local_min,
                mesh.local_max,
            );
            mesh.geometry_slot = slot;
        }
    }
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
