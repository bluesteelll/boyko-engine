//! T4 — [`BindlessTextureTable`]: the render-side owner of the bindless
//! texture-array descriptor set ([`boyko_rhi_vulkan::bindless::VulkanBindlessSet`]),
//! its free-list slot allocator, and the magenta error texture written into every
//! slot at init.
//!
//! This is the [`GpuUpload`](crate::gpu_upload::GpuUpload)`::Aux` for `TextureGpu` (T2
//! wires `TextureGpu::upload` to call [`BindlessTextureTable::register`]). LIVE wiring
//! (textured-PBR T6b/T6c): `boyko_app::runner` registers this as a
//! [`NonSendResource`](boyko_ecs::ecs::core::resources::resource::NonSendResource) at
//! boot, the TEXTURED raster pipeline binds its descriptor set at set 1, and its
//! fence-gated slot recycle ([`BindlessTextureTable::retire_ready_slots`]) is drained
//! every frame by
//! [`asset_refcount::retire_deferred_frees`](crate::asset_refcount::retire_deferred_frees).
//!
//! # Principle 0 — the free-list `Vec<u32>` is allocator-internal, not gameplay data
//!
//! [`BindlessSlotAllocator`](crate::bindless::BindlessSlotAllocator)'s
//! `free_slots`/`retiring_slots` are `Vec`s of bare
//! `u32` slot indices — NOT a parallel per-entity/per-texture data store (the
//! textures themselves live in `Assets<TextureGpu>`, T2, VM-native). This is the
//! same sanctioned exception as [`RetiredGpuBuffers`](crate::retired_gpu_buffers::RetiredGpuBuffers)'s
//! `entries: Vec<_>` / [`OrphanedMeshGpu`](crate::mesh_assets::OrphanedMeshGpu)'s
//! `orphans: Vec<_>`: a bounded, allocator-internal bookkeeping structure, not a
//! side store of durable per-entity state.

use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_log::codes::{OnceSite, W2202};
use boyko_rhi::enums::{
    BarrierAccess, BarrierStage, Format, ImageAspect, ImageLayout, ImageUsage, TextureDimension,
};
use boyko_rhi::{
    BufferDesc, BufferImageCopy, BufferUsage, ImageBarrierDesc, ImageSubresourceRange,
    MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue, TextureDesc,
};
use boyko_rhi_vulkan::bindless::{
    BINDLESS_IMAGE_BINDING, VulkanBindlessSet, create_bindless_texture_set,
    destroy_bindless_texture_set, write_bindless_texture,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::error::VulkanError;
use boyko_rhi_vulkan::ffi::VkImageView;
use boyko_rhi_vulkan::texture::VulkanTexture;

/// The reserved "no texture" / error slot. `register` never issues it;
/// `BindlessTextureTable::new` writes the magenta error texture here (and into
/// every other slot) before any real registration.
const RESERVED_SLOT: u32 = 0;

/// Opaque magenta — the classic "missing texture" tell. `(255, 0, 255, 255)`.
const ERROR_TEXTURE_RGBA: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];

/// Free-list slot allocator for the bindless texture array, with a fence-gated
/// recycle delay (P1-5).
///
/// Pure CPU bookkeeping — no device handle, no `unsafe` — so it is fully
/// unit-testable without a GPU (see this module's tests). [`BindlessTextureTable`]
/// embeds one and pairs it with the actual device-side descriptor write.
///
/// # Why the recycle delay exists (device-UAF)
///
/// With UPDATE_AFTER_BIND, a slot may be repointed at a NEW texture while an
/// in-flight frame's shader is still reading it through an OLD material that
/// resolved to this same slot index — `PARTIALLY_BOUND` only excuses an
/// UN-ACCESSED descriptor, not a slot an already-recorded (and not yet
/// GPU-complete) command buffer will actually index. So a freed slot is staged in
/// `retiring_slots` with the epoch at which it becomes safe to reuse
/// (`submission_epoch_at_free + RETIRE_DELAY`, the caller's responsibility to
/// stamp — mirrors [`RetiredGpuBuffers::push`](crate::retired_gpu_buffers::RetiredGpuBuffers::push))
/// and is returned to `free_slots` only once [`Self::retire_ready_slots`] observes
/// `epoch >= retire_frame`.
pub struct BindlessSlotAllocator {
    capacity: u32,
    free_slots: Vec<u32>,
    retiring_slots: Vec<(u32, u64)>,
}

impl BindlessSlotAllocator {
    /// Builds an allocator over `[1, capacity)` — slot 0 is permanently reserved
    /// (never enters `free_slots`, so [`Self::register`] can never return it).
    pub fn new(capacity: u32) -> Self {
        debug_assert!(
            capacity > 1,
            "invariant: capacity must reserve slot 0 plus at least one real slot"
        );
        // Descending fill so `Vec::pop` (removes the LAST element) yields slots in
        // ASCENDING order (1, 2, 3, ...) — deterministic and easy to reason about
        // in tests; the actual pop order carries no functional meaning otherwise.
        let free_slots: Vec<u32> = (1..capacity).rev().collect();
        Self {
            capacity,
            free_slots,
            retiring_slots: Vec::new(),
        }
    }

    /// The allocator's declared capacity — every issued/accepted slot satisfies
    /// `slot < capacity()`.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Pops a free slot (`1..capacity`, never `RESERVED_SLOT`), or `None` if the
    /// allocator is exhausted (every real slot is either in use or awaiting its
    /// fence horizon in `retiring_slots`).
    #[inline]
    pub fn register(&mut self) -> Option<u32> {
        self.free_slots.pop()
    }

    /// Stages `slot` for return to the free list once the caller-supplied
    /// `retire_frame` epoch has passed. `retire_frame` MUST already be
    /// `submission_epoch_at_free + RETIRE_DELAY` — this fn does not compute it
    /// (mirrors the F6/F7 `push` contract: the caller owns the epoch arithmetic).
    #[inline]
    pub fn free(&mut self, slot: u32, retire_frame: u64) {
        debug_assert!(
            slot != RESERVED_SLOT && slot < self.capacity,
            "invariant: freed slot must be a real, in-range, non-reserved slot"
        );
        self.retiring_slots.push((slot, retire_frame));
    }

    /// Returns every staged slot whose `retire_frame <= epoch` to the free list,
    /// retaining the rest. `swap_remove` scan (O(1) per removal, order-irrelevant)
    /// — mirrors [`RetiredGpuBuffers::drain_ready`](crate::retired_gpu_buffers::RetiredGpuBuffers::drain_ready)'s
    /// shape exactly, but this fn is device-free: it destroys nothing, only moves
    /// a `u32` between two `Vec`s.
    pub fn retire_ready_slots(&mut self, epoch: u64) {
        let mut i = 0;
        while i < self.retiring_slots.len() {
            if self.retiring_slots[i].1 > epoch {
                i += 1;
                continue;
            }
            let (slot, _) = self.retiring_slots.swap_remove(i);
            self.free_slots.push(slot);
        }
    }

    /// `true` iff nothing is queued for the fence-gated recycle (the golden
    /// early-out counterpart of [`RetiredGpuBuffers::is_empty`](crate::retired_gpu_buffers::RetiredGpuBuffers::is_empty)).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.retiring_slots.is_empty()
    }
}

/// Cold fallback for [`BindlessTextureTable::register`]'s allocator-exhaustion path
/// (practically unreachable at
/// [`boyko_rhi_vulkan::bindless::BINDLESS_TEXTURE_CAPACITY`] — see that fn's doc):
/// `debug_assert!`s the invariant violation and, in release (where the assert
/// compiles out), reports `boyko-W2202` so an exhausted table is visible in the log
/// instead of silently aliasing the error-texture slot with no trace.
///
/// The ad-hoc `"WARN: "` prefix this used to print is gone with the `eprintln!`. It was a
/// hand-written severity on a channel that carried no severity — and `mesh_geometry_table.rs`
/// wrote the same prefix for the same condition, which is why one code covers both sites with the
/// table named as an argument.
#[cold]
fn exhausted_slot_fallback(capacity: u32) -> u32 {
    debug_assert!(false, "invariant: BindlessTextureTable exhausted its {capacity} slots");
    report_bindless_table_exhausted("BindlessTextureTable", capacity, RESERVED_SLOT);
    RESERVED_SLOT
}

/// A `Once` latch is PROCESS state, so it is a named module-level `static` rather than one
/// tucked inside the reporter: an observer must be able to reset it, or its green only means
/// "nothing else in this binary tripped this condition first". See `OnceSite::reset`.
pub(crate) static W2202_SITE: OnceSite = OnceSite::new();

/// Reports `boyko-W2202`: a bindless slot allocator that ran out and aliased its reserved slot.
///
/// **Separate from [`exhausted_slot_fallback`] so it is reachable without the `debug_assert!`.**
/// The assert above is the invariant's gate and fires first in a debug build — which would make
/// any test of the reporting half a `#[should_panic]` that can observe nothing after the panic.
/// Splitting them lets the observer drive *this function*, the one a release build actually runs,
/// instead of re-emitting the same `warn!` beside it and proving only that the macro works.
///
/// Its `static FIRED` is what makes `Once` per SITE: `mesh_geometry_table.rs` shares this code
/// through its own copy of this function, and one exhausted table must not silence the other's
/// first report.
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

/// The render-side bindless texture table (T4): owns the device descriptor set,
/// the magenta error texture, and the fence-gated slot allocator.
///
/// # Device-UAF safety — three structural guards (no validation layer on this box)
///
/// 1. **Bounds**: [`BindlessSlotAllocator`] only ever issues `1..capacity`; every
///    write is `debug_assert!`-checked `< capacity` in
///    [`write_bindless_texture`].
/// 2. **Error texture in every slot**: [`Self::new`] writes
///    `ERROR_TEXTURE_RGBA` into EVERY slot (0 included) before returning, so an
///    unwritten or stale index samples a visibly-wrong magenta texture — never
///    UNDEFINED memory.
/// 3. **Fence-gated recycle (P1-5)**: [`Self::unregister`] does NOT return the
///    slot to the allocator immediately; [`Self::retire_ready_slots`] (called from
///    the host's per-frame retire step —
///    `asset_refcount::retire_deferred_frees`, wired at T6b) only recycles a
///    slot once its fence horizon has passed — see [`BindlessSlotAllocator`]'s
///    docs for the UAF this prevents.
pub struct BindlessTextureTable {
    set: VulkanBindlessSet,
    error_texture: VulkanTexture,
    allocator: BindlessSlotAllocator,
}

impl NonSendResource for BindlessTextureTable {}

impl BindlessTextureTable {
    /// Creates the bindless descriptor set (layout, UPDATE_AFTER_BIND pool, set,
    /// shared sampler), builds the 2x2 magenta error texture, and writes it into
    /// every slot (including `RESERVED_SLOT`) — the load-time, one-shot setup
    /// cost (Principle 1: `register`/`unregister` are load-time/rare, not
    /// per-frame; this constructor runs once).
    pub fn new(ctx: &VulkanContext) -> Result<Self, VulkanError> {
        let set = create_bindless_texture_set(ctx)?;
        let error_texture = match create_solid_color_texture(ctx, 2, 2, ERROR_TEXTURE_RGBA) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: `set` was just created on `ctx`, owned exclusively here,
                // never bound to any in-flight submission; destroy it once on this
                // edge.
                unsafe { destroy_bindless_texture_set(ctx, set) };
                return Err(e);
            }
        };

        let capacity = set.capacity();
        for slot in 0..capacity {
            // SAFETY: `slot < capacity == set.capacity()` (the loop bound);
            // `error_texture.view()` is a live view in `ShaderReadOnlyOptimal`
            // (`create_solid_color_texture`'s upload transitions it before
            // returning) that outlives this whole table (owned by `self` below);
            // the set was just allocated and is not yet bound to any command
            // buffer, so there is no in-flight reader to race.
            unsafe {
                write_bindless_texture(ctx, &set, BINDLESS_IMAGE_BINDING, slot, error_texture.view());
            }
        }

        let allocator = BindlessSlotAllocator::new(capacity);
        Ok(Self {
            set,
            error_texture,
            allocator,
        })
    }

    /// The table's declared capacity.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.allocator.capacity()
    }

    /// The owned descriptor set — bound at set 1 by the TEXTURED gbuffer raster
    /// pass (wired at T6c; see `gpu_scene`'s textured pipeline record path).
    #[inline]
    pub fn set(&self) -> &VulkanBindlessSet {
        &self.set
    }

    /// Allocates a slot and writes `image_view` into it, returning the slot index
    /// (a material's `tex == 0` then means "no texture" — see `RESERVED_SLOT`).
    ///
    /// On allocator exhaustion (every one of `capacity - 1` real slots in use or
    /// awaiting its fence horizon — practically unreachable at
    /// [`boyko_rhi_vulkan::bindless::BINDLESS_TEXTURE_CAPACITY`]), this is an
    /// engine invariant violation (`debug_assert!`); the release-safe fallback
    /// `exhausted_slot_fallback` logs a warning and aliases `RESERVED_SLOT`
    /// (the error texture) rather than issue an out-of-range write.
    pub fn register(&mut self, ctx: &VulkanContext, image_view: VkImageView) -> u32 {
        let slot = self
            .allocator
            .register()
            .unwrap_or_else(|| exhausted_slot_fallback(self.allocator.capacity()));
        if slot != RESERVED_SLOT {
            // SAFETY: `slot < self.allocator.capacity() == self.set.capacity()`
            // (the allocator only ever issues `1..capacity`); `image_view` is the
            // caller's contract (this fn's own doc: the caller supplies a live
            // view in `ShaderReadOnlyOptimal` that outlives every submission
            // sampling this slot until the matching `unregister`); this is a
            // freshly-allocated slot with no prior in-flight reference — the
            // fence-gated recycle only applies to a REUSED slot.
            unsafe {
                write_bindless_texture(ctx, &self.set, BINDLESS_IMAGE_BINDING, slot, image_view);
            }
        }
        slot
    }

    /// Stages `slot` for return to the free list once `retire_frame` has passed
    /// (P1-5). `retire_frame` MUST be `submission_epoch_at_free + RETIRE_DELAY`
    /// (the caller's responsibility — mirrors
    /// [`RetiredGpuBuffers::push`](crate::retired_gpu_buffers::RetiredGpuBuffers::push)).
    /// Does NOT touch the device: the slot's old descriptor write is left in
    /// place (still sampling the about-to-be-freed texture) until a NEW
    /// [`Self::register`] eventually overwrites it — the texture's own GPU
    /// resources are torn down separately by its owner (`Assets<TextureGpu>`, T2).
    #[inline]
    pub fn unregister(&mut self, slot: u32, retire_frame: u64) {
        self.allocator.free(slot, retire_frame);
    }

    /// Drains every slot whose fence horizon has passed back to the free list
    /// (P1-5). Device-free — see [`BindlessSlotAllocator::retire_ready_slots`].
    /// Wired (textured-PBR T6b) into the host's per-frame F6-style retire step:
    /// [`retire_deferred_frees`](crate::asset_refcount::retire_deferred_frees) calls this
    /// every frame, guarded on this table's presence (a failed
    /// [`BindlessTextureTable::new`] boot step never inserts it).
    #[inline]
    pub fn retire_ready_slots(&mut self, epoch: u64) {
        self.allocator.retire_ready_slots(epoch);
    }

    /// `true` iff no slot is awaiting its fence horizon.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.allocator.is_empty()
    }

    /// Tears down every owned device resource: the error texture, then the
    /// descriptor set (pool → layout → sampler). Waits for the device to go idle
    /// first (belt-and-braces, mirrors `UiRenderResources::destroy`) —
    /// callers that already know no in-flight submission touches this table's set
    /// (T6's future teardown, after its own `wait_idle`) still get a sound
    /// idempotent-cost `wait_idle` here.
    pub fn destroy(self, ctx: &VulkanContext) {
        let _ = ctx.wait_idle();
        // SAFETY: the device was just drained (`wait_idle` above), so no
        // submission references `self.error_texture` or `self.set`; each is owned
        // exclusively here and moved by value ⇒ destroyed exactly once.
        unsafe {
            ctx.destroy_texture(self.error_texture);
            destroy_bindless_texture_set(ctx, self.set);
        }
    }
}

/// Builds a `w`x`h` `R8G8B8A8_UNORM` `SAMPLED | TRANSFER_DST` texture, staged-fills
/// it with `rgba` repeated across every texel, and barriers it to
/// `ShaderReadOnlyOptimal` — the same single-fenced-submit staged-upload shape as
/// `UiRenderResources::create_atlas`'s `upload_atlas_pixels`,
/// generalized to an arbitrary small solid-color fill. Used by
/// [`BindlessTextureTable::new`] for the 2x2 magenta error texture; exposed `pub`
/// so the `#[ignore]` bindless integration test can build its own tiny test
/// textures without duplicating the staged-upload boilerplate.
///
/// On any partial failure every object created so far is torn down before the
/// error returns (no leak).
pub fn create_solid_color_texture(
    ctx: &VulkanContext,
    w: u32,
    h: u32,
    rgba: [u8; 4],
) -> Result<VulkanTexture, VulkanError> {
    debug_assert!(w > 0 && h > 0, "invariant: texture extent is non-zero");
    let pixel_count = (w as usize) * (h as usize);
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    for _ in 0..pixel_count {
        pixels.extend_from_slice(&rgba);
    }
    create_rgba_texture(ctx, w, h, &pixels)
}

/// Builds a `w`×`h` `R8G8B8A8_UNORM` `SAMPLED | TRANSFER_DST` texture from
/// TIGHTLY-PACKED straight-RGBA8 `pixels` (`w * h * 4` bytes) and barriers it to
/// `ShaderReadOnlyOptimal` — the general form of [`create_solid_color_texture`], which is
/// now one caller of it.
///
/// # Why a raw-bytes entry point exists (`docs/UI-PLAN-SPRITES.md` S-D5)
///
/// Every UI sprite gate builds its texture IN RUST — an 8×8 checkerboard, a 3×3
/// nine-slice source, a 4×4 flipbook grid — because `boyko_image` is a DECODER only (there
/// is no encoder in this tree), so a checked-in PNG could not be regenerated by anything
/// the repo owns, and a procedural texture is bit-reproducible, which is what an image pin
/// needs. A solid fill cannot express any of those three, so the shape the plan calls "the
/// `create_solid_color_texture`-shaped path" is this function.
///
/// # Errors
/// [`VulkanError`] on a texture / staging-buffer / encoder / fence create or submit
/// failure. On any partial failure every object created so far is torn down (no leak).
///
/// # Panics
/// If `pixels.len() != w * h * 4` (a mismatch would make the driver read past the staging
/// allocation — the same class of defect the 2026-07 audit found in the atlas upload, and
/// for the same reason this is a real check rather than a `debug_assert!`).
pub fn create_rgba_texture(
    ctx: &VulkanContext,
    w: u32,
    h: u32,
    pixels: &[u8],
) -> Result<VulkanTexture, VulkanError> {
    debug_assert!(w > 0 && h > 0, "invariant: texture extent is non-zero");
    assert_eq!(
        pixels.len() as u64,
        (w as u64) * (h as u64) * 4,
        "invariant: tightly-packed RGBA8 (pixels.len() == w * h * 4); a mismatch makes \
         vkCmdCopyBufferToImage read past the staging allocation"
    );

    let texture = ctx.create_texture(&TextureDesc {
        width: w,
        height: h,
        depth: 1,
        format: Format::R8G8B8A8Unorm,
        dimension: TextureDimension::D2,
        usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
        array_layers: 1,
        mip_levels: 1,
        view_format: None,
    })?;

    if let Err(e) = upload_solid_pixels(ctx, &texture, w, h, pixels) {
        // SAFETY: `texture` was just created on `ctx`, owned exclusively here,
        // and the upload's own submit (if any ran) is fence-waited internally
        // before this error could surface; destroy it once on this edge.
        unsafe { ctx.destroy_texture(texture) };
        return Err(e);
    }
    Ok(texture)
}

/// The staged upload + layout-transition submit for [`create_solid_color_texture`]
/// — mirrors `boyko_render::ui::resources::UiRenderResources::upload_atlas_pixels`
/// exactly (staging buffer → `copy_buffer_to_image` → UNDEFINED→TRANSFER_DST→
/// SHADER_READ_ONLY barriers → one fenced submit).
fn upload_solid_pixels(
    ctx: &VulkanContext,
    texture: &VulkanTexture,
    w: u32,
    h: u32,
    pixels: &[u8],
) -> Result<(), VulkanError> {
    let size = pixels.len() as u64;
    let staging = ctx.create_buffer(&BufferDesc {
        size,
        usage: BufferUsage::TRANSFER_SRC,
        location: MemoryLocation::HostVisibleCoherent,
    })?;
    let Some(dst) = ctx.buffer_mapped_ptr(&staging) else {
        // SAFETY: `staging` was just created, never submitted; destroy it once on
        // this edge.
        unsafe { ctx.destroy_buffer(staging) };
        return Err(VulkanError::Unsupported("staging buffer not host-mapped"));
    };
    // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent
    // staging buffer (exactly `size` bytes, just created); `pixels` is a
    // distinct, non-overlapping allocation of `size` bytes; this is the unique
    // writer before any submission binds the buffer. Host-coherent ⇒ no flush.
    unsafe {
        core::ptr::copy_nonoverlapping(pixels.as_ptr(), dst.as_ptr(), pixels.len());
    }

    let mut encoder = match ctx.create_command_encoder() {
        Ok(e) => e,
        Err(e) => {
            // SAFETY: `staging` was just created, never submitted; destroy once.
            unsafe { ctx.destroy_buffer(staging) };
            return Err(e);
        }
    };
    let fence = match ctx.create_fence(false) {
        Ok(f) => f,
        Err(e) => {
            // SAFETY: `encoder`/`staging` were just created, never submitted;
            // destroy each once.
            unsafe {
                ctx.destroy_command_encoder(encoder);
                ctx.destroy_buffer(staging);
            }
            return Err(e);
        }
    };

    let region = [BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: w,
        image_extent_h: h,
        image_extent_d: 1,
    }];

    let record = (|| -> Result<(), VulkanError> {
        encoder.begin()?;
        // UNDEFINED → TRANSFER_DST_OPTIMAL (the copy destination).
        encoder.image_barrier(&ImageBarrierDesc {
            texture,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::TRANSFER,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::TRANSFER_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::TransferDstOptimal,
            range: ImageSubresourceRange::COLOR,
        });
        encoder.copy_buffer_to_image(&staging, texture, ImageLayout::TransferDstOptimal, &region);
        // TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL (sample-ready).
        encoder.image_barrier(&ImageBarrierDesc {
            texture,
            src_stage: BarrierStage::TRANSFER,
            dst_stage: BarrierStage::FRAGMENT_SHADER,
            src_access: BarrierAccess::TRANSFER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
            old_layout: ImageLayout::TransferDstOptimal,
            new_layout: ImageLayout::ShaderReadOnlyOptimal,
            range: ImageSubresourceRange::COLOR,
        });
        encoder.end()?;
        let queue = ctx.rhi_queue();
        queue.submit(&encoder, &fence)?;
        ctx.wait_fence(&fence, u64::MAX)?;
        Ok(())
    })();

    // Tear down the setup-class transients. The submit (if it ran) is
    // fence-waited above.
    // SAFETY: `encoder`/`fence`/`staging` were created on `ctx`; the encoder's
    // only submission (if any) completed (fence-waited above on the Ok path, or
    // never submitted on an error path), and each is moved by value ⇒ destroyed
    // once.
    unsafe {
        ctx.destroy_command_encoder(encoder);
        ctx.destroy_fence(fence);
        ctx.destroy_buffer(staging);
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    // ════════════════════════════════════════════════════════════════════
    // `BindlessSlotAllocator` — pure CPU, device-free (the F6 `SelectionModel`
    // precedent's simpler cousin: here the REAL method IS the testable unit, no
    // oracle-model extraction needed, since `retire_ready_slots` destroys nothing).
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn new_allocator_never_issues_the_reserved_slot() {
        let mut a = BindlessSlotAllocator::new(8);
        let mut seen = Vec::new();
        while let Some(slot) = a.register() {
            seen.push(slot);
        }
        assert_eq!(seen.len(), 7, "capacity 8 minus the reserved slot 0 = 7 real slots");
        assert!(
            !seen.contains(&RESERVED_SLOT),
            "slot 0 must never be issued by register()"
        );
        assert!(
            seen.iter().all(|&s| s < 8),
            "every issued slot must be < capacity"
        );
    }

    #[test]
    fn register_issues_slots_in_ascending_order() {
        let mut a = BindlessSlotAllocator::new(5);
        assert_eq!(a.register(), Some(1));
        assert_eq!(a.register(), Some(2));
        assert_eq!(a.register(), Some(3));
        assert_eq!(a.register(), Some(4));
        assert_eq!(a.register(), None, "capacity 5 has exactly 4 real slots");
    }

    #[test]
    fn freed_slot_does_not_return_before_its_retire_frame() {
        let mut a = BindlessSlotAllocator::new(4);
        let s1 = a.register().expect("slot available");
        let s2 = a.register().expect("slot available");
        assert_eq!(a.register(), Some(3), "the 3rd of 3 real slots (capacity 4)");
        assert_eq!(a.register(), None, "exhausted");

        a.free(s1, /* retire_frame */ 10);
        assert!(!a.is_empty(), "a freed slot must be staged, not vanish");

        // Below the horizon: must not return.
        a.retire_ready_slots(5);
        assert_eq!(a.register(), None, "s1 is not yet past its retire_frame");

        // At the horizon (inclusive boundary): must return.
        a.retire_ready_slots(10);
        assert_eq!(a.register(), Some(s1), "s1 must recycle exactly at epoch == retire_frame");
        assert!(a.is_empty());

        // A second free/retire cycle on a different slot, well past its horizon.
        a.free(s2, 20);
        a.retire_ready_slots(999);
        assert_eq!(a.register(), Some(s2), "epoch > retire_frame must still recycle");
    }

    #[test]
    fn retire_ready_slots_on_an_empty_queue_is_a_noop() {
        let mut a = BindlessSlotAllocator::new(4);
        a.retire_ready_slots(1_000_000);
        assert!(a.is_empty());
        assert_eq!(a.register(), Some(1), "an untouched allocator still issues real slots");
    }

    #[test]
    fn mixed_horizons_recycle_only_the_ready_subset() {
        let mut a = BindlessSlotAllocator::new(16);
        // Drain every real slot (1..16 => 15 slots) so `register()` afterward can
        // only return a value that came back through the recycle path.
        let mut issued = Vec::new();
        while let Some(s) = a.register() {
            issued.push(s);
        }
        assert_eq!(issued.len(), 15);

        // Free three slots at three different horizons.
        a.free(issued[0], 5);
        a.free(issued[1], 20);
        a.free(issued[2], 5);

        a.retire_ready_slots(5);
        let mut recycled = Vec::new();
        while let Some(s) = a.register() {
            recycled.push(s);
        }
        recycled.sort_unstable();
        let mut expected = vec![issued[0], issued[2]];
        expected.sort_unstable();
        assert_eq!(
            recycled, expected,
            "only the two retire_frame<=5 slots must recycle; the retire_frame=20 slot stays queued"
        );

        a.retire_ready_slots(20);
        assert_eq!(a.register(), Some(issued[1]), "the remaining slot recycles once its horizon passes");
    }

    /// A tiny deterministic xorshift32 PRNG (mirrors
    /// `retired_gpu_buffers.rs`'s test idiom — no new dev-dependency).
    struct Xorshift32(u32);
    impl Xorshift32 {
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    /// Property: across many random register/free/retire sequences, every issued
    /// slot is always `< capacity` and never [`RESERVED_SLOT`], and a slot is
    /// never issued twice while still "in use" (i.e. between a `register`/`free`
    /// pair, before its `retire_frame` epoch is reached).
    #[test]
    fn property_every_issued_slot_is_in_range_and_never_double_issued_while_live() {
        let mut rng = Xorshift32(0xB16B_00B5);
        const CAPACITY: u32 = 32;
        for trial in 0..256 {
            let mut a = BindlessSlotAllocator::new(CAPACITY);
            let mut live: Vec<u32> = Vec::new();
            let mut epoch: u64 = 0;

            for _ in 0..64 {
                match rng.next_u32() % 3 {
                    0 => {
                        if let Some(slot) = a.register() {
                            assert!(slot < CAPACITY && slot != RESERVED_SLOT, "trial {trial}");
                            assert!(!live.contains(&slot), "trial {trial}: double-issued {slot}");
                            live.push(slot);
                        }
                    }
                    1 => {
                        if !live.is_empty() {
                            let idx = (rng.next_u32() as usize) % live.len();
                            let slot = live.swap_remove(idx);
                            a.free(slot, epoch + 2);
                        }
                    }
                    _ => {
                        epoch += 1;
                        a.retire_ready_slots(epoch);
                    }
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
    fn w2202_reports_the_exhausted_table_once_and_names_which_one() {
        // The sibling emitter in `mesh_geometry_table` shares this code. What must NOT happen is
        // that one exhausted table silences the other's first report, which is what a code-scoped
        // latch would have done -- so each site owns an `OnceSite` and this test pins the first
        // of the two. The `debug_assert!` in the fallback fires before the emission in a debug
        // build, so the reporter is exercised here through a direct call, exactly as the
        // production `register` path reaches it.
        arm();
        W2202_SITE.reset();

        watch(b'W', W2202.number());
        report_bindless_table_exhausted("BindlessTextureTable", 4096, RESERVED_SLOT);
        assert_eq!(watched(), 1, "the first exhaustion reports");

        watch(b'W', W2202.number());
        report_bindless_table_exhausted("BindlessTextureTable", 4096, RESERVED_SLOT);
        assert_eq!(watched(), 0, "later exhaustions are silent at the same site");
    }
}
