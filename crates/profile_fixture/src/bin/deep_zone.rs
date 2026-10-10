//! **G14(a)'s subject**: one binary, one `Deep` zone site, and nothing else that could emit.
//!
//! Built from this one source by two CI legs, `BOYKO_PROFILE=dev` and `BOYKO_PROFILE=shipping`. The
//! `dev` artifact must reference the profiler's emission path; the `shipping` artifact — where
//! `GLOBAL_TIER` is `Always` and the site's tier is `Deep` — must not, because
//! `const { $h::TIER as u8 <= GLOBAL_TIER as u8 }` folds to `false` and an `&&` chain with a
//! `const false` deletes the arm's codegen entirely.
//!
//! # Why a whole binary can answer a per-site question here
//!
//! It normally cannot, and rev 3 of the corpus asked it to, which is why that version of G14 had no
//! constructible RED: a census reports "symbol S is referenced in this object", per object, and
//! cannot attribute the reference to a site. The fix is not a better census — it is a binary with
//! **one** site. That property is carried by this file's shape and by `Cargo.toml`'s dependency
//! table, not by anything the census does.
//!
//! # Why the `dev` leg is not optional
//!
//! Zero references in *both* legs is `NOT RESOLVED (census inert)`, never green. A census that
//! names a symbol which was inlined away, renamed, or never existed reports absence perfectly and
//! measures nothing — the exact vacuity this campaign keeps finding. The `dev` leg is the positive
//! control that makes the `shipping` zero mean something.
//!
//! The **runtime** gate is armed below on purpose. Without it `ARM_MASK` is zero for the process,
//! and while that does not affect the compile-time fold under test, it would leave a reader unable
//! to tell a folded site from a disarmed one by running the binary.

boyko_diag::profiling_partition!(User);

boyko_diag::declare_zone!(
    FIXTURE_DEEP,
    name = "fixture_deep",
    scope = boyko_diag::profiling_abi::USER_SCOPE_BASE,
    tier = boyko_diag::profiling_abi::ZoneTier::Deep,
);

fn main() {
    boyko_diag::profiling_abi::arm_scope(boyko_diag::profiling_abi::USER_SCOPE_BASE);
    {
        let _z = boyko_diag::zone!(FIXTURE_DEEP);
    }
    // One line, three fields, so the harness reads the profile out of the artifact it just built
    // rather than out of the variable it thinks it set. `calls` is the behavioural half: under
    // `dev` the guard ran and recorded, under `shipping` there is no guard to run.
    println!(
        "profile={} tier={} calls={}",
        boyko_diag::profile::PROFILE_NAME,
        boyko_diag::profile::PROFILING_TIER,
        FIXTURE_DEEP.calls(),
    );
}
