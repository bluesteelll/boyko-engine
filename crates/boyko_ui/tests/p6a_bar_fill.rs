//! GATE 2 — the Bar fill width tracks a bound `0..1` value across updates, and the
//! driver is `Changed`-gated (0% work on a still frame).
//!
//! The track is a Row of definite size (so its content box width is known); the
//! `BarFill` child's main-axis (width) is `Unit::Pct`, driven by `ui_bar_apply`
//! from the track's `UiValue`. After the same-frame relayout (bar systems run
//! `.before` layout in the harness — the documented `ui_text_measure_system`
//! ordering), the fill's `ComputedRect.w` is `track_content_w * value`.
//!
//! The 0%-work gate is asserted WITHOUT a counting allocator: a still frame must
//! not advance the fill's `Changed<UiLayout>` tick (the bar driver writes the
//! fill's `UiLayout` set-if-changed; an idle frame writes nothing, so the tick is
//! frozen). This is the same per-thread-safe, allocator-free probe the P4 bind
//! 0%-work gate uses (`bind_text_still_frame_does_not_reformat_or_change_sink`).

mod p6a_common;

use p6a_common::{approx, P6a};

use boyko_ui::components::{ComputedRect, UiLayout};
use boyko_ui::resources::UiViewport;
use boyko_ui::units::{LayoutType, Unit};

/// A Row track of width `w`, height `h` (definite Px), at the origin.
fn row_track(w: f32, h: f32) -> UiLayout {
    UiLayout {
        layout_type: LayoutType::Row,
        width: Unit::Px(w),
        height: Unit::Px(h),
        ..UiLayout::default()
    }
}

/// A Column track of width `w`, height `h` (definite Px).
fn col_track(w: f32, h: f32) -> UiLayout {
    UiLayout {
        layout_type: LayoutType::Column,
        width: Unit::Px(w),
        height: Unit::Px(h),
        ..UiLayout::default()
    }
}

/// A fill node: full cross-axis, main-axis driven by the bar. Start at Pct(0).
fn fill_node(cross_full: Unit) -> UiLayout {
    UiLayout {
        // The cross axis is full; the driven main axis starts at Pct(0).
        width: Unit::Pct(0.0),
        height: cross_full,
        ..UiLayout::default()
    }
}

fn p6a() -> P6a {
    P6a::new(UiViewport { width: 1000.0, height: 800.0, scale_factor: 1.0, generation: 0 })
}

#[test]
fn bar_fill_width_tracks_value_across_updates() {
    let mut w = p6a();
    // Row track 200x40; fill width is the driven main axis, height = full (40).
    let (track, fill) = w.spawn_bar(
        ComputedRect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 },
        row_track(200.0, 40.0),
        0.5,
        fill_node(Unit::Px(40.0)),
    );
    w.run();

    // value=0.5 -> fill width = 200 * 0.5 = 100.
    approx(w.rect(fill).w, 100.0, "fill width at value 0.5");
    assert_eq!(w.rect(track).w, 200.0, "track is 200 wide");
    approx(w.fill_pct(fill, true).expect("fill width is a Pct"), 50.0, "fill Pct at 0.5");

    // Increase to 0.75.
    w.set_value(track, 0.75);
    w.run_settled();
    approx(w.rect(fill).w, 150.0, "fill width at value 0.75");

    // Decrease to 0.1.
    w.set_value(track, 0.1);
    w.run_settled();
    approx(w.rect(fill).w, 20.0, "fill width at value 0.1");

    // Full.
    w.set_value(track, 1.0);
    w.run_settled();
    approx(w.rect(fill).w, 200.0, "fill width at value 1.0 (full track)");

    // Empty.
    w.set_value(track, 0.0);
    w.run_settled();
    approx(w.rect(fill).w, 0.0, "fill width at value 0.0 (empty)");
}

#[test]
fn bar_value_clamped_out_of_range() {
    let mut w = p6a();
    let (track, fill) = w.spawn_bar(
        ComputedRect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 },
        row_track(200.0, 40.0),
        0.0,
        fill_node(Unit::Px(40.0)),
    );
    w.run();

    // Over 1.0 clamps to full.
    w.set_value(track, 1.5);
    w.run_settled();
    approx(w.rect(fill).w, 200.0, "value > 1 clamps to full");

    // Below 0 clamps to empty.
    w.set_value(track, -0.3);
    w.run_settled();
    approx(w.rect(fill).w, 0.0, "value < 0 clamps to empty");

    // NaN collapses to 0% (the quantized_pct non-finite guard).
    w.set_value(track, f32::NAN);
    w.run_settled();
    approx(w.rect(fill).w, 0.0, "NaN value collapses to 0%");
}

#[test]
fn bar_column_track_drives_fill_height() {
    let mut w = p6a();
    // Column track 40x200; the fill's HEIGHT is the driven main axis.
    let (track, fill) = w.spawn_bar(
        ComputedRect { x: 0.0, y: 0.0, w: 40.0, h: 200.0 },
        col_track(40.0, 200.0),
        0.25,
        UiLayout {
            width: Unit::Px(40.0),     // full cross
            height: Unit::Pct(0.0),    // driven main
            ..UiLayout::default()
        },
    );
    w.run();
    approx(w.rect(fill).h, 50.0, "column-track fill height = 200 * 0.25");
    approx(w.fill_pct(fill, false).expect("fill height is a Pct"), 25.0, "fill height Pct at 0.25");

    w.set_value(track, 0.5);
    w.run_settled();
    approx(w.rect(fill).h, 100.0, "column-track fill height = 200 * 0.5");
}

#[test]
fn bar_still_frame_does_no_work() {
    // 0%-work gate: after a value change settles, several still frames must NOT
    // advance the fill's Changed<UiLayout> tick (the bar driver writes the fill's
    // UiLayout set-if-changed; an idle frame writes nothing). Allocator-free,
    // per-thread-safe — the P4 bind 0%-work probe shape.
    let mut w = p6a();
    let (_track, fill) = w.spawn_bar(
        ComputedRect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 },
        row_track(200.0, 40.0),
        0.5,
        fill_node(Unit::Px(40.0)),
    );
    w.run();
    approx(w.rect(fill).w, 100.0, "settled at 0.5");

    // Let it fully settle (layout may run an extra frame on the first dirty pass).
    w.run();
    let tick_before = w.layout_changed_tick(fill).expect("fill has a UiLayout tick");

    // Several still frames: no value change, no fill UiLayout write.
    for _ in 0..5 {
        w.run();
    }
    let tick_after = w.layout_changed_tick(fill).expect("fill has a UiLayout tick");
    assert_eq!(
        tick_before, tick_after,
        "a still frame does not bump the fill's Changed<UiLayout> tick (0%-work gate)"
    );
    // And the geometry is unchanged.
    approx(w.rect(fill).w, 100.0, "fill width unchanged across still frames");
}

#[test]
fn bar_reapplied_identical_value_does_not_churn_tick() {
    // Writing the SAME value (re-inserting UiValue, which DOES bump Changed<UiValue>
    // so the driver runs) must NOT bump the fill's UiLayout tick: quantized_pct
    // produces the identical Pct, and set_fill_pct_if_changed suppresses the write.
    let mut w = p6a();
    let (track, fill) = w.spawn_bar(
        ComputedRect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 },
        row_track(200.0, 40.0),
        0.5,
        fill_node(Unit::Px(40.0)),
    );
    w.run();
    w.run();
    let tick_before = w.layout_changed_tick(fill).expect("fill tick");

    // Re-write the identical value several times.
    for _ in 0..3 {
        w.set_value(track, 0.5);
        w.run_settled();
    }
    let tick_after = w.layout_changed_tick(fill).expect("fill tick");
    assert_eq!(
        tick_before, tick_after,
        "re-applying the identical value does not churn the fill's UiLayout tick (set-if-changed)"
    );
    approx(w.rect(fill).w, 100.0, "fill width stays at 100");
}

#[test]
fn bar_quantization_absorbs_fp_noise() {
    // Two values that differ by less than 1/10000 quantize to the SAME Pct, so the
    // second update must not bump the fill's UiLayout tick (M1 — FP-noise damping).
    let mut w = p6a();
    let (track, fill) = w.spawn_bar(
        ComputedRect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 },
        row_track(200.0, 40.0),
        0.5,
        fill_node(Unit::Px(40.0)),
    );
    w.run();
    w.run();
    let tick_before = w.layout_changed_tick(fill).expect("fill tick");

    // 0.5 + 1e-6 quantizes (10000 steps) to the same 0.5000 fraction.
    w.set_value(track, 0.5 + 1.0e-6);
    w.run_settled();
    let tick_after = w.layout_changed_tick(fill).expect("fill tick");
    assert_eq!(
        tick_before, tick_after,
        "a sub-quantum value delta does not move the Pct (M1 quantization absorbs FP noise)"
    );
    approx(w.rect(fill).w, 100.0, "fill width unchanged for sub-quantum delta");
}
