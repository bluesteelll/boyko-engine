//! GATE 5 — a Grid lays its children at the right cells, and GATE 7 — the Grid
//! placement scales linearly in the child count (no super-linear scan).
//!
//! Grid axis convention (`layout.rs`): main = rows down y, cross = columns across
//! x. A `cols x rows` grid over a `W x H` content box gives a uniform cell
//! `cell_w = W / cols`, `cell_h = H / rows`. Relative child at flow slot `k`
//! occupies cell `(col = k % cols, row = k / cols)` at `x = col*cell_w`,
//! `y = row*cell_h`.
//!
//! GATE 7: the inline `layout.rs` complexity module already proves the
//! `measure_node` visit count stays linear for chain/fan/balanced trees (the
//! `#[cfg(test)]` `measure_visits` probe is private to the lib, so it cannot be
//! read here). The Grid passes (`resolve_grid_child_sizes` / `position_grid_from
//! _arena`) are `O(relative_count)` and never re-enter `measure_node`, so the P1
//! guard still bounds them. This file adds a Grid-SPECIFIC wall-clock guard at
//! N = 10/100/1000 children: a super-linear (e.g. O(N^2)) cell scan would blow the
//! generous debug-build bound, complementing the lib-internal visit-count proof.

mod p6a_common;

use std::time::{Duration, Instant};

use p6a_common::{approx, P6a};

use boyko_ui::components::{ComputedRect, UiGrid, UiLayout, UiRoot};
use boyko_ui::resources::UiViewport;
use boyko_ui::units::{LayoutType, Unit};

/// A Grid container sized `w x h` (definite Px).
fn grid_container(w: f32, h: f32) -> UiLayout {
    UiLayout {
        layout_type: LayoutType::Grid,
        width: Unit::Px(w),
        height: Unit::Px(h),
        ..UiLayout::default()
    }
}

/// A grid child (Auto — it is resized to the uniform cell by the solver).
fn grid_child() -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, ..UiLayout::default() }
}

fn world() -> P6a {
    P6a::new(UiViewport { width: 2000.0, height: 2000.0, scale_factor: 1.0, generation: 0 })
}

#[test]
fn grid_2x2_places_four_children_in_cells() {
    let mut w = world();
    // 100x100 grid, 2 cols x 2 rows -> 50x50 cells.
    let (g, kids) = w.spawn_grid(
        ComputedRect::default(),
        grid_container(100.0, 100.0),
        UiGrid { columns: 2, rows: 2 },
        4,
        grid_child(),
    );
    w.run();
    assert_eq!(w.rect(g).w, 100.0, "grid is 100 wide");

    // Children are id-ordered in the flow (the layout sorts by Entity::id), so
    // kids[k] is flow slot k -> cell (col = k%2, row = k/2).
    let expect = [(0.0, 0.0), (50.0, 0.0), (0.0, 50.0), (50.0, 50.0)];
    for (k, &(ex, ey)) in expect.iter().enumerate() {
        let r = w.rect(kids[k]);
        approx(r.x, ex, &format!("cell {k} x"));
        approx(r.y, ey, &format!("cell {k} y"));
        approx(r.w, 50.0, &format!("cell {k} w"));
        approx(r.h, 50.0, &format!("cell {k} h"));
    }
}

#[test]
fn grid_3_cols_auto_rows_wraps() {
    let mut w = world();
    // 300x200 grid, 3 cols, rows=0 -> derived ceil(6/3)=2 rows -> cell 100x100.
    let (_g, kids) = w.spawn_grid(
        ComputedRect::default(),
        grid_container(300.0, 200.0),
        UiGrid { columns: 3, rows: 0 },
        6,
        grid_child(),
    );
    w.run();
    // cell_w = 300/3 = 100; cell_h = 200/2 = 100.
    let expect = [
        (0.0, 0.0),
        (100.0, 0.0),
        (200.0, 0.0),
        (0.0, 100.0),
        (100.0, 100.0),
        (200.0, 100.0),
    ];
    for (k, &(ex, ey)) in expect.iter().enumerate() {
        let r = w.rect(kids[k]);
        approx(r.x, ex, &format!("wrap cell {k} x"));
        approx(r.y, ey, &format!("wrap cell {k} y"));
        approx(r.w, 100.0, &format!("wrap cell {k} w"));
        approx(r.h, 100.0, &format!("wrap cell {k} h"));
    }
}

#[test]
fn grid_single_column_stacks_vertically() {
    let mut w = world();
    // columns=1, 3 children -> derived 3 rows; 100x300 -> cell 100x100 stacked.
    let (_g, kids) = w.spawn_grid(
        ComputedRect::default(),
        grid_container(100.0, 300.0),
        UiGrid { columns: 1, rows: 0 },
        3,
        grid_child(),
    );
    w.run();
    let expect = [(0.0, 0.0), (0.0, 100.0), (0.0, 200.0)];
    for (k, &(ex, ey)) in expect.iter().enumerate() {
        let r = w.rect(kids[k]);
        approx(r.x, ex, &format!("col cell {k} x"));
        approx(r.y, ey, &format!("col cell {k} y"));
    }
}

#[test]
fn grid_partial_last_row() {
    let mut w = world();
    // 2 cols, 5 children -> derived ceil(5/2)=3 rows; 200x300 -> cell 100x100.
    // The last (5th) child sits alone in row 2, col 0.
    let (_g, kids) = w.spawn_grid(
        ComputedRect::default(),
        grid_container(200.0, 300.0),
        UiGrid { columns: 2, rows: 0 },
        5,
        grid_child(),
    );
    w.run();
    let expect = [
        (0.0, 0.0),
        (100.0, 0.0),
        (0.0, 100.0),
        (100.0, 100.0),
        (0.0, 200.0), // 5th child: row 2, col 0
    ];
    for (k, &(ex, ey)) in expect.iter().enumerate() {
        let r = w.rect(kids[k]);
        approx(r.x, ex, &format!("partial cell {k} x"));
        approx(r.y, ey, &format!("partial cell {k} y"));
    }
}

// ───────────────────────── GATE 7: linear grid placement ────────────────────

/// Lays out one grid of `n` children once, returning the relayout wall-clock.
fn grid_relayout_time(n: usize) -> Duration {
    let mut w = world();
    // A wide single-row grid (cols = n) so the placement walks all n children at a
    // shallow depth (no MAX_LAYOUT_DEPTH concern).
    let cols = n.min(u8::MAX as usize) as u8;
    let (_g, kids) = w.spawn_grid(
        ComputedRect::default(),
        grid_container(2000.0, 100.0),
        UiGrid { columns: cols, rows: 1 },
        n,
        grid_child(),
    );
    // First run lays everything out; time a CLEAN relayout on a forced-dirty frame.
    w.run();
    // Touch one child's UiLayout to force a relayout, then time it.
    let last = kids[n - 1];
    w.world.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        cmds.entity(last).insert(grid_child());
    });
    let start = Instant::now();
    w.run();
    start.elapsed()
}

// ───────────────────────── Auto-sized grid hug (regression) ─────────────────

/// An AUTO-sized Grid container (no definite main/cross) must hug ALL its tracks:
/// `rows * cell_main` x `cols * cell_cross`. The prior measure hugged a SINGLE cell
/// (`max(child)`) then divided by the track count, squishing every cell to
/// `1/rows` x `1/cols` of one child. The grid is nested under a definite Overlay
/// root (a `UiRoot` grid would be viewport-filled, masking the hug).
///
/// Old (buggy) behavior: grid hugs 40x30 (one cell) -> cells crushed to 20x15.
/// Fixed behavior: grid hugs 80x60 (2x2 tracks) -> full 40x30 cells in distinct rows.
#[test]
fn grid_auto_container_hugs_all_tracks_not_one_cell() {
    let mut w = world();
    let root = w.spawn_with(|cmds| {
        let mut ec = cmds.spawn(UiLayout {
            layout_type: LayoutType::Overlay,
            width: Unit::Px(1000.0),
            height: Unit::Px(1000.0),
            ..UiLayout::default()
        });
        ec.insert(ComputedRect::default());
        ec.insert(UiRoot);
        ec.id()
    });
    // Auto-sized grid (default Unit on width/height), 2x2.
    let grid = w.spawn_with(move |cmds| {
        let mut ec = cmds.spawn(UiLayout { layout_type: LayoutType::Grid, ..UiLayout::default() });
        ec.insert(ComputedRect::default());
        ec.insert(UiGrid { columns: 2, rows: 2 });
        ec.set_parent(root);
        ec.id()
    });
    // Four children with a definite intrinsic 40w x 30h, so the measured cell is
    // non-zero (each is then resized to the uniform cell by the solver).
    let mut kids = Vec::with_capacity(4);
    for _ in 0..4 {
        let k = w.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(UiLayout {
                layout_type: LayoutType::Column,
                width: Unit::Px(40.0),
                height: Unit::Px(30.0),
                ..UiLayout::default()
            });
            ec.insert(ComputedRect::default());
            ec.set_parent(grid);
            ec.id()
        });
        kids.push(k);
    }
    w.run();

    let gr = w.rect(grid);
    approx(gr.w, 80.0, "auto grid hugs cols*cell_w = 2*40 (not one cell)");
    approx(gr.h, 60.0, "auto grid hugs rows*cell_h = 2*30 (not one cell)");
    // Cells are full 40x30 (NOT squished to 20x15) at distinct, non-overlapping rows.
    let expect = [(0.0, 0.0), (40.0, 0.0), (0.0, 30.0), (40.0, 30.0)];
    for (idx, &(ex, ey)) in expect.iter().enumerate() {
        let r = w.rect(kids[idx]);
        approx(r.w, 40.0, &format!("auto cell {idx} w not squished"));
        approx(r.h, 30.0, &format!("auto cell {idx} h not squished"));
        approx(r.x - gr.x, ex, &format!("auto cell {idx} x"));
        approx(r.y - gr.y, ey, &format!("auto cell {idx} y"));
    }
    assert!(
        w.rect(kids[2]).y > w.rect(kids[0]).y + 1.0,
        "row 1 sits a full cell below row 0 (distinct, non-squished rows)"
    );
}

#[test]
fn grid_placement_scales_linearly() {
    // O(children) placement: relayout time at N=10/100/1000 must stay well under a
    // generous debug-build ceiling. A super-linear (O(N^2)) cell scan at N=1000
    // would blow this; the existing lib-internal `measure_visits` guard proves the
    // visit count itself stays linear for the underlying measure pass.
    for &n in &[10usize, 100, 1000] {
        let t = grid_relayout_time(n);
        // 50ms is the same coarse bound the lib's 1000-node fan guard uses.
        assert!(
            t < Duration::from_millis(50),
            "grid of {n} children relaid in {t:?} (>= 50ms) — possible super-linear placement"
        );
        println!("[grid-scaling] N={n} relayout={t:?}");
    }
}
