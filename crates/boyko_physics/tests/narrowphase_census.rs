//! **C1: the positive control of the box-box fallback census** (the `thinbox` lane,
//! `design_rev2.md` §6.2 (i)).
//!
//! The census (`narrowphase::box_box::fallback_census`) is what the pinned targets print to show
//! that the fix did not fire on their scenes: `phantom == 0 && hint_capped == 0`. A census that
//! cannot count would print the same zeros, so this binary shows it counting each event on a pose
//! built to reach it, one pose at a time:
//!
//! * the seed's pose 1 (G-L9b-1's `0x531ff99f772d936c`, a 3 mm box on a 33 m slab), cold: one
//!   fallback call, answered with the best face's own contact — `calls 1, phantom 1`;
//! * the pinned case-(a) witness a1 (a unit box yawed ~45° on the 50 m floor) under hint 8, an edge
//!   held within 1.05 of the best edge but past the face bound: `calls 1, hint_capped 1`;
//! * a pile knife-edge pair (two ~0.5 m boxes side by side, the pile stand-in's generator, pose 11
//!   of seed `0x9117_e5ee_d000_0001`), cold: an accepted edge `3.66e-5` m above the face —
//!   `calls 1, phantom 0`, `max_accepted_excess` in `(0, 5e-3)`.
//!
//! The counters are process-wide statics, so the binary holds this ONE test: nothing else runs
//! beside it to move them. Mutations M-C1, M-C2 and M-C3 (the `PHANTOM`, `HINT_CAPPED` and `CALLS`
//! increments deleted) each turn it red.
//!
//! Compiled only under the non-default `narrowphase-counts` feature. Without it this file is empty
//! and `running 0 tests` is the expected output — it is not a pass of anything. The leg must print
//! `running 1 test`:
//!
//! ```text
//! cargo test -p boyko-physics --features narrowphase-counts --test narrowphase_census
//! ```
#![cfg(feature = "narrowphase-counts")]

use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::{Quat, Vec3};
use boyko_physics::narrowphase::box_box::{box_box_contact, fallback_census};

fn v3(bits: [u32; 3]) -> Vec3 {
    Vec3::new(
        f32::from_bits(bits[0]),
        f32::from_bits(bits[1]),
        f32::from_bits(bits[2]),
    )
}

fn q4(bits: [u32; 4]) -> Quat {
    Quat::new(
        f32::from_bits(bits[0]),
        f32::from_bits(bits[1]),
        f32::from_bits(bits[2]),
        f32::from_bits(bits[3]),
    )
}

/// A pinned pose as `f32` bits: `(ac, aq, ah, bc, bq, bh)`.
type PoseBits = ([u32; 3], [u32; 4], [u32; 3], [u32; 3], [u32; 4], [u32; 3]);

/// The seed's pose 1 (`tests/box_box_fallback_depth.rs`): A the 33 m slab, B the 3 mm box.
const SEED_POSE_1: PoseBits = (
    [0x407c_4051, 0xc061_bdde, 0xbff1_0767],
    [0x3f1f_c023, 0x3f3f_b98c, 0xbe3e_e285, 0x3dfa_7326],
    [0x4205_81cd, 0x3e55_47d4, 0x41c0_387b],
    [0x3fe1_aff9, 0x418d_694a, 0x409f_8713],
    [0x3e81_4d3a, 0x3ecc_4676, 0xbf1b_de2e, 0x3f23_2f68],
    [0x3b87_b37a, 0x3b44_e355, 0x3c67_4a94],
);

/// T4's witness a1: A the 50×1×50 floor, B a 0.5 m box yawed ~45°.
const A1: PoseBits = (
    [0x0000_0000, 0xbf80_0000, 0x0000_0000],
    [0x0000_0000, 0x0000_0000, 0x0000_0000, 0x3f80_0000],
    [0x4248_0000, 0x3f80_0000, 0x4248_0000],
    [0x416d_170c, 0x3f00_04bd, 0x40fb_d5f8],
    [0xb766_f5de, 0x3ebf_3b65, 0x3850_03b2, 0x3f6d_792c],
    [0x3f00_0000, 0x3f00_0000, 0x3f00_0000],
);

/// A pile knife-edge pair, found by the lane's scratch probe (`impl/probe_pile_knife_edge.log`).
const PILE_KNIFE_EDGE: PoseBits = (
    [0x402a_4390, 0x40e4_54dc, 0xc00d_1340],
    [0xbf6a_4960, 0xbe40_bbf6, 0xbd83_c080, 0x3eb3_79d4],
    [0x3efe_d0e6, 0x3f07_785d, 0x3f09_2cfa],
    [0x4068_240d, 0x40e2_a92d, 0xbfd3_d778],
    [0xbf6a_50bf, 0xbe40_f24a, 0xbd83_9b47, 0x3eb3_466d],
    [0x3f03_85d7, 0x3efc_f8b4, 0x3f0a_9677],
);

/// Collides `pose` under `hint` and returns the census of that one call.
fn census_of(what: &str, pose: PoseBits, hint: Option<usize>) -> fallback_census::Snapshot {
    let (ac, aq, ah, bc, bq, bh) = pose;
    let before = fallback_census::take();
    assert_eq!(
        before.calls, 0,
        "{what}: the census must start from zero: {before:?}"
    );
    let c = box_box_contact(
        BodyIndex(0),
        BodyIndex(1),
        v3(ac),
        q4(aq),
        v3(ah),
        v3(bc),
        q4(bq),
        v3(bh),
        hint,
    );
    assert!(c.is_some(), "{what}: an overlapping pair has a manifold");
    let s = fallback_census::take();
    println!("C1 {what}: {s:?}");
    s
}

#[test]
fn the_fallback_census_counts_each_decision() {
    let _ = fallback_census::take();

    let s = census_of("seed pose 1, cold", SEED_POSE_1, None);
    assert!(
        s.calls == 1 && s.phantom == 1 && s.hint_capped == 0 && s.corner == 0,
        "the seed's phantom must count one call and one phantom answer: {s:?}"
    );

    let s = census_of("a1, hint 8", A1, Some(8));
    assert!(
        s.calls == 1 && s.phantom == 0 && s.hint_capped == 1 && s.corner == 0,
        "a1's held edge past the bound must count one call and one capped hint: {s:?}"
    );

    let s = census_of("pile knife-edge, cold", PILE_KNIFE_EDGE, None);
    assert!(
        s.calls == 1
            && s.phantom == 0
            && s.hint_capped == 0
            && s.corner == 0
            && s.max_accepted_excess > 0.0
            && s.max_accepted_excess < 5.0e-3,
        "the pile knife-edge must count one call accepting an edge above the face, within 5 mm: {s:?}"
    );
}
