//! **G2b (profiling rung 5a) — the label positive control.**
//!
//! **Three** pairs in one frame: one bracketed, one allocated and never bracketed, and one **begun
//! and never ended**. They must retire [`GpuLabel::Measured`] with a non-zero duration,
//! [`GpuLabel::NotBracketed`] with no numbers, and [`GpuLabel::Torn`] respectively.
//!
//! # The third pair is the whole gate, and the corpus's two clauses are not
//!
//! The corpus states G2b as two clauses — an unbracketed pass reads `NOT_BRACKETED`, a bracketed
//! one reads `MEASURED`. **Both are satisfied by a label computed from availability alone**, with
//! no witness at all, and that was MEASURED here: replacing `begun` with `available` in the label
//! match left the two-pair version of this gate GREEN. A bracketed pair is available and an
//! unbracketed one is not, so on those two inputs the witness and availability agree, and a gate
//! whose whole subject is their difference tested nothing.
//!
//! On a working driver they disagree in exactly one constructible place: a pair whose BEGIN was
//! recorded and whose END was not. Its begin query is written and its end query is not, so
//! **availability says `0`** — indistinguishable from a pass that never ran — while the witness
//! says `begun && !ended`, which is [`GpuLabel::Torn`], a recorder bug. (The fourth row, `LOST`,
//! needs a query that never returns and is not constructible on demand against a working driver; it
//! is named as not exercised on hardware rather than implied.)
//!
//! So: a stub labelling everything `NOT_BRACKETED` fails clause 2, a stub labelling everything
//! `MEASURED` fails clause 1, and **a label that reads availability where it should read the
//! witness fails clause 3** — which is the one that was missing.
//!
//! # What "non-zero duration" is worth here, stated rather than implied
//!
//! The bracketed pair encloses no work, and it still measures tens of ticks — the hardware
//! granularity, which is the floor under every GPU duration this corpus publishes. So the non-zero
//! clause is satisfied by the lattice, not by work, and what it actually discriminates is a label
//! path that carries a *real* number through versus one that fabricates a zero. That is the clause
//! worth having: `write_zero_pair`, the mechanism this whole seam replaces, reported exactly such a
//! zero and it read like a genuinely free pass.
//!
//! **The figure is deliberately not pinned, and rung 4 over-claimed by pinning it.** Rung 4 saw an
//! empty bracket read **128 ticks twice** and wrote that down as *"the hardware lattice step"*.
//! This gate's empty bracket reads **96**. Two identical readings do not establish a step — they
//! establish a common multiple of one. With 128 and 96 in hand the step divides `gcd = 32`, and it
//! is not established to be any particular value. What survives, and is the only thing this gate
//! rests on, is that an empty bracket does **not** read zero.
//!
//! # What it cannot claim
//!
//! Not that the duration is *correct* — only that it is labelled and non-zero. And not the `LOST`
//! row, for the reason above; it is pinned as a pure table in `gpu_zone`'s own unit test, where it
//! is reachable, and named here as **not exercised on hardware** rather than implied by the three
//! that are.

use boyko_rhi::{QueryPoolDesc, RhiCommandEncoder, RhiDevice, RhiQueue, TimestampStage};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::present::gpu_zone::{
    GPU_RING_DEPTH, GpuLabel, GpuZoneRecorder, QUERIES_PER_SLOT, RetireCause, RetireScratch,
};
use boyko_rhi_vulkan::rhi_impl::VulkanQueryPool;

/// The zone id the bracketed pair carries. An arbitrary non-zero value — zone `0` is the
/// engine-wide "unassigned" sentinel, so using it here would make "the recorder recorded the zone"
/// and "the recorder recorded nothing" the same assertion.
const ZONE_BRACKETED: u16 = 41;
/// The zone id of the pair that is allocated and never bracketed.
const ZONE_SILENT: u16 = 42;
/// The zone id of the pair whose BEGIN is recorded and whose END is not.
const ZONE_TORN: u16 = 43;

fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP gpu_zone_label_control: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

#[test]
fn a_bracketed_pair_measures_and_an_unbracketed_one_says_so_in_the_same_frame() {
    let Some(ctx) = boot_or_skip() else { return };
    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    if !device.device_caps().timestamps_usable() {
        eprintln!(
            "SKIP gpu_zone_label_control: GPU timestamps unusable (valid_bits={}, period={})",
            device.device_caps().timestamp_valid_bits,
            device.device_caps().timestamp_period
        );
        return;
    }

    let pools: [VulkanQueryPool; GPU_RING_DEPTH] = core::array::from_fn(|_| {
        device
            .create_query_pool(&QueryPoolDesc { count: QUERIES_PER_SLOT })
            .expect("gpu zone query pool")
    });
    let mut recorder = GpuZoneRecorder::new(pools);

    const FRAME: u32 = 0;
    const SUBMIT_EPOCH: u64 = 7;
    const RECORD_FRAME: u64 = 3;

    let slot = recorder.open_frame(FRAME, SUBMIT_EPOCH, RECORD_FRAME).expect("a fresh ring slot");
    let bracketed = recorder.alloc_pair(slot, ZONE_BRACKETED).expect("pair 0");
    let silent = recorder.alloc_pair(slot, ZONE_SILENT).expect("pair 1");
    let torn = recorder.alloc_pair(slot, ZONE_TORN).expect("pair 2");
    assert_ne!(bracketed, silent, "the bump allocator handed out one pair twice");
    assert_ne!(silent, torn, "the bump allocator handed out one pair twice");
    assert_eq!(recorder.used_pairs(slot), 3);

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("encoder");
    encoder.begin().expect("begin");
    let cmd = encoder.raw_command_buffer();
    // SAFETY: `cmd` is this encoder's live command buffer, in the recording state between `begin`
    //   and `end`; the reset is the first command, outside any render scope
    //   (`VUID-vkCmdResetQueryPool-renderpass`); `fns` is this device's table; and `bracketed` came
    //   from `alloc_pair` on this slot.
    unsafe {
        recorder.record_reset(device.device_fns(), cmd, slot);
        recorder.record_begin(device.device_fns(), cmd, slot, bracketed, TimestampStage::TopOfPipe);
        recorder.record_end(device.device_fns(), cmd, slot, bracketed);
        // The discriminating input: a BEGIN with no END. Its begin query is written and its end
        // query is not, so availability reports `0` for the pair — the same answer it gives for
        // `silent`, and the reason a label computed from availability alone cannot tell a recorder
        // bug from a pass that does not run.
        recorder.record_begin(device.device_fns(), cmd, slot, torn, TimestampStage::TopOfPipe);
    }
    // `silent` is deliberately never recorded. It is ALLOCATED — the recorder knows its zone — and
    // that is exactly the state a leg that does not run a pass leaves behind.
    encoder.end().expect("end");
    recorder.seal(slot);

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Boxed rather than a stack local: `RetireScratch` is ~9.3 KiB, and the host holds ONE beside
    // the recorder for the process's life rather than building it per frame.
    let mut scratch = Box::new(RetireScratch::new());

    let mut retired = Vec::new();
    recorder
        .retire(
            device,
            // Deliberately BELOW `submit_epoch + FRAMES_IN_FLIGHT`, so horn 1 cannot fire and the
            // only way this slot retires is by every bracketed pair actually coming back. A gate
            // that let the deadline retire the slot would be asserting the deadline, not the label.
            SUBMIT_EPOCH,
            RECORD_FRAME,
            &mut scratch,
            |frame, pairs| retired.push((frame, pairs.to_vec())),
        )
        .expect("retire");

    assert_eq!(retired.len(), 1, "the frame did not retire on its own completion");
    let (frame, pairs) = &retired[0];
    assert_eq!(frame.frame, FRAME);
    assert_eq!(frame.pairs, 3);
    assert_eq!(
        frame.cause,
        RetireCause::Complete,
        "the slot retired on a DEADLINE, so this gate would be measuring the deadline"
    );
    assert_eq!(frame.lost, 0, "a bracketed pair did not come back");
    assert_eq!(frame.torn, 1, "the torn pair was not counted");

    // Clause 2 — the one a uniformly-NOT_BRACKETED stub fails.
    let m = pairs[bracketed as usize];
    assert_eq!(m.label, GpuLabel::Measured, "a bracketed, available pair was not labelled MEASURED");
    assert_eq!(m.zone, ZONE_BRACKETED, "the pair came back under the wrong zone");
    assert!(
        m.dur_ticks > 0,
        "a MEASURED pair reported a zero duration. On this box an empty bracket still reads tens \
         of ticks (the hardware granularity), so a zero here means the label path fabricated a \
         number rather than carrying one."
    );
    assert!(m.begin_ticks > 0, "a MEASURED pair reported no begin stamp");

    // Clause 1 — the one a uniformly-MEASURED stub fails.
    let s = pairs[silent as usize];
    assert_eq!(
        s.label,
        GpuLabel::NotBracketed,
        "a pair the recorder never bracketed was labelled {:?}. Availability alone cannot say \
         this — an unbracketed pass and a lost query are both `available == 0`.",
        s.label
    );
    assert_eq!(s.dur_ticks, 0, "an unbracketed pair handed back a duration");
    assert_eq!(s.begin_ticks, 0, "an unbracketed pair handed back a begin stamp");

    // Clause 3 — the one that discriminates the WITNESS from availability, and the only one of the
    // three that does. Availability reports `0` for this pair exactly as it does for `silent`; only
    // the host-side marks can tell a half-recorded bracket from a pass that was never recorded.
    let t = pairs[torn as usize];
    assert_eq!(
        t.label,
        GpuLabel::Torn,
        "a pair with a BEGIN and no END was labelled {:?}. Availability says `0` for it and for \
         the never-bracketed pair alike, so a label reading availability where it must read the \
         witness lands here — this is the clause that catches it.",
        t.label
    );
    assert_eq!(t.zone, ZONE_TORN, "the torn pair came back under the wrong zone");
    assert_eq!(t.dur_ticks, 0, "a TORN pair handed back a duration");

    println!(
        "gpu_zone_label_control: measured pair = {} ticks at {} ns/tick, cause = {:?}",
        m.dur_ticks,
        device.device_caps().timestamp_period,
        frame.cause
    );

    // The slot is released and, on a device with host reset, immediately reusable.
    assert!(!recorder.in_flight(slot), "a retired slot is still marked in flight");
    assert_eq!(
        recorder.needs_cmd_reset(slot),
        !device.host_query_reset_supported(),
        "the fallback flag must be exactly the negation of the host-reset capability"
    );

    // SAFETY: created on `device`, and the one submission touching them was fence-waited above.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        for pool in recorder.into_pools() {
            device.destroy_query_pool(pool);
        }
    }
}
