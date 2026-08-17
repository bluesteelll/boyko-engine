//! Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / C1) — the bindless
//! per-mesh geometry table [`MeshGeometryTable`]: the device face of `Assets<MeshGpu>`
//! for the `VisibilityBuffer` path's compute shade/resolve passes.
//!
//! A direct sibling of [`BindlessTextureTable`](crate::bindless::BindlessTextureTable):
//! same free-list [`BindlessSlotAllocator`](crate::bindless::BindlessSlotAllocator),
//! same fence-gated recycle discipline, same reserved-slot-0 convention. It owns the
//! VB-only Set-3 device object
//! ([`VulkanGeometryBindlessSet`](boyko_rhi_vulkan::geometry_bindless::VulkanGeometryBindlessSet))
//! — two bindless `STORAGE_BUFFER` runtime arrays (`gMeshVerts[]` / `gMeshIndices[]`,
//! one slot per registered mesh, pointing at that mesh's OWN
//! [`MeshGpu`](crate::mesh::MeshGpu) vertex/index buffer) plus one plain
//! (non-bindless) `STORAGE_BUFFER` binding for `gMeshMeta[]` — a single growable SSBO
//! of `{index_width, vertex_count, index_count}` rows, the SAME shape
//! [`MaterialTable`](crate::material_table::MaterialTable) uses for `MaterialGpu[]`.
//!
//! # Reachable as of rung R8
//!
//! `ResolvedRenderPath.vb_geometry_table` is `path == VisibilityBuffer && mesh_leg &&
//! device supports it`, computed in `render_path_config::resolve_rules`. `VB_IMPLEMENTED`
//! (that module's own rung-staged const) flipped `true` at rung R8, so a
//! `VisibilityBuffer` request that survives the device-cap degrade now genuinely resolves
//! `vb_geometry_table == true` — `boyko_app::runner`'s boot seam then constructs a LIVE
//! [`MeshGeometryTable`] (`MeshGeometryTableSlot(Some(table))`), and the VB Set-2 geometry
//! set it owns is bound by `vb_resolve.comp.hlsl` (Decision 0). `MeshGeometryTableSlot`
//! stays `None` on every other boot (Deferred/Forward/ForwardPlus, or a VB request that
//! degraded for a missing device cap) — the 0%-gate.
//!
//! # `mesh_id` — where it lives, how the instance gather reads it (R8)
//!
//! [`MeshGeometryTable::register`] returns the allocated slot index, stored as
//! [`MeshGpu::geometry_slot`](crate::mesh::MeshGpu::geometry_slot) — mirroring how a
//! texture's bindless slot is stored on [`MaterialTextures`](crate::material::MaterialTextures)
//! at the OWNING asset record, not in a separate handle→slot side table. The VB instance-ring
//! builder (`crate::mesh_draw::MeshRenderScratch::sync_vb_instance_ring`) resolves
//! `MeshHandle → &MeshGpu → geometry_slot` from the SAME `mesh_ids` lane
//! [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) already scatters (the SAME
//! "copied from the asset table" pattern `DrawBatch::index_count`/`index_type` use) and packs
//! it as the [`VbInstanceRow::mesh_id`](crate::instance_model::VbInstanceRow::mesh_id) lane.
//!
//! # Per-mesh local bounds (virtual-geometry ladder, rung R2d-1)
//!
//! The table also owns a `gMeshBounds[]` buffer — one [`MeshLocalBounds`] row per slot,
//! keyed by the SAME `mesh_id`. R2d moves the VB frustum cull from per-BATCH to
//! per-INSTANCE granularity, and an instance's world AABB is its mesh's LOCAL AABB
//! transformed by the instance affine; the local box is a per-MESH property, so it lives
//! here rather than on a per-frame batch descriptor (and every rung above R2d — occlusion,
//! meshlet culling — wants the same `mesh_id`-keyed GPU-resident bounds).
//!
//! Rung R2d-1 ships the table and nothing else: the buffer is allocated, prefilled and
//! written, but bound to no descriptor set, read by no shader, and named by no framegraph
//! edge. The GPU sees one extra allocation and zero changed commands.

use bytemuck::{Pod, Zeroable};

use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_log::codes::{OnceSite, W2202};
use boyko_rhi::enums::IndexType;
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::error::VulkanError;
use boyko_rhi_vulkan::geometry_bindless::{
    GEOMETRY_INDICES_BINDING, GEOMETRY_META_BINDING, GEOMETRY_VERTS_BINDING,
    VulkanGeometryBindlessSet, create_geometry_bindless_set, destroy_geometry_bindless_set,
    write_geometry_buffer_slot,
};
use boyko_rhi_vulkan::memory::BoundBuffer;

use crate::bindless::BindlessSlotAllocator;

/// The reserved "no geometry-table slot" / degenerate-mesh slot —
/// [`MeshGeometryTable::register`] never issues it (mirrors
/// [`BindlessSlotAllocator`]'s own slot-0 reservation). A [`MeshGpu`](crate::mesh::MeshGpu)
/// registered while the table is absent (every non-VB boot) carries
/// `geometry_slot == VB_GEOMETRY_RESERVED_SLOT`. Slot 0's `gMeshMeta` row is left
/// ALL-ZERO (`index_width == vertex_count == index_count == 0` — the "zero-count meta"
/// degenerate choice): a future shader that accidentally dynamically indexes it hits
/// `debug_assert!(tri_count > 0)` at the fetch side (R8's `vb_geom_fetch.hlsli`) rather
/// than reading uninitialized/aliased geometry.
pub const VB_GEOMETRY_RESERVED_SLOT: u32 = 0;

/// One `gMeshMeta[]` row (Decision 0 / plan §Data structures): `{index_width,
/// vertex_count, index_count}`, padded to 16 B for std430 stability (a plain 12-byte
/// `uint` triplet is not itself a stable std430 array-element stride across every
/// driver's structured-buffer layout rules — padding to a whole 4-word lane removes
/// the ambiguity, the SAME discipline [`PerInstanceMaterial`](crate::mesh_draw::PerInstanceMaterial)'s
/// doc explains for its own float4-lane packing).
///
/// `index_width` is the index buffer's element width in BYTES (`2` for
/// [`IndexType::Uint16`], `4` for [`IndexType::Uint32`]) — see [`index_width_bytes`].
/// `tri_count` is NOT stored; it is the Decision-9 normalizer
/// [`tri_count`]`(index_count)`, computed on demand (host or shader
/// side) rather than duplicated as a fourth stored field.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, Default)]
pub struct MeshGeometryMeta {
    /// The index buffer's element width in bytes (`2` or `4`). Offset 0.
    pub index_width: u32,
    /// The mesh's unique-vertex count (mirrors [`MeshGpu::vertex_count`](crate::mesh::MeshGpu::vertex_count)).
    /// Offset 4.
    pub vertex_count: u32,
    /// The mesh's index count (mirrors [`MeshGpu::index_count`](crate::mesh::MeshGpu::index_count));
    /// `tri_count(index_count)` recovers the triangle count (Decision 9). Offset 8.
    pub index_count: u32,
    /// Pads the row to a 16-byte std430-stable stride — unused, always zero. Offset 12.
    pub _pad: u32,
}

/// The byte size of one [`MeshGeometryMeta`] row — the `gMeshMeta[]` SSBO's
/// per-mesh stride.
pub const MESH_GEOMETRY_META_BYTES: usize = 16;

const _: () = assert!(
    core::mem::size_of::<MeshGeometryMeta>() == MESH_GEOMETRY_META_BYTES,
    "MeshGeometryMeta must be 16 bytes (a uint triplet padded to one std430 lane)"
);
const _: () = assert!(core::mem::offset_of!(MeshGeometryMeta, index_width) == 0);
const _: () = assert!(core::mem::offset_of!(MeshGeometryMeta, vertex_count) == 4);
const _: () = assert!(core::mem::offset_of!(MeshGeometryMeta, index_count) == 8);
const _: () = assert!(core::mem::offset_of!(MeshGeometryMeta, _pad) == 12);

/// The magnitude of the "bounds unknown" sentinel's coordinates — deliberately huge
/// but **finite**.
///
/// An `f32::INFINITY` here would be the obvious encoding and is the wrong one. A plane
/// evaluation `dot(plane.xyz, corner) + plane.w` over an infinite corner produces a NaN
/// by two routes: `0.0 * inf` on any axis whose plane normal component is zero, and
/// `inf + (-inf)` when the normal's components have mixed signs. A NaN then does NOT
/// propagate through HLSL's `min`/`max` — SPIR-V `NMin`/`NMax` silently return the
/// OTHER operand — so it neither survives to be noticed nor announces itself; it just
/// makes a later `clamp`/compare take an arbitrary branch.
///
/// `1e30` avoids that entirely: a normalized plane's dot product against a `1e30`
/// corner peaks near `3e30`, four orders of magnitude below `f32::MAX` (`3.4e38`), so
/// every intermediate stays finite — while `1e30` is still orders of magnitude outside
/// any real scene extent.
pub const MESH_BOUNDS_UNKNOWN_COORD: f32 = 1e30;

/// One `gMeshBounds[]` row (virtual-geometry ladder, rung R2d): a mesh's LOCAL
/// (model-space) AABB, keyed by the SAME slot [`MeshGeometryTable::register`] hands
/// out — i.e. by `mesh_id`, the lane [`VbInstanceRow`](crate::instance_model::VbInstanceRow)
/// already carries at offset 48.
///
/// # Why per-MESH and not per-batch
///
/// R2d's per-INSTANCE frustum cull needs each instance's WORLD box, which is this local
/// box transformed by the instance affine. The local box is a property of the MESH, not
/// of a frame's batching, so it belongs in the per-mesh geometry table — and every rung
/// above R2d (occlusion, meshlet culling) needs exactly the same GPU-resident,
/// `mesh_id`-keyed bounds.
///
/// # Layout
///
/// `float3`-then-`u32`, twice: 32 B total, and each half is already a whole 16-byte
/// std430/std140 lane, so NO explicit trailing padding member is needed — which is why
/// this shape was chosen over `{ min: [f32;3], max: [f32;3] }` (24 B, whose array
/// stride is not stable across every driver's structured-buffer rules) or a
/// `float4`-pair with the extents in `.w` (same 32 B but with the sentinel spread over
/// four lanes). `_p0`/`_p1` are the two `u32` halves; both are always zero and exist
/// solely to make the two `float3`s land on their own lanes.
///
/// # The `min > max` reading — "unknown", never "invisible"
///
/// Slot contents that are not a registered mesh's real box are the INVERTED sentinel
/// [`Self::UNKNOWN`] (`min = +1e30`, `max = -1e30`), never zeros. An all-zero row is
/// indistinguishable from a legitimate point-sized mesh at the origin; `min > max`
/// cannot arise from any real geometry fold, so the encoding is unambiguous. This is
/// the same argument [`MeshGpu::local_min`](crate::mesh::MeshGpu::local_min)
/// (`mesh.rs:179-183`) makes for the C0 zero-vertex case, where the empty fold yields
/// `[f32::INFINITY; 3]` / `[f32::NEG_INFINITY; 3]` for exactly this reason.
///
/// **A consumer that observes `any(min > max)` must read it as "bounds unknown" and
/// KEEP the instance.** Absence of bounds is not evidence of invisibility; culling on
/// an unknown box would drop geometry that is genuinely on screen. That is the
/// invariant every future consumer of this table depends on.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct MeshLocalBounds {
    /// Model-space AABB minimum. Offset 0.
    pub min: [f32; 3],
    /// Lands `max` on its own 16-byte lane — unused, always zero. Offset 12.
    pub _p0: u32,
    /// Model-space AABB maximum. Offset 16.
    pub max: [f32; 3],
    /// Pads the row to a 32-byte std430-stable stride — unused, always zero. Offset 28.
    pub _p1: u32,
}

/// The byte size of one [`MeshLocalBounds`] row — the `gMeshBounds[]` buffer's per-mesh
/// stride.
pub const MESH_LOCAL_BOUNDS_BYTES: usize = 32;

const _: () = assert!(
    core::mem::size_of::<MeshLocalBounds>() == MESH_LOCAL_BOUNDS_BYTES,
    "MeshLocalBounds must be 32 bytes (two float3-plus-uint std430 lanes)"
);
const _: () = assert!(core::mem::offset_of!(MeshLocalBounds, min) == 0);
const _: () = assert!(core::mem::offset_of!(MeshLocalBounds, _p0) == 12);
const _: () = assert!(core::mem::offset_of!(MeshLocalBounds, max) == 16);
const _: () = assert!(core::mem::offset_of!(MeshLocalBounds, _p1) == 28);

impl MeshLocalBounds {
    /// The "bounds unknown" row every slot — INCLUDING the reserved
    /// [`VB_GEOMETRY_RESERVED_SLOT`] — starts as. See the type doc for why this is an
    /// inverted box rather than zeros, and for the KEEP obligation it places on a
    /// consumer.
    pub const UNKNOWN: Self = Self {
        min: [MESH_BOUNDS_UNKNOWN_COORD; 3],
        _p0: 0,
        max: [-MESH_BOUNDS_UNKNOWN_COORD; 3],
        _p1: 0,
    };

    /// Builds a row from a mesh's model-space AABB (the fold
    /// [`build_mesh_gpu`](crate::mesh_assets::build_mesh_gpu) already performs, stored
    /// on [`MeshGpu`](crate::mesh::MeshGpu) as `local_min`/`local_max`). No
    /// `Default` impl exists on purpose: a zeroed row is the ambiguous encoding this
    /// type's doc rejects, so "the default row" must be spelled [`Self::UNKNOWN`].
    ///
    /// # A box that is not a finite, non-inverted AABB becomes [`Self::UNKNOWN`]
    ///
    /// This is the CHOKE POINT that makes the finiteness the type doc claims actually
    /// true, rather than merely asserted. Both call sites source their pair from
    /// [`local_aabb`](crate::mesh_assets::local_aabb), which seeds its fold with
    /// `[f32::INFINITY; 3]` / `[f32::NEG_INFINITY; 3]` and RETURNS that seed for a
    /// zero-vertex mesh — so without this normalisation the very first such mesh
    /// writes an INFINITE row and defeats the whole reason [`Self::UNKNOWN`] uses a
    /// large-but-finite coordinate: `dot(n, p) + d` against an infinite corner can
    /// produce a NaN, and a NaN comparison picks the OTHER operand under `NMin`/`NMax`
    /// rather than propagating, so a single infinity turns a conservative KEEP into an
    /// arbitrary verdict.
    ///
    /// Infinities, NaNs and inverted inputs therefore all collapse to the one encoding
    /// a consumer already has to handle. That is the CONSERVATIVE direction: absence of
    /// bounds means KEEP, never cull.
    #[inline]
    pub const fn new(min: [f32; 3], max: [f32; 3]) -> Self {
        // Comparison-only finiteness: every comparison against NaN is false, and both
        // infinities fail their own bound, so this admits exactly the finite reals
        // without needing `f32::is_finite` in const context.
        const fn finite(x: f32) -> bool {
            x > f32::NEG_INFINITY && x < f32::INFINITY
        }
        let usable = finite(min[0])
            && finite(min[1])
            && finite(min[2])
            && finite(max[0])
            && finite(max[1])
            && finite(max[2])
            && min[0] <= max[0]
            && min[1] <= max[1]
            && min[2] <= max[2];
        if usable { Self { min, _p0: 0, max, _p1: 0 } } else { Self::UNKNOWN }
    }

    /// `true` iff this row is an inverted box (`min > max` on any axis) — i.e. the
    /// [`Self::UNKNOWN`] sentinel or an unwritten slot. The host mirror of the
    /// `any(min > max)` test a device-side consumer performs; a `true` here means
    /// "bounds unknown ⇒ KEEP", never "empty ⇒ cull".
    #[inline]
    pub fn is_unknown(&self) -> bool {
        self.min[0] > self.max[0] || self.min[1] > self.max[1] || self.min[2] > self.max[2]
    }
}

/// Stamps [`MeshLocalBounds::UNKNOWN`] over every row of `rows` — the `gMeshBounds[]`
/// prefill [`MeshGeometryTable::new`] applies to the whole buffer before any
/// registration.
///
/// Factored out of `new` (which needs a live device to allocate the mapped buffer)
/// purely so the PATTERN is unit-testable without a `VulkanContext` — the same
/// factoring rationale [`mesh_buffer_usage`] and `cube_geometry` use.
#[inline]
pub fn prefill_bounds_unknown(rows: &mut [MeshLocalBounds]) {
    rows.fill(MeshLocalBounds::UNKNOWN);
}

#[cfg(test)]
mod bounds_normalisation_tests {
    use super::{MESH_BOUNDS_UNKNOWN_COORD, MeshLocalBounds};

    /// The defect this normalisation exists for, pinned at the value that produces it:
    /// `local_aabb` seeds its fold with `±INFINITY` and RETURNS that seed for a
    /// zero-vertex mesh, so a raw constructor would write an infinite row into a table
    /// whose whole sentinel design depends on staying finite.
    #[test]
    fn the_zero_vertex_infinite_seed_becomes_the_finite_unknown_row() {
        let row = MeshLocalBounds::new([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        assert_eq!(row, MeshLocalBounds::UNKNOWN, "the infinite seed must collapse to UNKNOWN");
        assert!(
            row.min.iter().chain(row.max.iter()).all(|c| c.is_finite()),
            "an infinity here reaches a plane evaluation as a NaN, and a NaN picks the OTHER \
             operand under NMin/NMax instead of propagating"
        );
    }

    /// Every non-AABB input collapses to the ONE encoding a consumer already handles.
    /// Each case is a distinct route to a wrong verdict, not a restatement of the last.
    #[test]
    fn nan_partial_infinity_and_inversion_all_collapse_to_unknown() {
        for (min, max, why) in [
            ([f32::NAN, 0.0, 0.0], [1.0, 1.0, 1.0], "NaN in min"),
            ([0.0; 3], [1.0, f32::NAN, 1.0], "NaN in max"),
            ([0.0; 3], [f32::INFINITY, 1.0, 1.0], "one infinite corner"),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 1.0], "inverted on one axis only"),
        ] {
            assert_eq!(MeshLocalBounds::new(min, max), MeshLocalBounds::UNKNOWN, "{why}");
        }
    }

    /// The normalisation must not eat legitimate geometry — including the degenerate
    /// cases a real mesh genuinely produces: a point-sized box and a flat plane.
    #[test]
    fn real_boxes_including_degenerate_ones_survive_verbatim() {
        for (min, max, why) in [
            ([-1.0, -2.0, -3.0], [4.0, 5.0, 6.0], "an ordinary box"),
            ([0.0; 3], [0.0; 3], "a point at the origin"),
            ([-1.0, 0.0, -1.0], [1.0, 0.0, 1.0], "a flat ground plane"),
            (
                [-MESH_BOUNDS_UNKNOWN_COORD; 3],
                [MESH_BOUNDS_UNKNOWN_COORD; 3],
                "a box AT the sentinel magnitude but correctly ordered",
            ),
        ] {
            let row = MeshLocalBounds::new(min, max);
            assert_eq!((row.min, row.max), (min, max), "{why} must survive verbatim");
            assert!(!row.is_unknown(), "{why} must NOT read as bounds-unknown");
        }
    }
}

/// Decision 9's triangle-count normalizer (host mirror): every mesh draws a plain
/// non-restart triangle list, so `tri_count = index_count / 3` exactly — the SAME
/// formula `vb_geom_fetch.hlsli` computes device-side as
/// `gMeshMeta[mesh_id].tri_count` (R8). Exposed here so host code (and this rung's
/// tests) can compute it without a live GPU meta buffer. The
/// `debug_assert!(tri_count > 0)` guard against a GPU-undefined `% 0` belongs at the
/// FETCH side (R8's shader); this fn only exposes the value.
#[inline]
pub const fn tri_count(index_count: u32) -> u32 {
    index_count / 3
}

/// The index buffer's element width in bytes for a given [`IndexType`] — the
/// `gMeshMeta[].index_width` encoding (Decision 0's geometry-table pin).
#[inline]
pub const fn index_width_bytes(index_type: IndexType) -> u32 {
    match index_type {
        IndexType::Uint16 => 2,
        IndexType::Uint32 => 4,
    }
}

/// The `STORAGE_BUFFER` usage bit a [`MeshGpu`](crate::mesh::MeshGpu)'s vertex/index
/// buffers additionally need to serve as `gMeshVerts[]`/`gMeshIndices[]` entries
/// (Decision 0 / P2-b) — a pure fn of the boot-committed
/// `ResolvedRenderPath.vb_geometry_table` flag, so the usage-bit SELECTION is
/// unit-testable without a device. `false` ⇒ [`BufferUsage::NONE`] — today's exact
/// `VERTEX`/`INDEX`-only registration (byte-identical); `true` ⇒
/// [`BufferUsage::STORAGE`], OR'd into the buffer's usage alongside `VERTEX`/`INDEX`
/// (and, on an RT device, the HW-RT `ACCEL_BUILD_INPUT | SHADER_DEVICE_ADDRESS` bits —
/// see [`build_mesh_gpu`](crate::mesh_assets::build_mesh_gpu)).
#[inline]
pub const fn mesh_buffer_usage(vb_geometry_table: bool) -> BufferUsage {
    if vb_geometry_table { BufferUsage::STORAGE } else { BufferUsage::NONE }
}

/// Cold fallback for [`MeshGeometryTable::register`]'s allocator-exhaustion path
/// (practically unreachable at [`VulkanGeometryBindlessSet::capacity`]'s declared
/// size — mirrors [`crate::bindless`]'s own `exhausted_slot_fallback`).
#[cold]
fn exhausted_slot_fallback(capacity: u32) -> u32 {
    debug_assert!(false, "invariant: MeshGeometryTable exhausted its {capacity} slots");
    report_bindless_table_exhausted("MeshGeometryTable", capacity, VB_GEOMETRY_RESERVED_SLOT);
    VB_GEOMETRY_RESERVED_SLOT
}

/// A `Once` latch is PROCESS state, so it is a named module-level `static` rather than one
/// tucked inside the reporter: an observer must be able to reset it, or its green only means
/// "nothing else in this binary tripped this condition first". See `OnceSite::reset`.
pub(crate) static W2202_SITE: OnceSite = OnceSite::new();

/// Reports `boyko-W2202` for THIS table — the sibling of
/// [`crate::bindless`]'s function of the same name, deliberately not shared.
///
/// The two sites share a code and a message shape (the table's name is the argument that tells
/// them apart in a log) but each needs its **own** `static FIRED`, which is the whole per-SITE
/// argument: a single shared latch would let whichever table exhausted first silence the other's
/// only report. Two functions is what "per site" costs, and one `pub(crate)` helper taking a
/// `&OnceSite` would cost the same while reading as if the latch were shared.
#[cold]
#[inline(never)]
fn report_bindless_table_exhausted(table: &str, capacity: u32, fallback: u32) {
    if W2202_SITE.claim() {
        boyko_log::warn!(
            boyko_log::Render,
            W2202,
            "bindless table `{}` exhausted its {} slots -- aliasing reserved fallback slot {} \
             instead of writing out of range",
            table,
            capacity,
            fallback
        );
    }
}

/// The bindless per-mesh geometry table (Decision 0 / rung R-VBGEO): owns the VB-only
/// Set-3 device descriptor set, the `gMeshMeta[]` backing buffer, and the fence-gated
/// slot allocator. Constructed only on a VB boot that armed
/// `ResolvedRenderPath.vb_geometry_table` — see the module doc for that gate and for where
/// `mesh_id` (the slot this type hands out) is stored.
///
/// # Device-UAF safety — the SAME three structural guards as `BindlessTextureTable`
///
/// 1. **Bounds**: [`BindlessSlotAllocator`] only ever issues `1..capacity`; every write
///    is `debug_assert!`-checked in [`write_geometry_buffer_slot`].
/// 2. **Degenerate slot 0**: [`Self::new`] zero-inits the WHOLE `gMeshMeta[]` buffer
///    (every slot, including 0, starts as a zero-count row) before any real
///    registration — an unwritten/stale `mesh_id` reads a `tri_count == 0` row, never
///    UNDEFINED memory (the verts/indices arrays are `PARTIALLY_BOUND`, so an
///    unwritten descriptor is valid Vulkan as long as no shader dynamically indexes
///    it — the `tri_count > 0` fetch-side assert, R8, is what prevents that).
/// 3. **Fence-gated recycle**: [`Self::unregister`] does NOT return the slot to the
///    allocator immediately; [`Self::retire_ready_slots`] only recycles it once its
///    fence horizon has passed — see [`BindlessSlotAllocator`]'s docs.
///
/// # `gMeshBounds[]` — bound at `vb_cull_layout` @5 since rung R2d-2, still read by nothing
///
/// The table additionally owns a [`MeshLocalBounds`] row per slot ([`Self::bounds_buffer`]),
/// prefilled with the inverted "unknown" sentinel and written by [`Self::register`].
///
/// Rung R2d-1 shipped it bound to NOTHING. Rung R2d-2 binds it as one COMPUTE storage
/// buffer at `vb_cull_layout` @5 — identically in every frame-in-flight slot, because the
/// table is not per-FIF — and its presence became the conjunct that arms the whole cull:
/// `vb_cull_set` is `Some` and `batch_cull_armed` is true precisely when this buffer
/// exists, which is a VisibilityBuffer-resolved boot with the mesh leg and the
/// descriptor-indexing cap. Rung R2d-3's `vb_batch_cull.comp.hlsl` DECLARES @5 as
/// `StructuredBuffer<MeshLocalBounds> gMeshBounds` but loads nothing from it while the
/// per-instance `keep` predicate is hardwired, so the table is still read by no shader —
/// the arming rung is what turns the declaration into a load.
pub struct MeshGeometryTable {
    set: VulkanGeometryBindlessSet,
    meta_buffer: BoundBuffer,
    bounds_buffer: BoundBuffer,
    alloc: BindlessSlotAllocator,
}

impl NonSendResource for MeshGeometryTable {}

impl MeshGeometryTable {
    /// Builds the Set-3 descriptor set (layout, UPDATE_AFTER_BIND pool, set), the
    /// `gMeshMeta[]` host-visible-coherent backing buffer (zero-inited, then bound
    /// ONCE to binding 2), the `gMeshBounds[]` host-visible-coherent backing buffer
    /// (prefilled with [`MeshLocalBounds::UNKNOWN`], bound to nothing — see the type
    /// doc's R2d-1 section), and the free-list allocator.
    ///
    /// `debug_assert!`s the two device caps Decision 0 requires
    /// (`shaderStorageBufferArrayNonUniformIndexing` + `maxBoundDescriptorSets >= 4`)
    /// — both are ALREADY implied by the caller's own gate (`resolve_render_path`
    /// degrades `VisibilityBuffer` to `Deferred` when the first is absent; the second
    /// is the Vulkan-guaranteed floor), so this is belt-and-braces, not a fresh check.
    pub fn new(ctx: &VulkanContext) -> Result<Self, VulkanError> {
        debug_assert!(
            ctx.device_caps().storage_buffer_array_non_uniform_indexing_ok,
            "invariant: MeshGeometryTable requires shaderStorageBufferArrayNonUniformIndexing \
             (resolve_render_path degrades VisibilityBuffer to Deferred otherwise)"
        );
        debug_assert!(
            ctx.device_caps().max_bound_descriptor_sets >= 4,
            "invariant: the VB path's Set 3 needs maxBoundDescriptorSets >= 4 (the Vulkan floor)"
        );

        let set = create_geometry_bindless_set(ctx)?;
        let capacity = set.capacity();
        let meta_bytes = u64::from(capacity) * (MESH_GEOMETRY_META_BYTES as u64);
        let bounds_bytes = u64::from(capacity) * (MESH_LOCAL_BOUNDS_BYTES as u64);

        let meta_buffer = match ctx.create_buffer(&BufferDesc {
            size: meta_bytes,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        }) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: `set` was just created on `ctx`, owned exclusively here,
                // never bound to any in-flight submission; destroy it once on this edge.
                unsafe { destroy_geometry_bindless_set(ctx, set) };
                return Err(e);
            }
        };
        let Some(meta_ptr) = ctx.buffer_mapped_ptr(&meta_buffer) else {
            // SAFETY: `meta_buffer`/`set` were just created on `ctx`, owned exclusively
            // here, never submitted/bound; destroy each once on this edge.
            unsafe {
                ctx.destroy_buffer(meta_buffer);
                destroy_geometry_bindless_set(ctx, set);
            }
            return Err(VulkanError::Unsupported("mesh geometry meta buffer not host-mapped"));
        };
        // SAFETY: `meta_ptr` is the mapped first byte of a fresh `meta_bytes`-byte
        // host-coherent buffer (just created above, not yet bound to any descriptor
        // read); zero-filling it before the ONE binding-2 write below means every slot
        // (including the reserved slot 0) starts as an all-zero (zero-count) row —
        // guard #2 in this type's doc.
        unsafe {
            core::ptr::write_bytes(meta_ptr.as_ptr(), 0, meta_bytes as usize);
        }

        // Virtual-geometry ladder, rung R2d-1: the per-mesh local-AABB table. Same
        // construction idiom as `meta_buffer` above (host-visible-coherent, mapped for
        // the table's lifetime, one row per slot) — but it is NOT bound to any
        // descriptor here, by design (see this type's doc).
        let bounds_buffer = match ctx.create_buffer(&BufferDesc {
            size: bounds_bytes,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        }) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: `meta_buffer`/`set` were created above on `ctx`, are owned
                // exclusively here, and were never submitted (the binding-2 descriptor
                // write has not run yet); destroy each once on this edge.
                unsafe {
                    ctx.destroy_buffer(meta_buffer);
                    destroy_geometry_bindless_set(ctx, set);
                }
                return Err(e);
            }
        };
        let Some(bounds_ptr) = ctx.buffer_mapped_ptr(&bounds_buffer) else {
            // SAFETY: all three objects were created above on `ctx`, are owned
            // exclusively here, and were never submitted/bound; destroy each once, in
            // reverse creation order.
            unsafe {
                ctx.destroy_buffer(bounds_buffer);
                ctx.destroy_buffer(meta_buffer);
                destroy_geometry_bindless_set(ctx, set);
            }
            return Err(VulkanError::Unsupported("mesh geometry bounds buffer not host-mapped"));
        };
        // SAFETY: `bounds_ptr` is the mapped first byte of the buffer just created with
        // size `bounds_bytes == capacity * MESH_LOCAL_BOUNDS_BYTES`, so the slice covers
        // exactly the allocation — no row runs past the end. Alignment holds: the
        // pointer is `block map base + bind offset`, and both satisfy the buffer's
        // `VkMemoryRequirements::alignment` (>= 16 on every real device), while
        // `MeshLocalBounds` needs only 4. The reference is unique — the buffer was
        // created microseconds ago, is reachable from nowhere else, is bound to no
        // descriptor, and no submission exists that could read it — and the slice dies
        // at the end of this block.
        unsafe {
            let rows = core::slice::from_raw_parts_mut(
                bounds_ptr.as_ptr().cast::<MeshLocalBounds>(),
                capacity as usize,
            );
            prefill_bounds_unknown(rows);
        }

        // SAFETY: `meta_buffer.buffer` is a live `STORAGE_BUFFER`-usage buffer owned by
        // this table for its whole lifetime; binding 2 has no UPDATE_AFTER_BIND flag
        // (see `create_geometry_bindless_set`'s doc), so this MUST be — and is — the
        // ONLY write ever issued to it, before `set` is ever bound by any pipeline.
        unsafe {
            write_geometry_buffer_slot(
                ctx,
                &set,
                GEOMETRY_META_BINDING,
                0,
                meta_buffer.buffer,
                0,
                meta_bytes,
            );
        }


        Ok(Self { set, meta_buffer, bounds_buffer, alloc: BindlessSlotAllocator::new(capacity) })
    }

    /// The table's declared capacity.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.alloc.capacity()
    }

    /// The owned descriptor set. Bound at set index **2** by the VB compute passes
    /// (`boyko_rhi_vulkan::present::passes::vb` — `vb_resolve`/`vb_shade`), threaded to them as
    /// `GBufferScene::vb_geometry_set`; "Set-3" elsewhere in this module is the design-time name
    /// from Decision 0, not the shipped binding index.
    #[inline]
    pub fn set(&self) -> &VulkanGeometryBindlessSet {
        &self.set
    }

    /// The `gMeshBounds[]` backing buffer — one [`MeshLocalBounds`] row per slot,
    /// indexed by the same `mesh_id` as `gMeshMeta[]`.
    ///
    /// Since rung R2d-2 this is what `GBufferScene::vb_mesh_bounds` carries and what
    /// `vb_cull_set` binds at @5 (see the type doc); rung R2d-3's cull shader declares that
    /// binding but does not yet load from it. It is also what a test can inspect.
    #[inline]
    pub fn bounds_buffer(&self) -> &BoundBuffer {
        &self.bounds_buffer
    }

    /// Allocates a slot and registers `vertex_buffer`/`index_buffer` (a freshly built
    /// [`MeshGpu`](crate::mesh::MeshGpu)'s OWN buffers — Decision 0 preserves the
    /// deep "`MeshGpu` owns its buffers" invariant, no suballocated global buffer) as
    /// that slot's `gMeshVerts[]`/`gMeshIndices[]` entries, plus writes its
    /// `gMeshMeta[]` and `gMeshBounds[]` rows. Returns the slot index (the mesh's
    /// `mesh_id`).
    ///
    /// `local_min`/`local_max` are the mesh's MODEL-space AABB — exactly the fold
    /// [`build_mesh_gpu`](crate::mesh_assets::build_mesh_gpu) already performs and
    /// stores as [`MeshGpu::local_min`](crate::mesh::MeshGpu::local_min) /
    /// `local_max`, so no caller recomputes anything. A slot that is never registered
    /// keeps the [`MeshLocalBounds::UNKNOWN`] prefill.
    ///
    /// On allocator exhaustion this is an engine invariant violation
    /// (`debug_assert!`); the release-safe fallback `exhausted_slot_fallback` aliases
    /// [`VB_GEOMETRY_RESERVED_SLOT`] (the zero-count degenerate slot) rather than issue
    /// an out-of-range write.
    // Over clippy's arity threshold because a slot records four independent device facts
    // (verts, indices, meta, bounds), each already flat on `MeshGpu`. Bundling them into
    // a param struct would add a type whose only purpose is to be built at one call site
    // and destructured here, and would hide that `local_min`/`local_max` are plain copies
    // of fields the caller already holds.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &mut self,
        ctx: &VulkanContext,
        vertex_buffer: &BoundBuffer,
        vertex_count: u32,
        index_buffer: &BoundBuffer,
        index_count: u32,
        index_type: IndexType,
        local_min: [f32; 3],
        local_max: [f32; 3],
    ) -> u32 {
        let slot = self.alloc.register().unwrap_or_else(|| exhausted_slot_fallback(self.alloc.capacity()));
        if slot != VB_GEOMETRY_RESERVED_SLOT {
            // SAFETY: `slot < self.set.capacity()` (the allocator only ever issues
            // `1..capacity`); `vertex_buffer`/`index_buffer` are the caller's contract
            // (this fn's own doc: live `STORAGE_BUFFER`-usage buffers that outlive
            // every submission sampling this slot until the matching `unregister`);
            // this is a freshly-allocated slot with no prior in-flight reference — the
            // fence-gated recycle only applies to a REUSED slot.
            // Root cause (rung R8 GPU debug, round 6): `vertex_buffer.offset`/
            // `index_buffer.offset` is `HostVisibleBlock::create_bound_buffer`'s
            // SUB-ALLOCATION offset — where THIS buffer's own, freshly-`vkCreateBuffer`'d
            // `VkBuffer` is `vkBindBufferMemory`'d within the SHARED `VkDeviceMemory` block
            // (confirmed by reading `memory.rs`: EVERY `create_bound_buffer` call mints a
            // DISTINCT `VkBuffer` object with its OWN buffer-relative addressing starting at
            // 0 — there is no single "pool `VkBuffer`" multiple meshes share). A
            // `VkDescriptorBufferInfo.offset` is BYTES FROM THE START OF THAT `VkBuffer`
            // (buffer-relative), NOT a memory-bind offset — passing the ~1 MB sub-allocation
            // offset here asked the descriptor to start reading 1 MB into a buffer that is
            // only `vertex_buffer.size` (76 KB for this mesh) bytes long, hundreds of KB out
            // of bounds. Without `robustBufferAccess` (not enabled) and with validation off
            // on this box, that surfaced as a silent zero read, not a crash or a VUID — DXC's
            // `NonUniformResourceIndex`, the write mechanics, the layout, and the pipeline
            // were all sound the whole time (confirmed by the round-4/5 construction-time
            // slot-0/2 diagnostic writes, which happened to pass a literal `0` and therefore
            // worked). `create_bind_group`'s OWN `StorageBuffer`/`UniformBuffer` arm
            // (`rhi_impl/device.rs`) already hardcodes `offset: 0` for the exact same
            // reason — every `BoundBuffer` is its own dedicated `VkBuffer`, so `0` is always
            // "the whole buffer" regardless of where it lives in the shared block. Match that
            // established, engine-wide convention here.
            unsafe {
                write_geometry_buffer_slot(
                    ctx,
                    &self.set,
                    GEOMETRY_VERTS_BINDING,
                    slot,
                    vertex_buffer.buffer,
                    0,
                    vertex_buffer.size,
                );
                write_geometry_buffer_slot(
                    ctx,
                    &self.set,
                    GEOMETRY_INDICES_BINDING,
                    slot,
                    index_buffer.buffer,
                    0,
                    index_buffer.size,
                );
            }
            let meta = MeshGeometryMeta {
                index_width: index_width_bytes(index_type),
                vertex_count,
                index_count,
                _pad: 0,
            };
            // SAFETY: `ctx.buffer_mapped_ptr(&self.meta_buffer)` is `Some` (checked once
            // at `new`, never re-mapped/unmapped for this table's lifetime); `slot <
            // capacity` (checked above), so `slot * MESH_GEOMETRY_META_BYTES` lands a
            // whole row inside the buffer's `capacity * MESH_GEOMETRY_META_BYTES`-byte
            // extent; `meta` is a plain POD value written by-value (no drop to run).
            unsafe {
                let row = ctx
                    .buffer_mapped_ptr(&self.meta_buffer)
                    .expect("invariant: meta buffer stays host-mapped for the table's lifetime")
                    .as_ptr()
                    .add(slot as usize * MESH_GEOMETRY_META_BYTES)
                    .cast::<MeshGeometryMeta>();
                core::ptr::write(row, meta);
            }
            let bounds = MeshLocalBounds::new(local_min, local_max);
            debug_assert!(
                slot < self.alloc.capacity(),
                "invariant: the slot allocator issues only 1..capacity, so a row write \
                 cannot leave the bounds buffer's extent"
            );
            // SAFETY: `ctx.buffer_mapped_ptr(&self.bounds_buffer)` is `Some` (checked
            // once at `new`, never re-mapped/unmapped for this table's lifetime).
            // `slot < capacity` is STRUCTURAL, not checked at this site: the allocator
            // seeds its free list with `(1..capacity).rev()` and only ever pops from it
            // (`bindless.rs:87`, `:106-108`), so every slot it hands out is in range.
            // Therefore `slot * MESH_LOCAL_BOUNDS_BYTES` lands a whole row inside the
            // buffer's `capacity * MESH_LOCAL_BOUNDS_BYTES`-byte extent.
            // Alignment: the mapped base is `vkMapMemory` over the whole block
            // (`memory.rs:186-204`) offset by a sub-allocation offset aligned to
            // `VkMemoryRequirements::alignment` (`memory.rs:270-282`), which Vulkan
            // requires to be a power of two and at least 4 for a storage buffer, and
            // `slot * 32` preserves any alignment >= 4 — which is all `MeshLocalBounds`
            // needs. `bounds` is a plain POD value written by-value (no drop to run, and
            // the row it overwrites — the sentinel or a previous registration — has none
            // either).
            unsafe {
                let row = ctx
                    .buffer_mapped_ptr(&self.bounds_buffer)
                    .expect("invariant: bounds buffer stays host-mapped for the table's lifetime")
                    .as_ptr()
                    .add(slot as usize * MESH_LOCAL_BOUNDS_BYTES)
                    .cast::<MeshLocalBounds>();
                core::ptr::write(row, bounds);
            }
        }
        slot
    }

    /// Stages `slot` for return to the free list once `retire_frame` has passed
    /// (mirrors [`BindlessSlotAllocator::free`]'s contract exactly —
    /// `retire_frame` MUST be `submission_epoch_at_free + RETIRE_DELAY`).
    ///
    /// # The row is NOT restored to [`MeshLocalBounds::UNKNOWN`], deliberately
    ///
    /// Between retirement and the slot's recycle-and-rewrite, `gMeshBounds[slot]` still
    /// holds the dead mesh's real box — the SAME staleness the meta row and the
    /// vertex/index descriptors already carry, and for the same reason
    /// (`bindless.rs:272-277`: the old write is left in place because nothing may read
    /// the slot in that window). Reading it would require an instance whose `mesh_id`
    /// names a retired mesh, which is a use-after-free of the mesh itself and not a
    /// hazard this row introduces. Restoring it here is not possible without threading a
    /// `VulkanContext` through a `&mut self` bookkeeping call, and would leave the meta
    /// row stale beside a fresh bounds row — a worse invariant than the uniform one.
    /// Recorded rather than left for the next rung's author to rediscover.
    #[inline]
    pub fn unregister(&mut self, slot: u32, retire_frame: u64) {
        self.alloc.free(slot, retire_frame);
    }

    /// Drains every slot whose fence horizon has passed back to the free list.
    #[inline]
    pub fn retire_ready_slots(&mut self, epoch: u64) {
        self.alloc.retire_ready_slots(epoch);
    }

    /// `true` iff no slot is awaiting its fence horizon.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.alloc.is_empty()
    }

    /// Tears down the `gMeshBounds[]` and `gMeshMeta[]` buffers, then the Set-3 device
    /// objects — reverse creation order, the same discipline the error edges in
    /// [`Self::new`] follow. Waits for the device to go idle first (mirrors
    /// [`BindlessTextureTable::destroy`](crate::bindless::BindlessTextureTable::destroy)).
    pub fn destroy(self, ctx: &VulkanContext) {
        let _ = ctx.wait_idle();
        // SAFETY: the device was just drained (`wait_idle` above), so no submission
        // references `self.bounds_buffer`, `self.meta_buffer` or `self.set`; each is
        // owned exclusively here and moved by value ⇒ destroyed exactly once.
        unsafe {
            ctx.destroy_buffer(self.bounds_buffer);
            ctx.destroy_buffer(self.meta_buffer);
            destroy_geometry_bindless_set(ctx, self.set);
        }
    }
}

/// Always-present `NonSendResource` wrapper around an optional live
/// [`MeshGeometryTable`] (Rev-5 streaming invariant): `None` when
/// `ResolvedRenderPath.vb_geometry_table` is `false` (every non-VB boot, or a VB boot whose
/// device lacks the descriptor-indexing prerequisite), `Some` when the boot seam
/// (`boyko_app::runner`, right after `resolve_render_path`, before `app.finish()` / the
/// `upload_mesh_assets` boot drain) constructs and arms it — which `VB_IMPLEMENTED == true`
/// (rung R8) makes a genuinely reachable outcome.
///
/// Threaded as [`MeshGpu`](crate::mesh::MeshGpu)'s
/// [`GpuUpload::Aux`](crate::gpu_upload::GpuUpload::Aux) so the STREAMED mesh-upload
/// drain ([`upload_mesh_assets`](crate::gpu_upload::upload_mesh_assets)) can claim a
/// geometry-table slot for every runtime-loaded mesh once armed — mirrors
/// `TextureGpu::Aux = BindlessTextureTable` one level down. `Option`-wrapped here
/// (unlike the always-constructed texture table) because the table itself is not even
/// built when the flag is `false` (P2-b's "zero-cost leg toggle" — `Option` costs
/// nothing when `None`).
#[derive(Default)]
pub struct MeshGeometryTableSlot(pub Option<MeshGeometryTable>);

impl NonSendResource for MeshGeometryTableSlot {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tri_count_normalizes_index_count_by_three() {
        assert_eq!(tri_count(0), 0);
        assert_eq!(tri_count(3), 1);
        assert_eq!(tri_count(36), 12);
        assert_eq!(tri_count(9), 3);
        // Not a multiple of 3 is impossible for a real triangle-list index buffer, but
        // the normalizer itself is a plain integer division with no special-case.
        assert_eq!(tri_count(10), 3);
    }

    #[test]
    fn index_width_bytes_matches_index_type() {
        assert_eq!(index_width_bytes(IndexType::Uint16), 2);
        assert_eq!(index_width_bytes(IndexType::Uint32), 4);
    }

    #[test]
    fn mesh_buffer_usage_is_none_when_not_armed_and_storage_when_armed() {
        assert_eq!(mesh_buffer_usage(false), BufferUsage::NONE);
        assert_eq!(mesh_buffer_usage(true), BufferUsage::STORAGE);
    }

    #[test]
    fn mesh_geometry_meta_default_is_the_zero_count_degenerate_row() {
        // Rung R-VBGEO task 1 (slot 0's degenerate contents): a default-constructed
        // (or zero-inited) row reads as "zero triangles", never a stray non-zero
        // count that could pass a naive `tri_count > 0` check.
        let meta = MeshGeometryMeta::default();
        assert_eq!(meta.index_width, 0);
        assert_eq!(meta.vertex_count, 0);
        assert_eq!(meta.index_count, 0);
        assert_eq!(tri_count(meta.index_count), 0);
    }

    /// The R2d-1 prefill contract, on the PATTERN rather than on a device buffer:
    /// [`MeshGeometryTable::new`] needs a live `VulkanContext` to allocate the mapped
    /// `gMeshBounds[]`, so the fill is factored into the pure
    /// [`prefill_bounds_unknown`] and pinned here — the device path is that exact fn
    /// applied to a slice over the mapped allocation, so what this test proves is what
    /// the buffer holds.
    ///
    /// Every slot, INCLUDING the reserved slot 0, must read as the inverted sentinel:
    /// a zeroed slot 0 would be indistinguishable from a legitimate point-sized mesh at
    /// the origin.
    #[test]
    fn prefill_marks_every_slot_including_reserved_zero_as_unknown() {
        // Seeded with the ambiguous all-zero row precisely so a no-op fill fails.
        let mut rows = vec![MeshLocalBounds::new([0.0; 3], [0.0; 3]); 16];
        prefill_bounds_unknown(&mut rows);

        for (slot, row) in rows.iter().enumerate() {
            assert_eq!(row.min, [MESH_BOUNDS_UNKNOWN_COORD; 3], "slot {slot} min");
            assert_eq!(row.max, [-MESH_BOUNDS_UNKNOWN_COORD; 3], "slot {slot} max");
            assert!(row.is_unknown(), "slot {slot} must read as bounds-unknown");
            assert_eq!(*row, MeshLocalBounds::UNKNOWN, "slot {slot}");
        }
        assert!(
            rows[VB_GEOMETRY_RESERVED_SLOT as usize].is_unknown(),
            "the reserved slot 0 is prefilled too - an all-zero row there would read as a \
             legitimate point-sized mesh at the origin"
        );
    }

    /// The sentinel's two load-bearing properties: it is INVERTED on every axis (so
    /// `any(min > max)` detects it and no real fold can produce it), and it is FINITE
    /// (an infinity in a plane evaluation yields NaN, and NaN silently takes the other
    /// operand under `NMin`/`NMax` instead of propagating).
    #[test]
    fn unknown_sentinel_is_inverted_on_every_axis_and_finite() {
        let u = MeshLocalBounds::UNKNOWN;
        for axis in 0..3 {
            assert!(u.min[axis] > u.max[axis], "axis {axis} must be inverted");
            assert!(u.min[axis].is_finite(), "axis {axis} min must not be an infinity");
            assert!(u.max[axis].is_finite(), "axis {axis} max must not be an infinity");
        }
        assert!(u.is_unknown());
        // A real (even degenerate, point-sized) box is never mistaken for the sentinel.
        assert!(!MeshLocalBounds::new([0.0; 3], [0.0; 3]).is_unknown());
    }

    /// A registered row round-trips its min/max through the exact 32-byte stride the
    /// device buffer uses, and touches no neighbouring slot. `MeshGeometryTable::register`
    /// needs a device, so this exercises the same value construction + `slot * 32` row
    /// addressing over a host allocation of the identical `Pod` rows.
    #[test]
    fn registered_row_round_trips_min_max_at_its_own_stride() {
        const CAPACITY: usize = 8;
        const SLOT: usize = 3;
        let min = [-1.5_f32, 0.25, 2.0];
        let max = [3.5_f32, 1.75, 9.0];

        let mut rows = vec![MeshLocalBounds::new([0.0; 3], [0.0; 3]); CAPACITY];
        prefill_bounds_unknown(&mut rows);
        rows[SLOT] = MeshLocalBounds::new(min, max);

        // Read the row back out of the raw bytes at `SLOT * MESH_LOCAL_BOUNDS_BYTES` —
        // the same byte arithmetic `register` performs on the mapped pointer.
        let raw: &[u8] = bytemuck::cast_slice(&rows[..]);
        assert_eq!(raw.len(), CAPACITY * MESH_LOCAL_BOUNDS_BYTES);
        let row_bytes = &raw[SLOT * MESH_LOCAL_BOUNDS_BYTES..][..MESH_LOCAL_BOUNDS_BYTES];
        let back: &MeshLocalBounds = bytemuck::from_bytes(row_bytes);

        assert_eq!(back.min, min);
        assert_eq!(back.max, max);
        assert_eq!(back._p0, 0, "the lane padding must stay zero");
        assert_eq!(back._p1, 0, "the row padding must stay zero");
        assert!(!back.is_unknown(), "a registered row is never 'bounds unknown'");

        for (slot, row) in rows.iter().enumerate() {
            if slot != SLOT {
                assert!(row.is_unknown(), "slot {slot} must be untouched by the write");
            }
        }
    }

    /// Regression guard for the geometry table's OWN reserved-slot-0 + free-list
    /// churn behavior — the allocator logic itself is
    /// [`BindlessSlotAllocator`]'s (already exhaustively tested in `crate::bindless`);
    /// this pins that [`MeshGeometryTable::register`]/[`MeshGeometryTable::unregister`]
    /// delegate to it verbatim, F6/F7-style, at a capacity representative of this
    /// table's own consumer (register → free → retire → recycle, never double-issue
    /// while live, never issue the reserved slot).
    #[test]
    fn slot_alloc_recycle_under_churn_never_double_issues_or_returns_reserved() {
        let mut alloc = BindlessSlotAllocator::new(64);
        let mut live: Vec<u32> = Vec::new();
        let mut epoch: u64 = 0;

        for round in 0..200u32 {
            match round % 3 {
                0 => {
                    if let Some(slot) = alloc.register() {
                        assert_ne!(slot, VB_GEOMETRY_RESERVED_SLOT, "round {round}");
                        assert!(!live.contains(&slot), "round {round}: double-issued {slot}");
                        live.push(slot);
                    }
                }
                1 => {
                    if let Some(slot) = live.pop() {
                        alloc.free(slot, epoch + 2);
                    }
                }
                _ => {
                    epoch += 1;
                    alloc.retire_ready_slots(epoch);
                }
            }
        }
    }
}

#[cfg(test)]
mod l8a_w2202 {
    use super::*;
    use boyko_log::probe::{watch, watched};

    use crate::log_probe::arm;

    #[test]
    fn w2202s_second_site_has_its_own_latch() {
        // The point of a per-SITE `Once`: this site's first report must survive even if the
        // `bindless.rs` site has already spent its own latch in the same process. Both live in
        // this crate's `--lib` test binary, so a code-scoped latch would make exactly one of the
        // two tests fail depending on the order the harness picked.
        arm();
        // Resets THIS module's latch and not `crate::bindless`'s -- which is the claim: the two
        // sites share code `boyko-W2202` and do not share a latch, so the sibling having already
        // fired (it has, in its own test) cannot silence this one.
        W2202_SITE.reset();

        watch(b'W', W2202.number());
        report_bindless_table_exhausted("MeshGeometryTable", 1024, VB_GEOMETRY_RESERVED_SLOT);
        assert_eq!(watched(), 1, "this site reports even after the sibling has fired");

        watch(b'W', W2202.number());
        report_bindless_table_exhausted("MeshGeometryTable", 1024, VB_GEOMETRY_RESERVED_SLOT);
        assert_eq!(watched(), 0, "and its own latch then holds");
    }
}
