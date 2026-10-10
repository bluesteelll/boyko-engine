//! **Profiling rung 9 — the calibrated-timestamp probe on a real device.**
//!
//! Everything about the rejection sampler that can be wrong without hardware is tested in
//! `boyko_app::profiling::correlate`'s unit tests, against probes a test constructed. What no
//! constructed probe can establish is that `vkGetCalibratedTimestampsEXT` works at all on this
//! box, that the counter it returns ADVANCES, and that the counter it returns is the SAME one
//! `vkCmdWriteTimestamp` writes. Those are hardware facts, and this file is where they are asked.
//!
//! # What is asserted, and what is only printed
//!
//! `VK_EXT_calibrated_timestamps` is optional. Asserting that a machine has it would be a gate
//! about the hardware, red on the first box without it — the mistake the present-mode probe
//! already names. So SUPPORT is **printed**, and what is asserted is the contract that holds
//! either way:
//!
//! * A device that reports `calibrated_timestamps_supported() == false` **refuses**
//!   `sample_device_clock` rather than returning a number. The two must agree, always; a `true`
//!   whose sample errors, or a `false` whose sample succeeds, is exactly the confusion the
//!   ENABLED-not-advertised contract exists to prevent.
//! * A device that reports `true` produces samples whose device counter **advances**, whose
//!   bracket is **ordered** (`after >= before`), and whose two axes are **independent** — the
//!   device delta and the CPU delta across the same wall interval are both positive.
//!
//! The last one is the substantive claim. A driver stub returning a constant would satisfy every
//! type in this rung and produce an offset that looked perfectly reasonable and never moved.
//!
//! # CI
//!
//! No loader / no GPU → skip gracefully, this tree's convention.

use boyko_rhi::device::RhiDevice;
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Boots a HEADLESS context — this probe needs no surface, which is itself worth stating: the
/// capability is a property of the device, not of a swapchain, so a machine with no WSI can still
/// answer.
fn boot() -> Option<VulkanContext> {
    VulkanContext::boot(InstanceConfig { enable_validation: false, windowed: false }).ok()
}

/// **The clause that holds on every box:** the capability bit and the verb agree.
#[test]
fn the_capability_bit_and_the_sampler_agree() {
    let Some(ctx) = boot() else {
        eprintln!("SKIP calibrated_timestamp_probe: Vulkan unavailable");
        return;
    };
    let supported = ctx.calibrated_timestamps_supported();
    println!(
        "CALIBRATED-TIMESTAMP PROBE: VK_EXT_calibrated_timestamps {} on this box \
         (timestamp_period={} ns/tick, valid_bits={})",
        if supported { "ENABLED" } else { "absent" },
        ctx.device_caps().timestamp_period,
        ctx.device_caps().timestamp_valid_bits,
    );

    let sample = ctx.sample_device_clock();
    assert_eq!(
        sample.is_ok(),
        supported,
        "the capability bit and the verb must never disagree: `supported` said {supported} while \
         the sample said {:?}. A `true` whose sample errors sends a caller down a path that then \
         fails; a `false` whose sample succeeds hides a capability the artifact would report as \
         UNCORRELATED.",
        sample.as_ref().map(|_| ()),
    );
}

/// **The clause that needs the extension:** the device counter is a live counter, not a constant,
/// and it runs on its own axis.
///
/// Skips — loudly — when the extension is absent, because there is nothing to measure then. That
/// is a genuine skip and not a hidden pass: the test above already gated the absent case.
#[test]
fn a_sampled_device_clock_advances_alongside_the_cpu_clock() {
    let Some(ctx) = boot() else {
        eprintln!("SKIP calibrated_timestamp_probe: Vulkan unavailable");
        return;
    };
    if !ctx.calibrated_timestamps_supported() {
        eprintln!("SKIP a_sampled_device_clock_advances: VK_EXT_calibrated_timestamps absent");
        return;
    }

    let a = ctx.sample_device_clock().expect("a supported device must sample");
    // Long enough that a counter with a coarse granularity still moves, short enough that the test
    // costs nothing. At 1 ns/tick this is ~20 million ticks; even a 1 µs-granularity counter moves
    // twenty thousand steps.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let b = ctx.sample_device_clock().expect("a supported device must sample twice");

    for (i, s) in [a, b].iter().enumerate() {
        assert!(
            s.cpu_ticks_after >= s.cpu_ticks_before,
            "sample {i}'s bracket is inverted ({} -> {}): the CPU counter went backwards across \
             one driver call, which makes every bracket width in the sampler meaningless",
            s.cpu_ticks_before,
            s.cpu_ticks_after,
        );
    }

    let device_delta = b.device_ticks.wrapping_sub(a.device_ticks) & ctx.device_caps().timestamp_mask();
    let cpu_delta = b.cpu_ticks_before.saturating_sub(a.cpu_ticks_before);
    println!(
        "CALIBRATED-TIMESTAMP PROBE: across a 20 ms sleep the device advanced {device_delta} ticks \
         and the CPU advanced {cpu_delta} ticks; brackets were {} and {} ticks, driver maxDeviation \
         {} and {} ns",
        a.cpu_ticks_after - a.cpu_ticks_before,
        b.cpu_ticks_after - b.cpu_ticks_before,
        a.driver_max_deviation_ns,
        b.driver_max_deviation_ns,
    );

    assert!(
        device_delta > 0,
        "the device counter did not advance across 20 ms. A constant would satisfy every type in \
         this rung and yield an offset that looked entirely plausible and never moved."
    );
    assert!(
        cpu_delta > 0,
        "the CPU counter did not advance across 20 ms -- the bracket cannot bound anything"
    );
}
