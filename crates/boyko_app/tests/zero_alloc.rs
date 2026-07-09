//! R3/R5 zero-allocation gate (host plan performance budget: 0 heap allocations
//! per frame after warmup), scoped to the frame-loop BODY HELPERS.
//!
//! # Scoping (documented per the plan)
//!
//! The full windowed loop cannot run headless (it needs a live device +
//! window + swapchain), so this counting-allocator test drives the per-frame
//! CPU work in isolation, over fake mapped slots:
//!
//! * the unified gather refill (`MeshRenderScratch::gather_mixed_into` — clear +
//!   re-fill, capacity persists), over a MIXED input (static + interpolated rows)
//!   so the pair / out-slot lanes are exercised too,
//! * `upload_camera_ring` (composite bridge + one 80-B memcpy),
//! * `upload_instance_models` (one contiguous `cast_slice` memcpy of the unified
//!   ring — plan D5 unconditional / P2-1 no-vec-staging),
//! * `upload_pair_ring` + `upload_pair_out_slot` (the interp-ON pair + out-slot
//!   memcpys — refined-B),
//! * `gbuffer_push_from_view` (the stack push-constant bridge).
//!
//! The remaining per-frame heap surface in the runner is the draw-list
//! assembly, which reuses the host's parked `DrawListScratch` allocation
//! (crate-private; exercised by the windowed smoke). Everything else in the
//! frame body is stack POD by construction (`GBufferScene` on the stack).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_math::{Affine3A, Mat3, Vec3};
use boyko_render::gpu_transform3d::GpuTransform3D;
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::mesh_draw::MeshRenderScratch;
use boyko_render::{
    gbuffer_push_from_view, upload_camera_ring, upload_instance_models, upload_pair_out_slot,
    upload_pair_ring,
};
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::compute::B5_CAMERA_UBO_BYTES_M4;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::swapchain::FrameWriteToken;
use boyko_scene::{Projection, Transform, ViewUniform};

/// Counts every heap acquisition (alloc / alloc_zeroed / realloc); frees are
/// not counted (a free is not a per-frame allocation).
struct CountingAlloc;

static ACQUISITIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: pure delegation to `System` with a relaxed counter side-effect; the
// layout/pointer contracts are forwarded unchanged.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// A fake host-mapped ring slot over an ordinary heap buffer: the upload fns
/// read only `size` + `mapped`, so no device is needed. The backing storage
/// outlives the `BoundBuffer` (owned by the test frame).
fn fake_slot(storage: &mut Vec<u8>) -> BoundBuffer {
    BoundBuffer {
        buffer: VkBuffer::NULL,
        offset: 0,
        size: storage.len() as u64,
        mapped: core::ptr::NonNull::new(storage.as_mut_ptr()),
    }
}

/// A distinct test affine per `(mesh, ordinal)` so the memcpy'd bytes vary.
fn affine(mesh_id: u32, ordinal: u32) -> InstanceModelCol {
    InstanceModelCol {
        rows: [
            [1.0, 0.0, 0.0, mesh_id as f32],
            [0.0, 1.0, 0.0, ordinal as f32],
            [0.0, 0.0, 1.0, 0.0],
        ],
    }
}

#[test]
fn frame_helpers_allocate_zero_after_warmup() {
    const FRAMES: usize = 100;
    const W: u32 = 512;
    const H: u32 = 512;

    // ── Setup (allocations allowed): the fake slots, the view, the inputs. ──
    let mut cam_storage = vec![0u8; B5_CAMERA_UBO_BYTES_M4];
    let mut inst_storage = vec![0u8; 1024 * 48];
    let mut pair_storage = vec![0u8; 1024 * 96];
    let mut out_slot_storage = vec![0u8; 1024 * 4];
    let cam_slot = fake_slot(&mut cam_storage);
    let inst_slot = fake_slot(&mut inst_storage);
    let pair_slot = fake_slot(&mut pair_storage);
    let out_slot_slot = fake_slot(&mut out_slot_storage);

    let view = ViewUniform::from_camera(
        Affine3A {
            matrix3: Mat3::IDENTITY,
            translation: Vec3::new(0.0, 1.7, 6.0),
        },
        Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    );

    // A MIXED input: 64 rows, every 4th carries an interpolation pair (a dynamic
    // body). The gather scatters the static rows into the ring and records the
    // dynamic rows' pairs + out-slots — the R5 pair/out-slot lanes are exercised
    // inside the measured loop.
    let records: Vec<InstanceModelCol> = (0..64).map(|i| affine(i % 2, i)).collect();
    let pairs: Vec<GpuTransform3D> = (0..64)
        .map(|i| {
            GpuTransform3D::from_transform(&Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)))
        })
        .collect();
    let inputs: Vec<(u32, &InstanceModelCol, Option<&GpuTransform3D>)> = records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let pair = if i % 4 == 0 { Some(&pairs[i]) } else { None };
            ((i as u32) % 2, r, pair)
        })
        .collect();
    let meta = |_mesh: u32| (36u32, IndexType::Uint16);

    // SAFETY: no GPU work exists in this process (no device was booted), so no
    // submitted work can reference the fake slots — the `forge_unfenced` setup
    // seeding contract holds trivially.
    let token = unsafe { FrameWriteToken::forge_unfenced(0) };

    let mut scratch = MeshRenderScratch::default();

    // ── Warmup: one full frame body grows every lane/ring to steady state. ──
    scratch.gather_mixed_into(2, meta, || inputs.iter().copied());
    // SAFETY (all uploads, warmup + measured loop): each fake slot's `mapped`
    // points to a LIVE heap buffer of exactly `size` bytes (`fake_slot` over the
    // outliving storage Vecs), satisfying the memory precondition; the
    // token/slot contract holds trivially — no GPU work exists in this process,
    // so nothing reads the slots.
    unsafe {
        upload_camera_ring(&token, &cam_slot, &view, W, H);
        upload_instance_models(&token, &inst_slot, &scratch);
        upload_pair_ring(&token, &pair_slot, &scratch);
        upload_pair_out_slot(&token, &out_slot_slot, &scratch);
    }
    let _ = gbuffer_push_from_view(&view, W, H, true);

    // Precondition: the mixed input actually produced dynamic rows (so the pair /
    // out-slot uploads have work — otherwise the coverage would be vacuous).
    assert_eq!(scratch.dynamic_count(), 16, "every 4th of 64 rows is a dynamic body");

    // ── Measured frames: zero heap acquisitions (plan budget). ──────────────
    let before = ACQUISITIONS.load(Ordering::Relaxed);
    for _ in 0..FRAMES {
        scratch.gather_mixed_into(2, meta, || inputs.iter().copied());
        // SAFETY: identical contract to the warmup uploads above (the same live
        // fake slots, the same no-GPU token argument).
        unsafe {
            upload_camera_ring(&token, &cam_slot, &view, W, H);
            upload_instance_models(&token, &inst_slot, &scratch);
            upload_pair_ring(&token, &pair_slot, &scratch);
            upload_pair_out_slot(&token, &out_slot_slot, &scratch);
        }
        let mvp = gbuffer_push_from_view(&view, W, H, true);
        // Keep the push observable so the bridge is not optimized away.
        assert_eq!(mvp[84], 1, "instanced arm selected");
    }
    let after = ACQUISITIONS.load(Ordering::Relaxed);

    assert_eq!(
        after - before,
        0,
        "the frame helpers must allocate ZERO heap after warmup \
         ({} acquisitions over {FRAMES} frames)",
        after - before
    );

    // Sanity: the uploads actually wrote — the instance slot's leading bytes
    // equal the gathered ring's first record, and the out-slot slot's first
    // entry equals the first dynamic row's ring slot.
    let expect: &[u8] = bytemuck::bytes_of(&scratch.ring[0]);
    assert_eq!(&inst_storage[..48], expect, "the instance memcpy landed");
    let first_out_slot = u32::from_le_bytes(out_slot_storage[..4].try_into().unwrap());
    assert_eq!(
        first_out_slot, scratch.pair_out_slot[0],
        "the out-slot memcpy landed (first dynamic row's ring slot)"
    );
}
