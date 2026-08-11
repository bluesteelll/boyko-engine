//! **G14(b)'s subject**: the same shape as `deep_zone`, one tier lower, so the shipping build has
//! something it must NOT have folded.
//!
//! `Always` is the floor of `ZoneTier`, so every profile admits this site — including `off`, whose
//! tier column has no position below `Always` to select. A shipping build in which this binary
//! records nothing has a ceiling that folded everything, and a profiler that folds everything is
//! indistinguishable from one that was never compiled in.
//!
//! # Why not `__frame`, which the corpus names
//!
//! `__frame`'s bracket lives in `boyko_ecs`'s `fold_frame` and needs an `EcsMaster`, a world and a
//! schedule, so asserting on it means building and running the ECS under a second profile. The
//! property under test is not "`__frame` in particular runs" — it is "the shipping ceiling did not
//! delete the `Always` tier", and that is the same property one tier down from `deep_zone` with
//! nothing else changed. Keeping the two fixtures identical apart from the tier is what makes the
//! pair a controlled comparison rather than two separate observations.
//!
//! This is also why the two sites live in two BINARIES rather than one: `deep_zone`'s census
//! argument is that its image holds exactly one zone site, and an `Always` site in the same image
//! would put a legitimately-surviving emission reference in the shipping artifact — turning G14(a)
//! green-side-up into a false RED.

boyko_diag::profiling_partition!(User);

boyko_diag::declare_zone!(
    FIXTURE_ALWAYS,
    name = "fixture_always",
    scope = boyko_diag::profiling_abi::USER_SCOPE_BASE,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);

fn main() {
    boyko_diag::profiling_abi::arm_scope(boyko_diag::profiling_abi::USER_SCOPE_BASE);
    for _ in 0..10 {
        let _z = boyko_diag::zone!(FIXTURE_ALWAYS);
    }
    println!(
        "profile={} tier={} calls={}",
        boyko_diag::profile::PROFILE_NAME,
        boyko_diag::profile::PROFILING_TIER,
        FIXTURE_ALWAYS.calls(),
    );
}
