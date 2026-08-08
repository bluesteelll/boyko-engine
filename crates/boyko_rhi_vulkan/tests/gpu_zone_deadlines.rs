//! **The two retire horns, and the `LOST` row `G2b` could not reach** (profiling rung 5a).
//!
//! `G2b` exercises the `Complete` path only: every bracketed pair comes back and the slot retires
//! on its own completion. That leaves the subtlest code in the recorder ungated — the two
//! independent deadlines and the grace decrement between them, which is the one place an earlier
//! form of this design executed `0u8 - 1`: a debug panic, or in release a wrap to 255 that
//! silently restarts the deadline for another 255 frames.
//!
//! # How a pair is made LOST on purpose
//!
//! `LOST` means bracketed and never available, and against a working driver a query that never
//! returns is not constructible. It IS constructible one level up: submit the pool **reset** alone
//! and fence-wait it, so every query is definitively in the reset (unavailable) state, then record
//! the brackets into a second command buffer that is **never submitted**. The host-side witness
//! marks say begun-and-ended; the queries were never written. That is exactly the state a frame
//! whose submission was lost leaves behind, and it is the state the blocking design could not
//! express — it hung instead.
//!
//! The reset is submitted rather than merely recorded because Vulkan leaves a never-reset query's
//! state undefined, and a gate reading undefined state is asking the driver a question instead of
//! asking the recorder one.

use boyko_rhi::{QueryPoolDesc, RhiCommandEncoder, RhiDevice, RhiQueue};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::present::FRAMES_IN_FLIGHT;
use boyko_rhi_vulkan::present::gpu_zone::{
    GPU_FRAME_DEADLINE, GPU_RING_DEPTH, GpuLabel, GpuZoneRecorder, QUERIES_PER_SLOT,
    RETIRE_GRACE_FRAMES, RetireCause, RetireScratch,
};
use boyko_rhi_vulkan::rhi_impl::VulkanQueryPool;

const ZONE: u16 = 77;

fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP gpu_zone_deadlines: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// Submit `slot`'s pool reset on its own and wait for it, leaving every query definitively
/// unavailable.
fn reset_pool_and_wait(device: &VulkanContext, recorder: &GpuZoneRecorder, slot: usize) {
    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("encoder");
    encoder.begin().expect("begin");
    // SAFETY: the encoder is recording between `begin` and `end`; the reset is the only command
    //   and no render scope is open (`VUID-vkCmdResetQueryPool-renderpass`).
    unsafe { recorder.record_reset(device.device_fns(), encoder.raw_command_buffer(), slot) };
    encoder.end().expect("end");
    device.rhi_queue().submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");
    // SAFETY: fence-waited above, so neither is GPU-referenced.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
    }
}

/// Record a full bracket into a command buffer that is **never submitted**, then seal.
fn bracket_without_submitting(device: &VulkanContext, recorder: &GpuZoneRecorder, slot: usize) {
    let mut encoder = device.create_command_encoder().expect("encoder");
    encoder.begin().expect("begin");
    let cmd = encoder.raw_command_buffer();
    let pair = recorder.alloc_pair(slot, ZONE).expect("a pair");
    // SAFETY: the encoder is recording; `pair` came from `alloc_pair` on this slot, whose pool was
    //   reset and fence-waited above.
    unsafe {
        recorder.record_begin(device.device_fns(), cmd, slot, pair);
        recorder.record_end(device.device_fns(), cmd, slot, pair);
    }
    encoder.end().expect("end");
    recorder.seal(slot);
    // Deliberately NOT submitted. SAFETY: nothing was ever submitted, so the encoder is not
    //   GPU-referenced and destroying it is sound.
    unsafe { device.destroy_command_encoder(encoder) };
}

#[test]
fn the_epoch_horn_spends_its_grace_and_then_retires_the_slot_as_lost() {
    let Some(ctx) = boot_or_skip() else { return };
    let device: &VulkanContext = &ctx;
    if !device.device_caps().timestamps_usable() {
        eprintln!("SKIP gpu_zone_deadlines: GPU timestamps unusable");
        return;
    }

    let pools: [VulkanQueryPool; GPU_RING_DEPTH] = core::array::from_fn(|_| {
        device.create_query_pool(&QueryPoolDesc { count: QUERIES_PER_SLOT }).expect("pool")
    });
    let mut recorder = GpuZoneRecorder::new(pools);
    let mut scratch = Box::new(RetireScratch::new());

    const SUBMIT_EPOCH: u64 = 100;
    const RECORD_FRAME: u64 = 500;
    let slot = recorder.open_frame(0, SUBMIT_EPOCH, RECORD_FRAME).expect("a fresh slot");
    reset_pool_and_wait(device, &recorder, slot);
    bracket_without_submitting(device, &recorder, slot);

    let mut retired = Vec::new();
    let poll = |rec: &mut GpuZoneRecorder,
                    scratch: &mut RetireScratch,
                    epoch: u64,
                    frame: u64,
                    out: &mut Vec<(RetireCause, GpuLabel, u16)>| {
        rec.retire(device, epoch, frame, scratch, |f, pairs| {
            out.push((f.cause, pairs[0].label, f.lost));
        })
        .expect("retire");
    };

    // Neither horn: the epoch is short of `submit_epoch + FRAMES_IN_FLIGHT` and the frame counter
    // has not moved. Nothing may retire — a slot whose data might still arrive is not a slot to
    // give up on.
    poll(&mut recorder, &mut scratch, SUBMIT_EPOCH, RECORD_FRAME, &mut retired);
    assert!(retired.is_empty(), "a slot retired before either deadline could fire");
    assert!(recorder.in_flight(slot), "the slot was released with no deadline reached");

    // Horn 1 fires, but the grace is spent first — `RETIRE_GRACE_FRAMES` polls that decrement and
    // retire nothing.
    let past_epoch = SUBMIT_EPOCH + FRAMES_IN_FLIGHT as u64;
    for spent in 0..RETIRE_GRACE_FRAMES {
        poll(&mut recorder, &mut scratch, past_epoch, RECORD_FRAME, &mut retired);
        assert!(
            retired.is_empty(),
            "the slot retired after {spent} grace frames of {RETIRE_GRACE_FRAMES}"
        );
    }

    // Grace spent. This poll retires.
    poll(&mut recorder, &mut scratch, past_epoch, RECORD_FRAME, &mut retired);
    assert_eq!(retired.len(), 1, "the epoch deadline did not retire the slot after its grace");
    let (cause, label, lost) = retired[0];
    assert_eq!(cause, RetireCause::EpochDeadline);
    assert_eq!(
        label,
        GpuLabel::Lost,
        "a pair the recorder bracketed and the GPU never wrote must be LOST — not MEASURED (which \
         would publish a number that does not exist) and not NOT_BRACKETED (which would blame the \
         recorder for the submission)"
    );
    assert_eq!(lost, 1, "the LOST pair was not counted");
    assert!(!recorder.in_flight(slot), "a retired slot is still in flight");

    // SAFETY: the only submission touching these pools was fence-waited in `reset_pool_and_wait`.
    unsafe {
        for pool in recorder.into_pools() {
            device.destroy_query_pool(pool);
        }
    }
}

#[test]
fn the_frame_horn_fires_when_submits_freeze_and_frames_do_not() {
    let Some(ctx) = boot_or_skip() else { return };
    let device: &VulkanContext = &ctx;
    if !device.device_caps().timestamps_usable() {
        eprintln!("SKIP gpu_zone_deadlines: GPU timestamps unusable");
        return;
    }

    let pools: [VulkanQueryPool; GPU_RING_DEPTH] = core::array::from_fn(|_| {
        device.create_query_pool(&QueryPoolDesc { count: QUERIES_PER_SLOT }).expect("pool")
    });
    let mut recorder = GpuZoneRecorder::new(pools);
    let mut scratch = Box::new(RetireScratch::new());

    const SUBMIT_EPOCH: u64 = 42;
    const RECORD_FRAME: u64 = 7;
    let slot = recorder.open_frame(0, SUBMIT_EPOCH, RECORD_FRAME).expect("a fresh slot");
    reset_pool_and_wait(device, &recorder, slot);
    bracket_without_submitting(device, &recorder, slot);

    let mut retired = Vec::new();
    // The epoch is FROZEN at the value the slot recorded — horn 1 can never fire, which is exactly
    // the minimised-window case: the host loop keeps folding frames while `RenderEpoch` stands
    // still, so an epoch-only deadline would hold this slot forever and teardown is never reached
    // because the process is alive.
    let frozen_epoch = SUBMIT_EPOCH;

    // One frame short of the deadline: still nothing.
    recorder
        .retire(device, frozen_epoch, RECORD_FRAME + GPU_FRAME_DEADLINE, &mut scratch, |f, p| {
            retired.push((f.cause, p[0].label));
        })
        .expect("retire");
    assert!(
        retired.is_empty(),
        "the frame horn fired AT the deadline; it is `>` and one frame short must not retire"
    );

    // One past it.
    recorder
        .retire(device, frozen_epoch, RECORD_FRAME + GPU_FRAME_DEADLINE + 1, &mut scratch, |f, p| {
            retired.push((f.cause, p[0].label));
        })
        .expect("retire");
    assert_eq!(retired.len(), 1, "the frame horn never fired, so a frozen epoch holds slots forever");
    assert_eq!(
        retired[0].0,
        RetireCause::FrameDeadline,
        "the slot retired, but on the epoch horn — which cannot fire here, so the two horns are \
         not independent"
    );
    assert_eq!(retired[0].1, GpuLabel::Lost);

    // SAFETY: as the sibling test.
    unsafe {
        for pool in recorder.into_pools() {
            device.destroy_query_pool(pool);
        }
    }
}

#[test]
fn a_poll_whose_epoch_condition_is_false_never_decrements_a_spent_grace() {
    let Some(ctx) = boot_or_skip() else { return };
    let device: &VulkanContext = &ctx;
    if !device.device_caps().timestamps_usable() {
        eprintln!("SKIP gpu_zone_deadlines: GPU timestamps unusable");
        return;
    }

    let pools: [VulkanQueryPool; GPU_RING_DEPTH] = core::array::from_fn(|_| {
        device.create_query_pool(&QueryPoolDesc { count: QUERIES_PER_SLOT }).expect("pool")
    });
    let mut recorder = GpuZoneRecorder::new(pools);
    let mut scratch = Box::new(RetireScratch::new());

    const SUBMIT_EPOCH: u64 = 9;
    const RECORD_FRAME: u64 = 3;
    let slot = recorder.open_frame(0, SUBMIT_EPOCH, RECORD_FRAME).expect("a fresh slot");
    reset_pool_and_wait(device, &recorder, slot);
    bracket_without_submitting(device, &recorder, slot);

    // Spend the grace with the epoch condition TRUE, stopping one poll short of the retire.
    let past_epoch = SUBMIT_EPOCH + FRAMES_IN_FLIGHT as u64;
    let mut retired = 0usize;
    for _ in 0..RETIRE_GRACE_FRAMES {
        recorder
            .retire(device, past_epoch, RECORD_FRAME, &mut scratch, |_, _| retired += 1)
            .expect("retire");
    }
    assert_eq!(retired, 0, "the grace was not spent as expected, so this test's premise is gone");

    // Now poll many times with the epoch condition FALSE and the frame deadline unreached. This is
    // the input that used to execute `0u8 - 1`: grace is 0 and the epoch arm is not taken. In debug
    // that panicked; in release it wrapped to 255 and silently restarted the deadline. Neither may
    // happen, and neither may a retire.
    for _ in 0..8 {
        recorder
            .retire(device, SUBMIT_EPOCH, RECORD_FRAME, &mut scratch, |_, _| retired += 1)
            .expect("retire");
    }
    assert_eq!(retired, 0, "a poll below both deadlines retired the slot");
    assert!(recorder.in_flight(slot), "the slot was released with neither horn reached");

    // And the grace really is spent, not restarted: one poll with the epoch condition true retires
    // immediately. A wrapped grace of 255 would need 255 more polls, so this is the clause that
    // distinguishes "did not underflow" from "underflowed and nobody noticed".
    recorder
        .retire(device, past_epoch, RECORD_FRAME, &mut scratch, |_, _| retired += 1)
        .expect("retire");
    assert_eq!(
        retired, 1,
        "the slot did not retire on the first epoch-true poll after its grace was spent, so the \
         grace was silently restarted — which is what a `0u8 - 1` wrap to 255 looks like from here"
    );

    // SAFETY: as the sibling tests.
    unsafe {
        for pool in recorder.into_pools() {
            device.destroy_query_pool(pool);
        }
    }
}
