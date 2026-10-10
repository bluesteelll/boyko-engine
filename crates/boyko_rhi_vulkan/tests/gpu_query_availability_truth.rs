//! **G2c (profiling rung 4) — the availability truth control.**
//!
//! `read_query_pool_pairs_available` must answer *"is this pair's data actually there?"* from the
//! **driver's** availability words, not from whatever the caller's staging buffer happened to
//! hold. A pool with some pairs bracketed and some never written is the case the whole seam
//! exists for, and it is the case a wrong flag word gets silently wrong.
//!
//! # The three claims
//!
//! 1. **Before anything is written**, every pair reads unavailable.
//! 2. **After a fence**, a bracketed pair reads available and a never-written pair reads
//!    unavailable — in the SAME poll, so no clause is satisfied by a stub that answers uniformly.
//! 3. **An unavailable pair's begin/duration are ZERO**, never the driver's undefined bytes. The
//!    staging buffer is pre-filled with a sentinel before every poll, so an implementation that
//!    copied `scratch` straight through would hand the sentinel back and fail here.
//!
//! # The RED, and it is a build-independent one
//!
//! Change `GPU_ZONE_QUERY_FLAGS` to drop `VK_QUERY_RESULT_WITH_AVAILABILITY_BIT` (or set the wrong
//! value — `0x10` is `WITH_STATUS_BIT_KHR`, `0x20` is not defined at all). The driver then writes
//! only the value words, the stride still steps 16 bytes, and every availability word keeps the
//! sentinel ⇒ claim 1 fails immediately with "unavailable pairs read as available". Flip it the
//! other way — pre-zero the staging — and claim 2 fails instead. The two clauses fail in opposite
//! directions, which is why both are here.
//!
//! # What this gate does NOT claim, stated so it is not confused with G2a
//!
//! It cannot prove the reader never blocks. A blocking read **hangs**, and a hang is not a
//! showable red in this repository (`crates/boyko_app/tests/vb_bench_totality_gate.rs` — *"this
//! repository has no kill-after-timeout pattern to borrow"*). Claim 1 polls a pool no submission
//! has written, which is precisely the input that would hang a `WAIT_BIT` reader — so if this test
//! *returns at all*, the reader did not block. That escape is closed by G2a's `const _: () =
//! assert!(GPU_ZONE_QUERY_FLAGS & VK_QUERY_RESULT_WAIT_BIT == 0)`, a build failure, not by this
//! gate.
//!
//! Nor does it claim a measured duration is CORRECT — only that it is labelled. The 2×2 label and
//! the non-zero-duration clause are G2b's, at rung 5, with the recorder that produces them.

use boyko_rhi::{QueryPoolDesc, RhiCommandEncoder, RhiDevice, RhiQueue, TimestampStage};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Pairs in the pool. Two are bracketed and two are never written, so one poll carries both
/// answers — a stub that answers uniformly cannot pass whichever clause it is not answering.
const PAIRS: u32 = 4;
/// Pairs the recorder actually brackets, from index 0.
const BRACKETED: u32 = 2;

/// The sentinel every staging word carries into a poll.
///
/// Deliberately non-zero and deliberately not a plausible timestamp: if it survives into an
/// availability word the pair reads "available", and if it survives into a value word the caller
/// gets a duration near 2^63. Both are failures this test names rather than tolerates.
const SENTINEL: u64 = 0xDEAD_BEEF_DEAD_BEEF;

/// Boots an offscreen context (validation OFF — this gate measures a data path, not correctness),
/// or `None` with a SKIP log when no GPU / loader is present. The repository's established shape.
fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP gpu_query_availability_truth: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

#[test]
fn availability_is_read_from_the_driver_and_not_from_stale_staging() {
    let Some(ctx) = boot_or_skip() else { return };
    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    if !device.device_caps().timestamps_usable() {
        eprintln!(
            "SKIP gpu_query_availability_truth: GPU timestamps unusable (valid_bits={}, \
             period={} ns/tick)",
            device.device_caps().timestamp_valid_bits,
            device.device_caps().timestamp_period
        );
        return;
    }

    let queries = PAIRS * 2;
    let pool = device
        .create_query_pool(&QueryPoolDesc { count: queries })
        .expect("timestamp query pool");

    let mut scratch = [SENTINEL; (PAIRS * 2 * 2) as usize];
    let mut begin_ticks = [SENTINEL; PAIRS as usize];
    let mut dur_ticks = [SENTINEL; PAIRS as usize];
    let mut available = [0xFFu8; PAIRS as usize];

    // ── Phase 1: put the pool in a DEFINED reset state, write nothing, and poll. ──
    //
    // The reset is submitted and fence-waited rather than merely recorded: Vulkan leaves a
    // never-reset query's state undefined, and polling undefined state would make this clause's
    // answer the driver's business rather than the seam's. After the fence every query is
    // definitively "reset, unavailable" — which is also the exact input that would hang a
    // `WAIT_BIT` reader forever, so this poll returning at all is the non-blocking property
    // observed. (Observed, not asserted: see the module docs.)
    {
        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("encoder");
        encoder.begin().expect("begin");
        encoder.reset_query_pool(&pool, 0, queries);
        encoder.end().expect("end");
        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");
        // SAFETY: both were created on `device` and the submission was fence-waited above, so
        // neither is still GPU-referenced.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
    }

    device
        .read_query_pool_pairs_available(
            &pool,
            PAIRS,
            &mut scratch,
            &mut begin_ticks,
            &mut dur_ticks,
            &mut available,
        )
        .expect("a poll of a reset pool is Ok, not an error — VK_NOT_READY is a result");

    for i in 0..PAIRS as usize {
        assert_eq!(
            available[i], 0,
            "pair {i} read AVAILABLE from a pool nothing has written. The availability words \
             came from the staging sentinel, not from the driver — which is what a flag word \
             missing VK_QUERY_RESULT_WITH_AVAILABILITY_BIT produces."
        );
        assert_eq!(begin_ticks[i], 0, "an unavailable pair handed back a begin stamp");
        assert_eq!(dur_ticks[i], 0, "an unavailable pair handed back a duration");
    }

    // ── Phase 2: bracket SOME pairs, leave the rest reset-but-never-written, fence, poll once. ──
    {
        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("encoder");
        encoder.begin().expect("begin");
        encoder.reset_query_pool(&pool, 0, queries);
        for pair in 0..BRACKETED {
            encoder.write_timestamp(&pool, TimestampStage::TopOfPipe, pair * 2);
            encoder.write_timestamp(&pool, TimestampStage::BottomOfPipe, pair * 2 + 1);
        }
        encoder.end().expect("end");
        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");
        // SAFETY: as phase 1 — created on `device`, fence-waited, not GPU-referenced.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
    }

    // Re-poison every staging word. Without this, phase 1's driver-written zeros would already be
    // sitting in the availability slots and the "stale bytes" half of the claim would be testing
    // nothing.
    scratch = [SENTINEL; (PAIRS * 2 * 2) as usize];
    begin_ticks = [SENTINEL; PAIRS as usize];
    dur_ticks = [SENTINEL; PAIRS as usize];
    available = [0xFFu8; PAIRS as usize];

    device
        .read_query_pool_pairs_available(
            &pool,
            PAIRS,
            &mut scratch,
            &mut begin_ticks,
            &mut dur_ticks,
            &mut available,
        )
        .expect("a poll after the fence is Ok");

    for (pair, avail) in available.iter().enumerate().take(BRACKETED as usize) {
        assert_eq!(
            *avail, 1,
            "bracketed pair {pair} read UNAVAILABLE after its fence. Either the availability \
             words are not being read, or they are being read from the wrong offset."
        );
    }
    for pair in BRACKETED as usize..PAIRS as usize {
        assert_eq!(
            available[pair], 0,
            "never-written pair {pair} read AVAILABLE. This is the clause a uniform stub fails, \
             and it is the one that matters: a pair the recorder never bracketed must be \
             reportable, not waited on."
        );
        assert_eq!(begin_ticks[pair], 0, "a never-written pair handed back a begin stamp");
        assert_eq!(dur_ticks[pair], 0, "a never-written pair handed back a duration");
    }

    // Disclosure, not a gate: the durations themselves. Two back-to-back TOP/BOTTOM stamps on an
    // empty command buffer can legitimately measure zero ticks, so a non-zero assertion here would
    // be a flake rather than a claim — the non-zero-duration clause belongs to G2b, at rung 5,
    // where a real pass sits between the stamps.
    println!(
        "gpu_query_availability_truth: bracketed durations (ticks) = {:?}, period = {} ns/tick",
        &dur_ticks[..BRACKETED as usize],
        device.device_caps().timestamp_period
    );

    // ── The host-reset capability, reported rather than required (D18). ──
    let host_reset = device.host_query_reset_supported();
    println!("gpu_query_availability_truth: host_query_reset_supported = {host_reset}");
    if host_reset {
        device
            .reset_query_pool_host(&pool, 0, queries)
            .expect("a device that advertises host reset must accept it");
        scratch = [SENTINEL; (PAIRS * 2 * 2) as usize];
        available = [0xFFu8; PAIRS as usize];
        device
            .read_query_pool_pairs_available(
                &pool,
                PAIRS,
                &mut scratch,
                &mut begin_ticks,
                &mut dur_ticks,
                &mut available,
            )
            .expect("a poll after a host reset is Ok");
        for (i, avail) in available.iter().enumerate() {
            assert_eq!(
                *avail, 0,
                "pair {i} survived a host reset — `vkResetQueryPool` did not take effect, which \
                 would leave the recorder recycling slots that still hold the previous frame"
            );
        }
    } else {
        // Not a skip and not a failure: host reset is an optimisation with a fully specified
        // fallback, and this box's driver simply may not advertise it. What must hold either way
        // is that the verb REFUSES rather than calling an entry point the device did not enable.
        //
        // MEASURED 2026-08-09: this branch does NOT run on this box — the driver advertises
        // `hostQueryReset`, so the `if` arm is the one that executes. The Vulkan body's refusal
        // path is therefore **UNPROVEN on this hardware**, and it is named here rather than
        // claimed. The refusal itself is pinned where it IS reachable: `boyko_rhi`'s `MockDevice`
        // enables no feature, and `the_non_blocking_query_seam_default_bodies_are_unsupported`
        // asserts the `Unsupported` error there.
        device
            .reset_query_pool_host(&pool, 0, queries)
            .expect_err(
                "a device without the enabled feature must refuse host reset, not attempt it",
            );
    }

    // SAFETY: every submission touching this pool was fence-waited above, so nothing is still
    // reading or writing it.
    unsafe {
        device.destroy_query_pool(pool);
    }
}
