//! **KE13 — `QueryView::get` / `get_mut` silently IGNORED a dense
//! `With<C>` / `Without<C>` filter.** This file was RED BY DESIGN; it is now
//! the GATE for the repair.
//!
//! # What was wrong
//!
//! `QueryView::get` (`crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs`)
//! applied, in order: the matched-archetype bitset, the dynamic tag terms, the
//! typed enable term (gated on `const { F::CONTAINS_ENABLE_TERM }`), and the
//! dynamic enable terms. It **never called `F::filter_fetch`**, and it never
//! called `F::resolve_dense` — it resolved only `D`'s
//! (`if const { <D as QueryData>::HAS_DENSE }`).
//!
//! For a TABLE `With<C>` that is correct: the term is archetypal, so the matched
//! bitset already decided it. For a **DENSE** `With<C>` it is not. A dense id is
//! signature-excluded, so:
//!
//! * `With::<Dense>::IS_ARCHETYPAL` is **`false`**,
//! * `With::<Dense>::matches_component_set` returns **`true` for every archetype**
//!   (there is no mask bit to test), and
//! * the exact membership gate is the per-row `filter_fetch` — the one call
//!   `get` did not make.
//!
//! So `get` returned `Some(row)` for an entity that is **not a member of the
//! dense store**, and `Without<Dense>` was ignored in the same breath. `iter()`
//! on the same `QueryView` answered correctly, so `get` and `iter`
//! **disagreed** — which is what
//! [`get_agrees_with_iter_on_the_same_view`] asserts directly.
//!
//! # The doc comment on `get` asserted the opposite, in writing
//!
//! > Non-archetypal **change-detection** filters (`Changed<C>` / `Added<C>`) can
//! > never reach a `QueryView` … So `get` only ever sees archetypal and enable
//! > terms — **there is no silent-ignore path.**
//!
//! That reasoning was true when written and enumerates the term kinds
//! exhaustively — but it rested on "`With`/`Without` are archetypal", which the
//! Dense plan falsified: a dense `With` is the third kind, neither archetypal
//! nor enable. The paragraph was struck in the same commit as the repair,
//! because a reader who trusts it stops looking.
//!
//! # Relation to KE1 / rung R0
//!
//! Same shape as the defect R0 fixed — *a caller that does not make the dense
//! resolve call* — one path over. R0 fixed `Or`'s missing `HAS_DENSE` /
//! `resolve_dense`; this was `QueryView::get`'s missing `F`-side resolve **plus
//! a missing `filter_fetch`**, which is strictly worse: R0's bug made an arm
//! never true, this one applied no gate at all.
//!
//! There is a **recorded precedent in-tree** that this path was known to be
//! under-served: `iters/query/state.rs`'s `dense_get_iter_agree` test carries a
//! "PRE-EXISTING BUG (out of D0–D6 scope, flagged for the reviewer)" note that
//! `get` never calls `resolve_dense`, describing the **`D`-side** null-deref and
//! calling the repair "a one-line `resolve_dense` mirror". The `D` side had
//! since been fixed; **the `F` side had not**, and it was not a one-liner —
//! `get` had no `filter_fetch` call to feed.
//!
//! # How it was repaired
//!
//! Not by mirroring `Query::point_filter_passes` into `query_view.rs` — a
//! mirrored predicate is how the two drifted apart in the first place. The body
//! moved into `iters/query/point_filter.rs` and BOTH types call it, so a future
//! divergence is not expressible.
//!
//! # Running this file
//!
//! `cargo test -p boyko-ecs --test ke13_query_view_get_ignores_dense_filter`
//!
//! ⚠ It must read **`running 8 tests`**. There are no `#[ignore]`s left, so
//! passing `-- --ignored` now prints `running 0 tests` and exits 0 — a vacuous
//! pass, not a pass. The plain invocation above is the only one that measures
//! anything.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Or, With, Without};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::prelude::Entity;
use boyko_macros::{Bundle, Component};

/// DENSE membership subject.
#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct KDense770 {
    x: u32,
}

/// TABLE payload the query reads.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct KPay771 {
    p: u32,
}

/// TABLE membership subject — the control that proves `get` DOES apply an
/// archetypal filter.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct KTable772 {
    x: u32,
}

#[derive(Bundle)]
struct WithDense {
    p: KPay771,
    d: KDense770,
}

#[derive(Bundle)]
struct PayOnly {
    p: KPay771,
}

/// `member` carries the dense component, `outsider` does not. Both carry the
/// table payload, and both land in the SAME archetype `{KPay771}` — the dense id
/// is signature-excluded, so membership is invisible to the archetype mask. That
/// is precisely the case an archetype-level decision cannot make.
fn world() -> (EcsMaster, Entity, Entity) {
    let mut ecs = EcsMaster::new();
    let member = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(WithDense {
            p: KPay771 { p: 1 },
            d: KDense770 { x: 9 },
        })
        .id()
    });
    let outsider = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PayOnly { p: KPay771 { p: 2 } }).id()
    });
    (ecs, member, outsider)
}

/// Floor: `get` DOES apply a TABLE `With<C>`, and `iter` over the dense filter
/// answers correctly. If either half of this reds, every test below is
/// measuring the wrong thing — it was the non-ignored control while the rest of
/// the file was red by design, and it keeps that job now that they are green.
#[test]
fn controls_table_filter_applies_and_dense_iter_is_correct() {
    let (mut ecs, member, outsider) = world();

    // Table control: neither entity has KTable772, so `With<KTable772>` must
    // reject both through the matched-archetype bitset.
    assert!(
        ecs.query::<&KPay771, With<KTable772>>().get(member).is_none(),
        "control: get must apply a TABLE With<C> (archetype lacks it)"
    );

    // Dense control on the ITER path — this is what `get` must agree with.
    let mut seen: Vec<u32> = ecs
        .query::<&KPay771, With<KDense770>>()
        .iter()
        .map(|p| p.p)
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![1u32],
        "control: iter() applies the dense With<C> correctly — only the member"
    );
    let _ = outsider;
}

/// `get` on a NON-member must return `None` under `With<Dense>`.
///
/// Hand oracle: `None` for the outsider, `Some` for the member — matching what
/// `iter()` reports in the control above. Observed BEFORE the repair: `Some`
/// for BOTH, because the dense arm's per-row gate was never run.
#[test]
fn get_applies_a_dense_with_filter() {
    let (mut ecs, member, outsider) = world();

    assert_eq!(
        ecs.query::<&KPay771, With<KDense770>>().get(member),
        Some(&KPay771 { p: 1 }),
        "the dense MEMBER must be returned"
    );
    assert!(
        ecs.query::<&KPay771, With<KDense770>>().get(outsider).is_none(),
        "a NON-member must be rejected by With<Dense>. `Some` is the KE13 defect: \
         get never runs the dense arm's per-row filter_fetch, so it admits every \
         row of every archetype — and disagrees with iter() on the same view"
    );
}

/// The opposite polarity: `get` under `Without<Dense>` must reject the MEMBER.
///
/// Hand oracle: `None` for the member, `Some` for the outsider. Observed BEFORE
/// the repair: `Some` for BOTH — the excludes-nothing face of the same defect.
#[test]
fn get_applies_a_dense_without_filter() {
    let (mut ecs, member, outsider) = world();

    assert!(
        ecs.query::<&KPay771, Without<KDense770>>().get(member).is_none(),
        "the dense MEMBER must be EXCLUDED by Without<Dense>. `Some` is the KE13 \
         defect in its excludes-nothing polarity"
    );
    assert_eq!(
        ecs.query::<&KPay771, Without<KDense770>>().get(outsider),
        Some(&KPay771 { p: 2 }),
        "the non-member must still be returned"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// `get_mut` — the same defect, the other entry point
// ════════════════════════════════════════════════════════════════════════════

/// `get_mut` shares `get`'s shape line for line and has the SAME missing
/// `filter_fetch`. Untested until now, so a repair applied to `get` alone would
/// have read green.
#[test]
fn get_mut_applies_a_dense_with_filter() {
    let (mut ecs, member, outsider) = world();

    let mut view = ecs.query::<&mut KPay771, With<KDense770>>();
    assert!(
        view.get_mut(member).is_some(),
        "the dense MEMBER must be returned by get_mut"
    );
    assert!(
        view.get_mut(outsider).is_none(),
        "a NON-member must be rejected by With<Dense> on the get_mut path too"
    );
}

/// The opposite polarity through `get_mut`.
#[test]
fn get_mut_applies_a_dense_without_filter() {
    let (mut ecs, member, outsider) = world();

    let mut view = ecs.query::<&mut KPay771, Without<KDense770>>();
    assert!(
        view.get_mut(member).is_none(),
        "the dense MEMBER must be EXCLUDED by Without<Dense> on the get_mut path"
    );
    assert!(
        view.get_mut(outsider).is_some(),
        "the non-member must still be returned"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The property the register states, asserted directly
// ════════════════════════════════════════════════════════════════════════════

/// The register's own words are *"`get` … disagrees with `iter()` on the same
/// view"*. The two polarity tests assert a hand oracle, which is a PROXY for
/// that sentence; this one asserts the sentence — over ONE `QueryView` value,
/// so a future divergence that happens to satisfy both hand oracles separately
/// still reds here.
#[test]
fn get_agrees_with_iter_on_the_same_view() {
    let (mut ecs, member, outsider) = world();

    let view = ecs.query::<&KPay771, With<KDense770>>();
    let mut from_iter: Vec<u32> = view.iter().map(|p| p.p).collect();
    from_iter.sort_unstable();
    let mut from_get: Vec<u32> = [member, outsider]
        .into_iter()
        .filter_map(|e| view.get(e).map(|p| p.p))
        .collect();
    from_get.sort_unstable();

    assert_eq!(
        from_get, from_iter,
        "get and iter must answer the same set on the SAME view — this is the \
         property KE13 states, and the one a caller relies on when it switches \
         between the two"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Composed filters — the tuple AND fold and the `Or` fold reaching `get`
// ════════════════════════════════════════════════════════════════════════════

#[derive(Bundle)]
struct TableOnly {
    p: KPay771,
    t: KTable772,
}

#[derive(Bundle)]
struct DenseAndTable {
    p: KPay771,
    d: KDense770,
    t: KTable772,
}

/// Four entities, one per membership combination of `{KDense770, KTable772}`.
///
/// The three-entity [`world`] fixture cannot exercise a COMPOSED filter: no
/// entity there carries `KTable772`, so the matched-archetype bitset rejects
/// every row under any filter naming it and an assertion over that fixture
/// could not fail either way. The two extra rows are what make the composed
/// tests capable of reding.
///
/// Returns `(ecs, dense_only, table_only, both, neither)` with payloads
/// `1, 2, 3, 4` respectively.
fn world_composed() -> (EcsMaster, Entity, Entity, Entity, Entity) {
    let mut ecs = EcsMaster::new();
    let dense_only = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(WithDense {
            p: KPay771 { p: 1 },
            d: KDense770 { x: 9 },
        })
        .id()
    });
    let table_only = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(TableOnly {
            p: KPay771 { p: 2 },
            t: KTable772 { x: 9 },
        })
        .id()
    });
    let both = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(DenseAndTable {
            p: KPay771 { p: 3 },
            d: KDense770 { x: 9 },
            t: KTable772 { x: 9 },
        })
        .id()
    });
    let neither =
        ecs.run_system(|mut cmds: Commands| cmds.spawn(PayOnly { p: KPay771 { p: 4 } }).id());
    (ecs, dense_only, table_only, both, neither)
}

/// The AND fold with one dense arm and one table arm. The table arm alone is
/// archetypal and the matched bitset applies it; the dense arm has no mask bit,
/// so `table_only` is the row that separates a real per-row fold from the
/// archetype-level answer.
#[test]
fn get_applies_a_tuple_filter_mixing_dense_and_table() {
    let (mut ecs, dense_only, table_only, both, neither) = world_composed();

    assert_eq!(
        ecs.query::<&KPay771, (With<KDense770>, With<KTable772>)>()
            .get(both),
        Some(&KPay771 { p: 3 }),
        "both arms hold — the row must be returned"
    );
    assert!(
        ecs.query::<&KPay771, (With<KDense770>, With<KTable772>)>()
            .get(table_only)
            .is_none(),
        "the DENSE arm must reject this row. `Some` here is KE13 inside the AND \
         fold: the table arm is decided by the archetype bitset and the dense arm \
         is never evaluated at all"
    );
    assert!(
        ecs.query::<&KPay771, (With<KDense770>, With<KTable772>)>()
            .get(dense_only)
            .is_none(),
        "the table arm rejects it through the matched bitset"
    );
    assert!(
        ecs.query::<&KPay771, (With<KDense770>, With<KTable772>)>()
            .get(neither)
            .is_none(),
        "neither arm holds"
    );
}

/// KE1's `Or` forwarding reaching the POINT path. `neither` is the row that can
/// red: `Or`'s `matches_component_set` admits every archetype (its dense arm has
/// no mask bit to test), so the archetype-level gate lets that row through and
/// only the per-row fold can reject it.
#[test]
fn get_applies_an_or_filter_with_a_dense_arm() {
    let (mut ecs, dense_only, table_only, both, neither) = world_composed();

    assert_eq!(
        ecs.query::<&KPay771, Or<(With<KDense770>, With<KTable772>)>>()
            .get(dense_only),
        Some(&KPay771 { p: 1 }),
        "the dense arm holds"
    );
    assert_eq!(
        ecs.query::<&KPay771, Or<(With<KDense770>, With<KTable772>)>>()
            .get(table_only),
        Some(&KPay771 { p: 2 }),
        "the table arm holds"
    );
    assert_eq!(
        ecs.query::<&KPay771, Or<(With<KDense770>, With<KTable772>)>>()
            .get(both),
        Some(&KPay771 { p: 3 }),
        "both arms hold"
    );
    assert!(
        ecs.query::<&KPay771, Or<(With<KDense770>, With<KTable772>)>>()
            .get(neither)
            .is_none(),
        "NEITHER arm holds, so the row must be rejected. `Some` here is KE13 \
         under an `Or`: the fold admits every archetype and `get` runs no per-row \
         predicate, so the filter gates nothing"
    );
}
