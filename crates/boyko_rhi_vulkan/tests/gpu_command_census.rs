//! **G5 (profiling rung 5b) — the command census, two-sided.**
//!
//! One recording function, run twice against the same command buffer shape:
//!
//! * **disarmed** (`recorder: None`) ⇒ every profiling counter is `0`;
//! * **armed** ⇒ `timestamps == 2 × recorded_pairs` **and** `recorded_pairs` equals the count the
//!   host declared.
//!
//! # Why the arithmetic is an EQUALITY and not a lower bound
//!
//! An earlier form of the armed clause was `timestamps >= 2`. The instrument's own null probe —
//! two back-to-back stamps the collector recorded for its own calibration — satisfied that by
//! itself, so a recorder that dropped **every real bracket** passed. An equality against a
//! host-declared count has no such slack: the only way to satisfy it is to record exactly the
//! brackets that were declared.
//!
//! # The disarmed clause needs a positive control, and the corpus does not say so
//!
//! *"Every profiling counter is 0"* is also true of a witness that was never threaded through
//! anything — the vacuous-green shape this campaign keeps finding. So the disarmed leg additionally
//! asserts `stream_pos > 0`: the witness saw the frame's ordinary commands and reported no
//! profiling ones, which is a different statement from "the witness saw nothing".
//!
//! # Why this is not a golden pin
//!
//! `goldens/PINS.toml` pins the SHA-256 of a dumped BMP. A `vkCmdResetQueryPool` plus two
//! `vkCmdWriteTimestamp`s change **zero pixels**, so a pin is structurally incapable of the claim.
//! It is measured where the commands are instead.
//!
//! # What it cannot claim
//!
//! That the GPU *executed* anything. This is a host-side record of what was recorded, and a
//! recorded command whose every effect is empty leaves no observable trace — the same limit
//! `VbRecordProbe`'s own header states about itself. It stops at "the host recorded it".

#![cfg(feature = "profiling-census")]

use boyko_rhi::{QueryPoolDesc, RhiCommandEncoder, RhiDevice, RhiQueue};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::present::command_witness::CommandWitness;
use boyko_rhi_vulkan::present::gpu_zone::{
    GPU_RING_DEPTH, GpuZoneRecorder, QUERIES_PER_SLOT,
};
use boyko_rhi_vulkan::rhi_impl::{VulkanCommandEncoder, VulkanQueryPool};

/// The zones the armed leg brackets, in the order the recorder opens them.
const ZONES: [u16; 3] = [11, 22, 33];

/// Ordinary, non-profiling commands the "scene" records between its brackets. Any recorded
/// `vkCmd*` would do; a pool reset on a scratch pool is the cheapest one that is unambiguously a
/// command and touches nothing else.
const SCENE_COMMANDS: u32 = 4;

fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP gpu_command_census: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// The one recording function both legs run.
///
/// `recorder` is `Option` for the same reason the tree's existing collectors are threaded as
/// `Option<&TimestampCollector>`: the disarmed frame must run the SAME function, or the census
/// would be comparing two different recorders rather than one recorder in two states.
///
/// The witness increments sit **at the `vkCmd*` sites**, never derived from `recorder.is_some()` —
/// a counter derived from the arming predicate agrees with the predicate by construction, which is
/// the tautology this gate exists to avoid.
///
/// # The "scene" commands are query-pool resets, and the classification is the CALLER's
///
/// The ordinary commands between the brackets are `reset_query_pool` calls on a **scratch** pool,
/// through the RHI encoder. They are recorded `vkCmd*`s and nothing else — the witness classifies
/// by the site that calls it, not by what the command happens to be, which is exactly why the
/// counters are incremented at the site. Any recorded command would do; this is the cheapest one
/// this crate's public surface can record with no pipeline, no buffer and no descriptor set.
///
/// # Safety
///
/// The encoder must be between `begin()` and `end()`, and `scratch_pool` must be a live pool on
/// this device with at least one query.
unsafe fn record_scene(
    device: &VulkanContext,
    encoder: &mut VulkanCommandEncoder,
    scratch_pool: &VulkanQueryPool,
    witness: &mut CommandWitness,
    recorder: Option<(&GpuZoneRecorder, usize)>,
) {
    let fns = device.device_fns();
    let cmd = encoder.raw_command_buffer();

    if let Some((rec, slot)) = recorder {
        // SAFETY: caller contract; the reset is the region's first command, outside any render
        //   scope (`VUID-vkCmdResetQueryPool-renderpass`).
        unsafe { rec.record_reset(fns, cmd, slot) };
        witness.query_reset();
    }

    for zone in ZONES {
        let pair = recorder.map(|(rec, slot)| {
            let pair = rec.alloc_pair(slot, zone).expect("a pair");
            // SAFETY: caller contract; `pair` came from `alloc_pair` on this slot.
            unsafe { rec.record_begin(fns, cmd, slot, pair) };
            witness.open_pair(zone);
            witness.timestamp();
            pair
        });

        // The "scene": ordinary recorded commands between the brackets. Their only job is to move
        // the stream position, so a bracket that shifts by one command is visible as a shifted
        // position rather than as nothing.
        for _ in 0..SCENE_COMMANDS {
            encoder.reset_query_pool(scratch_pool, 0, 1);
            witness.command();
        }

        if let (Some((rec, slot)), Some(pair)) = (recorder, pair) {
            // SAFETY: caller contract; `pair` was allocated for this slot immediately above.
            unsafe { rec.record_end(fns, cmd, slot, pair) };
            witness.timestamp();
        }
    }
}

#[test]
fn the_disarmed_frame_records_no_profiling_command_and_the_armed_one_records_exactly_the_declared_brackets()
 {
    let Some(ctx) = boot_or_skip() else { return };
    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    if !device.device_caps().timestamps_usable() {
        eprintln!("SKIP gpu_command_census: GPU timestamps unusable");
        return;
    }

    let scratch_pool =
        device.create_query_pool(&QueryPoolDesc { count: 1 }).expect("scratch query pool");
    let pools: [VulkanQueryPool; GPU_RING_DEPTH] = core::array::from_fn(|_| {
        device
            .create_query_pool(&QueryPoolDesc { count: QUERIES_PER_SLOT })
            .expect("gpu zone query pool")
    });
    let mut recorder = GpuZoneRecorder::new(pools);

    // ── Leg 1: DISARMED. Same function, no recorder. ──
    let mut disarmed = CommandWitness::new();
    {
        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("encoder");
        encoder.begin().expect("begin");
        // SAFETY: the encoder is between `begin` and `end`, so `cmd` is recording; `scratch_pool`
        //   is live on this device; no render scope is open.
        unsafe {
            record_scene(device, &mut encoder, &scratch_pool, &mut disarmed, None);
        }
        encoder.end().expect("end");
        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");
        // SAFETY: fence-waited above, so neither is GPU-referenced.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
    }

    assert_eq!(disarmed.profiling_cmds(), 0, "a disarmed frame recorded a profiling command");
    assert_eq!(disarmed.query_resets(), 0);
    assert_eq!(disarmed.timestamps(), 0);
    assert_eq!(disarmed.recorded_pairs(), 0);
    assert!(disarmed.stamp_positions().is_empty());
    assert!(disarmed.zone_open_order().is_empty());
    // THE POSITIVE CONTROL. Without this, a witness that was never threaded through anything would
    // satisfy every line above.
    assert_eq!(
        disarmed.stream_pos(),
        SCENE_COMMANDS * ZONES.len() as u32,
        "the disarmed leg's witness saw no commands at all, so its zeros say nothing about the \
         profiler — they say the instrument was dead"
    );

    // ── Leg 2: ARMED. The same function, with a recorder. ──
    let slot = recorder.open_frame(0, 0, 0).expect("a fresh ring slot");
    let mut armed = CommandWitness::new();
    {
        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("encoder");
        encoder.begin().expect("begin");
        // SAFETY: as leg 1, plus `slot` was just opened on `recorder`.
        unsafe {
            record_scene(device, &mut encoder, &scratch_pool, &mut armed, Some((&recorder, slot)));
        }
        encoder.end().expect("end");
        recorder.seal(slot);
        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");
        // SAFETY: fence-waited above.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
    }

    // The host-declared count. THIS is what makes the armed clause an equality rather than a lower
    // bound: it is stated here, independently of anything the recorder reports.
    let declared_brackets = ZONES.len() as u16;

    assert_eq!(
        armed.recorded_pairs(),
        declared_brackets,
        "the recorder opened {} pairs against {declared_brackets} declared brackets",
        armed.recorded_pairs()
    );
    assert_eq!(
        armed.timestamps(),
        u32::from(declared_brackets) * 2,
        "a bracket is two timestamps; this leg recorded {} for {declared_brackets} brackets",
        armed.timestamps()
    );
    assert!(armed.timestamps_pair_up(), "the pairing equality does not hold");
    assert_eq!(armed.query_resets(), 1, "the frame top must reset the pool exactly once");
    assert_eq!(
        armed.profiling_cmds(),
        armed.query_resets() + armed.timestamps(),
        "profiling_cmds must be exactly the resets plus the timestamps it counted"
    );
    assert_eq!(
        armed.zone_open_order(),
        &ZONES,
        "the record-order witness disagrees with the order the recorder opened the pairs in"
    );

    // The stream positions are strictly increasing, and each bracket's close is `SCENE_COMMANDS`
    // past its open — the property that makes a bracket shifted by one command visible.
    let positions = armed.stamp_positions();
    assert_eq!(positions.len(), usize::from(declared_brackets) * 2);
    for w in positions.windows(2) {
        assert!(w[0] < w[1], "stream positions must be strictly increasing: {positions:?}");
    }
    for b in 0..usize::from(declared_brackets) {
        assert_eq!(
            positions[b * 2 + 1] - positions[b * 2],
            SCENE_COMMANDS + 1,
            "bracket {b} does not span the {SCENE_COMMANDS} scene commands recorded inside it"
        );
    }

    println!(
        "gpu_command_census: disarmed stream_pos={} profiling_cmds={} | armed stream_pos={} \
         profiling_cmds={} pairs={} stamps={:?}",
        disarmed.stream_pos(),
        disarmed.profiling_cmds(),
        armed.stream_pos(),
        armed.profiling_cmds(),
        armed.recorded_pairs(),
        positions
    );

    // SAFETY: every submission touching these pools was fence-waited above.
    unsafe {
        device.destroy_query_pool(scratch_pool);
        for pool in recorder.into_pools() {
            device.destroy_query_pool(pool);
        }
    }
}
