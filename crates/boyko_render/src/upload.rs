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

use boyko_rhi_vulkan::compute::{B5_CAMERA_UBO_BYTES_M4, M2_GRID_PARAMS_OFFSET};
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::swapchain::FrameWriteToken;
use boyko_scene::ViewUniform;

use crate::csm_config::{RESOLVED_CSM_BYTES, ResolvedCsm};
use crate::mesh_draw::MeshRenderScratch;
use crate::view::composite_from_view;

/// Writes the 80-byte b5 camera block (the marcher / resolve / SSAO
/// `CompositePushConstants` image) into ONE camera-ring slot from the resolved
/// engine view — the per-frame camera upload of the production G-buffer path.
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
pub unsafe fn upload_camera_ring(
    token: &FrameWriteToken,
    ring_slot: &BoundBuffer,
    view: &ViewUniform,
    width: u32,
    height: u32,
) {
    // The borrow IS the fence proof (mint-gated); no slot index is re-derivable
    // from the buffer, so the slot identity is the caller's contract.
    let _ = token;

    let pc = composite_from_view(view, width, height);
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

/// Uploads the gathered 48-byte [`InstanceModelCol`](crate::InstanceModelCol)
/// instance ring — the interp-OFF instance path (host plan R3) — into ONE
/// instance-SSBO ring slot: ONE contiguous `bytemuck` memcpy, zero staging,
/// zero allocation (plan P2-1).
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
    let bytes: &[u8] = bytemuck::cast_slice(scratch.ring.as_slice());
    assert!(
        bytes.len() as u64 <= ring_slot.size,
        "instance ring overflow: {} gathered instances ({} bytes) exceed the \
         {}-instance ({}-byte) slot (grow the boot instance capacity; dynamic \
         growth is host plan R7)",
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
