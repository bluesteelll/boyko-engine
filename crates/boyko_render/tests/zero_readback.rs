//! Wave-E capstone golden test: the GPU-resident column is mutated ON the GPU
//! over N frames through a REAL `boyko_ecs` [`Schedule`] with **ZERO per-frame
//! CPU readback**, and the final single readback is bit-exact (Phase 5 §thesis,
//! D2, the zero-readback proof).
//!
//! # What this proves (deliverables a + c)
//!
//! 1. The [`GpuSystem`] runs through the REAL parallel scheduler — `Schedule::run`
//!    with a worker pool — NOT the Wave-C `run_system_once` shortcut. A
//!    `SystemKind::GpuCompute` system is dispatched SOLO on the dispatcher at the
//!    apply-window barrier (`running == 0`) via `run_dispatcher` (schedule.rs EXC2
//!    path). The CPU producer is a concurrent worker system; the GpuSystem is
//!    ordered `.after(producer)` so the directed edge feeds barrier lowering.
//! 2. The barrier plan is wired the production way: `lower_barriers(
//!    schedule.gpu_barrier_inputs(), consumer_key)` → `GpuSystem::set_barriers`.
//! 3. Across all N frames the device→host readback COUNTER stays 0 (no frame ever
//!    maps the device column — the only CPU touch is the ONE final test-oracle
//!    readback, which bumps the counter to exactly 1).
//! 4. After N `+100` dispatches the column is bit-exact `(i*2 + 1) + 100*N`
//!    (`golden_chained`-shaped), and the validation layer recorded 0 messages.
//!
//! The device buffer is never CPU-mapped (the DeviceLocalBlock is never mapped),
//! so the `+100*N` can only have arrived through N real `vkCmdDispatch`es driven
//! by the scheduler — the load-bearing GPU-ECS risk, retired on a real column.
//!
//! Run single-threaded: `cargo test -p boyko-render -- --test-threads=1`.

mod common;

use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::gpu_intent::{GpuAccess, GpuAccessIntent, GpuStage};
use boyko_ecs::prelude::ThreadPoolBuilder;

use boyko_render::{GpuColumnManager, GpuSystem, RhiContext, gpu_integrate_spirv, lower_barriers};

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype};

/// 1024 rows — `MIN_ARCHETYPE_FOR_PARALLEL`-shaped, a real column.
const ROWS: usize = 1024;
/// Number of scheduled frames; each `Schedule::run` adds one GPU `+100` dispatch.
const FRAMES: u32 = 16;
/// The per-dispatch increment the `gpu_integrate` shader applies.
const STEP: u32 = 100;

/// The `write_pattern` seed: `seed[i] == i*2 + 1`.
fn golden_seed(i: u32) -> u32 {
    i.wrapping_mul(2).wrapping_add(1)
}

/// The CPU golden after `frames` GPU `+100` dispatches: `(i*2 + 1) + 100*frames`
/// — the `golden_chained` shape.
fn golden_after(i: u32, frames: u32) -> u32 {
    golden_seed(i).wrapping_add(STEP.wrapping_mul(frames))
}

/// The `write_pattern` seed bytes (`[i*2 + 1]` as little-endian `u32`s).
fn seed_bytes(rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows * STRIDE as usize);
    for i in 0..rows {
        out.extend_from_slice(&golden_seed(i as u32).to_le_bytes());
    }
    out
}

#[test]
fn gpu_column_mutated_over_n_frames_with_zero_readback() {
    let Some(ctx) = boot_or_skip("gpu_column_mutated_over_n_frames_with_zero_readback") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    // A low component id (< MAX_COMPONENTS = 512); each test binary owns an
    // isolated process-global registry, so reusing 200 across files never clashes.
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 200);

    // ── Setup: mint the device column, build the pipeline, upload the seed. ──
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

    // The GpuSystem's declared intent: a COMPUTE-stage WRITE of the target column.
    let mut intent = GpuAccessIntent::new(GpuStage::Compute);
    intent.push(handle, GpuAccess::Write);

    // ── Lower the barrier plan the production way (MF-6/MF-7). ──────────────
    //
    // `lower_barriers` consumes `Schedule::gpu_barrier_inputs()`, which walks the
    // conflict graph's directed `.after(producer)` edges and filters for
    // `GpuCompute` consumers. That data only exists AFTER `build`, and a built
    // schedule boxes its systems (boyko_render cannot downcast them), so the
    // wiring is two-phase: build a PROBE schedule with the same topology (CPU
    // producer → `.gpu().after(producer)` GpuSystem) to lower the plan, then
    // construct the FINAL GpuSystem WITH that plan for the schedule we run. The
    // probe + the real schedule share the exact same edges, so the lowered plan is
    // identical. (`set_barriers` exists for the same seam; here we pass the plan to
    // `GpuSystem::new` directly — equivalent and simpler for a single consumer.)
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let target_key = (arch, comp);
    let plan = {
        let probe_intent = intent.clone();
        let probe_gpu = GpuSystem::new(pipeline, target_key, probe_intent, Box::new([]));
        let mut probe_builder = ScheduleBuilder::new(Arc::clone(&pool));
        let producer_key = probe_builder.add_system(|| {}).key();
        probe_builder.add_system(probe_gpu).gpu().after(producer_key);
        let probe_schedule = probe_builder.build(&mut ecs);

        // Resolve every transient consumer index to the durable target key — there
        // is exactly one GpuCompute consumer in this schedule (MF-7 durable key).
        let mut plans = lower_barriers(probe_schedule.gpu_barrier_inputs(), |_consumer| {
            Some(target_key)
        });
        assert_eq!(
            plans.len(),
            1,
            "exactly one GpuCompute consumer => one lowered barrier plan"
        );
        let (_consumer, plan) = plans.pop().expect("one plan");
        assert!(!plan.is_empty(), "the producer->GpuSystem edge yields >= 1 barrier");
        plan
    };

    // ── Build the REAL schedule: CPU producer || GpuSystem(.gpu().after). ───
    let gpu_system = GpuSystem::new(pipeline, target_key, intent, plan);
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let producer_key = builder.add_system(|| {}).key();
    builder.add_system(gpu_system).gpu().after(producer_key);
    let mut schedule = builder.build(&mut ecs);

    // ── Move the RhiContext into the world as a NonSend resource. The
    //    dispatcher reaches it on THIS thread inside `run_dispatcher`. ──
    ecs.insert_non_send_resource(rhi);

    // ── Run N frames through the REAL dispatcher + worker pool. ─────────────
    //
    // Each `Schedule::run` dispatches the GpuSystem solo on the dispatcher at the
    // apply-window barrier (`running == 0`) via `run_dispatcher` — the
    // dispatcher-solo `SystemKind::GpuCompute` path. Between frames we assert the
    // per-frame device→host readback counter is ZERO: the steady-state frame path
    // never maps the device column (D2).
    {
        let rhi_ref = ecs.non_send_resource_mut::<RhiContext>();
        rhi_ref.reset_readback_count();
    }
    for frame in 1..=FRAMES {
        schedule.run(&mut ecs);

        // ZERO-READBACK ORACLE: no frame performed a device→host readback.
        let rhi_ref = ecs.non_send_resource_mut::<RhiContext>();
        assert_eq!(
            rhi_ref.readback_count(),
            0,
            "frame {frame}: the steady-state frame path must perform ZERO device readbacks"
        );
    }

    // ── ONE test-oracle readback (the only CPU touch of the column). ────────
    let rhi = ecs.non_send_resource_mut::<RhiContext>();
    let got = {
        let (device, mgr) = rhi.split_mut();
        mgr.readback_for_test(device, handle, seed.len())
            .expect("readback_for_test")
    };
    assert_eq!(got.len(), seed.len(), "readback length matches the uploaded span");

    // The single readback bumped the counter to exactly 1 (proving the counter is
    // wired AND that nothing else read back during the N frames).
    assert_eq!(
        rhi.readback_count(),
        1,
        "exactly ONE readback total — the final test oracle, none during the frames"
    );

    // Deliverable (c) — the GpuSystem ran via the REAL dispatcher-solo path.
    // Element 0's value witnesses it: `run_dispatcher` is the ONLY override that
    // records a dispatch; the worker `run_unsafe` is a no-op that NEVER touches the
    // RHI. So had the scheduler mis-routed the GpuCompute system to a worker (or
    // not dispatched it at all), the column would still read the bare seed
    // `i*2 + 1`. A value advanced by `+100*N` can only come from N real
    // `vkCmdDispatch`es issued inside `run_dispatcher` on the dispatcher at
    // `running == 0` (schedule.rs EXC2). `assert_ne` against the seed makes the
    // "the dispatcher path fired" claim explicit before the bit-exact golden.
    {
        let elem0 = u32::from_le_bytes([got[0], got[1], got[2], got[3]]);
        assert_ne!(
            elem0,
            golden_seed(0),
            "elem 0 still equals the seed => the GpuSystem never dispatched \
             (it must run via run_dispatcher on the dispatcher-solo GpuCompute path)"
        );
        assert_eq!(
            elem0,
            golden_after(0, FRAMES),
            "elem 0 advanced by exactly +100*N => N real dispatches via run_dispatcher"
        );
    }

    // Bit-exact golden: N GPU `+100` dispatches over the `i*2 + 1` seed.
    for i in 0..ROWS {
        let off = i * STRIDE as usize;
        let v = u32::from_le_bytes([got[off], got[off + 1], got[off + 2], got[off + 3]]);
        let want = golden_after(i as u32, FRAMES);
        assert_eq!(
            v, want,
            "row {i}: got {v}, want (i*2+1)+100*{FRAMES} = {want} after N GPU dispatches"
        );
    }

    // The oracle: a clean N-frame run recorded zero validation messages.
    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    drop(ecs);
}
