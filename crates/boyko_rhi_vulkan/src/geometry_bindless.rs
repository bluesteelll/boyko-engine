//! Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / P2-c) — the
//! `VisibilityBuffer` path's own descriptor Set 3: two bindless `STORAGE_BUFFER`
//! runtime arrays (`gMeshVerts[]` / `gMeshIndices[]`, one slot per registered mesh,
//! each entry pointing at that mesh's OWN [`MeshGpu`](crate) vertex/index buffer —
//! see `boyko_render::mesh::MeshGpu`) plus one plain (non-bindless)
//! `STORAGE_BUFFER` binding for `gMeshMeta[]` (a single growable SSBO of
//! `{index_width, vertex_count, index_count}` rows, indexed BY CONTENT — the same
//! shape [`MaterialTable`](crate) uses for `MaterialGpu[]`, not a bindless array of
//! separate buffers).
//!
//! # Sibling of [`crate::bindless`], not a reuse of [`crate::bindless::VulkanBindlessSet`]
//!
//! [`crate::bindless::VulkanBindlessSet`] is hard-shaped for the TEXTURE table (binding
//! 0 = `SAMPLED_IMAGE` array, binding 1 = an IMMUTABLE shared sampler) — the wrong
//! descriptor type and binding count for a `STORAGE_BUFFER` geometry array, so this
//! module is a NEW, parallel type rather than a generic reuse. [`BindlessSlotAllocator`](crate)
//! (the free-list + fence-gated recycle policy, `boyko_render::bindless`) IS reused
//! verbatim by the render-side [`MeshGeometryTable`](crate) — only the device-facing
//! descriptor-set SHAPE differs here.
//!
//! # Only ONE binding may be `VARIABLE_DESCRIPTOR_COUNT` per layout (a hard Vulkan rule)
//!
//! `VK_DESCRIPTOR_BINDING_VARIABLE_DESCRIPTOR_COUNT_BIT` may be set on at most the
//! LAST (highest-numbered) binding of a layout — so a 3-binding layout cannot make
//! BOTH the verts array (binding 0) AND the indices array (binding 1) variable-count.
//! [`crate::bindless::VulkanBindlessSet`] sidesteps this (it has exactly one array
//! binding); this module sidesteps it differently: `descriptorCount` is a FIXED
//! [`MESH_GEOMETRY_TABLE_CAPACITY`](crate::geometry_bindless::MESH_GEOMETRY_TABLE_CAPACITY) on every binding (mirroring the texture table's own
//! documented practice of "this engine always allocates the full capacity" — the
//! `VARIABLE_DESCRIPTOR_COUNT` flag it sets is unexploited belt-and-braces there too), so
//! no binding needs the variable-count flag at all; `PARTIALLY_BOUND` alone lets an
//! unwritten slot exist without being a valid-descriptor requirement, as long as no
//! shader invocation dynamically indexes it.
//!
//! # Device-create prerequisite (closed at rung R8)
//!
//! Building this layout requires the device to have ENABLED
//! `shaderStorageBufferArrayNonUniformIndexing` +
//! `descriptorBindingStorageBufferUpdateAfterBind`. `crate::device::create_device` now
//! requests BOTH, gated on `enable_vb_geometry_table` — which the caller sets only after
//! `query_device_caps` confirmed the CONJUNCTION of the two
//! (`DeviceCaps::storage_buffer_array_non_uniform_indexing_ok`), since requesting an
//! unsupported feature bit fails `vkCreateDevice` outright. A device that lacks either bit
//! therefore boots normally and degrades `VisibilityBuffer` to `Deferred` at resolve time
//! (`RenderPathDegrade::VbDeviceCapMissing`) rather than failing device create.

use core::ffi::c_void;
use core::ptr;

use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::ffi::*;

/// The geometry table's declared per-array capacity (one slot per registered mesh;
/// slot 0 reserved — see `boyko_render::mesh_geometry_table`). Vulkan 1.2 core requires
/// `maxPerStageDescriptorUpdateAfterBindStorageBuffers` AND
/// `maxDescriptorSetUpdateAfterBindStorageBuffers` `>= 500,000` for any device
/// advertising descriptor indexing at all (the same required-limits table
/// [`crate::bindless::BINDLESS_TEXTURE_CAPACITY`]'s doc relies on) — 4096 mirrors that
/// same conservative-headroom precedent (generous over any real mesh budget; slot 0
/// reserved leaves 4095 real slots).
pub const MESH_GEOMETRY_TABLE_CAPACITY: u32 = 4096;

/// Binding index of `ByteAddressBuffer gMeshVerts[]` (Set 3).
pub const GEOMETRY_VERTS_BINDING: u32 = 0;
/// Binding index of `ByteAddressBuffer gMeshIndices[]` (Set 3).
pub const GEOMETRY_INDICES_BINDING: u32 = 1;
/// Binding index of the plain `gMeshMeta[]` SSBO (Set 3) — NOT bindless (a single
/// buffer, `descriptorCount == 1`, indexed by content inside the shader, not by
/// `dstArrayElement`).
pub const GEOMETRY_META_BINDING: u32 = 2;

/// The owned VB-only Set-3 descriptor set: its dedicated 3-binding layout, its
/// UPDATE_AFTER_BIND pool, and the allocated set.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive when this set is written
/// ([`write_geometry_buffer_slot`]), bound, or destroyed
/// ([`destroy_geometry_bindless_set`]): each goes through the context's device
/// fn-table. No compile-time `'ctx` tie this phase (mirrors
/// [`crate::bindless::VulkanBindlessSet`]).
pub struct VulkanGeometryBindlessSet {
    pub(crate) set_layout: VkDescriptorSetLayout,
    pub(crate) pool: VkDescriptorPool,
    pub(crate) set: VkDescriptorSet,
    capacity: u32,
}

impl VulkanGeometryBindlessSet {
    /// The runtime arrays' declared capacity (bindings 0/1's `descriptorCount`) —
    /// every valid slot satisfies `slot < capacity()`.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// The raw `VkDescriptorSet` — bound at set index **2** by the VB compute passes
    /// (`crate::present::passes::vb` — `vb_resolve`/`vb_shade`), live since rung R8. The
    /// "Set 3" naming elsewhere in this module is Decision 0's design-time name, not the
    /// shipped binding index.
    #[inline]
    pub fn set(&self) -> VkDescriptorSet {
        self.set
    }

    /// The raw `VkDescriptorSetLayout` — passed to
    /// [`VulkanContext::create_compute_pipeline_vb`](crate::device::VulkanContext) as the
    /// LAST of its three set layouts (Set 0 = `vb_layout0`, Set 1 = `forward_layout1`,
    /// Set 2 = this).
    #[inline]
    pub fn set_layout(&self) -> VkDescriptorSetLayout {
        self.set_layout
    }
}

/// Creates the VB-only Set-3 geometry descriptor set (rung R-VBGEO, Decision 0 /
/// P2-c): a 3-binding UPDATE_AFTER_BIND-pool layout (binding 0 = `gMeshVerts[]`,
/// binding 1 = `gMeshIndices[]`, both `STORAGE_BUFFER` runtime arrays sized
/// [`MESH_GEOMETRY_TABLE_CAPACITY`]; binding 2 = the plain `gMeshMeta[]`
/// `STORAGE_BUFFER`, `descriptorCount == 1`), an UPDATE_AFTER_BIND-flagged pool sized
/// for exactly one set, and the set itself.
///
/// No slot is written here — every verts/indices slot (including the reserved slot 0)
/// starts as an uninitialized `STORAGE_BUFFER` descriptor (`PARTIALLY_BOUND` makes
/// this valid as long as no shader invocation dynamically indexes an unwritten slot);
/// binding 2 (meta) is likewise unwritten — the caller
/// (`boyko_render::mesh_geometry_table::MeshGeometryTable::new`) creates the meta
/// buffer and writes it ONCE via [`write_geometry_buffer_slot`] immediately after. On
/// any partial failure every object created so far is torn down before the error
/// returns (no leak).
pub fn create_geometry_bindless_set(
    ctx: &VulkanContext,
) -> Result<VulkanGeometryBindlessSet, VulkanError> {
    let device = ctx.device();
    let fns = ctx.device_fns();
    let capacity = MESH_GEOMETRY_TABLE_CAPACITY;

    let bindings = [
        VkDescriptorSetLayoutBinding {
            binding: GEOMETRY_VERTS_BINDING,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: capacity,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        },
        VkDescriptorSetLayoutBinding {
            binding: GEOMETRY_INDICES_BINDING,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: capacity,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        },
        VkDescriptorSetLayoutBinding {
            binding: GEOMETRY_META_BINDING,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        },
    ];
    // Bindings 0/1 (the bindless arrays) get `PARTIALLY_BOUND | UPDATE_AFTER_BIND` — no
    // `VARIABLE_DESCRIPTOR_COUNT` (see the module doc: only the LAST binding of a layout
    // may carry it, and this engine always allocates the full declared capacity anyway,
    // exactly like `crate::bindless::create_bindless_texture_set` already does in
    // practice). Binding 2 (meta) is a single fixed buffer, written once at construction
    // — no special flags needed.
    let binding_flags: [VkFlags; 3] = [
        VK_DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT | VK_DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT,
        VK_DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT | VK_DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT,
        0,
    ];
    let binding_flags_info = VkDescriptorSetLayoutBindingFlagsCreateInfo {
        s_type: VkStructureType::DescriptorSetLayoutBindingFlagsCreateInfo,
        p_next: ptr::null(),
        binding_count: binding_flags.len() as u32,
        p_binding_flags: binding_flags.as_ptr(),
    };
    let layout_info = VkDescriptorSetLayoutCreateInfo {
        s_type: VkStructureType::DescriptorSetLayoutCreateInfo,
        p_next: (&binding_flags_info as *const VkDescriptorSetLayoutBindingFlagsCreateInfo)
            .cast::<c_void>(),
        flags: VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT,
        binding_count: bindings.len() as u32,
        p_bindings: bindings.as_ptr(),
    };
    let mut set_layout = VkDescriptorSetLayout::NULL;
    // SAFETY: `device` is live; `layout_info` is fully initialized, its `p_bindings`
    // points at the live `bindings` array (3 entries, alive for this call) and its
    // `p_next` chains the live `binding_flags_info` local, whose `p_binding_flags`
    // points at the live `binding_flags` array (3 entries, same count as `bindings`);
    // `&mut set_layout` is a valid out-pointer; NULL allocator.
    let raw = unsafe {
        (fns.create_descriptor_set_layout)(device, &layout_info, ptr::null(), &mut set_layout)
    };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(VulkanError::Vk("vkCreateDescriptorSetLayout(geometry)", result));
    }

    let pool_sizes = [VkDescriptorPoolSize {
        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        descriptor_count: capacity + capacity + 1,
    }];
    let pool_info = VkDescriptorPoolCreateInfo {
        s_type: VkStructureType::DescriptorPoolCreateInfo,
        p_next: ptr::null(),
        flags: VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT,
        max_sets: 1,
        pool_size_count: pool_sizes.len() as u32,
        p_pool_sizes: pool_sizes.as_ptr(),
    };
    let mut pool = VkDescriptorPool::NULL;
    // SAFETY: `device` is live; `pool_info` is fully initialized referencing the live
    // `pool_sizes` array (1 entry, alive for this call); `&mut pool` is a valid
    // out-pointer; NULL allocator.
    let raw = unsafe { (fns.create_descriptor_pool)(device, &pool_info, ptr::null(), &mut pool) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        // SAFETY: `set_layout` was just created on `device`, owned exclusively here,
        // never bound to any command buffer; destroy it once on this edge.
        unsafe { (fns.destroy_descriptor_set_layout)(device, set_layout, ptr::null()) };
        return Err(VulkanError::Vk("vkCreateDescriptorPool(geometry)", result));
    }

    let alloc_info = VkDescriptorSetAllocateInfo {
        s_type: VkStructureType::DescriptorSetAllocateInfo,
        p_next: ptr::null(),
        descriptor_pool: pool,
        descriptor_set_count: 1,
        p_set_layouts: &set_layout,
    };
    let mut set = VkDescriptorSet::NULL;
    // SAFETY: `device` is live; `alloc_info` names the live `pool` + the live
    // `set_layout` local (both outlive the call); `&mut set` is a valid out-pointer
    // for the single set.
    let raw = unsafe { (fns.allocate_descriptor_sets)(device, &alloc_info, &mut set) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        // SAFETY: `pool` owns no live set yet (allocation failed); `set_layout` is
        // owned exclusively here; destroy each once, in reverse order, on this edge
        // (destroying the pool also frees any partially-allocated set).
        unsafe {
            (fns.destroy_descriptor_pool)(device, pool, ptr::null());
            (fns.destroy_descriptor_set_layout)(device, set_layout, ptr::null());
        }
        return Err(VulkanError::Vk("vkAllocateDescriptorSets(geometry)", result));
    }

    Ok(VulkanGeometryBindlessSet { set_layout, pool, set, capacity })
}

/// Destroys `s`, consuming it: the pool (which implicitly frees the allocated set),
/// then the layout.
///
/// # Safety
///
/// `ctx` must be the live context `s` was created on; no submission using `s.set` is
/// pending (fence-waited / `wait_idle`'d); `s` is destroyed exactly once (the by-value
/// move enforces this).
pub unsafe fn destroy_geometry_bindless_set(ctx: &VulkanContext, s: VulkanGeometryBindlessSet) {
    let device = ctx.device();
    let fns = ctx.device_fns();
    // SAFETY: per this fn's contract `device` is live and no submission references
    // `s`; each object is destroyed exactly once, in reverse creation order (pool →
    // layout).
    unsafe {
        (fns.destroy_descriptor_pool)(device, s.pool, ptr::null());
        (fns.destroy_descriptor_set_layout)(device, s.set_layout, ptr::null());
    }
}

/// Writes ONE `STORAGE_BUFFER` descriptor into `set` at `binding`/`dstArrayElement =
/// slot` — a single `vkUpdateDescriptorSets` write, valid even while OTHER slots of
/// the SAME live set are bound to an in-flight command buffer (the layout was created
/// with the UPDATE_AFTER_BIND pool/binding bits for bindings 0/1 — see
/// [`create_geometry_bindless_set`]; binding 2 has no such flag, so the caller must
/// only ever write it ONCE, before the set is first bound — mirrors
/// [`crate::bindless::write_bindless_texture`]'s shape for a `STORAGE_BUFFER` payload).
///
/// # Safety
///
/// The caller guarantees:
/// * `binding == GEOMETRY_META_BINDING` implies `slot == 0` (binding 2 has exactly one
///   descriptor) and this is the ONLY write ever issued to it; `binding` ∈
///   `{GEOMETRY_VERTS_BINDING, GEOMETRY_INDICES_BINDING}` implies `slot < set.capacity()`.
/// * `buffer` is a live `VkBuffer` (created with `STORAGE_BUFFER` usage) for as long as
///   any shader invocation may read this slot — including every in-flight frame at the
///   moment of the write; the caller's fence-gated slot-recycle discipline
///   (`BindlessSlotAllocator`, `boyko_render::bindless`) is what guarantees no
///   ALREADY-IN-FLIGHT shader invocation is still indexing a REUSED verts/indices slot
///   when this write targets it — a freshly-allocated (never-before-issued) slot has no
///   prior in-flight reference by construction.
/// * `ctx` is the live context `set` was created on.
pub unsafe fn write_geometry_buffer_slot(
    ctx: &VulkanContext,
    set: &VulkanGeometryBindlessSet,
    binding: u32,
    slot: u32,
    buffer: VkBuffer,
    offset: u64,
    range: u64,
) {
    debug_assert!(
        binding != GEOMETRY_META_BINDING || slot == 0,
        "invariant: the meta binding has exactly one descriptor"
    );
    debug_assert!(
        binding == GEOMETRY_META_BINDING || slot < set.capacity,
        "invariant: a verts/indices slot must be < capacity"
    );
    let buffer_info = VkDescriptorBufferInfo { buffer, offset, range };
    let write = VkWriteDescriptorSet {
        s_type: VkStructureType::WriteDescriptorSet,
        p_next: ptr::null(),
        dst_set: set.set,
        dst_binding: binding,
        dst_array_element: slot,
        descriptor_count: 1,
        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        p_image_info: ptr::null(),
        p_buffer_info: &buffer_info,
        p_texel_buffer_view: ptr::null(),
    };
    let fns = ctx.device_fns();
    // SAFETY: `ctx.device()`/`fns` are the live device + its command table (`ctx` is
    // live per the caller contract above); the write references the live `buffer_info`
    // local (alive for this call) naming the caller's live `buffer` at a
    // `binding`/`slot` pair checked above; bindings 0/1 were allocated from an
    // UPDATE_AFTER_BIND-flagged pool against an UPDATE_AFTER_BIND-flagged layout, so
    // writing them while other slots may be bound to in-flight work is valid; the
    // caller's fence-gated recycle discipline (this fn's `# Safety`) guarantees no
    // in-flight read of THIS slot races this specific write.
    unsafe { (fns.update_descriptor_sets)(ctx.device(), 1, &write, 0, ptr::null()) };
}
