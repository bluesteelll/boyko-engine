//! Wave-C capstone: the hand-written [`GpuSystem`] dispatches the `gpu_integrate`
//! compute shader ON a GPU-resident ECS column, projecting the `!Send`
//! [`RhiContext`] from the world's NonSend slab inside `run_unsafe` (MF-5).
//!
//! Flow: register a `Gpu`-classed component + a GPU-pure archetype, mint a device
//! column, build the compute pipeline from the committed `gpu_integrate.comp.spv`,
//! upload the known `write_pattern` pattern `[i*2 + 1]`, MOVE the `RhiContext` into
//! the world as a NonSend resource, build a [`GpuSystem`] over the target key, and
//! invoke it ONCE through the public `EcsMaster::run_system_once` (which mints the
//! `UnsafeEcsCell` the system projects). The shader adds `+100` to every element on
//! the GPU, so the readback golden is `(i*2 + 1) + 100` — the `golden_chained`
//! shape. The device buffer is never CPU-mapped (the only readback is the test
//! oracle, which fences first), so the +100 can only have arrived through a real
//! `vkCmdDispatch`. End with the validation-clean oracle (validation total == 0).
//!
//! Run single-threaded: `cargo test -p boyko-render -- --test-threads=1`.

mod common;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::access::Access;
use boyko_ecs::ecs::core::system::system::System;

use boyko_render::{GpuColumnManager, GpuSystem, RhiContext, gpu_integrate_spirv};
use boyko_ecs::ecs::core::system::gpu_intent::{GpuAccess, GpuAccessIntent, GpuStage};

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype};

const ROWS: usize = 1024;

/// The CPU golden for the `write_pattern` seed: `seed[i] == i*2 + 1`.
fn golden_write_pattern(i: u32) -> u32 {
    i.wrapping_mul(2).wrapping_add(1)
}

/// The CPU golden after ONE `gpu_integrate` dispatch on the seed: `(i*2 + 1) + 100`
/// — the `golden_chained` shape.
fn golden_after_one_dispatch(i: u32) -> u32 {
    golden_write_pattern(i).wrapping_add(100)
}

/// The `write_pattern` seed bytes (`[i*2 + 1]` as little-endian `u32`s).
fn seed_bytes(rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows * STRIDE as usize);
    for i in 0..rows {
        out.extend_from_slice(&golden_write_pattern(i as u32).to_le_bytes());
    }
    out
}

#[test]
fn gpu_system_dispatches_integrate_on_device_column() {
    let Some(ctx) = boot_or_skip("gpu_system_dispatches_integrate_on_device_column") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    // A low component id (< MAX_COMPONENTS = 512); each test binary has its own
    // isolated process-global registry, so the same id across files never clashes.
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 200);

    // ── Setup: mint the column, build the pipeline, upload the seed pattern. ──
    let seed = seed_bytes(ROWS);
    let (handle, pipeline) = {
        let (device, mgr): (_, &mut GpuColumnManager) = rhi.split_mut();
        let handle = mgr
            .create_column(device, &mut ecs, arch, comp, STRIDE, ROWS as u32)
            .expect("create_column");
        mgr.upload_initial(device, handle, &seed).expect("upload_initial seed");
        let pipeline = mgr
            .create_compute_pipeline(device, gpu_integrate_spirv())
            .expect("create_compute_pipeline from gpu_integrate.comp.spv");
        (handle, pipeline)
    };

    // Sanity: the resolved column matches the uploaded geometry before dispatch.
    let resolved = rhi.manager().resolve(arch, comp).expect("resolve a live column");
    assert_eq!(resolved.handle, handle, "resolve returns the current handle");
    assert_eq!(resolved.device_len, ROWS as u32, "uploaded rows recorded");

    // ── Build the GpuSystem (MF-5 / MF-7: target_key, not a cached handle). ──
    let mut intent = GpuAccessIntent::new(GpuStage::Compute);
    intent.push(handle, GpuAccess::Write);
    let mut gpu_system = GpuSystem::new(pipeline, (arch, comp), intent, Box::new([]));

    // The system declares EMPTY access — empty ⇔ it does not conflict with a
    // universal access (any read/write would). `SystemKind` is `boyko_ecs`-internal,
    // so the `is_gpu()` witness stands in for "resolves SystemKind::GpuCompute".
    assert!(
        !gpu_system.access().conflicts_with(&Access::universal()),
        "GpuSystem must declare EMPTY component/resource access (MF-5)"
    );
    assert!(
        gpu_system.is_gpu(),
        "GpuSystem must be GpuCompute-kind (registered via SystemConfig::gpu())"
    );

    // ── Move the RhiContext into the world as a NonSend resource (MF-5). ──
    ecs.insert_non_send_resource(rhi);

    // ── Invoke the system ONCE through the public run-system path (mints the
    //    UnsafeEcsCell the system projects). Wave C uses direct invocation; full
    //    Schedule integration is Wave E. ──
    ecs.run_system_once(&mut gpu_system);

    // ── Read back the device column (test-only oracle: fences first). ──
    let rhi = ecs.non_send_resource_mut::<RhiContext>();
    let got = {
        let (device, mgr) = rhi.split_mut();
        mgr.readback_for_test(device, handle, seed.len())
            .expect("readback_for_test")
    };
    assert_eq!(got.len(), seed.len(), "readback length matches the uploaded span");

    // Every element advanced by +100 — the `golden_chained`-shaped result, written
    // entirely on the GPU (the device buffer is never CPU-mapped).
    for i in 0..ROWS {
        let off = i * STRIDE as usize;
        let v = u32::from_le_bytes([got[off], got[off + 1], got[off + 2], got[off + 3]]);
        let want = golden_after_one_dispatch(i as u32);
        assert_eq!(
            v, want,
            "gpu_integrate +100 mismatch at i={i}: got {v}, want {want}"
        );
    }

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    // `rhi` is a `&mut RhiContext` borrowed from the world; the world's NonSend slab
    // drops the RhiContext (its `Drop` runs the idempotent `destroy_all` again) when
    // `ecs` drops at the end of scope.
    drop(ecs);
}
