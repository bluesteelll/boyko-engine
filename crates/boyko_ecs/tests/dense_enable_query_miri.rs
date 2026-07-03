//! Dense-enable plan (D0–D6) — Miri Tree-Borrows + data-race validation for the
//! dense-INCLUDE × enable-term query path (the "compile-but-lie" zero-row fix).
//!
//! The unsafe surface: the dense column access (`&Dn` / `&mut Dn` fetch through
//! the `DenseStore` row pointer) COMBINED with the per-row enable `filter_fetch`
//! (the `EnableColumn` bit read). The archetype-walking `iter` / `iter_mut`
//! cursor gathers each row's dense pointer and applies the enable bit; a
//! stranded / aliased pointer or a missing-slot deref on a disabled row would
//! surface here (phase-14a precedent: Miri-TB has caught real kernel bugs).
//!
//! Every test drives the PUBLIC query path (`EcsMaster::query` → `QueryView`),
//! so the real unsafe code is interpreted, not modelled. Surfaces:
//!
//! * (a) `Enabled<Tag>` iter over some-enabled / all-disabled dense rows — the
//!   per-row bit read interleaved with the dense gather (disabled rows skipped
//!   WITHOUT dereferencing their slot for the yielded item on a `&mut` write).
//! * (b) `Disabled<Tag>` iter including a no-column dense archetype (A1.1) — the
//!   NULL-column `filter_fetch` (`true`) path over a dense gather.
//! * (c) `&mut Dn` write-through under `Enabled<Tag>` — the write lands only on
//!   enabled rows and persists (no UB on the gather/bit-test/write chain).
//! * (d) `iter` / `iter_mut` over the enabled vs disabled set — the per-row bit
//!   test + dense fetch on the point-comparable set. (`get` / `get_mut` Miri
//!   coverage is DEFERRED: they null-deref on a dense `D` today — a pre-existing
//!   bug tracked as a follow-up; see the case-(d) note.)
//!
//! Run (toolchain note — nightly GNU):
//! ```text
//! RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu \
//!   MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks -Zmiri-disable-isolation" \
//!   cargo miri test -p boyko-ecs --test dense_enable_query_miri
//! ```
//! (The Commands / `run_system` RawVec apply-path leak reports are PRE-EXISTING
//! and unrelated — suppressed by `-Zmiri-ignore-leaks`.)

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter_enable::{Disabled, Enabled};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

/// 16-byte POD dense payload (signature-excluded, global column).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct Dn {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

/// A plain TABLE marker so distinct archetypes can be built.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Marker {
    m: u32,
}

/// Bitset enable tag (no `ComponentPool`; per-row bit in an `EnableColumn`).
#[derive(Component)]
#[component(storage = "bitset")]
#[repr(C)]
struct Tag;

/// `(Dn,)` bundle — a pure-dense entity (empty table signature).
#[derive(Bundle)]
struct DnOnly {
    d: Dn,
}

/// `(Marker, Dn)` bundle — a distinct-signature dense entity.
#[derive(Bundle)]
struct MarkerDn {
    mk: Marker,
    d: Dn,
}

#[inline]
fn dn(x: f32) -> Dn {
    Dn { x, y: x + 1.0, z: x + 2.0, w: x + 3.0 }
}

/// Spawn one pure-dense entity, optionally enabling `Tag` on it.
fn spawn_dn(ecs: &mut EcsMaster, x: f32, enabled: bool) {
    ecs.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(DnOnly { d: dn(x) });
        if enabled {
            e.enable::<Tag>();
        }
    });
}

/// Spawn one `(Marker, Dn)` entity, optionally enabling `Tag`.
fn spawn_marker_dn(ecs: &mut EcsMaster, m: u32, x: f32, enabled: bool) {
    ecs.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(MarkerDn { mk: Marker { m }, d: dn(x) });
        if enabled {
            e.enable::<Tag>();
        }
    });
}

// ════════════════════════════════════════════════════════════════════════════
// (a) Enabled<Tag> iter: per-row bit read interleaved with the dense gather.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_dense_enabled_iter_clean() {
    let mut ecs = EcsMaster::new();
    spawn_dn(&mut ecs, 0.0, true); // enabled
    spawn_dn(&mut ecs, 1.0, false); // disabled
    spawn_marker_dn(&mut ecs, 7, 2.0, true); // enabled, distinct archetype
    spawn_marker_dn(&mut ecs, 8, 3.0, false); // disabled

    let mut got: Vec<f32> = ecs
        .query::<&Dn, Enabled<Tag>>()
        .iter()
        .map(|d: &Dn| d.x)
        .collect();
    got.sort_by(f32::total_cmp);
    assert_eq!(
        got,
        vec![0.0, 2.0],
        "Enabled<Tag> yields only enabled dense rows (no UB on the bit/gather interleave)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// (b) Disabled<Tag> iter including a no-column dense archetype (A1.1): the
//     NULL-column filter_fetch (true) over a dense gather.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_dense_disabled_no_column_iter_clean() {
    let mut ecs = EcsMaster::new();
    // Present-Tag archetype: one enabled, one disabled row.
    spawn_dn(&mut ecs, 0.0, true);
    spawn_dn(&mut ecs, 1.0, false);
    // A distinct archetype that NEVER gains a Tag column ⇒ every row disabled.
    spawn_marker_dn(&mut ecs, 9, 42.0, false);

    let mut got: Vec<f32> = ecs
        .query::<&Dn, Disabled<Tag>>()
        .iter()
        .map(|d: &Dn| d.x)
        .collect();
    got.sort_by(f32::total_cmp);
    assert_eq!(
        got,
        vec![1.0, 42.0],
        "Disabled<Tag> yields disabled rows incl. the no-column archetype (NULL-column ⇒ true)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// (c) &mut Dn write-through under Enabled<Tag>: write lands on enabled rows only.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_dense_enabled_iter_mut_write_through_lands() {
    let mut ecs = EcsMaster::new();
    spawn_dn(&mut ecs, 0.0, true); // enabled
    spawn_dn(&mut ecs, 1.0, false); // disabled
    spawn_dn(&mut ecs, 2.0, true); // enabled

    {
        let mut view = ecs.query::<&mut Dn, Enabled<Tag>>();
        for d in view.iter_mut() {
            let d: &mut Dn = d;
            d.x += 100.0;
        }
    }

    // Read back ALL rows via the pure-dense cursor; only enabled rows changed.
    let mut all: Vec<f32> = ecs
        .query::<&Dn, ()>()
        .dense_iter()
        .map(|(_e, d): (_, &Dn)| d.x)
        .collect();
    all.sort_by(f32::total_cmp);
    assert_eq!(
        all,
        vec![1.0, 100.0, 102.0],
        "iter_mut writes land only on enabled rows (disabled row 1.0 untouched; no UB)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// (d) iter / iter_mut over enabled vs disabled dense entities: the per-row bit
//     test + dense fetch on the point-comparable set.
//
// NOTE (reviewer P2-a): this case does NOT exercise `get` / `get_mut`. Those
// currently null-deref on a dense `D` (a PRE-EXISTING bug unrelated to the
// dense-enable feature: `get`/`get_mut` never call `resolve_dense`, so
// `fetch.dense` stays null — see the follow-up tracked as "QueryView::get/get_mut
// null-deref on dense components"). Miri coverage of the `get`/`get_mut` unsafe
// surface is DEFERRED until that fix lands; this case instead witnesses that the
// `iter` set (which the enabled `get` MUST agree with) is exactly the enabled
// row, under Tree-Borrows.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_dense_enabled_iter_agrees() {
    let mut ecs = EcsMaster::new();
    // Capture the entity handles by spawning through create_entity-less path:
    // run_system spawns don't return the Entity, so re-derive via a scan.
    spawn_dn(&mut ecs, 10.0, true);
    spawn_dn(&mut ecs, 11.0, false);

    // The get/iter agreement: the iter set is exactly the enabled row.
    let got: Vec<f32> = ecs
        .query::<&Dn, Enabled<Tag>>()
        .iter()
        .map(|d: &Dn| d.x)
        .collect();
    assert_eq!(got, vec![10.0], "iter yields the enabled row (get/iter agree, no UB)");

    // get_mut write-through on the enabled query lands on the enabled row.
    {
        let mut view = ecs.query::<&mut Dn, Enabled<Tag>>();
        for d in view.iter_mut() {
            let d: &mut Dn = d;
            d.x += 5.0;
        }
    }
    let mut all: Vec<f32> = ecs
        .query::<&Dn, ()>()
        .dense_iter()
        .map(|(_e, d): (_, &Dn)| d.x)
        .collect();
    all.sort_by(f32::total_cmp);
    assert_eq!(all, vec![11.0, 15.0], "enabled row mutated (11.0 disabled untouched)");
}
