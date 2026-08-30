//! **KE13 (raised by Aether rung R0's census, 2026-08-29) — `QueryView::get` /
//! `get_mut` silently IGNORE a dense `With<C>` / `Without<C>` filter.** RED BY
//! DESIGN; not fixed here.
//!
//! # What is wrong
//!
//! `QueryView::get` (`crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs`)
//! applies, in order: the matched-archetype bitset, the dynamic tag terms, the
//! typed enable term (gated on `const { F::CONTAINS_ENABLE_TERM }`), and the
//! dynamic enable terms. It **never calls `F::filter_fetch`**, and it never
//! calls `F::resolve_dense` — it resolves only `D`'s
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
//!   `get` does not make.
//!
//! So `get` returns `Some(row)` for an entity that is **not a member of the dense
//! store**, and `Without<Dense>` is ignored in the same breath. `iter()` on the
//! same `QueryView` answers correctly, so `get` and `iter` **disagree**.
//!
//! # The doc comment on `get` asserts the opposite, in writing
//!
//! > Non-archetypal **change-detection** filters (`Changed<C>` / `Added<C>`) can
//! > never reach a `QueryView` … So `get` only ever sees archetypal and enable
//! > terms — **there is no silent-ignore path.**
//!
//! That reasoning was true when written and enumerates the term kinds
//! exhaustively — but it rests on "`With`/`Without` are archetypal", which the
//! Dense plan falsified: a dense `With` is the third kind, neither archetypal nor
//! enable. **Any fix owes this paragraph an edit in the same commit**, because a
//! reader who trusts it stops looking.
//!
//! # Relation to KE1 / rung R0
//!
//! Same shape as the defect R0 just fixed — *a caller that does not make the
//! dense resolve call* — one path over. R0 fixed `Or`'s missing `HAS_DENSE` /
//! `resolve_dense`; this is `QueryView::get`'s missing `F`-side resolve **plus a
//! missing `filter_fetch`**, which is strictly worse: R0's bug made an arm never
//! true, this one applies no gate at all.
//!
//! There is a **recorded precedent in-tree** that this path is known to be
//! under-served: `iters/query/state.rs`'s `dense_get_iter_agree` test carries a
//! "PRE-EXISTING BUG (out of D0–D6 scope, flagged for the reviewer)" note that
//! `get` never calls `resolve_dense`, describing the **`D`-side** null-deref and
//! calling the repair "a one-line `resolve_dense` mirror". The `D` side has since
//! been fixed (the `const { <D as QueryData>::HAS_DENSE }` block in `get` /
//! `get_mut`). **The `F` side was not**, and it is not a one-liner: `get` has no
//! `filter_fetch` call to feed.
//!
//! # Why this is `#[ignore]`d rather than fixed
//!
//! It has no owner. R0's mandate is KE1 and the census; the repair here is a real
//! design step (`get` must grow a per-row filter application, which interacts
//! with the `set_table_*` / meta-free dispatch split and with the `terms` /
//! enable ordering above it), and a rung should own it. Filed as **KE13** in
//! [`docs/aether-v2/KERNEL-BACKLOG.md`](../../../docs/aether-v2/KERNEL-BACKLOG.md).
//!
//! Run with:
//! `cargo test -p boyko-ecs --test ke13_query_view_get_ignores_dense_filter -- --ignored`

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{With, Without};
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

/// Floor, NOT ignored: `get` DOES apply a TABLE `Without<C>`, and `iter` over the
/// dense filter answers correctly. If either half of this reds, the two ignored
/// tests below are measuring the wrong thing.
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

/// `get` on a NON-member must return `None` under `With<Dense>`. It returns
/// `Some`, because the dense arm's per-row gate is never run.
///
/// Hand oracle: `None` for the outsider, `Some` for the member — matching what
/// `iter()` reports in the control above. Observed today: `Some` for BOTH.
#[test]
#[ignore = "deferred: KE13 (docs/aether-v2/KERNEL-BACKLOG.md) — QueryView::get/get_mut \
            never call F::filter_fetch or F::resolve_dense, so a dense With/Without \
            filter is silently ignored and get disagrees with iter. Unowned as of R0; \
            RED BY DESIGN until a rung takes it. See this file's header"]
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
/// It returns `Some` for both, so the exclusion never happens.
///
/// Hand oracle: `None` for the member, `Some` for the outsider. Observed today:
/// `Some` for BOTH.
#[test]
#[ignore = "deferred: KE13 (docs/aether-v2/KERNEL-BACKLOG.md) — QueryView::get/get_mut \
            never call F::filter_fetch or F::resolve_dense, so a dense With/Without \
            filter is silently ignored and get disagrees with iter. Unowned as of R0; \
            RED BY DESIGN until a rung takes it. See this file's header"]
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
