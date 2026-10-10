//! KE4 (Aether v2 rung R1) — `Option<Res<R>>` / `Option<ResMut<R>>`.
//!
//! # What the row asks for
//!
//! `docs/aether-v2/KERNEL-BACKLOG.md` KE4: the null branch returns `None`
//! instead of the `#[cold]` missing-resource panic; **access declaration
//! unchanged**.
//!
//! # The access clause is a correctness requirement, not a note
//!
//! The obvious wrong implementation declares NO access, reasoning that a param
//! which may resolve to `None` reads nothing. That is exactly wrong: whether
//! the resource is present is a *runtime* fact, and the scheduler's conflict
//! analysis is a *static* one. A system holding `Option<ResMut<R>>` that
//! declared nothing would be scheduled concurrently with a `Res<R>` reader, and
//! the two would alias the same slot the moment the resource exists — a data
//! race that no test of the `None` path can see.
//!
//! `option_res_declares_the_read_access` and
//! `option_resmut_declares_the_write_access` pin it by pairing each Option param
//! with its conflicting sibling in ONE system and requiring the B0002
//! intra-system conflict panic. An implementation with an empty `init_access`
//! passes every other test in this file and fails only those two.
//!
//! # The control
//!
//! `bare_res_panics_when_the_resource_is_absent` states the premise KE4's "Why"
//! column rests on — that the engine today answers optionality with a panic. It
//! passes both before and after this rung; it is here so the `None`-returning
//! tests cannot be read as vacuous.

// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::{Res, ResMut, SystemParam};
use boyko_macros::Resource;

/// Present in every test that needs a resolvable resource.
#[derive(Resource)]
struct KE4Present(u32);

/// Never inserted — the absent leg.
#[derive(Resource)]
struct KE4Absent(#[allow(dead_code)] u32);

/// Second present resource, for the write-through test.
#[derive(Resource)]
struct KE4Counter(u32);

// ── The absent leg — `None`, not a panic ────────────────────────────────────

/// `Option<Res<R>>` over a resource that was never inserted resolves to `None`.
#[test]
fn option_res_is_none_when_the_resource_is_absent() {
    let mut world = EcsMaster::new();
    let seen = world.run_system(|r: Option<Res<KE4Absent>>| r.is_some());
    assert!(
        !seen,
        "Option<Res<R>> must resolve to None for an absent resource, not panic"
    );
}

/// `Option<ResMut<R>>` over an absent resource resolves to `None`.
#[test]
fn option_resmut_is_none_when_the_resource_is_absent() {
    let mut world = EcsMaster::new();
    let seen = world.run_system(|r: Option<ResMut<KE4Absent>>| r.is_some());
    assert!(
        !seen,
        "Option<ResMut<R>> must resolve to None for an absent resource"
    );
}

/// The control. The bare `Res<R>` still takes the `#[cold]`
/// missing-resource panic — the behaviour KE4 makes optional, not the
/// behaviour it replaces.
///
/// Passes before AND after this rung; it exists so the two `None` tests above
/// cannot be read as vacuous.
#[test]
#[should_panic]
fn bare_res_panics_when_the_resource_is_absent() {
    let mut world = EcsMaster::new();
    world.run_system(|_r: Res<KE4Absent>| {});
}

// ── The present leg — the value comes through unchanged ─────────────────────

/// `Option<Res<R>>` over a present resource resolves to `Some` and derefs to
/// the inserted value.
#[test]
fn option_res_is_some_and_derefs_when_present() {
    let mut world = EcsMaster::new();
    world.insert_resource(KE4Present(123));

    let seen = world.run_system(|r: Option<Res<KE4Present>>| r.map(|r| r.0));
    assert_eq!(
        seen,
        Some(123),
        "Option<Res<R>> must deref to the inserted value when present"
    );
}

/// `Option<ResMut<R>>` writes through, and the write persists in the slab.
#[test]
fn option_resmut_writes_through_when_present() {
    let mut world = EcsMaster::new();
    world.insert_resource(KE4Counter(1));

    world.run_system(|r: Option<ResMut<KE4Counter>>| {
        let mut r = r.expect("resource was inserted");
        r.0 = 555;
    });

    assert_eq!(
        world.resource::<KE4Counter>().0,
        555,
        "Option<ResMut<R>>'s write must persist"
    );
}

/// A resource inserted between two runs flips the same param from `None` to
/// `Some` — the branch is resolved per invocation, not cached at init.
#[test]
fn option_res_reresolves_per_invocation() {
    let mut world = EcsMaster::new();

    let before = world.run_system(|r: Option<Res<KE4Present>>| r.is_some());
    world.insert_resource(KE4Present(7));
    let after = world.run_system(|r: Option<Res<KE4Present>>| r.map(|r| r.0));

    assert!(!before, "absent before the insert");
    assert_eq!(after, Some(7), "present after the insert");
}

// ── Access declaration UNCHANGED ────────────────────────────────────────────

/// `Option<Res<R>>` declares the same resource READ as `Res<R>`: pairing it
/// with a `ResMut<R>` on the same id in one system trips B0002.
#[test]
#[should_panic(expected = "boyko-B0002")]
fn option_res_declares_the_read_access() {
    let mut world = EcsMaster::new();
    world.insert_resource(KE4Present(1));
    world.run_system(|_w: ResMut<KE4Present>, _r: Option<Res<KE4Present>>| {});
}

/// `Option<ResMut<R>>` declares the same resource WRITE as `ResMut<R>`:
/// pairing it with a `Res<R>` on the same id in one system trips B0002.
#[test]
#[should_panic(expected = "boyko-B0002")]
fn option_resmut_declares_the_write_access() {
    let mut world = EcsMaster::new();
    world.insert_resource(KE4Present(1));
    world.run_system(|_w: Option<ResMut<KE4Present>>, _r: Res<KE4Present>| {});
}

/// The declaration holds even when the resource is ABSENT — the conflict graph
/// is static, and an implementation that skipped the declaration on the null
/// branch would be schedulable against a concurrent writer.
#[test]
#[should_panic(expected = "boyko-B0002")]
fn option_res_declares_access_even_when_absent() {
    let mut world = EcsMaster::new();
    // KE4Absent is deliberately never inserted.
    world.run_system(|_w: ResMut<KE4Absent>, _r: Option<Res<KE4Absent>>| {});
}

// ── Zero when unused ────────────────────────────────────────────────────────

/// The Option wrapper adds no per-system state: its `SystemParam::State` is the
/// inner param's own state type, so a program that does not use the feature
/// carries nothing extra, and one that does carries no more than the bare param.
#[test]
fn option_params_carry_the_inner_params_state() {
    fn state_size<P: SystemParam>() -> usize {
        std::mem::size_of::<P::State>()
    }
    assert_eq!(
        state_size::<Option<Res<'static, KE4Present>>>(),
        state_size::<Res<'static, KE4Present>>(),
        "Option<Res<R>> must reuse ResState<R> — no extra per-system state"
    );
    assert_eq!(
        state_size::<Option<ResMut<'static, KE4Present>>>(),
        state_size::<ResMut<'static, KE4Present>>(),
        "Option<ResMut<R>> must reuse ResMutState<R>"
    );
}
