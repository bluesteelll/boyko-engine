//! Token-typed per-slot ring uploads (host plan R3/R4, the "WHAT to upload"
//! side of the D1 layering).
//!
//! Every per-frame host write into slot-indexed mapped GPU memory races the
//! slot's PREVIOUS OCCUPANT (frame N−2 under `FRAMES_IN_FLIGHT == 2`) unless
//! the slot's in-flight fence was waited first — the `80bf033` motion-shadow
//! race class. These upload fns therefore demand a borrowed
//! [`FrameWriteToken`], mintable ONLY by `Renderer::wait_frame_in_flight`
//! (or the audited `forge_unfenced` setup hatch): the fence proof is a
//! compile-time precondition, not a convention. The caller (the `boyko_app`
//! runner — the "WHEN" side) selects `ring[token.slot()]` and passes that slot
//! buffer here.
//!
//! # Why these are `unsafe fn` (review P1)
//!
//! [`BoundBuffer`]'s fields are public, so SAFE code can construct one whose
//! `mapped` dangles or whose `size` overstates the mapping — a safe fn writing
//! through it would be unsound by definition. The memory precondition (a live
//! host-visible mapping of at least `size` bytes) is therefore an explicit
//! `# Safety` contract, discharged trivially by the intended callers (slots
//! minted by `RhiDevice::create_buffer` and owned by the host's scene bundles).

use boyko_rhi_vulkan::compute::{
    B5_CAMERA_UBO_BYTES_M4, EDITLIST_BUFFER_WORDS, M2_GRID_PARAMS_OFFSET, encode_edit_list,
};
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::swapchain::FrameWriteToken;
use boyko_scene::ViewUniform;
use boyko_sdf_math::SdfEdit;

use crate::aa_config::{RESOLVED_TAA_BYTES, ResolvedTaa};
use crate::csm_config::{RESOLVED_CSM_BYTES, ResolvedCsm};
use crate::ray_shadow_config::{RESOLVED_RAY_SHADOW_BYTES, ResolvedRayShadow};
use crate::shadow_atlas::{RESOLVED_SHADOW_ATLAS_BYTES, ResolvedShadowAtlas};
use crate::shadow_denoise_config::{
    RESOLVED_SHADOW_DENOISE_BYTES, RESOLVED_TEMPORAL_SHADOW_BYTES, ResolvedShadowDenoise,
    ResolvedTemporalShadow,
};
use crate::gpu_transform3d::GPU_TRANSFORM3D_BYTES;
use crate::mesh_draw::{
    MeshRenderScratch, PER_INSTANCE_MATERIAL_BYTES, PER_INSTANCE_MATERIAL_TEX_BYTES,
};
// TAA W3: un-walled from `hwrt` — the resolve's camera-only MV reconstruction needs the
// MotionCam ring upload on BOTH legs (see `boyko_render::motion_cam`'s module doc).
use crate::motion_cam::{MOTION_CAM_UBO_BYTES, MotionCam};
use crate::view::composite_from_view_sheared;

/// [`upload_camera_ring`] with an optional TAA rung-C1 b5 camera-basis shear
/// ([`composite_from_view_sheared`]) — the SAME `(jx, jy)`
/// [`crate::taa_jitter::NdcJitter`] the raster gbuffer push already applies, so the
/// marcher/resolve/SSAO/CSM/froxel-shared basis samples the identical final-NDC sub-pixel
/// position (I2) when the caller opts in via `TaaConfig::jitter_scope ==
/// JitterScope::RasterAndBasis`. `ndc_jitter == None` is byte-identical to
/// [`upload_camera_ring`] (a structural skip — see
/// [`composite_perspective_from_view_sheared`](crate::view::composite_perspective_from_view_sheared)'s
/// doc for why a computed `Some([0.0, 0.0])` is NOT an equivalent substitute).
///
/// `width`/`height` are the COMPOSITE extent (boot-fixed, plan D7): they size
/// the push's `count = width * height` dispatch bound AND its aspect lane, so
/// they MUST equal the extent the marcher dispatches at (the same extent
/// [`gbuffer_push_from_view`](crate::view::gbuffer_push_from_view) derives the
/// raster aspect from — both pushes are extent-derived by construction). Only
/// the leading 80 bytes are written; the slot's M4 grid-params tail (brick
/// clip-map origins) is left as seeded — bound-but-unread while brick is OFF.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than the full M4-sized b5 UBO:
/// writing into an undersized mapping would be out-of-bounds (UB), so the
/// guard is a hard assert in every build (one compare per frame).
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by
///   `RhiDevice::create_buffer` (`HostVisibleCoherent`) and not yet destroyed:
///   its `mapped` pointer targets at least `ring_slot.size` valid,
///   persistently-mapped bytes. A hand-built `BoundBuffer` with a dangling /
///   undersized `mapped` violates this.
/// * `ring_slot` is the FENCED slot's buffer — `camera_ring[token.slot()]`:
///   the token proves that slot's in-flight fence was waited THIS frame, so
///   the slot's previous occupant finished every GPU read of this buffer (the
///   sibling in-flight frame binds the OTHER slot). Passing a different slot's
///   buffer re-opens the `80bf033` write-after-read race.
pub unsafe fn upload_camera_ring_sheared(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    view: &ViewUniform,
    width: u32,
    height: u32,
    ndc_jitter: Option<[f32; 2]>,
) {
    // The borrow IS the fence proof (mint-gated); no slot index is re-derivable
    // from the buffer, so the slot identity is the caller's contract.
    let _ = token;

    let pc = composite_from_view_sheared(view, width, height, ndc_jitter);
    let bytes = pc.as_bytes();
    debug_assert_eq!(
        bytes.len(),
        M2_GRID_PARAMS_OFFSET,
        "invariant: the b5 camera block is exactly 80 bytes"
    );
    debug_assert_eq!(
        pc.count,
        width * height,
        "invariant: the camera push count covers the composite extent"
    );
    // Hard bound (review P1): an undersized slot would make the memcpy below
    // out-of-bounds — corrupting a neighbouring sub-allocation, not merely
    // rendering wrong. One compare per frame.
    assert!(
        ring_slot.size as usize >= B5_CAMERA_UBO_BYTES_M4,
        "camera ring slot too small: {} bytes < the {}-byte b5 UBO",
        ring_slot.size,
        B5_CAMERA_UBO_BYTES_M4
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the camera ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `ring_slot.size`
    // valid mapped host-coherent bytes, and `ring_slot.size >= 224` is
    // hard-asserted above — the 80-byte write at offset 0 is in-bounds. The
    // borrowed `FrameWriteToken` + the slot-identity contract prove this
    // slot's in-flight fence was waited THIS frame, so the slot's previous
    // occupant (frame N−2) finished every GPU read of this buffer and the
    // sibling in-flight frame binds the OTHER ring slot — the write overlaps
    // no GPU read (race-free, lock-free). `bytes` is a distinct stack-struct
    // view (no overlap).
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Writes the 80-byte b5 camera block (the marcher / resolve / SSAO
/// `CompositePushConstants` image) into ONE camera-ring slot from the resolved
/// engine view — the per-frame camera upload of the production G-buffer path.
///
/// Delegates to [`upload_camera_ring_sheared`] with `ndc_jitter = None` — byte-identical to
/// the pre-C1-lift upload (the structural skip; see that fn's doc).
///
/// # Panics
///
/// Same as [`upload_camera_ring_sheared`].
///
/// # Safety
///
/// Same contract as [`upload_camera_ring_sheared`].
pub unsafe fn upload_camera_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    view: &ViewUniform,
    width: u32,
    height: u32,
) {
    // SAFETY: forwards this fn's own preconditions verbatim to
    // `upload_camera_ring_sheared` — `token`/`ring_slot`/`view` are unchanged, and `None`
    // selects the byte-identical unsheared path (mirrors `composite_from_view`'s own
    // `_sheared(..., None)` delegation).
    unsafe {
        upload_camera_ring_sheared(token, ring_slot, view, width, height, None);
    }
}

/// Uploads the gathered 48-byte [`InstanceModelCol`](crate::InstanceModelCol)
/// UNIFIED instance ring (host plan R3/R5, refined-B) into ONE instance-SSBO ring
/// slot: ONE contiguous `bytemuck` memcpy, zero staging, zero allocation (plan P2-1).
///
/// The ring holds EVERY drawable (static + interpolated). Static rows carry their real
/// affines; interpolated rows carry placeholder bytes that the interp compute
/// overwrites on-GPU (via the out-slot lane) BEFORE the raster VS reads — the
/// data-race note: the CPU writes the whole ring on the FENCED slot, the compute
/// touches only the dynamic slots, and the static slots are never in `OutSlot` (so
/// never GPU-touched), single-writer-per-slot.
///
/// UNCONDITIONAL every frame (plan D5): correct by construction beats
/// probability-correct — no dirty gating, no fingerprints. An empty gather
/// (`scratch.ring` empty ⇒ `batches` empty ⇒ the recorder takes the legacy
/// draw) writes nothing.
///
/// # Panics
///
/// Panics if the gathered ring exceeds the slot's capacity: writing past the
/// mapped range would corrupt neighbouring sub-allocations (UB), so the guard
/// is a hard assert in every build. The boot capacity is the host's documented
/// initial instance budget; growth is a host (R7) concern.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by
///   `RhiDevice::create_buffer` (`HostVisibleCoherent`) and not yet destroyed:
///   its `mapped` pointer targets at least `ring_slot.size` valid,
///   persistently-mapped bytes. A hand-built `BoundBuffer` with a dangling /
///   undersized `mapped` violates this.
/// * `ring_slot` is the FENCED slot's buffer — `instance_ring[token.slot()]`
///   (the same token/slot contract as [`upload_camera_ring`]).
pub unsafe fn upload_instance_models(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.ring.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.ring.as_read_slice());
    assert!(
        bytes.len() as u64 <= ring_slot.size,
        "instance ring overflow: {} gathered instances ({} bytes) exceed the \
         {}-instance ({}-byte) slot (asset-streaming plan F7: \
         `GpuSceneBundles::grow_instance_family_if_needed` grows this ring on a non-RT \
         device before this call; a live overflow here means either an RT device's hard \
         INSTANCE_CAPACITY cap (F7 W3 — growth is out of scope there) or a caller-ordering \
         bug that skipped the grow)",
        scratch.ring.len(),
        bytes.len(),
        ring_slot.size / 48,
        ring_slot.size
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the instance ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `ring_slot.size`
    // valid mapped host-coherent bytes, and `bytes.len() <= ring_slot.size` is
    // hard-asserted above — the write is in-bounds. The borrowed
    // `FrameWriteToken` + the slot-identity contract prove this slot's
    // in-flight fence was waited THIS frame (the slot's previous occupant
    // finished its GPU reads; the sibling frame binds the other slot) —
    // race-free, lock-free. `bytes` is the scratch's own heap buffer, a
    // distinct non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Multi-paradigm render-path plan, rung R8 (Decision 0): uploads the gathered 64-byte
/// [`VbInstanceRow`](crate::instance_model::VbInstanceRow) VB-path instance ring
/// ([`MeshRenderScratch::vb_ring`], built by
/// [`MeshRenderScratch::sync_vb_instance_ring`](crate::mesh_draw::MeshRenderScratch::sync_vb_instance_ring))
/// into ONE VB instance-SSBO ring slot — mirrors [`upload_instance_models`] exactly (ONE
/// contiguous `bytemuck` memcpy, zero staging, zero allocation), against a DEDICATED ring
/// (distinct from the 48-byte `InstanceModelCol` ring [`upload_instance_models`] targets).
///
/// Called ONLY on a `VisibilityBuffer`-resolved boot (the caller's own gate, `boyko_app::runner`
/// — this fn is unconditionally correct either way, the SAME "call-site decides" discipline
/// [`MeshRenderScratch::sync_vb_instance_ring`]'s doc states).
///
/// # Panics
///
/// Panics if the gathered ring exceeds the slot's capacity — the SAME hard overflow discipline
/// as [`upload_instance_models`].
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — `vb_instance_rings[token.slot()]` (the same
///   token/slot contract as [`upload_instance_models`]).
pub unsafe fn upload_vb_instance_rows(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.vb_ring.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.vb_ring.as_read_slice());
    assert!(
        bytes.len() as u64 <= ring_slot.size,
        "VB instance ring overflow: {} gathered instances ({} bytes) exceed the \
         {}-instance ({}-byte) slot (rung R8 v1 has no growth-past-INSTANCE_CAPACITY support \
         for the VB ring yet — mirrors the pre-F7 `instance_rings` state)",
        scratch.vb_ring.len(),
        bytes.len(),
        ring_slot.size / 64,
        ring_slot.size
    );

    let mapped =
        ring_slot.mapped.expect("invariant: the VB instance ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `ring_slot.size` valid mapped
    // host-coherent bytes, and `bytes.len() <= ring_slot.size` is hard-asserted above — the
    // write is in-bounds. The borrowed `FrameWriteToken` + the slot-identity contract prove
    // this slot's in-flight fence was waited THIS frame (race-free, lock-free, the SAME
    // reasoning as `upload_instance_models`). `bytes` is the scratch's own buffer, a distinct
    // non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Asset-streaming plan F8+ (owner: material-drives-albedo-too): uploads the gathered
/// per-instance [`MeshRenderScratch::material_ids`] lane (id + `base_color`, a
/// [`PerInstanceMaterial`](crate::mesh_draw::PerInstanceMaterial) per instance) into ONE
/// instance-material-SSBO ring slot — ONE contiguous `bytemuck` memcpy, zero staging,
/// zero allocation (mirrors [`upload_instance_models`]'s discipline exactly).
///
/// Called ONLY on a frame with [`MeshRenderScratch::any_non_default_material`] (Principle 1 —
/// a default frame does ZERO material-upload work); the caller (the runner) is responsible for
/// that gate. The lane is index-aligned with the instance ring (`instance_materials[i]` names
/// the SAME instance `instances[i]` does), so it shares the instance ring's growth/cap
/// discipline (asset-streaming plan F8 §1.2): grown in lockstep by
/// `GpuSceneBundles::grow_instance_family_if_needed` on the non-RT leg, hard-capped at
/// `INSTANCE_CAPACITY` on an RT device (the F7 W3 gate).
///
/// # Panics
///
/// Panics if the gathered lane exceeds the slot's capacity — the SAME hard overflow discipline
/// as [`upload_instance_models`] (the F7 C3 discipline: a live overflow on an RT device's hard
/// cap must ABORT, never OOB-write).
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — `pm_instance_material_rings[token.slot()]` (the
///   same token/slot contract as [`upload_instance_models`]).
pub unsafe fn upload_instance_materials(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.material_ids.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.material_ids.as_read_slice());
    assert!(
        bytes.len() as u64 <= ring_slot.size,
        "instance-material ring overflow: {} gathered material payloads ({} bytes) exceed the \
         {}-instance ({}-byte) slot (asset-streaming plan F7 W3: an RT device hard-caps the \
         whole instance family at INSTANCE_CAPACITY — the material ring shares that cap; reduce \
         the scene's simultaneous drawable count or raise the boot INSTANCE_CAPACITY)",
        scratch.material_ids.len(),
        bytes.len(),
        ring_slot.size / PER_INSTANCE_MATERIAL_BYTES as u64,
        ring_slot.size
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the instance-material ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `ring_slot.size` valid mapped
    // host-coherent bytes, and `bytes.len() <= ring_slot.size` is hard-asserted above — the
    // write is in-bounds. The borrowed `FrameWriteToken` + the slot-identity contract prove
    // this slot's in-flight fence was waited THIS frame (the slot's previous occupant finished
    // its VERTEX reads of the material SSBO; the sibling frame binds the other slot) —
    // race-free, lock-free. `bytes` is the scratch's own heap buffer, a distinct
    // non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Textured-PBR rung T6c: uploads the gathered per-instance
/// [`MeshRenderScratch::material_tex`] lane (base_color + material id + five bindless
/// texture slots + the metallic/roughness fallback scalars, a
/// [`PerInstanceMaterialTex`](crate::mesh_draw::PerInstanceMaterialTex) per instance) into
/// ONE TEXTURED instance-material-SSBO ring slot — ONE contiguous `bytemuck` memcpy, zero
/// staging, zero allocation (mirrors [`upload_instance_materials`]'s discipline exactly).
///
/// Called ONLY on a frame with [`MeshRenderScratch::any_textured_material`] (Principle 1 —
/// a non-textured frame does ZERO material-upload work); the caller (the runner) is
/// responsible for that gate. The lane is index-aligned with the instance ring
/// (`instance_materials_tex[i]` names the SAME instance `instances[i]` does).
///
/// # Panics
///
/// Panics if the gathered lane exceeds the slot's capacity — the SAME hard overflow
/// discipline as [`upload_instance_materials`]. UNLIKE `upload_instance_materials`'s ring,
/// the TEXTURED instance-material ring does NOT participate in the F7/F7-hwrt lockstep
/// grow (a disclosed T6c limitation): it stays fixed at its boot
/// [`crate::mesh_draw::PerInstanceMaterialTex`]-stride capacity for the whole process
/// lifetime, so a scene whose gathered instance count grows past that capacity while using
/// textured materials panics here rather than silently corrupting memory.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — `tex_instance_material_rings[token.slot()]`
///   (the same token/slot contract as [`upload_instance_materials`]).
pub unsafe fn upload_instance_materials_tex(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.material_tex.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.material_tex.as_read_slice());
    assert!(
        bytes.len() as u64 <= ring_slot.size,
        "TEXTURED instance-material ring overflow: {} gathered material payloads ({} bytes) \
         exceed the {}-instance ({}-byte) slot (T6c: this ring does NOT participate in F7 \
         growth — reduce the scene's simultaneous drawable count or raise the boot \
         INSTANCE_CAPACITY)",
        scratch.material_tex.len(),
        bytes.len(),
        ring_slot.size / PER_INSTANCE_MATERIAL_TEX_BYTES as u64,
        ring_slot.size
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the TEXTURED instance-material ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `ring_slot.size` valid mapped
    // host-coherent bytes, and `bytes.len() <= ring_slot.size` is hard-asserted above — the
    // write is in-bounds. The borrowed `FrameWriteToken` + the slot-identity contract prove
    // this slot's in-flight fence was waited THIS frame (the slot's previous occupant finished
    // its VERTEX reads of the material SSBO; the sibling frame binds the other slot) —
    // race-free, lock-free. `bytes` is the scratch's own heap buffer, a distinct
    // non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Uploads the gathered 96-byte [`GpuTransform3D`](crate::GpuTransform3D)
/// interpolation-PAIR ring — the interp-ON pair path (host plan R5) — into ONE
/// pair-SSBO ring slot: ONE contiguous `bytemuck` memcpy, zero staging, zero
/// allocation. The B2 interp compute pre-pass reads this slot as its
/// `TransformPair` input; its per-instance model output lands in the draw SSBO the
/// raster VS reads.
///
/// UNCONDITIONAL every frame (plan D5): correct by construction beats
/// probability-correct — no dirty gating, no fingerprints (the fingerprint gate was
/// KILLED; a stride-64 hash collision class made it silently wrong under id-recycled
/// respawn). An empty gather (`scratch.pair_ring` empty — a frame that took the
/// affine gather, or a scene with no interpolated body) writes nothing; the interp
/// activation's `instance_count == 0` then records no dispatch (byte-identical to
/// interp OFF).
///
/// # Panics
///
/// Panics if the gathered pair ring exceeds the slot's capacity: writing past the
/// mapped range would corrupt neighbouring sub-allocations (UB), so the guard is a
/// hard assert in every build. The boot capacity is the host's documented initial
/// instance budget; growth is a host (R7) concern.
///
/// # Safety
///
/// * `slot_buffer` is a LIVE host-visible buffer minted by
///   `RhiDevice::create_buffer` (`HostVisibleCoherent`) and not yet destroyed:
///   its `mapped` pointer targets at least `slot_buffer.size` valid,
///   persistently-mapped bytes. A hand-built `BoundBuffer` with a dangling /
///   undersized `mapped` violates this.
/// * `slot_buffer` is the FENCED slot's buffer — `interp.pairs[token.slot()]`
///   (the same token/slot contract as [`upload_camera_ring`]): the token proves
///   that slot's in-flight fence was waited THIS frame, so the slot's previous
///   occupant finished every COMPUTE read of this pair SSBO and the sibling
///   in-flight frame binds the OTHER slot. Passing a different slot's buffer
///   re-opens the `80bf033` write-after-read race.
pub unsafe fn upload_pair_ring(
    token: &FrameWriteToken,
    slot_buffer: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.pair_ring.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.pair_ring.as_read_slice());
    assert!(
        bytes.len() as u64 <= slot_buffer.size,
        "pair ring overflow: {} gathered pairs ({} bytes) exceed the {}-pair \
         ({}-byte) slot (asset-streaming plan F7: `GpuSceneBundles::\
         grow_instance_family_if_needed` grows this ring in lockstep with the instance \
         ring on a non-RT device before this call — see `upload_instance_models`'s \
         overflow message for the RT-capped / caller-ordering alternatives)",
        scratch.pair_ring.len(),
        bytes.len(),
        slot_buffer.size / GPU_TRANSFORM3D_BYTES as u64,
        slot_buffer.size
    );

    let mapped = slot_buffer
        .mapped
        .expect("invariant: the pair ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `slot_buffer.size` valid
    // mapped host-coherent bytes, and `bytes.len() <= slot_buffer.size` is
    // hard-asserted above — the write is in-bounds. The borrowed `FrameWriteToken`
    // + the slot-identity contract prove this slot's in-flight fence was waited
    // THIS frame (the slot's previous occupant finished its COMPUTE reads of the
    // pair SSBO; the sibling frame binds the other slot) — race-free, lock-free.
    // `bytes` is the scratch's own heap buffer, a distinct non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Uploads the gathered per-dynamic-instance OUT-SLOT lane
/// ([`pair_out_slot`](crate::MeshRenderScratch::pair_out_slot) — one `u32` per
/// interpolated instance) into ONE out-slot-SSBO ring slot — the interp-ON path
/// (host plan R5, refined-B): ONE contiguous `bytemuck` memcpy, zero staging, zero
/// allocation. The B2 interp compute pre-pass reads this slot as its `OutSlot`
/// binding — `out_slot[d]` is dynamic instance `d`'s offset into the SHARED instance
/// ring the compute scatters its interpolated model column into.
///
/// UNCONDITIONAL every frame (plan D5), paired with [`upload_pair_ring`]: the two
/// lanes are parallel (`pair_out_slot.len() == pair_ring.len()` — the dynamic count),
/// so they upload together. An empty gather (`pair_out_slot` empty — a pure-static
/// scene or no interpolated body) writes nothing; `dynamic_count() == 0` then records
/// no dispatch (byte-identical to interp OFF).
///
/// # Panics
///
/// Panics if the gathered out-slot lane exceeds the slot's capacity: writing past the
/// mapped range would corrupt neighbouring sub-allocations (UB), so the guard is a
/// hard assert in every build.
///
/// # Safety
///
/// * `slot_buffer` is a LIVE host-visible buffer minted by
///   `RhiDevice::create_buffer` (`HostVisibleCoherent`) and not yet destroyed: its
///   `mapped` pointer targets at least `slot_buffer.size` valid, persistently-mapped
///   bytes. A hand-built `BoundBuffer` with a dangling / undersized `mapped` violates
///   this.
/// * `slot_buffer` is the FENCED slot's buffer — `interp.out_slot[token.slot()]` (the
///   same token/slot contract as [`upload_camera_ring`]): the token proves that slot's
///   in-flight fence was waited THIS frame, so the slot's previous occupant finished
///   every COMPUTE read of this out-slot SSBO and the sibling in-flight frame binds the
///   OTHER slot. Passing a different slot's buffer re-opens the `80bf033` race.
pub unsafe fn upload_pair_out_slot(
    token: &FrameWriteToken,
    slot_buffer: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.pair_out_slot.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.pair_out_slot.as_read_slice());
    assert!(
        bytes.len() as u64 <= slot_buffer.size,
        "out-slot ring overflow: {} gathered out-slots ({} bytes) exceed the {}-slot \
         ({}-byte) buffer (asset-streaming plan F7: `GpuSceneBundles::\
         grow_instance_family_if_needed` grows this ring in lockstep with the instance \
         ring on a non-RT device before this call — see `upload_instance_models`'s \
         overflow message for the RT-capped / caller-ordering alternatives)",
        scratch.pair_out_slot.len(),
        bytes.len(),
        slot_buffer.size / 4,
        slot_buffer.size
    );

    let mapped = slot_buffer
        .mapped
        .expect("invariant: the out-slot ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `slot_buffer.size` valid
    // mapped host-coherent bytes, and `bytes.len() <= slot_buffer.size` is
    // hard-asserted above — the write is in-bounds. The borrowed `FrameWriteToken`
    // + the slot-identity contract prove this slot's in-flight fence was waited THIS
    // frame (the slot's previous occupant finished its COMPUTE reads of the out-slot
    // SSBO; the sibling frame binds the other slot) — race-free, lock-free. `bytes`
    // is the scratch's own heap buffer, a distinct non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// HW-RT rung R2a-3: uploads the gathered per-instance MESH-ID (BLAS-index) lane
/// ([`mesh_ids`](crate::MeshRenderScratch::mesh_ids) — one `u32` per drawable, parallel to the
/// instance ring) into ONE mesh-id-SSBO ring slot: ONE contiguous `bytemuck` memcpy, zero
/// staging, zero allocation. The TLAS-instance packer compute pre-pass reads this slot at
/// binding 1 (`MeshIds[i]`) to resolve instance `i`'s BLAS device address from the per-mesh
/// address table.
///
/// UNCONDITIONAL on an RT device every frame (the pack path mirror of
/// [`upload_instance_models`]): the mesh-id lane is scattered in lock-step with the ring
/// (`mesh_ids.len() == ring.len()`). An empty gather (`scratch.mesh_ids` empty ⇒ no drawable ⇒
/// the pack records no dispatch) writes nothing.
///
/// # Panics
///
/// Panics if the gathered lane exceeds the slot's capacity: writing past the mapped range
/// would corrupt neighbouring sub-allocations (UB), so the guard is a hard assert in every
/// build. The boot capacity is the host's documented instance budget; growth is a host concern.
///
/// # Safety
///
/// * `slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `slot.size` valid, persistently-mapped bytes. A hand-built [`BoundBuffer`] with a dangling
///   / undersized `mapped` violates this.
/// * `slot` is the FENCED slot's buffer — `mesh_id_rings[token.slot()]` (the same token/slot
///   contract as [`upload_instance_models`]): the token proves that slot's in-flight fence was
///   waited THIS frame, so the slot's previous occupant finished every COMPUTE read of this
///   mesh-id SSBO and the sibling in-flight frame binds the OTHER slot — race-free, lock-free.
#[cfg(feature = "hwrt")]
pub unsafe fn upload_mesh_ids(
    token: &FrameWriteToken,
    slot: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.mesh_ids.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.mesh_ids.as_read_slice());
    assert!(
        bytes.len() as u64 <= slot.size,
        "mesh-id ring overflow: {} gathered mesh-ids ({} bytes) exceed the {}-slot \
         ({}-byte) buffer (asset-streaming plan F7 W3: an RT device hard-caps the whole \
         instance family at INSTANCE_CAPACITY — the TLAS packer's `instance_arrays`/\
         backing/scratch are sized once for it, so this ring never grows; reduce the \
         scene's simultaneous drawable count or raise the boot INSTANCE_CAPACITY)",
        scratch.mesh_ids.len(),
        bytes.len(),
        slot.size / 4,
        slot.size
    );

    let mapped = slot
        .mapped
        .expect("invariant: the mesh-id ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `slot.size` valid mapped host-coherent
    // bytes, and `bytes.len() <= slot.size` is hard-asserted above — the write is in-bounds. The
    // borrowed `FrameWriteToken` + the slot-identity contract prove this slot's in-flight fence
    // was waited THIS frame (the slot's previous occupant finished its COMPUTE reads of the
    // mesh-id SSBO; the sibling frame binds the other slot) — race-free, lock-free. `bytes` is
    // the scratch's own heap buffer, a distinct non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// HW-RT Rung 3b: uploads the gathered 48-byte
/// [`PrevInstanceModelCol`](crate::PrevInstanceModelCol)-derived PREVIOUS-frame instance ring
/// ([`prev_ring`](crate::MeshRenderScratch::prev_ring)) into ONE prev-instance-SSBO ring slot —
/// the mesh-motion-vector mirror of [`upload_instance_models`]: ONE contiguous `bytemuck` memcpy,
/// zero staging, zero allocation. The gbuffer MV vertex shader reads this slot at binding 1
/// (`prev_instances[base_instance + SV_InstanceID]`) to compute each mesh pixel's per-object
/// `prev_world`, so its motion vector is `cur_world − prev_world`.
///
/// Uploaded ONLY when the temporal denoiser is on (the runner gates the CALL on `feature = "hwrt"`
/// plus `temporal_enabled` plus the `mv` ring's presence — the SAME gate that binds the MV
/// pipeline). The lane is scattered INDEX-ALIGNED with `scratch.ring` (`prev_ring.len() ==
/// ring.len()`), so a prev row lands in the SAME slot the current row did. An empty gather
/// (`prev_ring` empty ⇒ no drawable ⇒ the recorder takes the legacy draw) writes nothing.
///
/// # Panics
///
/// Panics if the gathered prev ring exceeds the slot's capacity: writing past the mapped range
/// would corrupt neighbouring sub-allocations (UB), so the guard is a hard assert in every build.
/// The boot capacity is the host's documented instance budget (the prev ring is sized identically
/// to the current instance ring); growth is a host concern.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes. A hand-built [`BoundBuffer`] with a
///   dangling / undersized `mapped` violates this.
/// * `ring_slot` is the FENCED slot's buffer — `prev_instance_rings[token.slot()]` (the same
///   token/slot contract as [`upload_instance_models`]): the token proves that slot's in-flight
///   fence was waited THIS frame, so the slot's previous occupant finished every VERTEX read of
///   this prev-instance SSBO and the sibling in-flight frame binds the OTHER slot — race-free,
///   lock-free.
#[cfg(feature = "hwrt")]
pub unsafe fn upload_prev_instance_models(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    scratch: &MeshRenderScratch,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    if scratch.prev_ring.is_empty() {
        return;
    }
    let bytes: &[u8] = bytemuck::cast_slice(scratch.prev_ring.as_read_slice());
    assert!(
        bytes.len() as u64 <= ring_slot.size,
        "prev-instance ring overflow: {} gathered instances ({} bytes) exceed the \
         {}-instance ({}-byte) slot (asset-streaming plan F7 W3: an RT device hard-caps \
         the whole instance family at INSTANCE_CAPACITY — the TLAS packer's \
         `instance_arrays`/backing/scratch are sized once for it, so this ring never \
         grows; reduce the scene's simultaneous drawable count or raise the boot \
         INSTANCE_CAPACITY)",
        scratch.prev_ring.len(),
        bytes.len(),
        ring_slot.size / core::mem::size_of::<crate::InstanceModelCol>() as u64,
        ring_slot.size
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the prev-instance ring slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `ring_slot.size` valid mapped host-coherent
    // bytes, and `bytes.len() <= ring_slot.size` is hard-asserted above — the write is in-bounds.
    // The borrowed `FrameWriteToken` + the slot-identity contract prove this slot's in-flight fence
    // was waited THIS frame (the slot's previous occupant finished its VERTEX reads of the
    // prev-instance SSBO; the sibling frame binds the other slot) — race-free, lock-free. `bytes` is
    // the scratch's own heap buffer, a distinct non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// HW-RT Rung 3b: copies the frame's [`MotionCam`] (the 128-byte camera view-proj pair — this
/// frame's `cur` + last frame's `prev`, column-major, see [`MOTION_CAM_UBO_BYTES`]) into ONE
/// motion-cam-UBO ring slot — the mesh-motion-vector camera upload (mirroring [`upload_csm_ring`]).
/// The gbuffer MV vertex shader reads it at binding 2 as `{ float4x4 cur_view_proj; float4x4
/// prev_view_proj; }` to project `cur_world` / `prev_world` into the two clip spaces whose Δuv is
/// the screen-space motion vector.
///
/// Uploaded ONLY when the temporal denoiser is on (the runner gates the CALL on `feature = "hwrt"`
/// plus `temporal_enabled` plus the `mv` ring's presence — the SAME gate that binds the MV
/// pipeline). [`MotionCamState::advance`](crate::MotionCamState::advance) re-derives the pair from
/// the LIVE camera each frame (a boot-seed would go stale the moment the camera moves), and the
/// 128-byte memcpy is cheaper than a change gate.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than [`MOTION_CAM_UBO_BYTES`]: the memcpy would be
/// out-of-bounds (UB), so the guard is a hard assert in every build.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — `motion_cam_ubo[token.slot()]` (the same token/slot
///   contract as [`upload_csm_ring`]): the token proves that slot's in-flight fence was waited THIS
///   frame, so the slot's previous occupant finished every VERTEX read of this UBO and the sibling
///   in-flight frame binds the OTHER ring slot — race-free, lock-free.
pub unsafe fn upload_motion_cam_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    cam: &MotionCam,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy (review P1 discipline): an undersized slot would make the
    // 128-byte write out-of-bounds. One compare per frame.
    assert!(
        ring_slot.size as usize >= MOTION_CAM_UBO_BYTES,
        "motion-cam UBO slot too small: {} bytes < the {}-byte MotionCam pair",
        ring_slot.size,
        MOTION_CAM_UBO_BYTES
    );

    let bytes = cam.to_bytes();
    let mapped = ring_slot
        .mapped
        .expect("invariant: the motion-cam UBO slot is host-visible mapped");
    // SAFETY: `bytes` is a distinct 128-byte stack array (`MotionCam::to_bytes`). `mapped` targets
    // >= `ring_slot.size >= MOTION_CAM_UBO_BYTES` valid mapped host-coherent bytes (hard-asserted
    // above) — the write is in-bounds. The borrowed `FrameWriteToken` + the slot-identity contract
    // prove this slot's in-flight fence was waited THIS frame (the previous occupant's MV VS reads
    // finished; the sibling frame binds the other slot) — race-free, lock-free. The two regions are
    // distinct allocations (no overlap).
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Writes the staged light-table bytes (`[LightHeaderGpu || GpuLight[]]`, e.g.
/// [`LightTableStaging::bytes`](crate::light_system::LightTableStaging::bytes)) into
/// ONE light-STAGING ring slot — the host half of the rung L0-r0 on-change upload
/// (host plan R4/D5). The recorder's `light_upload` pass then copies these bytes into
/// the device light table on the GPU timeline (gated by `GBufferScene::light_dirty`).
///
/// # Why the staging is a per-slot RING (the R4 race analysis, pinned)
///
/// Frame N's recorded staging→table copy READS the staging buffer on the GPU while it
/// executes; under the D5 generation protocol BOTH in-flight slots want the copy on
/// the two consecutive frames after a change, so a SINGLE staging instance would be
/// host-REWRITTEN on frame N+1 while frame N's copy may still be reading it — the
/// `80bf033` host-write-vs-GPU-read class, on the transfer stage instead of a shader
/// stage. Ringing the staging per in-flight slot restores the token discipline: slot
/// `s`'s staging is only ever read by frames occupying slot `s`, and the borrowed
/// token proves slot `s`'s fence was waited THIS frame — the previous occupant's copy
/// (frame N−2) retired before this write.
///
/// # Panics
///
/// Panics if `bytes` exceeds `staging_slot.size`: the memcpy would run past the
/// mapped range (UB), so the guard is a hard assert in every build. Size staging
/// slots at the full-table capacity (`LIGHT_HEADER_BYTES + MAX_LIGHTS *
/// GPU_LIGHT_BYTES`) so any staged table fits.
///
/// # Safety
///
/// * `staging_slot` is a LIVE host-visible buffer minted by
///   `RhiDevice::create_buffer` (`HostVisibleCoherent`) and not yet destroyed: its
///   `mapped` pointer targets at least `staging_slot.size` valid, persistently-mapped
///   bytes. A hand-built `BoundBuffer` with a dangling / undersized `mapped` violates
///   this.
/// * `staging_slot` is the FENCED slot's buffer — `light_staging[token.slot()]` (the
///   same token/slot contract as [`upload_camera_ring`]). Passing a different slot's
///   buffer re-opens the host-write-vs-GPU-copy race above.
pub unsafe fn upload_light_table(
    token: &FrameWriteToken,
    staging_slot: &BoundBuffer,
    bytes: &[u8],
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy: an oversized table would write past the mapped
    // sub-allocation — corruption, not merely wrong lighting. One compare per upload.
    assert!(
        bytes.len() as u64 <= staging_slot.size,
        "light table overflow: {} staged bytes exceed the {}-byte staging slot \
         (size the staging ring at LIGHT_HEADER_BYTES + MAX_LIGHTS * GPU_LIGHT_BYTES)",
        bytes.len(),
        staging_slot.size
    );

    let mapped = staging_slot
        .mapped
        .expect("invariant: the light staging slot is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `staging_slot.size` valid
    // mapped host-coherent bytes, and `bytes.len() <= staging_slot.size` is
    // hard-asserted above — the write is in-bounds. The borrowed `FrameWriteToken` +
    // the slot-identity contract prove this slot's in-flight fence was waited THIS
    // frame, so the slot's previous occupant (frame N−2) finished its recorded
    // staging→table copy (its transfer READ of this buffer) and the sibling in-flight
    // frame copies from the OTHER ring slot — race-free, lock-free. `bytes` is the
    // staging resource's own heap buffer, a distinct non-overlapping region.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
    }
}

/// Writes the frame's [`ResolvedCsm`] (the 336-byte `#[repr(C)]` cascade selection —
/// byte-identical to the resolve's binding-13 UBO shape, see [`RESOLVED_CSM_BYTES`])
/// into ONE cascade-UBO ring slot — the per-frame CSM upload of the production
/// G-buffer path (host plan R4).
///
/// Uploaded UNCONDITIONALLY every frame, mirroring [`upload_camera_ring`] (plan D5's
/// correct-by-construction rationale at 1/300th the pair-ring size): the production
/// `resolve_csm_cascades` re-fits from the LIVE camera each frame, so a boot-seed
/// would go stale the moment the camera or sun moves, and a change gate would cost a
/// 336-byte compare + host tracking state to save a ~336-byte memcpy. A DISABLED
/// selection uploads as all-zero (`csm_mode_word == 0`) — consistent with the
/// bound-but-unread OFF path.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than [`RESOLVED_CSM_BYTES`]: the memcpy
/// would be out-of-bounds (UB), so the guard is a hard assert in every build.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at
///   least `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — `csm_cascade_ring[token.slot()]` (the
///   same token/slot contract as [`upload_camera_ring`]): the resolve of the slot's
///   previous occupant retired behind the waited fence, and the sibling in-flight
///   frame binds the OTHER ring slot.
pub unsafe fn upload_csm_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    resolved: &ResolvedCsm,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy (review P1 discipline): an undersized slot would
    // make the 336-byte write out-of-bounds. One compare per frame.
    assert!(
        ring_slot.size as usize >= RESOLVED_CSM_BYTES,
        "CSM cascade UBO slot too small: {} bytes < the {}-byte ResolvedCsm mirror",
        ring_slot.size,
        RESOLVED_CSM_BYTES
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the CSM cascade UBO slot is host-visible mapped");
    // SAFETY: `resolved` is a live `#[repr(C)]` POD of exactly `RESOLVED_CSM_BYTES`
    // (const-asserted at its definition) with no padding holes (every pad lane is an
    // explicit zeroed field), so reading its raw bytes is defined. `mapped` targets
    // >= `ring_slot.size >= RESOLVED_CSM_BYTES` valid mapped host-coherent bytes
    // (hard-asserted above) — the write is in-bounds. The borrowed `FrameWriteToken`
    // + the slot-identity contract prove this slot's in-flight fence was waited THIS
    // frame (the previous occupant's resolve finished its UBO reads; the sibling
    // frame binds the other slot) — race-free, lock-free. The two regions are
    // distinct allocations (no overlap).
    unsafe {
        core::ptr::copy_nonoverlapping(
            (resolved as *const ResolvedCsm).cast::<u8>(),
            mapped.as_ptr(),
            RESOLVED_CSM_BYTES,
        );
    }
}

/// Copies the resolved [`ResolvedRayShadow`] (the HWRT `rayQuery` mesh-shadow tuning —
/// cone/tmax/tmin/bias, byte-identical to the HWRT resolve's binding-20 UBO shape, see
/// [`RESOLVED_RAY_SHADOW_BYTES`]) PLUS the rung-3b `frame_index` seed into ONE HWRT
/// shadow-params-UBO ring slot — the per-frame upload of the HWRT resolve path (mirroring
/// [`upload_csm_ring`]).
///
/// `frame_index` lands at UBO byte offset [`RESOLVED_RAY_SHADOW_BYTES`] (16), matching the
/// HLSL `RayShadowUbo.SHADOW_FRAME_SEED` field — it advances the shadow ray's Vogel-disk cone
/// rotation by the golden angle every frame so the temporal shadow denoiser has something to
/// average. It is packed HERE, not routed through the cold [`ResolvedRayShadow`] resolve: it
/// is a HOT per-frame counter (the runner's monotonic frame index), not author-tunable policy,
/// so folding it into the cold resolve would force a needless extra write of the other four
/// scalars every frame for no benefit (one-producer-per-field, cold/hot separation).
///
/// Uploaded every HWRT frame (the runner gates the CALL on `feature = "hwrt"` +
/// `ray_query_enabled()`, the SAME gate that mints the ring), exactly like
/// [`upload_csm_ring`]: `resolve_ray_shadow_system` re-derives the 16-byte UBO from the cold
/// [`RayShadowConfig`](crate::ray_shadow_config::RayShadowConfig) each frame, so a boot-seed
/// would go stale the moment the author retunes, and the 20-byte memcpy pair is cheaper than a
/// change gate. A default config + `frame_index == 0` uploads the byte-identical R2a-4b consts.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than `RESOLVED_RAY_SHADOW_BYTES + 4` (20 B — the
/// resolved mirror plus the `frame_index` seed): the memcpy pair would be out-of-bounds (UB),
/// so the guard is a hard assert in every build.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — `ray_shadow_ubo[token.slot()]` (the same
///   token/slot contract as [`upload_csm_ring`]): the resolve of the slot's previous occupant
///   retired behind the waited fence, and the sibling in-flight frame binds the OTHER ring slot.
pub unsafe fn upload_ray_shadow_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    resolved: &ResolvedRayShadow,
    frame_index: u32,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy pair (review P1 discipline): an undersized slot would make
    // the 20-byte write out-of-bounds. One compare per frame.
    const RAY_SHADOW_UBO_WRITE_BYTES: usize = RESOLVED_RAY_SHADOW_BYTES + 4;
    assert!(
        ring_slot.size as usize >= RAY_SHADOW_UBO_WRITE_BYTES,
        "HWRT shadow-params UBO slot too small: {} bytes < the {}-byte ResolvedRayShadow + frame-seed mirror",
        ring_slot.size,
        RAY_SHADOW_UBO_WRITE_BYTES
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the HWRT shadow-params UBO slot is host-visible mapped");
    // SAFETY: `resolved` is a live `#[repr(C)]` POD of exactly `RESOLVED_RAY_SHADOW_BYTES`
    // (const-asserted at its definition) with no padding holes (4 packed `f32`s), so reading its
    // raw bytes is defined. `frame_index.to_le_bytes()` is a 4-byte stack array, always valid to
    // read. `mapped` targets >= `ring_slot.size >= RAY_SHADOW_UBO_WRITE_BYTES` (20) valid mapped
    // host-coherent bytes (hard-asserted above), so both writes — `[0..16)` then `[16..20)` via
    // `mapped.add(RESOLVED_RAY_SHADOW_BYTES)` — are in-bounds and non-overlapping. The borrowed
    // `FrameWriteToken` + the slot-identity contract prove this slot's in-flight fence was
    // waited THIS frame (the previous occupant's resolve finished its UBO reads; the sibling
    // frame binds the other slot) — race-free, lock-free. Within `mapped` the two destination
    // ranges `[0..16)` and `[16..20)` are non-overlapping, and each copy's src and dst are
    // distinct allocations.
    unsafe {
        core::ptr::copy_nonoverlapping(
            (resolved as *const ResolvedRayShadow).cast::<u8>(),
            mapped.as_ptr(),
            RESOLVED_RAY_SHADOW_BYTES,
        );
        core::ptr::copy_nonoverlapping(
            frame_index.to_le_bytes().as_ptr(),
            mapped.as_ptr().add(RESOLVED_RAY_SHADOW_BYTES),
            4,
        );
    }
}

/// Copies the resolved [`ResolvedShadowDenoise`] (the HW-RT rung 3a à-trous edge-stop scalars —
/// `sigma_z`/`sigma_n`, byte-identical to the à-trous set's binding-4 UBO shape, see
/// [`RESOLVED_SHADOW_DENOISE_BYTES`]) into ONE à-trous edge-stop-UBO ring slot — the per-frame
/// upload of the spatial-denoise path (mirroring [`upload_ray_shadow_ring`]).
///
/// Uploaded every denoise-armed HW-RT frame (the runner gates the CALL on `feature = "hwrt"` +
/// `ray_query_enabled()`, the SAME gate that mints the ring), exactly like
/// [`upload_ray_shadow_ring`]: `resolve_shadow_denoise_policy` re-derives the 16-byte UBO from the
/// cold [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) each frame, so a
/// boot-seed would go stale the moment the author retunes `sigma_z`/`sigma_n`, and the 16-byte
/// memcpy is cheaper than a change gate. A default config uploads the ON-default edge-stop scalars.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than [`RESOLVED_SHADOW_DENOISE_BYTES`]: the memcpy would
/// be out-of-bounds (UB), so the guard is a hard assert in every build.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — the renderer's `shadow_denoise_ubo[token.slot()]`
///   (the same token/slot contract as [`upload_ray_shadow_ring`]): the resolve of the slot's
///   previous occupant retired behind the waited fence, and the sibling in-flight frame binds the
///   OTHER ring slot.
pub unsafe fn upload_shadow_denoise_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    resolved: &ResolvedShadowDenoise,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy (review P1 discipline): an undersized slot would make the
    // 16-byte write out-of-bounds. One compare per frame.
    assert!(
        ring_slot.size as usize >= RESOLVED_SHADOW_DENOISE_BYTES,
        "à-trous edge-stop UBO slot too small: {} bytes < the {}-byte ResolvedShadowDenoise mirror",
        ring_slot.size,
        RESOLVED_SHADOW_DENOISE_BYTES
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the à-trous edge-stop UBO slot is host-visible mapped");
    // SAFETY: `resolved` is a live `#[repr(C)]` POD of exactly `RESOLVED_SHADOW_DENOISE_BYTES`
    // (const-asserted at its definition) with no padding holes (4 packed `f32`s — two scalars + two
    // zeroed std140 pad lanes), so reading its raw bytes is defined. `mapped` targets
    // >= `ring_slot.size >= RESOLVED_SHADOW_DENOISE_BYTES` valid mapped host-coherent bytes
    // (hard-asserted above) — the write is in-bounds. The borrowed `FrameWriteToken` + the
    // slot-identity contract prove this slot's in-flight fence was waited THIS frame (the previous
    // occupant's à-trous reads finished; the sibling frame binds the other slot) — race-free,
    // lock-free. The two regions are distinct allocations (no overlap).
    unsafe {
        core::ptr::copy_nonoverlapping(
            (resolved as *const ResolvedShadowDenoise).cast::<u8>(),
            mapped.as_ptr(),
            RESOLVED_SHADOW_DENOISE_BYTES,
        );
    }
}

/// Copies the resolved [`ResolvedTemporalShadow`] (the HW-RT Rung 3b temporal reproject scalars —
/// `feedback_max`/`feedback_min`/`variance_gamma`/`depth_tol`, byte-identical to the temporal
/// pass's binding-6 UBO shape, see [`RESOLVED_TEMPORAL_SHADOW_BYTES`]) into ONE temporal-UBO ring
/// slot — the per-frame upload of the temporal-denoise path (a SEPARATE carrier from the à-trous
/// [`upload_shadow_denoise_ring`], so the shipped Spatial upload byte-stream is untouched).
///
/// Uploaded every temporal-armed HW-RT frame (the runner gates the CALL on `feature = "hwrt"` + the
/// temporal-UBO ring existing, the SAME gate that mints it), exactly like
/// [`upload_shadow_denoise_ring`]: `resolve_temporal_shadow_policy` re-derives the 16-byte UBO from
/// the cold [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) each frame, so
/// a boot-seed would go stale the moment the author retunes the temporal scalars, and the 16-byte
/// memcpy is cheaper than a change gate. A default config uploads the ON-default temporal scalars.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than [`RESOLVED_TEMPORAL_SHADOW_BYTES`]: the memcpy would
/// be out-of-bounds (UB), so the guard is a hard assert in every build.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — the renderer's `temporal_shadow_ubo[token.slot()]`
///   (the same token/slot contract as [`upload_shadow_denoise_ring`]): the temporal pass of the
///   slot's previous occupant retired behind the waited fence, and the sibling in-flight frame binds
///   the OTHER ring slot.
pub unsafe fn upload_temporal_shadow_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    resolved: &ResolvedTemporalShadow,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy (review P1 discipline): an undersized slot would make the
    // 16-byte write out-of-bounds. One compare per frame.
    assert!(
        ring_slot.size as usize >= RESOLVED_TEMPORAL_SHADOW_BYTES,
        "temporal shadow UBO slot too small: {} bytes < the {}-byte ResolvedTemporalShadow mirror",
        ring_slot.size,
        RESOLVED_TEMPORAL_SHADOW_BYTES
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the temporal shadow UBO slot is host-visible mapped");
    // SAFETY: `resolved` is a live `#[repr(C)]` POD of exactly `RESOLVED_TEMPORAL_SHADOW_BYTES`
    // (const-asserted at its definition) with no padding holes (4 packed `f32`s), so reading its raw
    // bytes is defined. `mapped` targets >= `ring_slot.size >= RESOLVED_TEMPORAL_SHADOW_BYTES` valid
    // mapped host-coherent bytes (hard-asserted above) — the write is in-bounds. The borrowed
    // `FrameWriteToken` + the slot-identity contract prove this slot's in-flight fence was waited
    // THIS frame (the previous occupant's temporal reproject finished its UBO reads; the sibling
    // frame binds the other slot) — race-free, lock-free. The two regions are distinct allocations
    // (no overlap).
    unsafe {
        core::ptr::copy_nonoverlapping(
            (resolved as *const ResolvedTemporalShadow).cast::<u8>(),
            mapped.as_ptr(),
            RESOLVED_TEMPORAL_SHADOW_BYTES,
        );
    }
}

/// Copies the resolved [`ResolvedTaa`] (the Stage-4/rung-T2 TAA temporal-resolve tunables —
/// `default_blend`/`min_blend`/`variance_gamma` plus the eight T2 mode words/scalars,
/// byte-identical to the resolve's binding-5 UBO shape, see [`RESOLVED_TAA_BYTES`]) into ONE
/// TAA-tunables UBO ring slot — the per-frame upload of the TAA resolve path, mirroring
/// [`upload_temporal_shadow_ring`] (a SEPARATE carrier from every hwrt shadow UBO, NOT
/// `hwrt`-gated: TAA runs on the pure-software leg too).
///
/// Uploaded every TAA-armed frame (the runner gates the CALL on the TAA UBO ring slot existing —
/// `GBufferFrame::taa_ubo_slot`, the SAME gate that mints it): a boot-seed would go stale the
/// moment a future policy retunes any tunable, and a 48-byte memcpy is cheaper than a change-gate.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than [`RESOLVED_TAA_BYTES`]: the memcpy would be
/// out-of-bounds (UB), so the guard is a hard assert in every build.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — the renderer's `taa_ubo[token.slot()]` (the same
///   token/slot contract as [`upload_temporal_shadow_ring`]): the resolve of the slot's previous
///   occupant retired behind the waited fence, and the sibling in-flight frame binds the OTHER
///   ring slot.
pub unsafe fn upload_taa_ring(token: &FrameWriteToken, ring_slot: &BoundBuffer, resolved: &ResolvedTaa) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy (review P1 discipline): an undersized slot would make the
    // 48-byte write out-of-bounds. One compare per frame.
    assert!(
        ring_slot.size as usize >= RESOLVED_TAA_BYTES,
        "TAA tunables UBO slot too small: {} bytes < the {}-byte ResolvedTaa mirror",
        ring_slot.size,
        RESOLVED_TAA_BYTES
    );

    let mapped = ring_slot.mapped.expect("invariant: the TAA tunables UBO slot is host-visible mapped");
    // SAFETY: `resolved` is a live `#[repr(C)]` POD of exactly `RESOLVED_TAA_BYTES` (const-asserted
    // at its definition) with no padding holes (twelve packed 4-byte `f32`/`u32` scalars), so
    // reading its raw bytes is defined. `mapped` targets >= `ring_slot.size >= RESOLVED_TAA_BYTES`
    // valid mapped host-coherent bytes (hard-asserted above) — the write is in-bounds. The borrowed
    // `FrameWriteToken` + the slot-identity contract prove this slot's in-flight fence was waited
    // THIS frame (the previous occupant's resolve finished its UBO read; the sibling frame binds
    // the other slot) — race-free, lock-free. The two regions are distinct allocations (no overlap).
    unsafe {
        core::ptr::copy_nonoverlapping(
            (resolved as *const ResolvedTaa).cast::<u8>(),
            mapped.as_ptr(),
            RESOLVED_TAA_BYTES,
        );
    }
}

/// Copies the fitted [`ResolvedShadowAtlas`] (the punctual spot/point atlas selection,
/// byte-identical to the resolve's binding-15 UBO shape, see [`RESOLVED_SHADOW_ATLAS_BYTES`])
/// into ONE atlas-UBO ring slot — the per-frame punctual upload of the production G-buffer path
/// (the punctual host rung, mirroring [`upload_csm_ring`]).
///
/// Uploaded UNCONDITIONALLY every frame, exactly like [`upload_csm_ring`]: `resolve_shadow_atlas`
/// re-fits from the LIVE camera each frame (the `spot_priority` = range²/dist² proxy is
/// camera-dependent), so a boot-seed would go stale the moment the camera moves, and a change gate
/// would cost a 1296-byte compare + host tracking state to save a ~1296-byte memcpy. A DISABLED
/// selection uploads as all-zero (`mode_word == 0`) — consistent with the bound-but-unread OFF
/// path.
///
/// # Panics
///
/// Panics if `ring_slot.size` is smaller than [`RESOLVED_SHADOW_ATLAS_BYTES`]: the memcpy would be
/// out-of-bounds (UB), so the guard is a hard assert in every build.
///
/// # Safety
///
/// * `ring_slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at least
///   `ring_slot.size` valid, persistently-mapped bytes.
/// * `ring_slot` is the FENCED slot's buffer — `atlas_ubo[token.slot()]` (the same token/slot
///   contract as [`upload_csm_ring`]): the resolve of the slot's previous occupant retired behind
///   the waited fence, and the sibling in-flight frame binds the OTHER ring slot.
pub unsafe fn upload_atlas_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    resolved: &ResolvedShadowAtlas,
) {
    // The borrow IS the fence proof — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the memcpy (review P1 discipline): an undersized slot would make the
    // 1296-byte write out-of-bounds. One compare per frame.
    assert!(
        ring_slot.size as usize >= RESOLVED_SHADOW_ATLAS_BYTES,
        "shadow-atlas UBO slot too small: {} bytes < the {}-byte ResolvedShadowAtlas mirror",
        ring_slot.size,
        RESOLVED_SHADOW_ATLAS_BYTES
    );

    let mapped = ring_slot
        .mapped
        .expect("invariant: the shadow-atlas UBO slot is host-visible mapped");
    // SAFETY: `resolved` is a live `#[repr(C)]` POD of exactly `RESOLVED_SHADOW_ATLAS_BYTES`
    // (const-asserted at its definition) with no padding holes (every trailing word — including
    // the host-only `face_point_mask` and the final `_pad` — is an explicit field), so reading
    // its raw bytes is defined. `mapped` targets >= `ring_slot.size >= RESOLVED_SHADOW_ATLAS_BYTES`
    // valid mapped host-coherent bytes (hard-asserted above) — the write is in-bounds. The
    // borrowed `FrameWriteToken` + the slot-identity contract prove this slot's in-flight fence
    // was waited THIS frame (the previous occupant's resolve finished its UBO reads; the sibling
    // frame binds the other slot) — race-free, lock-free. The two regions are distinct allocations
    // (no overlap).
    unsafe {
        core::ptr::copy_nonoverlapping(
            (resolved as *const ResolvedShadowAtlas).cast::<u8>(),
            mapped.as_ptr(),
            RESOLVED_SHADOW_ATLAS_BYTES,
        );
    }
}

/// Encodes `edits` into the marcher's binding-0 edit-list SSBO (`slot`) — the R7 SDF
/// instance path's ONE-SHOT boot-static write (host plan R7). Word 0 becomes
/// `edit_count`, then the packed edit array (see
/// [`encode_edit_list`](boyko_rhi_vulkan::compute::encode_edit_list)); the pixel region
/// past the array is left as the boot seed wrote it (the shader owns those words).
///
/// # Why NOT a per-slot ring (unlike the sibling uploads)
///
/// The edit list is a SINGLE shared `BoundBuffer`, not a `FRAMES_IN_FLIGHT` ring. In v1
/// it is boot-static: the edits are known once (the startup gather), and this write runs
/// exactly once on the first frame BEFORE the first marcher dispatch reads the buffer —
/// so it races nothing (there is no previous occupant, no sibling in-flight read of a
/// non-empty list). The borrowed [`FrameWriteToken`] is still required as the mint proof
/// that we are on the fenced, dispatcher-solo write path; a dynamic per-frame edit path
/// (ring + generation gate) is a deferred campaign.
///
/// # Panics
///
/// Panics if the encoded word count exceeds `slot.size` (in words): writing past the
/// mapped range would corrupt neighbouring sub-allocations (UB), so the guard is a hard
/// assert in every build. `edits.len()` must be `<= MAX_SDF_EDITS`
/// (debug-asserted inside `encode_edit_list` — exceeding the fixed cap is a caller bug;
/// the gather already clamps).
///
/// # Safety
///
/// * `slot` is a LIVE host-visible buffer minted by `RhiDevice::create_buffer`
///   (`HostVisibleCoherent`) and not yet destroyed: its `mapped` pointer targets at
///   least `slot.size` valid, persistently-mapped bytes. A hand-built `BoundBuffer` with
///   a dangling / undersized `mapped` violates this.
/// * `slot` is written on the fenced, dispatcher-solo path proved by the borrowed
///   `token`, BEFORE the first marcher dispatch reads it. In v1 this is the single
///   boot-static write, so no in-flight GPU read of a non-empty edit list can be racing
///   it (the boot seed the marcher last read is the empty list, and no sibling frame
///   rewrites this shared buffer).
pub unsafe fn upload_sdf_edit_list(token: &FrameWriteToken, slot: &BoundBuffer, edits: &[SdfEdit]) {
    // The borrow IS the fence/dispatcher-solo proof (mint-gated) — see `upload_camera_ring`.
    let _ = token;

    // Hard bound BEFORE the write: the encoder touches the header + the full edit array
    // (up to `EDITLIST_BUFFER_WORDS` words); an undersized slot would make that write
    // out-of-bounds. One compare on the single boot-static write.
    assert!(
        slot.size as usize >= EDITLIST_BUFFER_WORDS * 4,
        "edit-list SSBO too small: {} bytes < the {}-byte packed edit-list buffer",
        slot.size,
        EDITLIST_BUFFER_WORDS * 4
    );

    let mapped = slot
        .mapped
        .expect("invariant: the edit-list SSBO is host-visible mapped");
    // SAFETY: per this fn's contract `mapped` targets >= `slot.size` valid mapped
    // host-coherent bytes, and `slot.size >= EDITLIST_BUFFER_WORDS * 4` is hard-asserted
    // above, so a `[u32; EDITLIST_BUFFER_WORDS]` view of the mapping is in-bounds and
    // 4-byte aligned (the RHI allocates SSBOs at >= 16-byte alignment). The mapping is a
    // distinct sub-allocation, borrowed exclusively for this single write (no other
    // `&mut` alias exists — the boot-static write is the only writer). `encode_edit_list`
    // writes only initialized `u32` words.
    let buf = unsafe {
        core::slice::from_raw_parts_mut(mapped.as_ptr().cast::<u32>(), EDITLIST_BUFFER_WORDS)
    };
    encode_edit_list(buf, edits);
}

#[cfg(test)]
mod tests {
    use core::ptr::NonNull;

    use boyko_rhi_vulkan::ffi::VkBuffer;

    use super::*;

    /// A host-visible `BoundBuffer` view over caller-owned storage — the same no-GPU
    /// test hatch `boyko_app`'s `zero_alloc.rs` uses for these upload fns: only
    /// `size` and `mapped` are read, so no device is needed. `storage` outlives the
    /// slot (the caller's stack frame).
    fn fake_slot(storage: &mut [u8]) -> BoundBuffer {
        BoundBuffer {
            buffer: VkBuffer::NULL,
            offset: 0,
            size: storage.len() as u64,
            mapped: NonNull::new(storage.as_mut_ptr()),
        }
    }

    /// The rung-3b write footprint — `RESOLVED_RAY_SHADOW_BYTES` (the resolved mirror)
    /// plus the 4-byte `frame_index` seed `upload_ray_shadow_ring` appends — is 20 B,
    /// the minimum a host ring slot must be minted at for the upload to not panic.
    #[test]
    fn ray_shadow_ring_write_is_20_bytes() {
        assert_eq!(RESOLVED_RAY_SHADOW_BYTES + 4, 20);
    }

    /// The resolved mirror stays exactly 16 B: the rung-3b frame seed rides in the SAME
    /// upload but is NOT folded into the cold `ResolvedRayShadow` (cold/hot separation,
    /// one-producer-per-field — see `upload_ray_shadow_ring`'s doc).
    #[test]
    fn resolved_ray_shadow_is_still_16_bytes() {
        assert_eq!(core::mem::size_of::<ResolvedRayShadow>(), 16);
        assert_eq!(RESOLVED_RAY_SHADOW_BYTES, 16);
    }

    /// `upload_ray_shadow_ring` packs the resolved mirror into `[0..16)` and
    /// `frame_index` (little-endian) into `[16..20)` — the exact byte shape the HLSL
    /// `RayShadowUbo` cbuffer reads (`cone_radius/tmax/tmin/bias` @0, `SHADOW_FRAME_SEED`
    /// @16).
    #[test]
    fn ray_shadow_ring_packs_resolved_and_frame_seed() {
        let mut storage = [0xAAu8; 32];
        let slot = fake_slot(&mut storage);
        let resolved = ResolvedRayShadow { cone_radius: 0.035, tmax: 1e4, tmin: 1e-3, bias: 1e-3 };
        let frame_index = 0x1234_5678u32;

        // SAFETY: no GPU device exists in this process, so no submitted work can
        // reference slot 0 — the `forge_unfenced` no-fence-needed setup contract holds
        // trivially (mirrors `boyko_app`'s `zero_alloc.rs` test usage).
        let token = unsafe { FrameWriteToken::forge_unfenced(0) };
        // SAFETY: `slot.mapped` targets `storage`'s live 32-byte backing (owned by this
        // stack frame, outliving the call) — `slot.size == 32 >= 20` satisfies the hard
        // bound the fn asserts. The token/slot contract holds trivially (see above).
        unsafe {
            upload_ray_shadow_ring(&token, &slot, &resolved, frame_index);
        }

        let mut expected = [0u8; 16];
        expected[0..4].copy_from_slice(&resolved.cone_radius.to_le_bytes());
        expected[4..8].copy_from_slice(&resolved.tmax.to_le_bytes());
        expected[8..12].copy_from_slice(&resolved.tmin.to_le_bytes());
        expected[12..16].copy_from_slice(&resolved.bias.to_le_bytes());
        assert_eq!(&storage[0..16], &expected);
        assert_eq!(&storage[16..20], &frame_index.to_le_bytes());
    }
}
