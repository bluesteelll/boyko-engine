//! **AB-11 (owner ballot) — `With<F>` / `Without<F>` over a BITSET flag are two
//! different silent wrong answers.** RED BY DESIGN; not fixed here.
//!
//! # Why this file exists next to `ke1_or_dense_blindness.rs`
//!
//! Aether rung R0 / backlog KE1 fixed `Or`'s blindness to a DENSE arm. The
//! corpus records a second silent-wrong-answer on flag filters, and the two are
//! the same *class* — a presence test resolved through a mechanism the storage
//! kind does not participate in — but **NOT the same mechanism**, so R0 does not
//! touch it. The evidence, read at the source:
//!
//! | | KE1 (fixed by R0) | AB-11 (this file) |
//! |---|---|---|
//! | defect lives in | `impl_or_filter_tuple` (the combinator) | `With<C>` / `Without<C>` (the leaves) |
//! | needs an `Or`? | YES — a bare `Query<&P, Changed<Dense>>` was always correct | NO — `Query<&P, With<Flag>>` is already wrong |
//! | proximate cause | `HAS_DENSE`/`resolve_dense` not forwarded ⇒ NULL store pointer | no `STORAGE_IS_BITSET` branch at all ⇒ falls through to `mask.contains(id)` |
//! | fix shape | forward the plumbing (mechanical) | **undecided** — see below |
//!
//! `With::matches_component_set` and `Without::matches_component_set` branch on
//! `const { C::STORAGE_IS_DENSE }` and then fall through to `mask.contains(state.id)`
//! / `!mask.contains(state.id)`. A `StorageKind::Bitset` id is filtered out of
//! **every** archetype signature (`component_registry::is_signature_storage`), so
//! that mask bit is never set for any archetype in any world:
//!
//! * `With<Flag>` — `mask.contains` is always false ⇒ **matches nothing, ever**.
//! * `Without<Flag>` — `!mask.contains` is always true ⇒ **excludes nothing, ever**.
//!
//! Neither reports anything. `Added<C>` / `Changed<C>` already const-refuse a
//! bitset `C` (`assert_storage_supports_change_detection`); `With` / `Without`
//! carry no such guard, and the correct spelling — `Enabled<T>` / `Disabled<T>` —
//! is a *different type* the author gets no nudge toward.
//!
//! # Why the tests below are `#[ignore]`d rather than fixed
//!
//! The disposition is owner ballot **AB-11**, and the two live options produce
//! *different* files here:
//!
//! * **compile-refusal** (mirroring `Added`/`Changed`'s const-assert) ⇒ these
//!   tests stop compiling and must be DELETED, replaced by `trybuild`
//!   `compile_fail` fixtures in `enable_filter_compile_fail`;
//! * **make it work** (route `With`/`Without` through the enable column) ⇒ these
//!   tests are un-`#[ignore]`d unchanged and go green.
//!
//! Landing either one here would pre-empt a decision the owner has not made. What
//! is NOT optional is that the wrong answer be *demonstrated* rather than
//! asserted from a code reading — that is what these tests are for, and they were
//! run and observed red before being marked.
//!
//! Run them with:
//! `cargo test -p boyko-ecs --test ab11_flag_filter_polarity -- --ignored`

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::filter::Or;
use boyko_ecs::ecs::core::iters::query::{With, Without};
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::Component;

/// A derived BITSET enable tag — `StorageKind::Bitset`, signature-excluded, no
/// per-archetype `ComponentPool`.
#[derive(Component)]
#[component(storage = "bitset")]
struct Flag750;

/// Table payload; its `v` is the identity the hand oracle is written in.
#[derive(Clone, Copy)]
#[derive(Component)]
#[repr(C)]
struct FP751 {
    v: u32,
}

fn spawn(ecs: &mut EcsMaster, arch: ArchetypeId, v: u32) -> Entity {
    let bytes = v.to_ne_bytes();
    ecs.create_entity(arch, &[(FP751::component_id(), &bytes)])
        .expect("create_entity on the direct path")
}

/// Four rows in one archetype; the flag bit is set on `v == 1` and `v == 4`.
fn world() -> (EcsMaster, [Entity; 4]) {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[FP751::component_id()]);
    let rows = [
        spawn(&mut ecs, arch, 1),
        spawn(&mut ecs, arch, 2),
        spawn(&mut ecs, arch, 4),
        spawn(&mut ecs, arch, 8),
    ];
    ecs.enable::<Flag750>(rows[0]);
    ecs.enable::<Flag750>(rows[2]);
    (ecs, rows)
}

/// Control: the flag bits ARE set, and `Enabled<T>` — the correct spelling —
/// reads them. This test is NOT ignored: it is the floor that stops the two
/// below from passing vacuously over a world where nothing was ever flagged.
#[test]
fn enabled_filter_reads_the_flag_bits() {
    use boyko_ecs::ecs::core::iters::query::Enabled;
    let (mut ecs, _rows) = world();
    let mut seen: Vec<u32> = ecs
        .query::<&FP751, Enabled<Flag750>>()
        .iter()
        .map(|p| p.v)
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![1u32, 4],
        "the fixture's flag bits must be readable through Enabled<T>; if this \
         fails the two AB-11 tests below are measuring nothing"
    );
}

/// `With<Flag>` must match the flagged rows. It matches NOTHING today.
///
/// Hand oracle: `[1, 4]` — the two rows `world()` enabled. Observed today: `[]`.
#[test]
#[ignore = "deferred: owner ballot AB-11 owns the disposition of With<F>/Without<F> \
            over a bitset flag (refuse at compile time vs route through the enable \
            column). RED BY DESIGN until AB-11 is decided; see this file's header"]
fn with_over_a_bitset_flag_matches_the_flagged_rows() {
    let (mut ecs, _rows) = world();
    let mut seen: Vec<u32> = ecs
        .query::<&FP751, With<Flag750>>()
        .iter()
        .map(|p| p.v)
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![1u32, 4],
        "With<Flag> must match the flagged rows. An EMPTY result is the AB-11 \
         defect: a bitset id is signature-excluded, so With's \
         `mask.contains(state.id)` is false for every archetype in every world"
    );
}

/// `Without<Flag>` must exclude the flagged rows. It excludes NOTHING today —
/// the opposite polarity of the same silent failure, which is why one test
/// cannot stand for both.
///
/// Hand oracle: `[2, 8]` — the two unflagged rows. Observed today: `[1, 2, 4, 8]`.
#[test]
#[ignore = "deferred: owner ballot AB-11 owns the disposition of With<F>/Without<F> \
            over a bitset flag (refuse at compile time vs route through the enable \
            column). RED BY DESIGN until AB-11 is decided; see this file's header"]
fn without_over_a_bitset_flag_excludes_the_flagged_rows() {
    let (mut ecs, _rows) = world();
    let mut seen: Vec<u32> = ecs
        .query::<&FP751, Without<Flag750>>()
        .iter()
        .map(|p| p.v)
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![2u32, 8],
        "Without<Flag> must exclude the flagged rows. Returning ALL FOUR is the \
         AB-11 defect in its excludes-nothing polarity"
    );
}

/// The mechanism is INDEPENDENT of `Or` — this is the evidence that AB-11 is not
/// a leftover of KE1. `Or<(With<Flag>, …)>` is wrong for exactly the reason the
/// bare `With<Flag>` is: the arm's own archetype predicate reads a mask bit that
/// is never set. R0's dense forwarding cannot reach it, because a bitset leaf
/// declares no dense plumbing to forward (`HAS_DENSE` keys off
/// `STORAGE_IS_DENSE`, which is false for a bitset tag).
///
/// Hand oracle: `[1, 4]` — the flagged rows, reached through the flag arm; the
/// sibling `With<FP751>` arm is deliberately a component NO row has, so the flag
/// arm is the only thing that can match. Observed today: `[]`.
#[test]
#[ignore = "deferred: owner ballot AB-11 owns the disposition of With<F>/Without<F> \
            over a bitset flag (refuse at compile time vs route through the enable \
            column). RED BY DESIGN until AB-11 is decided; see this file's header"]
#[allow(clippy::type_complexity)] // query DSL type under test
fn or_does_not_introduce_and_cannot_fix_the_flag_polarity() {
    /// A table component NO entity in `world()` carries, so its arm never matches.
    #[derive(Clone, Copy)]
    #[derive(Component)]
    #[repr(C)]
    struct Absent752 {
        _v: u32,
    }

    let (mut ecs, _rows) = world();
    let mut seen: Vec<u32> = ecs
        .query::<&FP751, Or<(With<Flag750>, With<Absent752>)>>()
        .iter()
        .map(|p| p.v)
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![1u32, 4],
        "the flag arm is the only one that can match. An empty result shows the \
         AB-11 defect survives inside Or — it is a leaf defect, not a combinator \
         defect, and R0's dense forwarding does not reach it"
    );
}
