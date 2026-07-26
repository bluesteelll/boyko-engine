//! `RhiDevice` implementation for `VulkanContext` — the resource-creation
//! surface (buffers, textures, samplers, bind-group layouts + bind groups,
//! shader modules, compute + graphics pipelines, fences, query pools, and the
//! `hwrt` acceleration structures). Split out of the parent `rhi_impl` module; a
//! pure structural move — the trait impl body is unchanged.

// `use super::*` surfaces the types + module-level consts/free-fns DEFINED in `rhi_impl`
// (mod.rs) and, through the parent's `use crate::ffi::*` glob, the FFI names. A glob import
// does NOT re-export the parent's NAMED `use` bindings, so the `boyko_rhi` / `crate::*`
// items the moved impl references are re-declared below, pruned to exactly what it uses.
use super::*;

use core::ffi::c_void;
use core::ptr::{self, NonNull};

use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BufferDesc, ComputePipelineDesc,
    DescriptorKind, GraphicsPipelineDesc, MemoryLocation, MipMode, QueryPoolDesc, RhiDevice,
    SamplerDesc, TextureDesc,
};

use crate::compute::ComputeError;
use crate::device::{DeviceFns, VulkanContext};
use crate::error::VulkanError;
use crate::memory::BoundBuffer;
use crate::texture::VulkanTexture;

impl RhiDevice<Vulkan> for VulkanContext {
    type Error = VulkanError;

    fn create_buffer(&self, desc: &BufferDesc) -> Result<BoundBuffer, VulkanError> {
        // Plan A2: a zero-size buffer is an invalid request — it would yield a
        // `VkDescriptorBufferInfo.range == 0`. Reject it loud in debug; there is no
        // silent 0→1 size divergence (the created size, the stored
        // `BoundBuffer.size`, and the later descriptor `range` are all `desc.size`).
        debug_assert!(desc.size > 0, "invariant: zero-size buffer");
        // The agnostic `BufferUsage` bits equal the Vulkan `VK_BUFFER_USAGE_*`
        // bits (plan D5), so the projection is an identity cast on the u32 family.
        let usage: VkFlags = desc.usage.bits();
        match desc.location {
            // The host-visible foundation block (plan Q1).
            MemoryLocation::HostVisibleCoherent => {
                let block = self.host_block()?;
                let bound = block.borrow_mut().create_bound_buffer(desc.size, usage)?;
                Ok(bound)
            }
            // The Phase-5 device-local (VRAM) block (plan D3/MF-8). Always add the
            // `TRANSFER_SRC | TRANSFER_DST` usage so the staging upload + the
            // test-only readback (`vkCmdCopyBuffer`) can name the buffer as either
            // copy endpoint regardless of the caller's declared usage. The result
            // carries `mapped == None` — never host-mappable.
            MemoryLocation::DeviceLocal => {
                let usage = usage
                    | VK_BUFFER_USAGE_TRANSFER_SRC_BIT
                    | VK_BUFFER_USAGE_TRANSFER_DST_BIT;
                let block = self.device_block()?;
                let bound = block.borrow_mut().create_bound_buffer(desc.size, usage)?;
                Ok(bound)
            }
        }
    }

    unsafe fn destroy_buffer(&self, buffer: BoundBuffer) {
        // Plan A3: if a `BoundBuffer` exists, it was sub-allocated from one of the
        // shared blocks, so that block MUST already be initialized. A silent
        // early-return on `Err` here would drop the owned `buffer` WITHOUT
        // destroying its `VkBuffer` / returning its sub-allocation — a leak. The
        // matching block's `*_block()` only fails on the first-ever allocation
        // (which already happened to mint `buffer`), so these `expect`s are
        // unreachable by construction. `mapped` discriminates the origin block: a
        // host-visible buffer carries `Some(ptr)`, a device-local one `None`.
        if buffer.mapped.is_some() {
            let block = self
                .host_block()
                .expect("invariant: host block initialized when a host BoundBuffer exists");
            // SAFETY: `buffer` was produced by `create_buffer(HostVisibleCoherent)`
            // on this device's shared host block, the GPU is no longer using it
            // (caller fence-waited per the trait contract), and the by-value move
            // destroys it exactly once. The block is borrowed `&mut`
            // single-threaded.
            unsafe { block.borrow_mut().destroy_bound_buffer(buffer) };
        } else {
            let block = self
                .device_block()
                .expect("invariant: device block initialized when a device BoundBuffer exists");
            // SAFETY: `buffer` was produced by `create_buffer(DeviceLocal)` on this
            // device's shared device-local block, the GPU is no longer using it
            // (caller fence-waited), and the by-value move destroys it exactly once.
            // The block is borrowed `&mut` single-threaded.
            unsafe { block.borrow_mut().destroy_bound_buffer(buffer) };
        }
    }

    fn buffer_mapped_ptr(&self, buffer: &BoundBuffer) -> Option<NonNull<u8>> {
        // A host-visible buffer carries its persistent map pointer in `mapped`; a
        // device-local buffer carries `None` (it is never mapped, plan D3/MF-8),
        // honoring the device.rs:91 "`None` if not host-mappable" contract.
        buffer.mapped
    }

    fn create_texture(&self, desc: &TextureDesc) -> Result<VulkanTexture, VulkanError> {
        // SAFETY: `self.device()`/`self.device_fns()` are the live device + its
        // command table; `self.memory_properties()` are this physical device's
        // properties; `VulkanTexture::create` upholds the rest of the FFI
        // invariants internally (documented per `unsafe` block there).
        unsafe {
            VulkanTexture::create(
                self.device(),
                self.device_fns(),
                self.memory_properties(),
                desc,
            )
        }
    }

    unsafe fn destroy_texture(&self, texture: VulkanTexture) {
        // SAFETY: `texture` was created on this device by `create_texture`; the GPU
        // is no longer using it (caller fence-waited / `wait_idle`'d per the trait
        // contract); the by-value move destroys it exactly once. `destroy` tears
        // down the view → image → dedicated memory in reverse order.
        unsafe { texture.destroy(self.device(), self.device_fns()) };
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> Result<VulkanSampler, VulkanError> {
        // Rung 5 / GUI P5b: a deterministic sampler. The agnostic `Filter`/
        // `AddressMode` discriminants equal the `VkFilter`/`VkSamplerAddressMode`
        // constants (`as_i32()` no-op lowering, asserted in `abi_guard.rs`); the
        // single address mode applies to all three axes. Anisotropy / mip-bias /
        // compare are disabled (no `samplerAnisotropy` feature is requested at
        // device creation, so anisotropy MUST be FALSE).
        let address = desc.address_mode.as_i32();
        // GUI P5b Decision T4-D: map the agnostic `MipMode` to the Vulkan mip state.
        // `None` pins NEAREST mip mode + `minLod == maxLod == 0.0` (no mipmapping),
        // so a sampled read always reads the base level — the MSDF-atlas requirement
        // (a mipped read corrupts the per-channel median). It is the only variant in
        // P5b; the `match` makes the no-mip guarantee DECLARED, not accidental.
        let (mipmap_mode, min_lod, max_lod) = match desc.mip {
            MipMode::None => (VK_SAMPLER_MIPMAP_MODE_NEAREST, 0.0, 0.0),
        };
        // CSM Increment 0: lower the optional hardware depth-comparison op. `None`
        // keeps `compareEnable = VK_FALSE` + `compareOp = VK_COMPARE_OP_NEVER` —
        // byte-identical to every existing sampler. `Some(op)` builds a COMPARISON
        // sampler (`compareEnable = VK_TRUE`, `compareOp = op`) so a shadow-map PCF
        // read returns the filtered pass/fail of `reference (op) stored_depth`. The
        // agnostic `CompareOp` discriminant equals the `VkCompareOp` constant (asserted
        // in `abi_guard.rs`), so the lowering is an `as_i32()` no-op.
        let (compare_enable, compare_op) = match desc.compare {
            None => (VK_FALSE, VK_COMPARE_OP_NEVER),
            Some(op) => (VK_TRUE, op.as_i32()),
        };
        let info = VkSamplerCreateInfo {
            s_type: VkStructureType::SamplerCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            mag_filter: desc.mag_filter.as_i32(),
            min_filter: desc.min_filter.as_i32(),
            mipmap_mode,
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mip_lod_bias: 0.0,
            anisotropy_enable: VK_FALSE,
            max_anisotropy: 1.0,
            compare_enable,
            compare_op,
            min_lod,
            max_lod,
            border_color: VK_BORDER_COLOR_FLOAT_OPAQUE_BLACK,
            unnormalized_coordinates: VK_FALSE,
        };
        let mut sampler = VkSampler::NULL;
        // SAFETY: `device` is live; `info` is a fully-initialized `#[repr(C)]`
        // `VkSamplerCreateInfo` (null `p_next`, no GPU memory backing a sampler);
        // `&mut sampler` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_sampler)(self.device(), &info, ptr::null(), &mut sampler)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateSampler", result));
        }
        Ok(VulkanSampler { sampler })
    }

    unsafe fn destroy_sampler(&self, sampler: VulkanSampler) {
        // SAFETY: `sampler.sampler` was created on this device by `create_sampler`,
        // the GPU is no longer using it (caller fence-waited / `wait_idle`'d per the
        // trait contract), and the by-value move destroys it exactly once.
        unsafe {
            (self.device_fns().destroy_sampler)(self.device(), sampler.sampler, ptr::null())
        };
    }

    fn create_bind_group_layout(
        &self,
        desc: &BindGroupLayoutDesc,
    ) -> Result<VulkanBindGroupLayout, VulkanError> {
        // Render P1a: a heterogeneous set-0 layout from `desc.entries` — one
        // `VkDescriptorSetLayoutBinding` per entry, its `descriptor_type` the entry's
        // `DescriptorKind` cast `as i32` (the discriminants equal the
        // `VK_DESCRIPTOR_TYPE_*` constants — asserted in `abi_guard.rs`), its
        // `stage_flags` the entry's `ShaderStage` bits (identity cast, also asserted).
        // The bindings are a fixed-capacity inline array — zero heap allocation.
        let count = desc.entries.len();
        // 2026-07 audit: this used to `clamp(1, CAP)`, which turned an EMPTY entry slice into
        // `count == 1` and then panicked at `desc.entries[0]` below — from a safe `pub fn`. A
        // clamp cannot invent a slot that does not exist. An out-of-range count is a caller
        // error, rejected here before any Vulkan object is created (no leak, no panic).
        if !(1..=MAX_BIND_GROUP_BINDINGS).contains(&count) {
            return Err(VulkanError::Unsupported(
                "bind-group-layout entry count must be in 1..=MAX_BIND_GROUP_BINDINGS",
            ));
        }
        // Review M2: every declared binding must fit the inline-array capacity so the
        // retained `(binding, kind)` pairs (read at `create_bind_group` to target each
        // write) stay addressable. Debug-only; the contiguous-0..N convention every
        // call site uses trivially satisfies it.
        debug_assert!(
            desc.entries
                .iter()
                .take(count)
                .all(|e| (e.binding as usize) < MAX_BIND_GROUP_BINDINGS),
            "invariant: bind-group-layout binding must be < MAX_BIND_GROUP_BINDINGS"
        );
        // Retain the per-entry `(binding, kind)` pairs (review M1/M2) so
        // `create_bind_group` can cross-check the entry variant against the declared
        // kind and target each write at the layout's binding. POD copy, zero heap.
        let entries: [BindGroupLayoutBinding; MAX_BIND_GROUP_BINDINGS] =
            core::array::from_fn(|i| {
                if i < count {
                    BindGroupLayoutBinding {
                        binding: desc.entries[i].binding,
                        kind: desc.entries[i].kind,
                    }
                } else {
                    BindGroupLayoutBinding {
                        binding: i as u32,
                        kind: DescriptorKind::StorageBuffer,
                    }
                }
            });
        let bindings: [VkDescriptorSetLayoutBinding; MAX_BIND_GROUP_BINDINGS] =
            core::array::from_fn(|i| {
                if i < count {
                    let e = &desc.entries[i];
                    VkDescriptorSetLayoutBinding {
                        binding: e.binding,
                        descriptor_type: e.kind.as_i32(),
                        descriptor_count: e.count,
                        stage_flags: e.stage.bits(),
                        p_immutable_samplers: ptr::null(),
                    }
                } else {
                    VkDescriptorSetLayoutBinding {
                        binding: i as u32,
                        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                        descriptor_count: 0,
                        stage_flags: 0,
                        p_immutable_samplers: ptr::null(),
                    }
                }
            });
        let info = VkDescriptorSetLayoutCreateInfo {
            s_type: VkStructureType::DescriptorSetLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            binding_count: count as u32,
            p_bindings: bindings.as_ptr(),
        };
        let mut set_layout = VkDescriptorSetLayout::NULL;
        // SAFETY: `device` is live; `info` is fully initialized and its `p_bindings`
        // points to the first `count` (<= cap) entries of the live `bindings` inline
        // array (alive for the call), each a fully-initialized binding whose type +
        // stage come from `desc.entries[i]`; `&mut set_layout` is a valid out-pointer;
        // NULL allocator. `binding_count` bounds the driver's read to the initialized
        // prefix (the unused tail is never read).
        let raw = unsafe {
            (self.device_fns().create_descriptor_set_layout)(
                self.device(),
                &info,
                ptr::null(),
                &mut set_layout,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk(
                "vkCreateDescriptorSetLayout(bind group)",
                result,
            ));
        }
        Ok(VulkanBindGroupLayout {
            set_layout,
            entries,
            entry_count: count,
        })
    }

    unsafe fn destroy_bind_group_layout(&self, layout: VulkanBindGroupLayout) {
        // SAFETY: `layout.set_layout` was created on this device by
        // `create_bind_group_layout`, no bind group or pipeline referencing it is in
        // flight (caller contract), and the by-value move destroys it exactly once.
        unsafe {
            (self.device_fns().destroy_descriptor_set_layout)(
                self.device(),
                layout.set_layout,
                ptr::null(),
            )
        };
    }

    fn create_bind_group(
        &self,
        desc: &BindGroupDesc<Vulkan>,
    ) -> Result<VulkanBindGroup, VulkanError> {
        let device = self.device();
        let fns = self.device_fns();

        // Render P1a: one descriptor per `desc.entries` entry, written into the
        // layout's bindings in slice order. The count must equal the layout's entry
        // count and each entry's variant must match its layout entry's kind (caller
        // contract). The pool is sized per the per-kind histogram, the set is
        // allocated once, and `vkUpdateDescriptorSets` writes the whole set ONCE at
        // create — there is NO per-frame rewrite.
        let count = desc.entries.len();
        // 2026-07 audit: same clamp-then-panic bug as `create_bind_group_layout`, but WORSE
        // here — the panic on `desc.entries[i]` fires AFTER `vkCreateDescriptorPool` (below),
        // and a raw `VkDescriptorPool` has no RAII, so the unwind leaked it. Reject an
        // out-of-range count before the pool exists.
        if !(1..=MAX_BIND_GROUP_BINDINGS).contains(&count) {
            return Err(VulkanError::Unsupported(
                "bind-group entry count must be in 1..=MAX_BIND_GROUP_BINDINGS",
            ));
        }
        // Review M1: the group's arity must equal the layout's declared entry count —
        // one descriptor write per layout binding, no more, no fewer. (The doc on
        // `BindGroupDesc` promises this check; it is now real because the layout
        // retains its `entry_count`.) Debug-only; vanishes in release.
        debug_assert!(
            count == desc.layout.entry_count,
            "P1a: BindGroupDesc.entries.len() must equal the layout's entry count"
        );

        // --- Per-kind descriptor histogram → pool sizes (one entry per kind that
        //     actually appears, so the pool is sized exactly). The kinds map onto fixed
        //     histogram slots ([`KIND_COUNT`] of them); `pool_sizes` is a fixed inline
        //     array (zero heap). ---
        let mut hist = [0u32; KIND_COUNT];
        for entry in desc.entries.iter().take(count) {
            hist[descriptor_kind_slot(bind_group_entry_kind(entry))] += 1;
        }
        let mut pool_sizes = [VkDescriptorPoolSize {
            descriptor_type: 0,
            descriptor_count: 0,
        }; KIND_COUNT];
        let mut pool_size_count = 0usize;
        for (slot, &n) in hist.iter().enumerate() {
            if n > 0 {
                pool_sizes[pool_size_count] = VkDescriptorPoolSize {
                    descriptor_type: DESCRIPTOR_KIND_VK[slot],
                    descriptor_count: n,
                };
                pool_size_count += 1;
            }
        }
        let dp_info = VkDescriptorPoolCreateInfo {
            s_type: VkStructureType::DescriptorPoolCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            max_sets: 1,
            pool_size_count: pool_size_count as u32,
            p_pool_sizes: pool_sizes.as_ptr(),
        };
        let mut descriptor_pool = VkDescriptorPool::NULL;
        // SAFETY: `device` is live; `dp_info` is fully initialized referencing the
        // first `pool_size_count` (<= KIND_COUNT) entries of the live `pool_sizes`
        // inline array (alive for the call); `&mut descriptor_pool` is a valid
        // out-pointer; NULL allocator. `pool_size_count` bounds the driver's read.
        let raw = unsafe {
            (fns.create_descriptor_pool)(device, &dp_info, ptr::null(), &mut descriptor_pool)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateDescriptorPool(bind group)", result));
        }

        let set_layout = desc.layout.set_layout;
        let ds_alloc = VkDescriptorSetAllocateInfo {
            s_type: VkStructureType::DescriptorSetAllocateInfo,
            p_next: ptr::null(),
            descriptor_pool,
            descriptor_set_count: 1,
            p_set_layouts: &set_layout,
        };
        let mut descriptor_set = VkDescriptorSet::NULL;
        // SAFETY: `device` is live; `ds_alloc` names the live pool + references the
        // caller's live `set_layout` (the `set_layout` local, alive for the call);
        // `&mut descriptor_set` is a valid out-pointer.
        let raw =
            unsafe { (fns.allocate_descriptor_sets)(device, &ds_alloc, &mut descriptor_set) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `descriptor_pool` was just created and owns no live set yet
            // (allocation failed); destroy it once on this error path so it never
            // leaks (this also frees any partially-allocated set).
            unsafe { (fns.destroy_descriptor_pool)(device, descriptor_pool, ptr::null()) };
            return Err(VulkanError::Vk(
                "vkAllocateDescriptorSets(bind group)",
                result,
            ));
        }

        // --- Build the per-entry image-info + buffer-info inline arrays. Each kind
        //     populates exactly one of them at its slot; the WRITE at slot `i` points
        //     at whichever the kind reads (`p_image_info` for the three image kinds,
        //     `p_buffer_info` for the two buffer kinds), the other staying null. Each
        //     write's `dst_binding` is the LAYOUT entry's binding (caller contract:
        //     entries are in layout order, so `desc.layout`'s binding `i`). Image kinds
        //     declare the layout the descriptor records: GENERAL for a storage image,
        //     SHADER_READ_ONLY_OPTIMAL for a sampled one — the caller transitions each
        //     via `image_barrier` before access (the P1a SAFETY contract), and
        //     validation cross-checks the recorded layout at access time. All three
        //     inline arrays are fixed-capacity (zero heap) and outlive the update call. ---
        let mut image_infos = [VkDescriptorImageInfo {
            sampler: VkSampler::NULL,
            image_view: VkImageView::NULL,
            image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        }; MAX_BIND_GROUP_BINDINGS];
        let mut buffer_infos = [VkDescriptorBufferInfo {
            buffer: VkBuffer::NULL,
            offset: 0,
            range: 0,
        }; MAX_BIND_GROUP_BINDINGS];
        // HW-RT rung R2a-4a: the per-entry acceleration-structure `p_next` scratch, PARALLEL to
        // `image_infos`/`buffer_infos`. An AS binding's `VkWriteDescriptorSet.p_next` points at
        // this array's slot `i` (NOT a closure-local, which would DANGLE past the `from_fn`
        // closure — the batched `vkUpdateDescriptorSets` below reads every `p_next` at once). It
        // is address-stable to that update because it is declared here, out of the closure. The
        // array itself is only READ by the driver for the AS-kind slots; the buffer/image slots
        // leave their scratch entry untouched (a harmless zeroed default). Gated `hwrt` because it
        // names the RT FFI type; a non-`hwrt` build binds no AS, so it needs no scratch.
        #[cfg(feature = "hwrt")]
        let mut as_writes: [crate::accel_ffi::VkWriteDescriptorSetAccelerationStructureKHR;
            MAX_BIND_GROUP_BINDINGS] = core::array::from_fn(|_| {
            crate::accel_ffi::VkWriteDescriptorSetAccelerationStructureKHR {
                s_type: crate::accel_ffi::ST_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
                _pad: 0,
                p_next: ptr::null(),
                acceleration_structure_count: 0,
                _pad2: 0,
                p_acceleration_structures: ptr::null(),
            }
        });
        let writes: [VkWriteDescriptorSet; MAX_BIND_GROUP_BINDINGS] = core::array::from_fn(|i| {
            if i >= count {
                // Unused tail slot — never read (`descriptor_count: 0`, and the update
                // below passes only `count` writes). A harmless null-pointing write.
                return VkWriteDescriptorSet {
                    s_type: VkStructureType::WriteDescriptorSet,
                    p_next: ptr::null(),
                    dst_set: descriptor_set,
                    dst_binding: i as u32,
                    dst_array_element: 0,
                    descriptor_count: 0,
                    descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    p_image_info: ptr::null(),
                    p_buffer_info: ptr::null(),
                    p_texel_buffer_view: ptr::null(),
                };
            }
            let entry = &desc.entries[i];
            let kind = bind_group_entry_kind(entry);
            // Review M1: the entry's variant MUST match the kind the layout declared at
            // this slot (the doc-promised cross-check, now real because the layout
            // retains its per-entry kinds). The agnostic `BindGroupEntry` carries no
            // explicit binding; the layout↔group correspondence is positional, so slot
            // `i` of the group pairs with slot `i` of the layout. Debug-only.
            debug_assert!(
                kind == desc.layout.entries[i].kind,
                "P1a: BindGroupEntry variant must match the layout's DescriptorKind at this slot"
            );
            // Review M2: write at the binding the layout actually DECLARED, not the
            // positional slice index, so the write targets the right binding under any
            // binding numbering. For the contiguous-0..N convention every call site uses
            // (`layout.entries[i].binding == i`), this is byte-identical to the prior
            // positional `i as u32`.
            let dst_binding = desc.layout.entries[i].binding;
            let mut p_image_info: *const c_void = ptr::null();
            let mut p_buffer_info: *const VkDescriptorBufferInfo = ptr::null();
            // HW-RT rung R2a-4a: the write's `p_next`. Null for every P1a resource kind (the
            // driver reads none); an AS binding points it at the `as_writes[i]` scratch below.
            // Only the `hwrt` AS arm reassigns it, so a non-`hwrt` build never mutates it.
            #[cfg_attr(not(feature = "hwrt"), allow(unused_mut))]
            let mut p_next: *const c_void = ptr::null();
            match *entry {
                BindGroupEntry::StorageImage { texture } => {
                    // SDFDDGI I2: a MULTI-LAYER texture (array_view != NULL) binds its
                    // `VK_IMAGE_VIEW_TYPE_2D_ARRAY` view so a shader `RWTexture2DArray` write
                    // reaches EVERY layer (the DDGI probe atlas is an 8-layer `Texture2DArray`
                    // whose update pass writes `gIrrOut[uint3(x, y, layer)]` across all layers;
                    // `.view` is only layer 0's single-layer 2D render view — binding it would
                    // clamp the storage write to layer 0 / mismatch the descriptor's array type).
                    // A single-layer image has `array_view == NULL` → falls back to the
                    // full-subresource `.view`, BYTE-IDENTICAL to every existing StorageImage caller
                    // (all bind single-layer G-buffer images: gNormal/gMaterial/gViewT/ssao).
                    let image_view = if texture.array_view != VkImageView::NULL {
                        texture.array_view
                    } else {
                        texture.view
                    };
                    image_infos[i] = VkDescriptorImageInfo {
                        sampler: VkSampler::NULL,
                        image_view,
                        image_layout: VK_IMAGE_LAYOUT_GENERAL,
                    };
                    p_image_info = (&image_infos[i] as *const VkDescriptorImageInfo).cast();
                }
                BindGroupEntry::SampledImage { texture, sampler } => {
                    image_infos[i] = VkDescriptorImageInfo {
                        sampler: sampler.sampler,
                        image_view: texture.view,
                        image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    };
                    p_image_info = (&image_infos[i] as *const VkDescriptorImageInfo).cast();
                }
                BindGroupEntry::CombinedImage { texture, sampler } => {
                    // CSM Increment 1b: a MULTI-LAYER texture (array_view != NULL) binds its
                    // `VK_IMAGE_VIEW_TYPE_2D_ARRAY` sample view so a shader `Texture2DArray`
                    // resolves correctly (the cascade shadow map @ resolve binding 12). A
                    // single-layer texture has `array_view == NULL` → falls back to the
                    // full-subresource `.view`, BYTE-IDENTICAL to every existing combined-image
                    // caller (all bind single-layer images: present-blit, brick atlas, mesh-SDF).
                    let image_view = if texture.array_view != VkImageView::NULL {
                        texture.array_view
                    } else {
                        texture.view
                    };
                    image_infos[i] = VkDescriptorImageInfo {
                        sampler: sampler.sampler,
                        image_view,
                        image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    };
                    p_image_info = (&image_infos[i] as *const VkDescriptorImageInfo).cast();
                }
                BindGroupEntry::StorageBuffer { buffer }
                | BindGroupEntry::UniformBuffer { buffer } => {
                    buffer_infos[i] = VkDescriptorBufferInfo {
                        buffer: buffer.buffer,
                        offset: 0,
                        range: buffer.size,
                    };
                    p_buffer_info = &buffer_infos[i];
                }
                // HW-RT rung R2a-4a: bind a TLAS. The AS handle rides the extension `p_next`
                // chain (NOT `p_image_info`/`p_buffer_info`, which stay null). TWO pointer
                // lifetimes are pinned to survive the SINGLE batched `vkUpdateDescriptorSets`
                // below (the `from_fn` closure returns, so any closure-local would DANGLE):
                //   (i)  `p_next` → `&as_writes[i]`, the out-of-closure per-entry scratch;
                //   (ii) `p_acceleration_structures` → `&accel.handle`, read DIRECTLY from the
                //        borrowed `&'a BoundAccelStruct` (`desc.entries[i]` holds the `&'a`, so
                //        `accel.handle`'s address is stable for the whole `create_bind_group`
                //        call — never a copied local).
                #[cfg(feature = "hwrt")]
                BindGroupEntry::AccelerationStructure { accel } => {
                    as_writes[i] = crate::accel_ffi::VkWriteDescriptorSetAccelerationStructureKHR {
                        s_type: crate::accel_ffi::ST_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
                        _pad: 0,
                        p_next: ptr::null(),
                        acceleration_structure_count: 1,
                        _pad2: 0,
                        // `accel.handle` lives in the borrowed `&'a BoundAccelStruct` (address
                        // stable for the call); taking its address does not copy the handle into
                        // a local. The pointer is read by the driver during the update below.
                        p_acceleration_structures: &accel.handle,
                    };
                    p_next = (&as_writes[i]
                        as *const crate::accel_ffi::VkWriteDescriptorSetAccelerationStructureKHR)
                        .cast();
                }
                // A non-`hwrt` build binds `A::AccelerationStructure = ()`, so this variant IS
                // constructible (`accel: &()`) but is nonsensical — there is no RT device to bind
                // against. Defensive loud panic (not a compiler-dead arm): a caller who binds an AS
                // without `hwrt` gets a clean abort, not a silently no-op'd descriptor. Keeps the
                // match exhaustive without naming the AS FFI in a non-`hwrt` build.
                #[cfg(not(feature = "hwrt"))]
                BindGroupEntry::AccelerationStructure { .. } => {
                    unreachable!(
                        "invariant: BindGroupEntry::AccelerationStructure requires feature=\"hwrt\""
                    )
                }
            }
            VkWriteDescriptorSet {
                s_type: VkStructureType::WriteDescriptorSet,
                p_next,
                dst_set: descriptor_set,
                dst_binding,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: kind.as_i32(),
                p_image_info,
                p_buffer_info,
                p_texel_buffer_view: ptr::null(),
            }
        });
        // SAFETY: `device` is live; the first `count` (<= cap) `writes` reference the
        // freshly-allocated `descriptor_set`, each at its layout entry's binding, with
        // `descriptor_type` matching that binding's kind. For an image kind
        // `p_image_info` points at the matching `image_infos[i]` local (which names the
        // caller's live image view + optional sampler); for a buffer kind
        // `p_buffer_info` points at `buffer_infos[i]` (which names the caller's live
        // buffer with its full range). For an AS kind (HW-RT R2a-4a) `p_next` points at
        // `as_writes[i]`, whose `p_acceleration_structures` points at `accel.handle` inside the
        // caller's borrowed `&'a BoundAccelStruct` — BOTH lifetimes outlive this single batched
        // call: `as_writes` is declared out of the `from_fn` closure (address-stable), and the
        // `&'a` AS borrow is live for the whole `create_bind_group`. The non-relevant pointers
        // stay null, which the driver ignores for that descriptor type. All inline info arrays,
        // the AS scratch, and the `writes` array outlive the call; only the first `count` writes
        // are passed (the count bounds the driver's read). The set is not bound to any pending
        // command buffer (it was just allocated), so writing it is sound — and it is written
        // exactly ONCE here, never per-frame.
        unsafe {
            (fns.update_descriptor_sets)(device, count as u32, writes.as_ptr(), 0, ptr::null())
        };

        Ok(VulkanBindGroup {
            descriptor_pool,
            descriptor_set,
        })
    }

    unsafe fn destroy_bind_group(&self, group: VulkanBindGroup) {
        // SAFETY: `group.descriptor_pool` was created on this device by
        // `create_bind_group`, no submission using its set is pending (caller
        // fence-waited / `wait_idle`'d per the trait contract), and the by-value move
        // destroys it exactly once. Destroying the pool frees the set allocated from
        // it (no separate set free needed).
        unsafe {
            (self.device_fns().destroy_descriptor_pool)(
                self.device(),
                group.descriptor_pool,
                ptr::null(),
            )
        };
    }

    fn create_shader_module(&self, spirv: &[u32]) -> Result<VulkanShaderModule, VulkanError> {
        debug_assert!(!spirv.is_empty(), "invariant: SPIR-V word slice is non-empty");
        let sm_info = VkShaderModuleCreateInfo {
            s_type: VkStructureType::ShaderModuleCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            // `code_size` is in BYTES.
            code_size: spirv.len() * 4,
            p_code: spirv.as_ptr(),
        };
        let mut module = VkShaderModule::NULL;
        // SAFETY: `device` is live; `sm_info` is a fully-initialized `#[repr(C)]`
        // struct whose `p_code` points to `code_size` bytes of 4-byte-aligned
        // SPIR-V (`&[u32]` is word-aligned) alive for the call; `&mut module` is a
        // valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_shader_module)(
                self.device(),
                &sm_info,
                ptr::null(),
                &mut module,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateShaderModule", result));
        }
        Ok(VulkanShaderModule { module })
    }

    unsafe fn destroy_shader_module(&self, module: VulkanShaderModule) {
        // SAFETY: `module.module` was created on this device by
        // `create_shader_module`, no pipeline referencing it is in flight (caller
        // contract), and the by-value move destroys it exactly once.
        unsafe {
            (self.device_fns().destroy_shader_module)(self.device(), module.module, ptr::null())
        };
    }

    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc<Vulkan>,
    ) -> Result<ComputePipeline, VulkanError> {
        // The device-shared compute pipeline layout declares one COMPUTE push range
        // of `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` (sized to the largest consumer). A
        // pipeline may USE fewer bytes than the layout declares (valid Vulkan), so any
        // request that is a non-empty multiple of 4 (Vulkan's push-range granularity)
        // and fits the shared range is accepted; a larger request would read past the
        // declared range, so it is rejected as `Unsupported`. This covers both the
        // 4-byte `sdf_editlist` path and the 80-byte `sdf_depth_composite` marcher.
        if desc.push_constant_bytes == 0
            || !desc.push_constant_bytes.is_multiple_of(4)
            || desc.push_constant_bytes > COMPUTE_PUSH_CONSTANT_RANGE_BYTES
        {
            return Err(VulkanError::Unsupported(
                "push_constant_bytes must be a multiple of 4 within the shared compute push range",
            ));
        }
        // The shared pipeline layout is needed at pipeline-create time (plan Q1).
        let layouts = self.compute_layouts()?;

        // Render P1a: pick the pipeline layout. `None` → the device-shared
        // single-STORAGE_BUFFER fixed layout (the packed-buffer path, byte-identical
        // to before, NOT owned by the pipeline). `Some(bgl)` → a DEDICATED layout
        // declaring `set 0` = the vocabulary bind-group layout + the shared COMPUTE
        // push range, owned by the pipeline and torn down with it. The dedicated
        // layout is created first; if pipeline creation fails below it is rolled back
        // before the error returns.
        let (pipeline_layout, owns_layout) = match desc.bind_group_layout {
            None => (layouts.pipeline_layout, false),
            Some(bgl) => {
                let set_layout = bgl.set_layout;
                let push_range = VkPushConstantRange {
                    stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                    offset: 0,
                    size: COMPUTE_PUSH_CONSTANT_RANGE_BYTES,
                };
                let pl_info = VkPipelineLayoutCreateInfo {
                    s_type: VkStructureType::PipelineLayoutCreateInfo,
                    p_next: ptr::null(),
                    flags: 0,
                    set_layout_count: 1,
                    p_set_layouts: &set_layout,
                    push_constant_range_count: 1,
                    p_push_constant_ranges: &push_range,
                };
                let mut layout = VkPipelineLayout::NULL;
                // SAFETY: `device` is live; `pl_info` is fully initialized referencing
                // the `set_layout` local (the caller's live vocabulary set-layout, alive
                // for this whole fn) at `set 0` + the `push_range` local (alive for this
                // whole fn); `&mut layout` is a valid out-pointer; NULL allocator.
                let raw = unsafe {
                    (self.device_fns().create_pipeline_layout)(
                        self.device(),
                        &pl_info,
                        ptr::null(),
                        &mut layout,
                    )
                };
                let result = VkResult::from_raw(raw);
                if !result.is_success() {
                    return Err(VulkanError::Vk("vkCreatePipelineLayout(compute)", result));
                }
                (layout, true)
            }
        };

        // Rung 1a: assemble the specialization blob as SAME-SCOPE stack locals so
        // the pointers stay valid through the `vkCreateComputePipelines` call below.
        // This MUST stay inline in `create_compute_pipeline` — extracting it into a
        // helper that returns `p_spec` would dangle the locals on return.
        const MAX_SPEC: usize = 8;
        let spec_n = desc.spec_constants.len().min(MAX_SPEC); // release-safe clamp
        debug_assert!(
            desc.spec_constants.len() <= MAX_SPEC,
            "spec-const count exceeds MAX_SPEC"
        );
        let mut spec_data: [u32; MAX_SPEC] = [0; MAX_SPEC];
        let mut spec_map: [VkSpecializationMapEntry; MAX_SPEC] =
            core::array::from_fn(|_| VkSpecializationMapEntry {
                constant_id: 0,
                offset: 0,
                size: 0,
            });
        for i in 0..spec_n {
            let sc = desc.spec_constants[i];
            spec_data[i] = sc.value;
            let offset = (i * 4) as u32;
            spec_map[i] = VkSpecializationMapEntry {
                constant_id: sc.id,
                offset,
                size: 4usize,
            };
            debug_assert!((offset as usize) + 4 <= spec_n * 4, "spec map entry past blob");
        }
        let spec_info = VkSpecializationInfo {
            map_entry_count: spec_n as u32,
            p_map_entries: spec_map.as_ptr(),
            data_size: spec_n * 4,
            p_data: spec_data.as_ptr() as *const c_void,
        };
        // Linchpin: empty ⇒ LITERAL null, never a zero-count struct pointer.
        let p_spec: *const VkSpecializationInfo = if spec_n == 0 {
            ptr::null()
        } else {
            &spec_info as *const VkSpecializationInfo
        };

        // SAFETY: spec_data, spec_map, and spec_info are stack locals of this function, in the
        // same lexical scope as the vkCreateComputePipelines call below; the driver reads and
        // COPIES specialization data synchronously during that call and retains nothing past it,
        // so the pointers stay valid for the whole call. spec_n == 0 ⇒ p_spec is a literal null ⇒
        // the create-info is byte-identical to the pre-spec-const path. Each map entry's
        // offset+size (= i*4 + 4) is <= data_size (= spec_n*4), so blob reads are in-bounds.
        let stage = VkPipelineShaderStageCreateInfo {
            s_type: VkStructureType::PipelineShaderStageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: desc.module.module,
            p_name: desc.entry.as_ptr(),
            p_specialization_info: p_spec.cast(),
        };
        let cp_info = VkComputePipelineCreateInfo {
            s_type: VkStructureType::ComputePipelineCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage,
            layout: pipeline_layout,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };
        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `device` is live; null pipeline cache (`0`) is valid; one
        // create-info is fully initialized, referencing the live shader module +
        // the chosen `pipeline_layout` (the device-shared fixed layout or the
        // just-created dedicated one); `&mut pipeline` is a valid out-pointer for the
        // single pipeline; NULL allocator. The module is owned by the caller's
        // `VulkanShaderModule`, alive for this call.
        let raw = unsafe {
            (self.device_fns().create_compute_pipelines)(
                self.device(),
                0,
                1,
                &cp_info,
                ptr::null(),
                &mut pipeline,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            if owns_layout {
                // SAFETY: the dedicated `pipeline_layout` was just created on this
                // device and is not yet owned by any pipeline (creation failed);
                // destroy it once on this error path so it never leaks. The shared
                // layout (`owns_layout == false`) is left alone — it is the device's.
                unsafe {
                    (self.device_fns().destroy_pipeline_layout)(
                        self.device(),
                        pipeline_layout,
                        ptr::null(),
                    )
                };
            }
            return Err(VulkanError::from(ComputeError::VkError(
                "vkCreateComputePipelines",
                result,
            )));
        }
        Ok(ComputePipeline {
            pipeline,
            layout: pipeline_layout,
            owns_layout,
        })
    }

    unsafe fn destroy_compute_pipeline(&self, pipeline: ComputePipeline) {
        // SAFETY: `pipeline.pipeline` was created on this device, no submission
        // using it is pending (caller contract), and the by-value move destroys it
        // exactly once. A dedicated layout (`owns_layout`) is torn down AFTER the
        // pipeline (reverse creation order); the device-shared layout is left alone.
        unsafe {
            (self.device_fns().destroy_pipeline)(self.device(), pipeline.pipeline, ptr::null());
            if pipeline.owns_layout {
                (self.device_fns().destroy_pipeline_layout)(
                    self.device(),
                    pipeline.layout,
                    ptr::null(),
                );
            }
        };
    }

    fn create_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        // Textured-PBR T6c (plan Decision D5): the real work moved to `build_graphics_pipeline`
        // (a Vulkan-only inherent method, below) so it can be shared with
        // `create_graphics_pipeline_bindless`'s 2-set path. `set1: None, VK_COMPARE_OP_LESS,
        // depth_write: true` here reuses the IDENTICAL code path (not merely a `None`-gated
        // branch) every pre-T6c/pre-R4b-b caller took — byte-identical
        // `VkPipelineLayoutCreateInfo`/depth-stencil state by construction.
        self.build_graphics_pipeline(desc, None, VK_COMPARE_OP_LESS, true)
    }

    unsafe fn destroy_graphics_pipeline(&self, pipeline: VulkanGraphicsPipeline) {
        // SAFETY: both handles were created on this device by
        // `create_graphics_pipeline`, no submission using the pipeline is pending
        // (caller contract), and the by-value move destroys each exactly once.
        // Reverse creation order: the pipeline (created last) is destroyed before its
        // dedicated empty layout (created first).
        unsafe {
            (self.device_fns().destroy_pipeline)(self.device(), pipeline.pipeline, ptr::null());
            (self.device_fns().destroy_pipeline_layout)(
                self.device(),
                pipeline.layout,
                ptr::null(),
            );
        }
    }

    fn create_fence(&self, signaled: bool) -> Result<VulkanFence, VulkanError> {
        let fence_info = VkFenceCreateInfo {
            s_type: VkStructureType::FenceCreateInfo,
            p_next: ptr::null(),
            // `VK_FENCE_CREATE_SIGNALED_BIT` == 0x1.
            flags: if signaled { 0x0000_0001 } else { 0 },
        };
        let mut fence = VkFence::NULL;
        // SAFETY: `device` is live; `fence_info` is fully initialized; `&mut
        // fence` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_fence)(self.device(), &fence_info, ptr::null(), &mut fence)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateFence", result));
        }
        Ok(VulkanFence { fence })
    }

    unsafe fn destroy_fence(&self, fence: VulkanFence) {
        // SAFETY: `fence.fence` was created on this device, is not pending (caller
        // contract), and the by-value move destroys it exactly once.
        unsafe { (self.device_fns().destroy_fence)(self.device(), fence.fence, ptr::null()) };
    }

    fn wait_fence(&self, fence: &VulkanFence, timeout_ns: u64) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `&fence.fence` names one live fence;
        // `wait_all = VK_TRUE` blocks until it is signaled (or the timeout
        // elapses). After this returns `Ok` the submission that signals it has
        // completed — the fence-before-readback discipline.
        let raw = unsafe {
            (self.device_fns().wait_for_fences)(
                self.device(),
                1,
                &fence.fence,
                VK_TRUE,
                timeout_ns,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkWaitForFences", result));
        }
        Ok(())
    }

    fn reset_fence(&self, fence: &VulkanFence) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `&fence.fence` names one live fence to reset
        // to unsignaled (no submission referencing it is pending — caller resets
        // only after a `wait_fence`).
        let raw =
            unsafe { (self.device_fns().reset_fences)(self.device(), 1, &fence.fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkResetFences", result));
        }
        Ok(())
    }

    fn create_query_pool(&self, desc: &QueryPoolDesc) -> Result<VulkanQueryPool, VulkanError> {
        debug_assert!(desc.count > 0, "invariant: a query pool needs >= 1 query");
        let create_info = VkQueryPoolCreateInfo {
            s_type: VkStructureType::QueryPoolCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            query_type: VK_QUERY_TYPE_TIMESTAMP,
            query_count: desc.count,
            // A TIMESTAMP pool sets no pipeline-statistics flags.
            pipeline_statistics: 0,
        };
        let mut pool = VkQueryPool::NULL;
        // SAFETY: `device` is live; `create_info` is fully initialized (a TIMESTAMP pool of
        // `count` queries); `&mut pool` is a valid out-pointer; NULL allocator. The queries
        // are UNDEFINED at creation — the caller resets them before the first write.
        let raw = unsafe {
            (self.device_fns().create_query_pool)(self.device(), &create_info, ptr::null(), &mut pool)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateQueryPool", result));
        }
        Ok(VulkanQueryPool { pool, count: desc.count })
    }

    unsafe fn destroy_query_pool(&self, pool: VulkanQueryPool) {
        // SAFETY: `pool.pool` was created on this device, no submission writing/reading it is
        // pending (caller contract), and the by-value move destroys it exactly once.
        unsafe { (self.device_fns().destroy_query_pool)(self.device(), pool.pool, ptr::null()) };
    }

    fn read_query_pool_ns(
        &self,
        pool: &VulkanQueryPool,
        pair_count: u32,
        scratch: &mut [u64],
        out_ns: &mut [f64],
    ) -> Result<(), VulkanError> {
        debug_assert!(
            out_ns.len() >= pair_count as usize,
            "invariant: out_ns must hold pair_count ns values"
        );
        self.fetch_query_pair_ticks(pool, pair_count, scratch)?;
        // × `timestampPeriod`. The tick deltas themselves (masking + wrap handling) are the
        // shared helper's business; this method is only the ns SCALE.
        let period = self.device_caps().timestamp_period as f64;
        for i in 0..pair_count as usize {
            out_ns[i] = scratch[i] as f64 * period;
        }
        Ok(())
    }

    fn read_query_pool_ticks(
        &self,
        pool: &VulkanQueryPool,
        pair_count: u32,
        scratch: &mut [u64],
        out_ticks: &mut [u64],
    ) -> Result<(), VulkanError> {
        debug_assert!(
            out_ticks.len() >= pair_count as usize,
            "invariant: out_ticks must hold pair_count tick values"
        );
        self.fetch_query_pair_ticks(pool, pair_count, scratch)?;
        out_ticks[..pair_count as usize].copy_from_slice(&scratch[..pair_count as usize]);
        Ok(())
    }

    // ===== HW-RT ACCELERATION-STRUCTURE VERBS (rung R2a-1; `feature="hwrt"` overrides) =====
    // Each delegates to a `crate::accel` inherent helper (the real `vkGet*`/`vkCreate*` FFI).
    // Present ONLY under `hwrt`; a default build inherits the `#[cold]` erroring defaults.

    #[cfg(feature = "hwrt")]
    fn get_acceleration_structure_build_sizes(
        &self,
        kind: boyko_rhi::AsKind,
        geometry: &boyko_rhi::AsGeometryDesc,
    ) -> Result<boyko_rhi::AsBuildSizes, VulkanError> {
        self.build_sizes(kind, geometry)
    }

    #[cfg(feature = "hwrt")]
    fn create_acceleration_structure(
        &self,
        kind: boyko_rhi::AsKind,
        buffer: &BoundBuffer,
        size: u64,
    ) -> Result<crate::accel::BoundAccelStruct, VulkanError> {
        self.create_accel(kind, buffer.buffer, size)
    }

    #[cfg(feature = "hwrt")]
    fn get_acceleration_structure_device_address(
        &self,
        accel: &crate::accel::BoundAccelStruct,
    ) -> Result<u64, VulkanError> {
        self.accel_device_address(accel)
    }

    #[cfg(feature = "hwrt")]
    fn get_buffer_device_address(&self, buffer: &BoundBuffer) -> Result<u64, VulkanError> {
        self.buffer_device_address(buffer.buffer)
    }

    #[cfg(feature = "hwrt")]
    unsafe fn destroy_acceleration_structure(&self, accel: crate::accel::BoundAccelStruct) {
        // SAFETY: the RhiDevice contract — the GPU is no longer using `accel` (caller
        // fence-waited/`wait_idle`'d) and it is destroyed once (by-value move).
        unsafe { self.destroy_accel(accel) };
    }

    fn create_command_encoder(&self) -> Result<VulkanCommandEncoder, VulkanError> {
        let layouts = self.compute_layouts()?;
        // SAFETY: the device is live; `layouts` are this device's shared compute
        // layouts; the encoder takes a raw pointer to this context's `DeviceFns`
        // (which outlives any encoder built from `&self`).
        let enc = unsafe {
            VulkanCommandEncoder::new(
                self.device(),
                self.device_fns() as *const DeviceFns,
                self.queue_family_index(),
                layouts.set_layout,
                layouts.pipeline_layout,
            )
        };
        // HW-RT rung R2a-1: wire the AS command table (a raw pointer into this context, which
        // outlives the encoder) so `cmd_build_acceleration_structures` can reach the FFI; null
        // when ray query is off. No-op on a non-hwrt build.
        #[cfg(feature = "hwrt")]
        let enc = enc.map(|mut e| {
            let p = self
                .accel_fns_opt()
                .map_or(ptr::null(), |f| f as *const crate::accel::AccelFns);
            e.set_accel_fns(p);
            e
        });
        enc
    }

    unsafe fn destroy_command_encoder(&self, enc: VulkanCommandEncoder) {
        // SAFETY: `enc` was created on this device, its last submission has
        // completed (caller contract), and the by-value move destroys it exactly
        // once. `destroy` tears down the descriptor pool + command pool (which
        // frees the set + command buffer) in reverse order.
        unsafe { enc.destroy(self.device(), self.device_fns()) };
    }

    fn wait_idle(&self) -> Result<(), VulkanError> {
        // SAFETY: `device` is live; `vkDeviceWaitIdle` blocks until every queue is
        // idle — the belt-and-braces teardown sync (plan W4).
        let raw = unsafe { (self.device_fns().device_wait_idle)(self.device()) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkDeviceWaitIdle", result));
        }
        Ok(())
    }
}

impl VulkanContext {
    /// The shared body of [`RhiDevice::read_query_pool_ns`] and
    /// [`RhiDevice::read_query_pool_ticks`]: host-waits + reads `2 * pair_count` raw timestamps
    /// from `pool` and COMPACTS them in place into `scratch[0..pair_count]` as masked tick
    /// deltas. The two public readers differ only in what they do with those integers (scale to
    /// ns, or copy out), so the `vkGetQueryPoolResults` FFI call and the mask/wrap arithmetic
    /// exist exactly once.
    ///
    /// # Why the in-place compaction is sound
    ///
    /// Pair `i` reads `scratch[2i]`/`scratch[2i+1]` and writes `scratch[i]`. Since `i <= 2i` for
    /// every `i >= 0` and the loop runs in ASCENDING `i`, slot `i` was already consumed at step
    /// `floor(i/2) <= i` before this step overwrites it. No input is destroyed before it is read,
    /// so the caller needs no second buffer.
    ///
    /// # Panics
    ///
    /// `debug_assert`s that `2 * pair_count` fits both `pool.count` and `scratch`.
    fn fetch_query_pair_ticks(
        &self,
        pool: &VulkanQueryPool,
        pair_count: u32,
        scratch: &mut [u64],
    ) -> Result<(), VulkanError> {
        let query_count = pair_count * 2;
        debug_assert!(
            query_count <= pool.count,
            "invariant: 2 * pair_count must fit the pool's query count"
        );
        debug_assert!(
            scratch.len() >= query_count as usize,
            "invariant: scratch must hold 2 * pair_count raw timestamps"
        );

        // SAFETY: `device` is live; `pool.pool` is a live TIMESTAMP pool whose `[0..query_count)`
        // queries were reset + written this frame (caller contract, after `wait_fence`);
        // `scratch.as_mut_ptr()` names `query_count` `u64` slots (asserted above) — `data_size`
        // is exactly that many bytes and `stride` is 8 (one `u64` per query). `64_BIT | WAIT_BIT`
        // reads each result as a 64-bit value, blocking until it is available. NULL is not passed.
        let raw = unsafe {
            (self.device_fns().get_query_pool_results)(
                self.device(),
                pool.pool,
                0,
                query_count,
                (query_count as usize) * 8,
                scratch.as_mut_ptr().cast::<c_void>(),
                8,
                VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WAIT_BIT,
            )
        };
        let result = VkResult::from_raw(raw);
        // `WAIT_BIT` makes the call return ONLY once every requested query is available, so the sole
        // success code here is `VK_SUCCESS`; the positive non-error `VK_NOT_READY`/`VK_INCOMPLETE`
        // (which `is_success()` would also accept, meaning an unwritten/partial query) cannot occur.
        // Callers MUST read only WRITTEN (begin,end) pairs — an unwritten query would block this call
        // forever and never reach here (the timing harnesses enforce this: the isolated smoke reads 1
        // pair; the combined harness asserts all four passes active). So `is_success()` here is
        // unambiguously a fully-available result.
        if !result.is_success() {
            return Err(VulkanError::Vk("vkGetQueryPoolResults", result));
        }

        // Mask each raw timestamp to the queue family's valid bits BEFORE subtracting (high
        // bits above the valid width are hardware garbage). The `wrapping_sub` + post-subtraction
        // mask handles a counter wrap across the pair.
        let mask = self.device_caps().timestamp_mask();
        for i in 0..pair_count as usize {
            let begin = scratch[2 * i] & mask;
            let end = scratch[2 * i + 1] & mask;
            scratch[i] = end.wrapping_sub(begin) & mask;
        }
        Ok(())
    }

    /// The shared graphics-pipeline builder (textured-PBR T6c, plan Decision D5): builds the
    /// `VkPipelineLayout` from `desc.bind_group_layout` at set 0 plus, when `set1` is `Some`,
    /// a SECOND raw `VkDescriptorSetLayout` at set 1 (FRAGMENT-visible — the caller passes
    /// [`crate::bindless::VulkanBindlessSet::set_layout`] directly, no wrapper type), then
    /// builds the rest of the pipeline state exactly as before. `set1: None` is the path
    /// EVERY existing caller (`RhiDevice::create_graphics_pipeline`) takes — the produced
    /// `VkPipelineLayoutCreateInfo` is BYTE-IDENTICAL to the pre-T6c single-set (or zero-set)
    /// layout: `set_layout_count`/`p_set_layouts[0]`/the push range are computed the SAME way,
    /// just from a 3-element inline array instead of a scalar local (the driver reads only the
    /// first `set_layout_count` entries either way, so the extra unread slots are inert). This
    /// fn is the SOLE body `create_graphics_pipeline`, [`Self::create_graphics_pipeline_bindless`],
    /// AND (multi-paradigm render-path plan, rung R4b-b) [`Self::create_graphics_pipeline_forward`]
    /// call — a shared code path, not a `None`-gated branch, so every pre-T6c/pre-R4b-b pipeline's
    /// layout is untouched BY CONSTRUCTION.
    ///
    /// Rung R4b-b boot-panic fix: an earlier revision of this fn took a THIRD `set2` parameter so
    /// `forward_opaque.fs.hlsl`'s (then Set-2) shadow bindings could sit past an empty Set-1
    /// placeholder. That placeholder was a ZERO-BINDING [`BindGroupLayoutDesc`], which
    /// [`RhiDevice::create_bind_group_layout`]'s own `1..=MAX_BIND_GROUP_BINDINGS` invariant
    /// REJECTS — a real `GpuSceneBundles::boot` panic (`debug_assert!` in `create_bind_group_layout`,
    /// `device.rs:205`), caught post-implementation. The shader's shadow bindings were renumbered
    /// to Set 1 instead (`forward_opaque.fs.hlsl`'s doc), so Forward is a plain 2-set
    /// `[Set0, Set1]` pipeline — the SAME shape [`Self::create_graphics_pipeline_bindless`]
    /// already builds, no placeholder needed. `set2` was removed; `set1` now serves BOTH the
    /// bindless texture set (T6c) AND Forward's shadow set (R4b-b) — two DIFFERENT call sites,
    /// never both at once.
    ///
    /// `depth_compare` (rung R4b-b): the depth-test compare op, `VK_COMPARE_OP_LESS` for every
    /// pre-R4b-b caller (Deferred's custom-linear depth, nearer = smaller `z`) or
    /// `VK_COMPARE_OP_GREATER` for Forward's hardware reverse-Z (Decision 4, nearer = larger `z`).
    fn build_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
        set1: Option<VkDescriptorSetLayout>,
        depth_compare: i32,
        depth_write: bool,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        let device = self.device();
        let fns = self.device_fns();

        // --- The pipeline layout. Rungs 2..4 use a layout with NO descriptor sets
        //     (rung 2 empty; rung 3/4 add ONE `VERTEX`-stage push-constant range of
        //     `desc.push_constant_bytes` bytes at offset 0 — the MVP `float4x4`).
        //     Rung 5 ADDS one bind-group layout at `set 0` (the COMBINED_IMAGE_SAMPLER)
        //     when `desc.bind_group_layout` is `Some`, so a `bind_descriptor_set` can
        //     bind a matching group before the sampling draw; `None` keeps the
        //     rungs-2..4 no-descriptor path byte-identical (count 0, null array).
        //     Textured-PBR T6c ADDS an optional SECOND set (`set1`) — see this fn's doc.
        //     Created first; if pipeline creation fails below, it is torn down before
        //     the error returns (reverse-order rollback). The `push_range` +
        //     `set_layouts` locals must outlive the create call, so they are bound
        //     here (the layout-info pointers below reference them). ---
        // The push range spans `VERTEX | FRAGMENT`: every existing graphics shader pushes from the
        // VERTEX stage only (the gbuffer/cascade/spot pipelines), and a fragment stage that declares
        // no push block simply ignores the range — so widening the visibility is byte-neutral for
        // them. The Shadow Phase 5 Inc-2 POINT depth FS (`punctual_depth.fs`) READS the `cam_eye@64`
        // lane (`light_pos`/`inv_range`), which requires the range to cover `FRAGMENT`. Push-constant
        // stage flags are part of the pipeline LAYOUT, not the recorded command stream, and the
        // recorders keep pushing with `VK_SHADER_STAGE_VERTEX_BIT` (a subset), so the rendered output
        // of every pre-Inc-2 pipeline is unchanged (the 0%-gate holds).
        let push_range = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
            offset: 0,
            size: desc.push_constant_bytes,
        };
        let has_push = desc.push_constant_bytes > 0;
        let set0 = desc
            .bind_group_layout
            .map_or(VkDescriptorSetLayout::NULL, |bgl| bgl.set_layout);
        let has_set0 = desc.bind_group_layout.is_some();
        // Textured-PBR T6c (D5) / rung R4b-b: a set-1-only layout (skipping set 0) is not a
        // shape this engine ever needs — both `set1` callers (the TEXTURED raster pipeline's
        // bindless set, Forward's shadow set) always declare set 0 too.
        debug_assert!(
            set1.is_none() || has_set0,
            "invariant: a set-1 pipeline layout always also declares set 0"
        );
        // `set_layout_count` is EXACTLY `u32::from(has_set0)` when `set1` is `None` (the
        // byte-identical value `create_graphics_pipeline`'s pre-T6c code computed) or `2` when
        // `set1` is `Some` (T6c's textured pipeline OR R4b-b's Forward pipeline). The array
        // always holds both slots; the driver reads only the first `set_layout_count` of them.
        let set_layouts: [VkDescriptorSetLayout; 2] =
            [set0, set1.unwrap_or(VkDescriptorSetLayout::NULL)];
        let set_layout_count: u32 = if set1.is_some() { 2 } else { u32::from(has_set0) };
        let has_any_set = set_layout_count > 0;
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: VkStructureType::PipelineLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count,
            p_set_layouts: if has_any_set {
                set_layouts.as_ptr()
            } else {
                ptr::null()
            },
            push_constant_range_count: u32::from(has_push),
            p_push_constant_ranges: if has_push {
                &push_range
            } else {
                ptr::null()
            },
        };
        let mut layout = VkPipelineLayout::NULL;
        // SAFETY: `device` is live; `pl_info` is fully initialized with either zero
        // descriptor sets (null array valid for count 0) or `set_layout_count` sets
        // pointing at the live `set_layouts` inline array (alive for this whole fn) — slot
        // 0 the caller's live bind-group set-layout when `has_set0`, slot 1 the caller's
        // live bindless set-layout (T6c) or Forward's shadow set-layout (R4b-b) when
        // `set1.is_some()` — and either zero push ranges (null array valid for count 0) or
        // one range pointing at the `push_range` local (alive for this whole fn) when
        // `has_push`; `&mut layout` is a valid out-pointer; NULL allocator.
        let raw =
            unsafe { (fns.create_pipeline_layout)(device, &pl_info, ptr::null(), &mut layout) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreatePipelineLayout(graphics)", result));
        }

        // --- Two shader stages (vertex + fragment). ---
        let stages = [
            VkPipelineShaderStageCreateInfo {
                s_type: VkStructureType::PipelineShaderStageCreateInfo,
                p_next: ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_VERTEX_BIT,
                module: desc.vertex_module.module,
                p_name: desc.vertex_entry.as_ptr(),
                p_specialization_info: ptr::null(),
            },
            VkPipelineShaderStageCreateInfo {
                s_type: VkStructureType::PipelineShaderStageCreateInfo,
                p_next: ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_FRAGMENT_BIT,
                module: desc.fragment_module.module,
                p_name: desc.fragment_entry.as_ptr(),
                p_specialization_info: ptr::null(),
            },
        ];

        // --- Vertex input. Rung 2: empty (positions come from the vertex shader's
        //     SV_VertexID — no vertex buffer). Rung 3: one binding (binding 0,
        //     per-vertex rate, the layout's stride) + one attribute per layout entry.
        //     The `binding`/`attributes` locals must outlive the create call below;
        //     they are bound here so the `vertex_input` pointers stay valid. The
        //     unused tail of `attributes` (slots >= the layout's count) is never read:
        //     `vertex_attribute_description_count` bounds the driver's read. ---
        let mut vk_bindings: [VkVertexInputBindingDescription; 1] =
            [VkVertexInputBindingDescription {
                binding: 0,
                stride: 0,
                input_rate: VK_VERTEX_INPUT_RATE_VERTEX,
            }];
        let mut vk_attributes: [VkVertexInputAttributeDescription; MAX_VERTEX_ATTRIBUTES] =
            core::array::from_fn(|_| VkVertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: VK_FORMAT_UNDEFINED,
                offset: 0,
            });
        let attribute_count = match &desc.vertex_layout {
            None => 0usize,
            Some(layout) => {
                debug_assert!(
                    layout.attributes.len() <= MAX_VERTEX_ATTRIBUTES,
                    "invariant: rung-3 vertex layout has <= MAX_VERTEX_ATTRIBUTES attributes"
                );
                vk_bindings[0].stride = layout.stride;
                // The agnostic `VertexFormat` discriminant equals the `VkFormat`
                // constant (asserted in `abi_guard.rs`).
                for (slot, attr) in vk_attributes.iter_mut().zip(layout.attributes.iter()) {
                    slot.location = attr.location;
                    slot.binding = 0;
                    slot.format = attr.format.as_i32();
                    slot.offset = attr.offset;
                }
                // Release-safe: the count handed to the driver never exceeds the
                // initialized inline slots, even if a (debug-asserted above) over-count
                // were to slip through in a release build — `vertex_attribute_description_count`
                // then matches exactly the slots written by the `zip` loop.
                layout.attributes.len().min(MAX_VERTEX_ATTRIBUTES)
            }
        };
        let has_vertex_layout = attribute_count > 0;
        let vertex_input = VkPipelineVertexInputStateCreateInfo {
            s_type: VkStructureType::PipelineVertexInputStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            vertex_binding_description_count: u32::from(has_vertex_layout),
            p_vertex_binding_descriptions: if has_vertex_layout {
                vk_bindings.as_ptr()
            } else {
                ptr::null()
            },
            vertex_attribute_description_count: attribute_count as u32,
            p_vertex_attribute_descriptions: if has_vertex_layout {
                vk_attributes.as_ptr()
            } else {
                ptr::null()
            },
        };

        let input_assembly = VkPipelineInputAssemblyStateCreateInfo {
            s_type: VkStructureType::PipelineInputAssemblyStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            // The agnostic `PrimitiveTopology` discriminant equals the
            // `VkPrimitiveTopology` constant (asserted in `abi_guard.rs`).
            topology: desc.topology.as_i32(),
            primitive_restart_enable: VK_FALSE,
        };

        // Dynamic viewport + scissor: counts of 1 with null pointers (the rects come
        // from `cmd_set_viewport`/`cmd_set_scissor`, recorded before the draw).
        let viewport_state = VkPipelineViewportStateCreateInfo {
            s_type: VkStructureType::PipelineViewportStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            viewport_count: 1,
            p_viewports: ptr::null(),
            scissor_count: 1,
            p_scissors: ptr::null(),
        };

        // CSM Increment 0: lower the configurable cull mode + optional depth bias.
        // `CullMode::None` == `VK_CULL_MODE_NONE` and `depth_bias: None` ==
        // `depthBiasEnable = VK_FALSE` + zeroed factors — byte-identical to the prior
        // hardcoded rasterization state, so every existing pipeline (which passes those
        // defaults) re-emits the SAME bytes. The agnostic `CullMode` discriminant
        // equals the `VkCullModeFlags` bits (asserted in `abi_guard.rs`), so the cull
        // lowering is an `as_u32()` no-op. A shadow-map depth pass selects
        // `CullMode::Front` + `Some(DepthBias { .. })`.
        let cull_mode: VkFlags = desc.cull_mode.as_u32();
        let (depth_bias_enable, db_constant, db_slope, db_clamp) = match desc.depth_bias {
            None => (VK_FALSE, 0.0, 0.0, 0.0),
            Some(b) => (VK_TRUE, b.constant_factor, b.slope_factor, b.clamp),
        };
        let rasterization = VkPipelineRasterizationStateCreateInfo {
            s_type: VkStructureType::PipelineRasterizationStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            depth_clamp_enable: VK_FALSE,
            rasterizer_discard_enable: VK_FALSE,
            polygon_mode: VK_POLYGON_MODE_FILL,
            cull_mode,
            front_face: VK_FRONT_FACE_COUNTER_CLOCKWISE,
            depth_bias_enable,
            depth_bias_constant_factor: db_constant,
            depth_bias_clamp: db_clamp,
            depth_bias_slope_factor: db_slope,
            line_width: 1.0,
        };

        let multisample = VkPipelineMultisampleStateCreateInfo {
            s_type: VkStructureType::PipelineMultisampleStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            rasterization_samples: VK_SAMPLE_COUNT_1_BIT,
            sample_shading_enable: VK_FALSE,
            min_sample_shading: 0.0,
            p_sample_mask: ptr::null(),
            alpha_to_coverage_enable: VK_FALSE,
            alpha_to_one_enable: VK_FALSE,
        };

        // One opaque (blend-disabled) color-blend attachment state PER color (MRT)
        // attachment, each with an all-channel write mask so the fragment color reaches
        // every channel of its target (Phase-6 S0 rung 6). The G-buffer geometry pass
        // declares two (albedo + normal); rungs 2..5 declare one (`color_formats.len()
        // == 1`). The count MUST equal the dynamic-rendering format count below, or the
        // driver rejects the pipeline. The first `color_attachment_count` entries are
        // identical opaque states; the inline tail is never read (the count bounds it).
        //
        // CSM Increment 0: an EMPTY `color_formats` is the DEPTH-ONLY path —
        // `colorAttachmentCount = 0`, a null color-blend attachment array, and a null
        // `pColorAttachmentFormats` below (a depth-only shadow-map pass). A depth-only
        // pipeline then REQUIRES a depth format (validation rejects a pipeline with
        // neither color nor depth); the relaxed assert pins that.
        debug_assert!(
            !desc.color_formats.is_empty() || desc.depth_format.is_some(),
            "invariant: a graphics pipeline needs >= 1 color attachment format OR a depth format (depth-only)"
        );
        debug_assert!(
            desc.color_formats.len() <= MAX_COLOR_ATTACHMENTS,
            "invariant: graphics pipeline color-attachment count exceeds the fixed cap"
        );
        // Release-safe: the count handed to the driver never exceeds the initialized
        // inline slots even if a (debug-asserted) over-count slipped through a release
        // build — it is clamped to the cap, matching the arrays' length.
        let color_attachment_count = desc.color_formats.len().min(MAX_COLOR_ATTACHMENTS);
        // GUI P5a Decision 3: lower the optional `BlendState`. `None` keeps the
        // pre-P5a opaque (blend-disabled) write byte-identical (every existing
        // pipeline). `Some(bs)` enables blending with `bs`'s factors/op on ALL color
        // attachments (P5a UI is single-target; a future MRT-per-target widening
        // turns `Option<BlendState>` into a slice). The agnostic `BlendFactor`/
        // `BlendOp` discriminants equal the `VkBlendFactor`/`VkBlendOp` constants
        // (asserted in `abi_guard.rs`), so each lowering is an `as_i32()` no-op.
        let (blend_enable, src_color, dst_color, color_op, src_alpha, dst_alpha, alpha_op) =
            match desc.blend {
                None => (VK_FALSE, 0, 0, 0, 0, 0, 0),
                Some(bs) => (
                    VK_TRUE,
                    bs.src_color.as_i32(),
                    bs.dst_color.as_i32(),
                    bs.color_op.as_i32(),
                    bs.src_alpha.as_i32(),
                    bs.dst_alpha.as_i32(),
                    bs.alpha_op.as_i32(),
                ),
            };
        // `from_fn` (not `[x; N]`) avoids requiring `Copy` on the FFI struct; every
        // slot is the identical (opaque or blended) all-channel-write state.
        let blend_attachments: [VkPipelineColorBlendAttachmentState; MAX_COLOR_ATTACHMENTS] =
            core::array::from_fn(|_| VkPipelineColorBlendAttachmentState {
                blend_enable,
                src_color_blend_factor: src_color,
                dst_color_blend_factor: dst_color,
                color_blend_op: color_op,
                src_alpha_blend_factor: src_alpha,
                dst_alpha_blend_factor: dst_alpha,
                alpha_blend_op: alpha_op,
                color_write_mask: VK_COLOR_COMPONENT_R_BIT
                    | VK_COLOR_COMPONENT_G_BIT
                    | VK_COLOR_COMPONENT_B_BIT
                    | VK_COLOR_COMPONENT_A_BIT,
            });
        // CSM Increment 0: a non-empty `color_formats` keeps the prior
        // `p_attachments = blend_attachments.as_ptr()` (byte-identical). An EMPTY
        // (depth-only) pipeline has `attachment_count = 0` + a null `p_attachments`,
        // and the WHOLE color-blend state is omitted (`p_color_blend_state = null`
        // below) — Vulkan allows a null color-blend state when there are no color
        // attachments.
        let has_color = color_attachment_count > 0;
        let color_blend = VkPipelineColorBlendStateCreateInfo {
            s_type: VkStructureType::PipelineColorBlendStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            logic_op_enable: VK_FALSE,
            logic_op: 0,
            attachment_count: color_attachment_count as u32,
            p_attachments: if has_color {
                blend_attachments.as_ptr()
            } else {
                ptr::null()
            },
            blend_constants: [0.0; 4],
        };
        let p_color_blend_state: *const VkPipelineColorBlendStateCreateInfo = if has_color {
            &color_blend
        } else {
            ptr::null()
        };

        let dynamic_states = [VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR];
        let dynamic_state = VkPipelineDynamicStateCreateInfo {
            s_type: VkStructureType::PipelineDynamicStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            dynamic_state_count: dynamic_states.len() as u32,
            p_dynamic_states: dynamic_states.as_ptr(),
        };

        // Depth-stencil state (Phase-6 S0 rung 4). Declared ONLY when a depth format
        // is present: depth test enabled, compare op `depth_compare` (`LESS` for
        // every Deferred/CSM/atlas caller — nearer fragment wins; `GREATER` for Forward's
        // hardware reverse-Z, rung R4b-b Decision 4 — nearer fragment has the LARGER stored
        // depth; `EQUAL` for ForwardPlus's zero-overdraw `forward_opaque` pass, rung R5), no
        // depth-bounds, no stencil. `depth_write` (rung R5) is `true` for every pre-R5 caller
        // (byte-identical `VK_TRUE`) and `false` ONLY for the ForwardPlus `forward_opaque`
        // variant, which relies entirely on `depth_prepass`'s own GREATER+write-ON pass to
        // have already committed the final depth value — an EQUAL test with writes disabled
        // costs no depth bandwidth and cannot perturb the prepass-owned value. A `None`
        // `depth_format` (rungs 1..3) leaves both the depth-stencil pointer null and
        // `depth_attachment_format` UNDEFINED, so the rung-2/3 no-depth pipelines stay
        // byte-identical. The `depth_state` local must outlive the create call, so it is
        // bound here. The agnostic `Format` discriminant equals the `VkFormat` constant
        // (asserted in `abi_guard.rs`); `VK_COMPARE_OP_LESS`/`VK_COMPARE_OP_GREATER`/
        // `VK_COMPARE_OP_EQUAL` are the FFI constants.
        let depth_state = VkPipelineDepthStencilStateCreateInfo {
            s_type: VkStructureType::PipelineDepthStencilStateCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            depth_test_enable: VK_TRUE,
            depth_write_enable: if depth_write { VK_TRUE } else { VK_FALSE },
            depth_compare_op: depth_compare,
            depth_bounds_test_enable: VK_FALSE,
            stencil_test_enable: VK_FALSE,
            front: VkStencilOpState::default(),
            back: VkStencilOpState::default(),
            min_depth_bounds: 0.0,
            max_depth_bounds: 1.0,
        };
        let depth_attachment_format = match desc.depth_format {
            Some(fmt) => fmt.as_i32(),
            None => VK_FORMAT_UNDEFINED,
        };
        let p_depth_stencil_state: *const c_void = if desc.depth_format.is_some() {
            (&depth_state as *const VkPipelineDepthStencilStateCreateInfo).cast()
        } else {
            ptr::null()
        };

        // The dynamic-rendering attachment-format chain (no `VkRenderPass`). The
        // color-attachment formats declared here are the W2-b SAFETY contract: each
        // MUST equal the format of the same-index `begin_rendering` color attachment
        // any bound pipeline renders into — AND the count must equal the rendering
        // scope's color-attachment count — or the validation layer faults at DRAW
        // time. The format count equals `color_attachment_count` (the same value the
        // color-blend `attachment_count` above uses, so the two stay consistent).
        // `depth_attachment_format` carries the same contract for the depth attachment
        // (rung 4). The agnostic `Format` discriminant equals the `VkFormat` constant
        // (asserted in `abi_guard.rs`). The `color_formats` inline array's first
        // `color_attachment_count` entries are lowered from the desc; the tail is never
        // read (the count bounds the driver's read).
        let mut color_formats: [i32; MAX_COLOR_ATTACHMENTS] = [VK_FORMAT_UNDEFINED; MAX_COLOR_ATTACHMENTS];
        for (slot, fmt) in color_formats.iter_mut().zip(desc.color_formats.iter()) {
            *slot = fmt.as_i32();
        }
        let rendering_info = VkPipelineRenderingCreateInfo {
            s_type: VkStructureType::PipelineRenderingCreateInfo,
            p_next: ptr::null(),
            view_mask: 0,
            color_attachment_count: color_attachment_count as u32,
            // CSM Increment 0: null the format array for the depth-only path
            // (`color_attachment_count == 0`); the non-empty path is byte-identical.
            p_color_attachment_formats: if has_color {
                color_formats.as_ptr()
            } else {
                ptr::null()
            },
            depth_attachment_format,
            stencil_attachment_format: VK_FORMAT_UNDEFINED,
        };

        let gp_info = VkGraphicsPipelineCreateInfo {
            s_type: VkStructureType::GraphicsPipelineCreateInfo,
            // Chain the dynamic-rendering format struct (no render pass, OQ-6).
            p_next: (&rendering_info as *const VkPipelineRenderingCreateInfo).cast(),
            flags: 0,
            stage_count: stages.len() as u32,
            p_stages: stages.as_ptr(),
            p_vertex_input_state: &vertex_input,
            p_input_assembly_state: &input_assembly,
            p_tessellation_state: ptr::null(),
            p_viewport_state: &viewport_state,
            p_rasterization_state: &rasterization,
            p_multisample_state: &multisample,
            p_depth_stencil_state,
            p_color_blend_state,
            p_dynamic_state: &dynamic_state,
            layout,
            // Dynamic rendering: no render pass object (OQ-6, CLOSED).
            render_pass: 0,
            subpass: 0,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };

        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `device` is live; null pipeline cache (`0`) is valid; one
        // fully-initialized `VkGraphicsPipelineCreateInfo` references the live
        // `layout` (which references the `push_range` local for `has_push`, alive for
        // this whole fn), the two live caller-owned shader modules (via `stages`,
        // alive for the call), and the complete set of fixed-function sub-state
        // structs + dynamic-rendering format chain (all stack locals alive for the
        // call). `vertex_input` points at the `vk_bindings`/`vk_attributes` locals
        // when `has_vertex_layout`, whose first `attribute_count` (<= cap, asserted)
        // entries are initialized and bound by the driver's matching counts; an empty
        // layout uses null arrays with count 0. Tessellation state is null; the
        // depth-stencil state is the `depth_state` local (alive for this whole fn)
        // when `desc.depth_format` is `Some`, else null (rungs 1..3). `render_pass`
        // is `VK_NULL_HANDLE` (dynamic rendering). `&mut pipeline` is a valid
        // out-pointer for the single pipeline; NULL allocator.
        //
        // `p_color_blend_state` points at the `color_blend` local whose
        // `p_attachments` is the `blend_attachments` inline array (alive for the call),
        // its first `color_attachment_count` entries (= the format count below) read by
        // the driver — OR is null for the DEPTH-ONLY path (`color_attachment_count == 0`),
        // valid because Vulkan permits a null color-blend state with no color
        // attachments. The dynamic-rendering format chain's `p_color_attachment_formats`
        // is the `color_formats` inline array (alive for the call), first
        // `color_attachment_count` entries valid — OR null for the depth-only path.
        //
        // FORMAT CONTRACT (W2-b): `rendering_info.p_color_attachment_formats` declares
        // `desc.color_formats` (count + per-index format) and `.depth_attachment_format`
        // declares the rung-4 depth format; each MUST equal the same-index format (and
        // the count) of every `begin_rendering` color/depth attachment the pipeline is
        // later bound inside, or validation faults at draw time (not here). The MRT
        // color-blend `attachment_count` equals the format count, so the two never
        // disagree. The agnostic↔Vk discriminant equality is asserted in
        // `abi_guard.rs`; the cross-check against the bound rendering scope is the
        // caller's contract (encoded in `GraphicsPipelineDesc`↔`RenderingDesc`).
        let raw = unsafe {
            (fns.create_graphics_pipelines)(device, 0, 1, &gp_info, ptr::null(), &mut pipeline)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: the `layout` was created above and is not yet owned by any
            // pipeline (creation failed); destroy it exactly once on this error path
            // so it never leaks. NOTE: this single-handle rollback is correct ONLY
            // because `create_info_count == 1` above; a future BATCHED create path must
            // additionally destroy the successfully-created pipelines that
            // `vkCreateGraphicsPipelines` writes alongside VK_NULL_HANDLE on partial
            // failure (per-handle cleanup), or they leak.
            unsafe { (fns.destroy_pipeline_layout)(device, layout, ptr::null()) };
            return Err(VulkanError::Vk("vkCreateGraphicsPipelines", result));
        }

        Ok(VulkanGraphicsPipeline { pipeline, layout })
    }

    /// Textured-PBR T6c (plan Decision D5): builds a 2-set graphics pipeline — set 0 exactly
    /// as `desc.bind_group_layout` declares (the TEXTURED raster's `PerInstanceMaterialTex`
    /// SSBO layout, VERTEX), set 1 = `set1_layout` (the bindless texture-array set's raw
    /// `VkDescriptorSetLayout`, FRAGMENT-visible —
    /// [`crate::bindless::VulkanBindlessSet::set_layout`]). A Vulkan-only, ADDITIVE inherent
    /// method: `boyko_rhi::GraphicsPipelineDesc` itself is UNCHANGED (no new field), so every
    /// one of its existing construction sites across the workspace (other gbuffer/CSM/UI/test
    /// pipelines) needs no edit and takes the untouched [`Self::build_graphics_pipeline`]`(desc,
    /// None)` path via `RhiDevice::create_graphics_pipeline`.
    pub fn create_graphics_pipeline_bindless(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
        set1_layout: VkDescriptorSetLayout,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        self.build_graphics_pipeline(desc, Some(set1_layout), VK_COMPARE_OP_LESS, true)
    }

    /// Multi-paradigm render-path plan, rung R4b-b (Set 0 unified at rung R5): builds plain
    /// `Forward`'s mesh raster pipeline — set 0 exactly as `desc.bind_group_layout` declares
    /// (the UNIFIED Forward-family 7-binding core set: instances/instance_materials/Camera/
    /// LightBuf/Materials/ClusterGrid/LightIndexList — this base FS references only the first 5,
    /// a subset, the SAME idiom `forward_sky_pipeline` uses), set 1 =
    /// `set1_layout` (the CSM + punctual-atlas shadow set, `forward_opaque.fs.hlsl`'s Set 1 —
    /// renumbered from an original Set 2 design: with no bindless texture table this v1 rung,
    /// Set 1 is free, so the pipeline layout is a plain 2-set `[Set0, Set1]`, the SAME shape
    /// [`Self::create_graphics_pipeline_bindless`] builds; a `set2`-with-empty-set1-placeholder
    /// shape was tried first and rejected — `RhiDevice::create_bind_group_layout`'s own
    /// `1..=MAX_BIND_GROUP_BINDINGS` invariant forbids a zero-binding layout, which crashed
    /// `GpuSceneBundles::boot`, `build_graphics_pipeline`'s doc). Depth-tests
    /// `VK_COMPARE_OP_GREATER` (Decision 4: hardware reverse-Z — a nearer fragment has the LARGER
    /// stored depth), unlike every other graphics pipeline in this engine (`VK_COMPARE_OP_LESS`,
    /// Deferred's custom-linear depth). A Vulkan-only, ADDITIVE inherent method (the
    /// [`Self::create_graphics_pipeline_bindless`] precedent): `boyko_rhi::GraphicsPipelineDesc`
    /// itself is UNCHANGED (no new field), so every pre-existing pipeline construction site
    /// across the workspace needs no edit.
    pub fn create_graphics_pipeline_forward(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
        set1_layout: VkDescriptorSetLayout,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        self.build_graphics_pipeline(desc, Some(set1_layout), VK_COMPARE_OP_GREATER, true)
    }

    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus): builds the `depth_prepass`
    /// pipeline — a DEPTH-ONLY pipeline (`desc.color_formats` empty, zero color attachments;
    /// `build_graphics_pipeline`'s existing CSM/atlas depth-only shape, the SAME one
    /// `RhiDevice::create_graphics_pipeline` already builds for the cascade/spot-atlas depth
    /// passes) with set 0 ONLY (`desc.bind_group_layout`, reused from
    /// [`Self::create_graphics_pipeline_forward`]'s own `forward_layout0` — the prepass VS
    /// references only its `instances` binding, a subset of that layout, the SAME
    /// bound-but-unread-subset idiom `forward_sky_pipeline` already relies on) — no set 1.
    /// `VK_COMPARE_OP_GREATER` (Decision 4, hardware reverse-Z) with depth WRITE ON: this pass
    /// is `forward_opaque`'s sole depth producer under `ForwardPlus`, committing the final
    /// per-pixel depth before any color work runs.
    pub fn create_graphics_pipeline_forward_prepass(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        self.build_graphics_pipeline(desc, None, VK_COMPARE_OP_GREATER, true)
    }

    /// Multi-paradigm render-path plan, rung R8: builds the `vb_raster` mesh id-raster pipeline
    /// (Decision 9) — a 1-set pipeline (set 0 ONLY, `desc.bind_group_layout` = `vb_layout0`; its
    /// VS references only `gVbInstances`/the push, a bound-but-unread subset — the SAME idiom
    /// [`Self::create_graphics_pipeline_forward_prepass`] already establishes for its own 1-set
    /// depth-only pipeline). `VK_COMPARE_OP_GREATER` (Decision 4, hardware reverse-Z) with depth
    /// WRITE ON: `vb_raster` is the SOLE depth producer for the VB path (mirrors
    /// `create_graphics_pipeline_forward`'s own reverse-Z contract for `forward_opaque`).
    ///
    /// UNLIKE `create_graphics_pipeline_forward_prepass` this pipeline is NOT depth-only —
    /// `desc.color_formats` carries the `vb_id` `R32G32_UINT` color attachment
    /// (`build_graphics_pipeline` itself is agnostic to color-attachment count; the two builders
    /// differ only in which pass's caller supplies a non-empty `color_formats`) — a SEPARATE,
    /// precisely-named wrapper rather than reusing the prepass one so a reader is never misled
    /// by a "prepass"-named builder producing `vb_raster_pipeline`.
    pub fn create_graphics_pipeline_vb_raster(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        self.build_graphics_pipeline(desc, None, VK_COMPARE_OP_GREATER, true)
    }

    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus): builds the `forward_opaque`
    /// FROXEL pipeline variant — the SAME 2-set `[Set0, Set1]` shape
    /// [`Self::create_graphics_pipeline_forward`] builds (`set1_layout` = the UNCHANGED
    /// CSM/punctual shadow set), but `VK_COMPARE_OP_EQUAL` with depth WRITE OFF (Decision 4's
    /// EQUAL-depth zero-overdraw contract): `depth_prepass` already committed the final depth
    /// this frame, so `forward_opaque` under `ForwardPlus` only TESTS against it — a fragment
    /// survives iff its interpolated depth exactly matches the prepass-written value, letting
    /// hardware early-Z reject every occluded fragment before the froxel-culled inline shade
    /// runs. `desc.bind_group_layout` is the UNIFIED 7-binding `forward_layout0` (declares
    /// `ClusterGrid`/`LightIndexList` at bindings 5/6) — the SAME layout object
    /// [`Self::create_graphics_pipeline_forward`] builds its pipeline against (rung R5
    /// code-review fix: exactly ONE Set-0 layout for the whole Forward family, since Vulkan
    /// treats two structurally-identical-but-distinct `VkDescriptorSetLayout` handles as
    /// pipeline/descriptor-set INCOMPATIBLE — never two separate layout objects for one
    /// descriptor set).
    pub fn create_graphics_pipeline_forward_plus(
        &self,
        desc: &GraphicsPipelineDesc<Vulkan>,
        set1_layout: VkDescriptorSetLayout,
    ) -> Result<VulkanGraphicsPipeline, VulkanError> {
        self.build_graphics_pipeline(desc, Some(set1_layout), VK_COMPARE_OP_EQUAL, false)
    }

    /// Multi-paradigm render-path plan, rung R-SDFFWD: builds a 2-set COMPUTE pipeline — Set 0 =
    /// `desc.bind_group_layout` (REQUIRED; unlike [`RhiDevice::create_compute_pipeline`]'s
    /// optional `None` = device-shared fallback, a 2-set compute pipeline always owns a dedicated
    /// layout), Set 1 = `set1_layout` (a layout built elsewhere and reused verbatim — the SAME
    /// "one physical descriptor set shared by two Vulkan pipelines" idiom
    /// [`Self::create_graphics_pipeline_forward`] already establishes on the graphics side; the
    /// `sdf_forward_march` compute pass's OWN Set 1 is `GBufferScene::forward_layout1`, the
    /// Forward-family shadow set reused verbatim by BOTH the `HAS_MESH` and mesh-less compute
    /// pipeline variants).
    ///
    /// Mirrors [`RhiDevice::create_compute_pipeline`]'s dedicated-layout push-range sizing (the
    /// FULL `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` shared budget, regardless of `desc
    /// .push_constant_bytes`'s own smaller size — a pipeline may use fewer bytes than its layout
    /// declares) and [`Self::build_graphics_pipeline`]'s 2-set `p_set_layouts`/`set_layout_count`
    /// construction, specialized to a single COMPUTE stage (no vertex input / rasterization
    /// state, no specialization constants — this pass's two variants are separate compiled SPIR-V
    /// modules, not one spec-constant-branched module).
    pub fn create_compute_pipeline_forward(
        &self,
        desc: &ComputePipelineDesc<Vulkan>,
        set1_layout: VkDescriptorSetLayout,
    ) -> Result<ComputePipeline, VulkanError> {
        if desc.push_constant_bytes == 0
            || !desc.push_constant_bytes.is_multiple_of(4)
            || desc.push_constant_bytes > COMPUTE_PUSH_CONSTANT_RANGE_BYTES
        {
            return Err(VulkanError::Unsupported(
                "push_constant_bytes must be a multiple of 4 within the shared compute push range",
            ));
        }
        let bgl = desc.bind_group_layout.expect(
            "invariant: create_compute_pipeline_forward always builds a dedicated 2-set layout \
             (Set 0 is required, unlike create_compute_pipeline's optional device-shared fallback)",
        );
        let set_layouts = [bgl.set_layout, set1_layout];
        let push_range = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            offset: 0,
            size: COMPUTE_PUSH_CONSTANT_RANGE_BYTES,
        };
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: VkStructureType::PipelineLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count: 2,
            p_set_layouts: set_layouts.as_ptr(),
            push_constant_range_count: 1,
            p_push_constant_ranges: &push_range,
        };
        let mut pipeline_layout = VkPipelineLayout::NULL;
        // SAFETY: `self.device()` is live; `pl_info` is fully initialized referencing the
        // `set_layouts` local (both the caller's live Set-0 vocabulary layout and the live Set-1
        // shadow layout, alive for this whole fn) + the `push_range` local (alive for this whole
        // fn); `&mut pipeline_layout` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_pipeline_layout)(
                self.device(),
                &pl_info,
                ptr::null(),
                &mut pipeline_layout,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreatePipelineLayout(compute-forward)", result));
        }

        let stage = VkPipelineShaderStageCreateInfo {
            s_type: VkStructureType::PipelineShaderStageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: desc.module.module,
            p_name: desc.entry.as_ptr(),
            p_specialization_info: ptr::null(),
        };
        let cp_info = VkComputePipelineCreateInfo {
            s_type: VkStructureType::ComputePipelineCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage,
            layout: pipeline_layout,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };
        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `self.device()` is live; null pipeline cache (`0`) is valid; one create-info is
        // fully initialized, referencing the live shader module + the just-created dedicated
        // `pipeline_layout`; `&mut pipeline` is a valid out-pointer for the single pipeline; NULL
        // allocator. The module is owned by the caller's `VulkanShaderModule`, alive for this call.
        let raw = unsafe {
            (self.device_fns().create_compute_pipelines)(
                self.device(),
                0,
                1,
                &cp_info,
                ptr::null(),
                &mut pipeline,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `pipeline_layout` was just created on this device above and is not yet
            // owned by any pipeline (this create failed); destroying it once here prevents a leak
            // on this error path.
            unsafe {
                (self.device_fns().destroy_pipeline_layout)(
                    self.device(),
                    pipeline_layout,
                    ptr::null(),
                )
            };
            return Err(VulkanError::from(ComputeError::VkError("vkCreateComputePipelines", result)));
        }
        Ok(ComputePipeline { pipeline, layout: pipeline_layout, owns_layout: true })
    }

    /// Multi-paradigm render-path plan, rung R8: builds a 3-set COMPUTE pipeline for the `vb_resolve`
    /// FUSED pass — Set 0 = `desc.bind_group_layout` (REQUIRED, the VB-only core+images vocabulary,
    /// `vb_layout0`), Set 1 = `set1_layout` (`GBufferScene::forward_layout1`, the Forward-family
    /// shadow set, REUSED VERBATIM — the SAME idiom [`Self::create_compute_pipeline_forward`]
    /// already establishes), Set 2 = `set2_layout` (the Decision-0 geometry table's OWN Set,
    /// `MeshGeometryTable::set().set_layout()`). Otherwise a byte-for-byte mirror of
    /// [`Self::create_compute_pipeline_forward`]'s push-range sizing + pipeline-layout/pipeline
    /// construction, widened from 2 to 3 set layouts.
    pub fn create_compute_pipeline_vb(
        &self,
        desc: &ComputePipelineDesc<Vulkan>,
        set1_layout: VkDescriptorSetLayout,
        set2_layout: VkDescriptorSetLayout,
    ) -> Result<ComputePipeline, VulkanError> {
        if desc.push_constant_bytes == 0
            || !desc.push_constant_bytes.is_multiple_of(4)
            || desc.push_constant_bytes > COMPUTE_PUSH_CONSTANT_RANGE_BYTES
        {
            return Err(VulkanError::Unsupported(
                "push_constant_bytes must be a multiple of 4 within the shared compute push range",
            ));
        }
        let bgl = desc.bind_group_layout.expect(
            "invariant: create_compute_pipeline_vb always builds a dedicated 3-set layout \
             (Set 0 is required, unlike create_compute_pipeline's optional device-shared fallback)",
        );
        let set_layouts = [bgl.set_layout, set1_layout, set2_layout];
        let push_range = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            offset: 0,
            size: COMPUTE_PUSH_CONSTANT_RANGE_BYTES,
        };
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: VkStructureType::PipelineLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count: 3,
            p_set_layouts: set_layouts.as_ptr(),
            push_constant_range_count: 1,
            p_push_constant_ranges: &push_range,
        };
        let mut pipeline_layout = VkPipelineLayout::NULL;
        // SAFETY: `self.device()` is live; `pl_info` is fully initialized referencing the
        // `set_layouts` local (the caller's live Set-0 VB vocabulary layout, the live Set-1
        // shadow layout, and the live Set-2 geometry-table layout, all alive for this whole fn)
        // + the `push_range` local (alive for this whole fn); `&mut pipeline_layout` is a valid
        // out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_pipeline_layout)(
                self.device(),
                &pl_info,
                ptr::null(),
                &mut pipeline_layout,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreatePipelineLayout(compute-vb)", result));
        }

        let stage = VkPipelineShaderStageCreateInfo {
            s_type: VkStructureType::PipelineShaderStageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: desc.module.module,
            p_name: desc.entry.as_ptr(),
            p_specialization_info: ptr::null(),
        };
        let cp_info = VkComputePipelineCreateInfo {
            s_type: VkStructureType::ComputePipelineCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage,
            layout: pipeline_layout,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };
        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `self.device()` is live; null pipeline cache (`0`) is valid; one create-info is
        // fully initialized, referencing the live shader module + the just-created dedicated
        // `pipeline_layout`; `&mut pipeline` is a valid out-pointer for the single pipeline; NULL
        // allocator. The module is owned by the caller's `VulkanShaderModule`, alive for this call.
        let raw = unsafe {
            (self.device_fns().create_compute_pipelines)(
                self.device(),
                0,
                1,
                &cp_info,
                ptr::null(),
                &mut pipeline,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `pipeline_layout` was just created on this device above and is not yet
            // owned by any pipeline (this create failed); destroying it once here prevents a leak
            // on this error path.
            unsafe {
                (self.device_fns().destroy_pipeline_layout)(
                    self.device(),
                    pipeline_layout,
                    ptr::null(),
                )
            };
            return Err(VulkanError::from(ComputeError::VkError("vkCreateComputePipelines", result)));
        }
        Ok(ComputePipeline { pipeline, layout: pipeline_layout, owns_layout: true })
    }

    /// Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3 / `docs/VB-P2-CLASSIFICATION-PLAN.md`):
    /// builds a 4-set COMPUTE pipeline for the `vb_shade` TEXTURED variant (`vb_shade_tex.comp.spv`,
    /// `-D TEXTURED=1`) — Set 0 = `desc.bind_group_layout` (REQUIRED, `vb_layout0`, the SAME layout
    /// object the base `vb_shade`/`vb_resolve` pipelines are built against — R5, a textured frame
    /// binds a DIFFERENT descriptor SET instance against this SAME layout object, never a second
    /// layout), Set 1 = `set1_layout` (`GBufferScene::forward_layout1`, the Forward-family shadow
    /// set, REUSED VERBATIM), Set 2 = `set2_layout` (the Decision-0 geometry table's OWN Set), Set 3
    /// = `set3_layout` (the shared bindless texture-array table — the SAME layout object
    /// `gbuffer_mrt.fs.hlsl`'s TEXTURED variant binds, `BindlessTextureTable::set().set_layout()`).
    /// Otherwise a byte-for-byte mirror of [`Self::create_compute_pipeline_vb`]'s push-range
    /// sizing and pipeline-layout/pipeline construction, widened from 3 to 4 set layouts.
    /// Vulkan's guaranteed `maxBoundDescriptorSets` floor is exactly 4
    /// (`DeviceCaps::max_bound_descriptor_sets`'s own doc — `MeshGeometryTable::new` already
    /// `debug_assert!`s this at construction for every `VisibilityBuffer`-resolved boot, textured
    /// or not), so no additional floor check is needed here.
    pub fn create_compute_pipeline_vb_textured(
        &self,
        desc: &ComputePipelineDesc<Vulkan>,
        set1_layout: VkDescriptorSetLayout,
        set2_layout: VkDescriptorSetLayout,
        set3_layout: VkDescriptorSetLayout,
    ) -> Result<ComputePipeline, VulkanError> {
        if desc.push_constant_bytes == 0
            || !desc.push_constant_bytes.is_multiple_of(4)
            || desc.push_constant_bytes > COMPUTE_PUSH_CONSTANT_RANGE_BYTES
        {
            return Err(VulkanError::Unsupported(
                "push_constant_bytes must be a multiple of 4 within the shared compute push range",
            ));
        }
        let bgl = desc.bind_group_layout.expect(
            "invariant: create_compute_pipeline_vb_textured always builds a dedicated 4-set layout \
             (Set 0 is required, unlike create_compute_pipeline's optional device-shared fallback)",
        );
        let set_layouts = [bgl.set_layout, set1_layout, set2_layout, set3_layout];
        let push_range = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            offset: 0,
            size: COMPUTE_PUSH_CONSTANT_RANGE_BYTES,
        };
        let pl_info = VkPipelineLayoutCreateInfo {
            s_type: VkStructureType::PipelineLayoutCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count: 4,
            p_set_layouts: set_layouts.as_ptr(),
            push_constant_range_count: 1,
            p_push_constant_ranges: &push_range,
        };
        let mut pipeline_layout = VkPipelineLayout::NULL;
        // SAFETY: `self.device()` is live; `pl_info` is fully initialized referencing the
        // `set_layouts` local (the caller's live Set-0 VB vocabulary layout, the live Set-1 shadow
        // layout, the live Set-2 geometry-table layout, and the live Set-3 bindless-texture layout,
        // all alive for this whole fn) + the `push_range` local (alive for this whole fn); `&mut
        // pipeline_layout` is a valid out-pointer; NULL allocator.
        let raw = unsafe {
            (self.device_fns().create_pipeline_layout)(
                self.device(),
                &pl_info,
                ptr::null(),
                &mut pipeline_layout,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreatePipelineLayout(compute-vb-textured)", result));
        }

        let stage = VkPipelineShaderStageCreateInfo {
            s_type: VkStructureType::PipelineShaderStageCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: desc.module.module,
            p_name: desc.entry.as_ptr(),
            p_specialization_info: ptr::null(),
        };
        let cp_info = VkComputePipelineCreateInfo {
            s_type: VkStructureType::ComputePipelineCreateInfo,
            p_next: ptr::null(),
            flags: 0,
            stage,
            layout: pipeline_layout,
            base_pipeline_handle: VkPipeline::NULL,
            base_pipeline_index: -1,
        };
        let mut pipeline = VkPipeline::NULL;
        // SAFETY: `self.device()` is live; null pipeline cache (`0`) is valid; one create-info is
        // fully initialized, referencing the live shader module + the just-created dedicated
        // `pipeline_layout`; `&mut pipeline` is a valid out-pointer for the single pipeline; NULL
        // allocator. The module is owned by the caller's `VulkanShaderModule`, alive for this call.
        let raw = unsafe {
            (self.device_fns().create_compute_pipelines)(
                self.device(),
                0,
                1,
                &cp_info,
                ptr::null(),
                &mut pipeline,
            )
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `pipeline_layout` was just created on this device above and is not yet
            // owned by any pipeline (this create failed); destroying it once here prevents a leak
            // on this error path.
            unsafe {
                (self.device_fns().destroy_pipeline_layout)(
                    self.device(),
                    pipeline_layout,
                    ptr::null(),
                )
            };
            return Err(VulkanError::from(ComputeError::VkError("vkCreateComputePipelines", result)));
        }
        Ok(ComputePipeline { pipeline, layout: pipeline_layout, owns_layout: true })
    }
}
