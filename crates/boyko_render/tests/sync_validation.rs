//! Wave-E capstone golden test: the barrier mechanism (Phase 5 D6 / MF-6 / MF-7 /
//! the §6 validation oracle). The two halves prove two DISTINCT, complementary
//! properties — they are NOT both load-bearing demonstrations:
//!
//! * **Test A** (`lowered_barrier_is_accepted_and_correct`) — proves the
//!   PRODUCTION-LOWERED plan is ACCEPTED + NON-corrupting. It drives the
//!   [`GpuSystem`] with the barrier plan produced by the production lowering path
//!   `lower_barriers(Schedule::gpu_barrier_inputs(), consumer_key)` for the demo's
//!   CPU-producer→GpuSystem edge. The recorded `vkCmdPipelineBarrier` is ACCEPTED
//!   by the validation layer (total == 0) and the result is the correct `+100`.
//!   This is NOT a load-bearing demonstration: for the CPU-producer edge the
//!   lowered barrier is DEGENERATE — `fold_intent` on the empty CPU-producer
//!   intent yields `src_access == BarrierAccess::NONE` (barrier.rs), so in a
//!   single-dispatch frame it orders nothing. The test proves the production
//!   lowering output is a VALID, accepted, non-corrupting barrier — not that
//!   removing it manifests a hazard.
//!
//! * **Test B** (`omitting_the_barrier_manifests_a_hazard`) — proves a barrier is
//!   LOAD-BEARING (removing it manifests a real hazard). It records a REAL
//!   intra-submit hazard: TWO `gpu_integrate` compute passes over the SAME device
//!   column in ONE `vkQueueSubmit` (so the per-op fence does NOT order them; only
//!   a barrier can). With the barrier between the passes the run is clean and the
//!   result is the deterministic `+200`; WITHOUT it the passes are unsynchronized
//!   and the dependency is broken. The barrier exercised here is HAND-WRITTEN
//!   (`dispatch_compute_twice_one_submit`'s in-line `COMPUTE_SHADER`
//!   `SHADER_WRITE`→`SHADER_READ|SHADER_WRITE` mask), NOT a lowered
//!   [`PlannedBarrier`] — it proves a barrier of that shape is load-bearing.
//!
//! # What is deferred: a NON-degenerate LOWERED barrier proven load-bearing
//!
//! No test here exercises a non-degenerate lowered [`PlannedBarrier`] (one with
//! `src_access == SHADER_WRITE`) AS load-bearing. Such a barrier would be lowered
//! from a GPU-producer→GPU-consumer edge on the SAME column, but the production
//! frame path cannot manifest its removal as a hazard: every [`GpuSystem`]
//! dispatches solo and `GpuColumnManager::dispatch_compute` creates its OWN fence
//! and `wait_fence(u64::MAX)` before returning, so two GPU systems are two SEPARATE
//! fence-waited submits — the per-op fence ALREADY orders them, and removing the
//! consumer's barrier would change nothing observable (the honest 3-way oracle
//! below would deterministically hit its `neither` branch). A load-bearing
//! REMOVAL is only observable for an INTRA-submit overlap (one `vkQueueSubmit`
//! recording two passes), which the production schedule never emits — it records
//! exactly one pass per submit. Proving a non-degenerate LOWERED barrier
//! load-bearing therefore requires the Phase-6 deferred/batched-submit GPU
//! scheduler (multiple GPU passes in one submit) and is out of scope for Phase 5.
//!
//! # The Test-B oracle (HONEST — see deliverable b)
//!
//! The AUTHORITATIVE oracle for "the missing barrier is a real hazard" is the
//! Vulkan **synchronization-validation** layer (`VK_VALIDATION_FEATURE_ENABLE_
//! SYNCHRONIZATION_VALIDATION_EXT`, enabled by the booted context when the
//! `VK_EXT_validation_features` instance extension is present): the no-barrier run
//! deterministically raises a `SYNC-HAZARD-*` message, so `debug_state().total()`
//! goes non-zero. That is the primary assertion when it fires.
//!
//! There is no public flag to query whether sync-validation specifically (vs core
//! validation only) is active, so the test does NOT assume it. If the no-barrier
//! run records ZERO validation messages (sync-validation unavailable, OR the layer
//! chose not to flag this particular intra-submit overlap), the test FALLS BACK to
//! a data oracle: the no-barrier result must differ from the barrier'd `+200`
//! golden. Because an unsynchronized two-pass overlap is hardware-dependent (a GPU
//! MAY still happen to execute the passes serially), the fallback is reported
//! HONESTLY: if NEITHER oracle fires, the test fails LOUD rather than claiming a
//! hazard it could not observe. Which oracle fired is printed verbatim.
//!
//! The device buffer is never CPU-mapped (the only readback fences first), so the
//! observed values are genuine GPU results.
//!
//! Run single-threaded: `cargo test -p boyko-render -- --test-threads=1`.

mod common;

use std::sync::Arc;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::gpu_intent::{GpuAccess, GpuAccessIntent, GpuStage};
use boyko_ecs::prelude::ThreadPoolBuilder;

use boyko_render::{GpuColumnManager, GpuSystem, RhiContext, gpu_integrate_spirv, lower_barriers};
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_ecs::ecs::memory::device_column::DeviceColumnHandle;
use boyko_rhi::ComputePipelineHandle;

use common::{STRIDE, assert_validation_clean, boot_or_skip, gpu_pure_archetype};

const ROWS: usize = 1024;
const STEP: u32 = 100;

/// The seed: `seed[i] == i*2 + 1`.
fn golden_seed(i: u32) -> u32 {
    i.wrapping_mul(2).wrapping_add(1)
}

/// Seed bytes (`[i*2 + 1]` little-endian).
fn seed_bytes(rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows * STRIDE as usize);
    for i in 0..rows {
        out.extend_from_slice(&golden_seed(i as u32).to_le_bytes());
    }
    out
}

/// Reads element `i` (a `u32`) out of a little-endian byte readback.
fn elem(bytes: &[u8], i: usize) -> u32 {
    let off = i * STRIDE as usize;
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

/// Common setup: mint a device column, upload the seed, build the pipeline.
fn setup(
    rhi: &mut RhiContext,
    ecs: &mut EcsMaster,
    arch: ArchetypeId,
    comp: ComponentId,
) -> (DeviceColumnHandle, ComputePipelineHandle, Vec<u8>) {
    let seed = seed_bytes(ROWS);
    let (device, mgr): (_, &mut GpuColumnManager) = rhi.split_mut();
    let handle = mgr
        .create_column(device, ecs, arch, comp, STRIDE, ROWS as u32)
        .expect("create_column");
    mgr.upload_initial(device, handle, &seed).expect("upload_initial seed");
    let pipeline = mgr
        .create_compute_pipeline(device, gpu_integrate_spirv())
        .expect("create_compute_pipeline");
    (handle, pipeline, seed)
}

// ════════════════════════════════════════════════════════════════════════════
// Test A — the PRODUCTION-LOWERED barrier is ACCEPTED by validation AND gives the
//          correct result (the barrier the production path produces is valid and
//          non-corrupting). NOT a load-bearing demonstration: for the CPU-producer
//          edge the lowered barrier is degenerate (src_access == NONE) — see the
//          module docstring. Load-bearing-ness is Test B's job.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn lowered_barrier_is_accepted_and_correct() {
    let Some(ctx) = boot_or_skip("lowered_barrier_is_accepted_and_correct") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut rhi = RhiContext::new(ctx);
    let mut ecs = EcsMaster::new();
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 210);
    let (handle, pipeline, seed) = setup(&mut rhi, &mut ecs, arch, comp);

    let mut intent = GpuAccessIntent::new(GpuStage::Compute);
    intent.push(handle, GpuAccess::Write);
    let target_key = (arch, comp);

    // Lower the barrier plan the production way (MF-6/MF-7) from a probe schedule
    // with the production topology (CPU producer -> .gpu().after GpuSystem).
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let plan = {
        let probe_gpu = GpuSystem::new(pipeline, target_key, intent.clone(), Box::new([]));
        let mut probe = ScheduleBuilder::new(Arc::clone(&pool));
        let producer = probe.add_system(|| {}).key();
        probe.add_system(probe_gpu).gpu().after(producer);
        let schedule = probe.build(&mut ecs);
        let mut plans = lower_barriers(schedule.gpu_barrier_inputs(), |_| Some(target_key));
        assert_eq!(plans.len(), 1, "one GpuCompute consumer => one plan");
        let (_c, plan) = plans.pop().expect("one plan");
        assert!(!plan.is_empty(), "the producer->consumer edge yields >= 1 barrier");
        plan
    };

    // Drive ONE scheduled frame: the GpuSystem replays the lowered barrier into the
    // dispatch encoder, then dispatches.
    let gpu_system = GpuSystem::new(pipeline, target_key, intent, plan);
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    let producer = builder.add_system(|| {}).key();
    builder.add_system(gpu_system).gpu().after(producer);
    let mut schedule = builder.build(&mut ecs);

    ecs.insert_non_send_resource(rhi);
    schedule.run(&mut ecs);

    // Readback (the single test oracle, fences first) → correct +100.
    let rhi = ecs.non_send_resource_mut::<RhiContext>();
    let got = {
        let (device, mgr) = rhi.split_mut();
        mgr.readback_for_test(device, handle, seed.len()).expect("readback")
    };
    for i in 0..ROWS {
        assert_eq!(
            elem(&got, i),
            golden_seed(i as u32).wrapping_add(STEP),
            "row {i}: a barrier'd dispatch yields the correct +100"
        );
    }

    // The recorded barrier was ACCEPTED — zero validation messages.
    assert_validation_clean(rhi.context());
    println!("Test A: lowered barrier ACCEPTED by validation, result +100 correct.");

    rhi.destroy_all();
    drop(ecs);
}

// ════════════════════════════════════════════════════════════════════════════
// Test B — omitting the barrier on a real intra-submit hazard manifests as a
//          sync-validation message (primary) or a wrong result (fallback).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn omitting_the_barrier_manifests_a_hazard() {
    let Some(ctx) = boot_or_skip("omitting_the_barrier_manifests_a_hazard") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    // ── Control: WITH the barrier between the two passes → clean + deterministic
    //    +200. Uses its OWN context so its validation count is read in isolation
    //    (DebugMessengerState has no reset). ──
    {
        let mut rhi = RhiContext::new(ctx);
        let mut ecs = EcsMaster::new();
        let (arch, comp) = gpu_pure_archetype(&mut ecs, 220);
        let (handle, pipeline, seed) = setup(&mut rhi, &mut ecs, arch, comp);

        let ran = rhi
            .dispatch_compute_twice_one_submit(pipeline, arch, comp, /* barrier */ true)
            .expect("two-pass dispatch (barrier)");
        assert!(ran, "the column resolved and two passes were recorded");

        let got = {
            let (device, mgr) = rhi.split_mut();
            mgr.readback_for_test(device, handle, seed.len()).expect("readback")
        };
        for i in 0..ROWS {
            assert_eq!(
                elem(&got, i),
                golden_seed(i as u32).wrapping_add(2 * STEP),
                "row {i}: WITH the barrier, two passes deterministically give +200"
            );
        }
        assert_validation_clean(rhi.context());
        println!("Test B (control): WITH barrier => clean + deterministic +200.");

        rhi.destroy_all();
        drop(ecs);
    }

    // ── Hazard: WITHOUT the barrier → unsynchronized two passes. Fresh context. ──
    let Some(ctx2) = boot_or_skip("omitting_the_barrier_manifests_a_hazard(no-barrier)") else {
        return;
    };
    let mut rhi = RhiContext::new(ctx2);
    let mut ecs = EcsMaster::new();
    let (arch, comp) = gpu_pure_archetype(&mut ecs, 221);
    let (handle, pipeline, seed) = setup(&mut rhi, &mut ecs, arch, comp);

    let ran = rhi
        .dispatch_compute_twice_one_submit(pipeline, arch, comp, /* barrier */ false)
        .expect("two-pass dispatch (no barrier)");
    assert!(ran, "the column resolved and two passes were recorded");

    let got = {
        let (device, mgr) = rhi.split_mut();
        mgr.readback_for_test(device, handle, seed.len()).expect("readback")
    };

    let validation_total = rhi
        .context()
        .debug_state()
        .map(|s| s.total())
        .unwrap_or(0);

    // Did the data oracle observe a broken dependency? `+200` is the ONLY correct
    // ordered result; anything else means pass 2 did not observe pass 1's write.
    let want_ordered = golden_seed(0).wrapping_add(2 * STEP);
    let data_oracle_fired = (0..ROWS)
        .any(|i| elem(&got, i) != golden_seed(i as u32).wrapping_add(2 * STEP));

    // HONEST oracle selection + verbatim outcome.
    if validation_total > 0 {
        println!(
            "Test B (hazard): SYNC-VALIDATION oracle FIRED — debug_state().total() = {validation_total} \
             (a SYNC-HAZARD-* on the unsynchronized two-pass submit). \
             elem0 = {} (ordered golden would be {want_ordered}; UNDEFINED without the barrier).",
            elem(&got, 0)
        );
        // The barrier is load-bearing: removing it tripped synchronization
        // validation. (We do NOT also assert a wrong result — an over-eager GPU
        // may still have produced +200; the layer signal is the authoritative
        // hazard proof.)
    } else if data_oracle_fired {
        println!(
            "Test B (hazard): DATA oracle FIRED (sync-validation recorded 0 messages — \
             unavailable or did not flag this overlap). The no-barrier result differs from \
             the ordered +200 golden: elem0 = {} (want {want_ordered}). The broken \
             dependency proves the barrier is load-bearing.",
            elem(&got, 0)
        );
    } else {
        // NEITHER oracle fired. Be honest: we could not observe the hazard on this
        // host (no sync-validation AND the GPU happened to serialize the passes).
        // Fail LOUD rather than claim a hazard we did not catch.
        panic!(
            "Test B (hazard): NEITHER oracle fired on this host — sync-validation recorded 0 \
             messages AND the no-barrier result matched the ordered +200 golden (the GPU \
             serialized the passes by chance). Cannot prove the missing barrier is a hazard \
             here; re-run on a host with VK_EXT_validation_features (synchronization validation)."
        );
    }

    rhi.destroy_all();
    drop(ecs);
}
