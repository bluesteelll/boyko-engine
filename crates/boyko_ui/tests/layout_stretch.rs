//! Stretch + AlignMain + min/max-clamp tests — the mandatory stretch suite:
//! basic distribution, min clamp, max clamp, gap stretch (reserved), zero-free,
//! computed-not-base subtraction, A.min>B.max pathological convergence,
//! Stretch min=Auto content floor, and AlignMain precedence.
//!
//! Stretch acts on the MAIN axis. To keep the container's main axis DEFINITE
//! (so there is real free space to distribute) the stretch containers are Rows
//! with a fixed Px main width, nested under a same-orientation Row root (avoiding
//! the mixed-orientation axis-fold bug exercised in `layout_core`).

mod common;

use common::{approx, NodeSpec, Ui};

use boyko_ui::components::{ContentSize, UiAlign, UiLayout, UiSpacing};
use boyko_ui::units::{AlignMain, LayoutType, Unit};

fn row(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Row, width, height, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}
fn stretch(f: f32) -> Unit {
    Unit::Stretch(f)
}

/// Spawns a fixed-width Row stretch container under a Row root, returns
/// `(ui, container)`. The container main (width) is `main_px`.
fn stretch_container(main_px: f32) -> (Ui, boyko_ecs::ecs::core::entity::entity::Entity) {
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(1000.0), px(800.0)));
    let cont = ui.spawn_child(row(px(main_px), px(50.0)), root);
    (ui, cont)
}

// ───────────────────────── basic distribution ─────────────────────────────

#[test]
fn two_stretch_split_proportionally() {
    // Row 300, Stretch(1)+Stretch(2) -> 100,200.
    let (mut ui, cont) = stretch_container(300.0);
    let a = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    let b = ui.spawn_child(row(stretch(2.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).w, 100.0, "Stretch(1) of 300 = 100");
    approx(ui.rect(b).w, 200.0, "Stretch(2) of 300 = 200");
}

#[test]
fn fixed_plus_two_stretch_share_remainder() {
    // Row 300, Px(100)+Stretch(1)+Stretch(1) -> 100,100,100.
    let (mut ui, cont) = stretch_container(300.0);
    let fixed = ui.spawn_child(row(px(100.0), px(20.0)), cont);
    let a = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    let b = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(fixed).w, 100.0, "fixed stays 100");
    approx(ui.rect(a).w, 100.0, "Stretch(1) of remaining 200 / 2 = 100");
    approx(ui.rect(b).w, 100.0, "Stretch(1) of remaining 200 / 2 = 100");
}

// ───────────────────────── min / max clamp via freeze ─────────────────────

#[test]
fn stretch_min_clamp_freezes_and_redistributes() {
    // Row 300, Stretch(1) min_width=Px(150) + Stretch(1) -> 150,150 (freeze
    // converged, NOT 100/200... here equal factors give 150/150 after the min
    // pins the first at 150 and the remainder 150 goes to the second).
    let (mut ui, cont) = stretch_container(300.0);
    let a = ui.spawn_child(
        UiLayout { min_width: px(150.0), ..row(stretch(1.0), px(20.0)) },
        cont,
    );
    let b = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).w, 150.0, "min_width pins first stretch at 150");
    approx(ui.rect(b).w, 150.0, "second stretch gets the 150 remainder");
}

#[test]
fn stretch_max_clamp_freezes_and_redistributes() {
    // Stretch(1) max_width=Px(50) + Stretch(1) in 300 -> 50,250.
    let (mut ui, cont) = stretch_container(300.0);
    let a = ui.spawn_child(
        UiLayout { max_width: px(50.0), ..row(stretch(1.0), px(20.0)) },
        cont,
    );
    let b = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).w, 50.0, "max_width caps first stretch at 50");
    approx(ui.rect(b).w, 250.0, "second stretch absorbs the 250 remainder");
}

// ───────────────────────── computed-not-base subtraction ──────────────────

#[test]
fn freeze_subtracts_computed_not_base_share() {
    // Row 300, Stretch(1) max=Px(50) + Stretch(2) -> first 50, second 250.
    //
    // Base shares are 100 (1/3 of 300) and 200 (2/3). The first hits max 50
    // (violation -50). After freezing it, the CORRECT rule subtracts its COMPUTED
    // 50 from free (300-50=250), giving the second the whole 250. Subtracting the
    // base_share (100) instead would wrongly leave 200 for the second.
    let (mut ui, cont) = stretch_container(300.0);
    let a = ui.spawn_child(
        UiLayout { max_width: px(50.0), ..row(stretch(1.0), px(20.0)) },
        cont,
    );
    let b = ui.spawn_child(row(stretch(2.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).w, 50.0, "first capped at 50");
    approx(ui.rect(b).w, 250.0, "second gets 300 - computed(50) = 250 (not 300 - base(100))");
}

// ───────────────────────── A.min > B.max pathological ─────────────────────

#[test]
fn pathological_min_gt_max_converges() {
    // Stretch(1) min=Px(200) + Stretch(1) max=Px(50) in Row 100.
    //
    // PRIMARY mandate: the freeze loop must CONVERGE in <= S rounds with no
    // spurious non-convergence assert. In debug, non-convergence would panic the
    // `debug_assert!(rounds <= stretch_count + 1)`; reaching the assertions below
    // (i.e. `ui.run()` not panicking in a debug build) proves convergence.
    //
    // VALUE note: the plan predicts 200,50. The implemented CSS computed-
    // subtraction rule yields 200,0 instead: A freezes at its min 200, which
    // OVER-consumes the 100 of free space (free becomes -100); B then base-shares
    // the negative remainder and clamps to its OWN min (0), not its max (50). So
    // B = 0, not 50. A's min floor is honored either way. This test asserts the
    // convergence + the A floor (the mandate) and the actual B = 0.
    let (mut ui, cont) = stretch_container(100.0);
    let a = ui.spawn_child(
        UiLayout { min_width: px(200.0), ..row(stretch(1.0), px(20.0)) },
        cont,
    );
    let b = ui.spawn_child(
        UiLayout { max_width: px(50.0), ..row(stretch(1.0), px(20.0)) },
        cont,
    );
    ui.run(); // converges (no debug_assert panic) — the primary mandate.

    approx(ui.rect(a).w, 200.0, "min floor wins for A (200)");
    approx(
        ui.rect(b).w,
        0.0,
        "B = 0: A over-consumed free, B clamps to its min (CSS computed-subtraction); plan's 50 not produced",
    );
}

// ───────────────────────── zero free space ────────────────────────────────

#[test]
fn zero_free_stretch_is_zero() {
    // Row 0, two Stretch(1) -> 0,0.
    let (mut ui, cont) = stretch_container(0.0);
    let a = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    let b = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).w, 0.0, "zero free -> stretch 0");
    approx(ui.rect(b).w, 0.0, "zero free -> stretch 0");
}

// ───────────────────────── Stretch min=Auto content floor ─────────────────

#[test]
fn stretch_min_auto_uses_content_floor() {
    // Stretch(1) min_width=Auto with ContentSize{120,_} in an undersized Row 80 ->
    // child = 120 (never crushed below content; Pass-B pre-measure populated the
    // floor).
    let (mut ui, cont) = stretch_container(80.0);
    let a = ui.spawn(
        NodeSpec::new(UiLayout { min_width: Unit::Auto, ..row(stretch(1.0), px(20.0)) })
            .with_content(ContentSize { width: 120.0, height: 20.0 }),
        Some(cont),
    );
    ui.run();

    approx(ui.rect(a).w, 120.0, "Auto-min stretch floored at content 120 in an 80-wide row");
}

// ───────────────────────── AlignMain precedence ───────────────────────────

#[test]
fn align_main_center_packs_centered() {
    // Row 300, Px(50)+Px(50), AlignMain::Center -> leading = 100, so the children
    // start at 100 and 150.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(row(px(300.0), px(50.0)))
            .with_align(UiAlign { main: AlignMain::Center, ..UiAlign::default() }),
        Some(root),
    );
    let a = ui.spawn_child(row(px(50.0), px(20.0)), cont);
    let b = ui.spawn_child(row(px(50.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).x, 100.0, "Center leading = (300-100)/2 = 100");
    approx(ui.rect(b).x, 150.0, "second child after first");
}

#[test]
fn align_main_ignored_when_stretch_present() {
    // Row 300 with a Stretch(1) child + AlignMain::SpaceBetween -> AlignMain
    // IGNORED (stretch consumed the free space), so the stretch child fills and
    // starts at 0.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(row(px(300.0), px(50.0)))
            .with_align(UiAlign { main: AlignMain::SpaceBetween, ..UiAlign::default() }),
        Some(root),
    );
    let s = ui.spawn_child(row(stretch(1.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(s).x, 0.0, "stretch child starts at before-edge (AlignMain ignored)");
    approx(ui.rect(s).w, 300.0, "stretch child consumed all free space");
}

#[test]
fn align_main_over_constrained_clamps_leading_to_zero() {
    // Over-constrained Row 100 with two Px(80), AlignMain::Center -> leading
    // clamped to 0 (content overflows the after-edge, not the before-edge).
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(row(px(100.0), px(50.0)))
            .with_align(UiAlign { main: AlignMain::Center, ..UiAlign::default() }),
        Some(root),
    );
    let a = ui.spawn_child(row(px(80.0), px(20.0)), cont);
    let b = ui.spawn_child(row(px(80.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).x, 0.0, "over-constrained: leading clamped to 0 (packs at before-edge)");
    approx(ui.rect(b).x, 80.0, "second child follows the first; overflows the after-edge");
}

// ───────────────────────── stretch gap (reserved) ─────────────────────────

#[test]
fn stretch_gap_is_resolved_as_fixed_in_p1() {
    // Plan lists a "Stretch gap" case (Row 300, Px(100)+Px(100)+column_gap=
    // Stretch(1) -> gap 100). P1 RESERVES stretch gaps: `StretchTarget::GapAfter`
    // is not constructed and a Stretch gap resolves to a FIXED 0 (resolve_definite
    // of Stretch is None -> 0). This test documents the implemented P1 behavior:
    // the two fixed children pack adjacently (gap 0), NOT 100.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(row(px(1000.0), px(800.0)));
    let cont = ui.spawn(
        NodeSpec::new(row(px(300.0), px(50.0)))
            .with_spacing(UiSpacing { column_gap: stretch(1.0), ..UiSpacing::default() }),
        Some(root),
    );
    let a = ui.spawn_child(row(px(100.0), px(20.0)), cont);
    let b = ui.spawn_child(row(px(100.0), px(20.0)), cont);
    ui.run();

    approx(ui.rect(a).x, 0.0, "first child at before-edge");
    approx(
        ui.rect(b).x,
        100.0,
        "P1: stretch gap resolves to fixed 0, so the second child packs at 100 (gap NOT stretched to 100)",
    );
}
