//! **G7, two-sided** — `boyko-E2101` fires exactly when validation was requested and this process
//! is not getting it.
//!
//! # The corpus specifies the opposite polarity, and the tree refutes its premise
//!
//! `logging/ladder`'s L7-gate row says "`E2101` fires on a validation-**on** run and is absent on a
//! validation-**off** run", resting on disposition **F2**: "a chained validation-features node is
//! unbuildable here". Measured before this file was written: **it is built and it works.**
//! `create_instance` enables `VK_EXT_validation_features` when present and chains
//! `VkValidationFeaturesEXT` as the head of the instance `p_next`; with the escape hatch unset,
//! `cargo test -p boyko_rhi_vulkan --test compute` boots and passes 4 of 4.
//!
//! So on a correct box a validation-on run must be **silent**, and the specified gate would have
//! been red against a working engine. `E2101` is re-cut to *validation was requested and this
//! process is not getting it* — the hatch took it, or the extension is absent. The full argument is
//! in the corpus's L7 block.
//!
//! # Why this leg needs no GPU
//!
//! The escape-hatch arm fires at the top of `VulkanContext::boot`, **before** the loader is
//! touched. A host with no Vulkan at all still reaches it, so this gate is deterministic on every
//! machine rather than skipping on the ones that most need it.
//!
//! # What it cannot claim
//!
//! That a live layer **catches** anything. This crate's own
//! `tests/compute.rs::negative_chained_barrier_hazard` documents that synchronization validation is
//! enabled here and still does not flag a compute→compute RAW hazard. Presence and sensitivity are
//! two questions; only the first is observable from inside the engine, and this gate is about the
//! first. `M25` stands.

use boyko_log::level::Level;
use boyko_log::target::{LogTarget, set_target_level, target_stats};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// `delivered + sync_routed`: a test thread may or may not hold a diagnostics lane, and an `Error`
/// from an un-laned thread takes the synchronous channel instead of the ring. Asserting only
/// `delivered` would be green or red depending on which harness thread ran the test.
fn observed() -> u64 {
    let s = target_stats(<boyko_log::RhiVulkan as LogTarget>::ID);
    s.0 + s.3
}

fn drain() {
    for _ in 0..64 {
        if boyko_log::lifecycle::drain_once().is_some() {
            return;
        }
        std::thread::yield_now();
    }
}

/// One test, because both legs mutate one process-global environment variable and one
/// process-global `Once` latch, and two `#[test]`s would race on both.
///
/// The order is load-bearing: the **negative** leg runs first, while the latch is unfired. Run the
/// other way round, the latch would already be spent and the negative leg would pass for the wrong
/// reason — a green that means "silenced", not "silent".
#[test]
fn e2101_fires_only_when_validation_was_requested_and_withheld() {
    set_target_level(<boyko_log::RhiVulkan as LogTarget>::ID, Level::Trace);

    // ── NEGATIVE: nothing was requested, so there is nothing to withhold ────────────────────────
    //
    // `enable_validation: false` short-circuits `validation_requested`'s `&&`, so the environment
    // is not even read and the effective flag is false whatever it holds. A boot that fails for
    // want of a GPU is fine here: the claim is about what was NOT emitted.
    let before = observed();
    let _ = VulkanContext::boot(InstanceConfig { enable_validation: false, ..InstanceConfig::default() });
    drain();
    assert_eq!(
        observed(),
        before,
        "boyko-E2101 fired for a caller that never asked for validation"
    );

    // ── POSITIVE: requested, and the escape hatch took it ───────────────────────────────────────
    //
    // SAFETY: this binary carries exactly one test and the harness has spawned nothing else, so no
    //   other thread can observe the environment mid-write.
    unsafe { std::env::set_var("BOYKO_DISABLE_VALIDATION", "1") };
    let before = observed();
    let _ = VulkanContext::boot(InstanceConfig { enable_validation: true, ..InstanceConfig::default() });
    drain();
    assert!(
        observed() > before,
        "boyko-E2101 did not fire: validation was requested, BOYKO_DISABLE_VALIDATION withheld \
         it, and the engine said nothing -- which is the state every golden leg has run in"
    );

    // `RatePolicy::Once` per site: the answer is a property of the PROCESS, not of the boot, so a
    // host booting several contexts is told once and not once per context.
    let after_first = observed();
    let _ = VulkanContext::boot(InstanceConfig { enable_validation: true, ..InstanceConfig::default() });
    drain();
    assert_eq!(observed(), after_first, "the Once latch let a second E2101 through");
}
