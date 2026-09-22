// GATES G0: `reflect_off_twin_plus` — G7a's POSITIVE CONTROL P (GATES D10): the twin's
// source plus exactly one trivial `#[inline(never)]` fn. Built feature-off, its symbol
// multiset must DIFFER from `reflect_off_twin`'s — if it does not, the extraction is
// broken and every equality the instrument reports is an equality between two empty
// sets. (The determinism null's expected value is exactly zero, so it certifies
// determinism and nothing else; only this control proves the instrument can SEE.)
//
// This file is a copy of src/bin/reflect_on.rs (the twin's source) plus the marker —
// necessarily a separate file, since "the twin plus one fn" cannot share the twin's
// path. G7a's harness is the drift gate: it measures the multiset delta and must see
// exactly the marker.

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_macros::Component;

/// Copy of `reflect_on`'s `FixturePod` (see the header for why this file is a copy).
///
/// **Tracked at CORE C7** (2026-08-21): `reflect_on.rs` gained `#[component(reflect)]` +
/// `#[derive(Default)]` there, and this file's whole contract is *"the twin's source plus
/// exactly one fn"*. Leaving it un-tracked would make G7a's positive control differ from
/// its baseline by the marker **plus a `Default` impl** — a delta that is still non-zero,
/// so the control would keep reporting a pass while no longer measuring what its own
/// header says it measures. The `reflect` emission is `#[cfg(feature = "reflect")]` and
/// this bin is built feature-OFF for G7a, so the key contributes nothing to the image;
/// the point is that the two SOURCES stay one edit apart.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(C)]
pub struct FixturePod {
    /// One POD field so the derive has a real (non-ZST) subject.
    pub value: f32,
}

fn main() {
    let pod = FixturePod { value: 1.0 };
    core::hint::black_box(pod.value);

    // Referenced from main so no lint fires and no linker config can strip it silently.
    core::hint::black_box(positive_control_marker());

    // CORE C8 / D27 — the funnel touch, mirrored from `reflect_on.rs`. This file's whole
    // contract is "the twin's source plus exactly one fn", so a statement added there and
    // not here would make G7a's positive control differ from its baseline by the marker
    // PLUS a funnel call — a delta that is still non-zero, so the control would keep
    // reporting a pass while no longer measuring what its header says it measures.
    core::hint::black_box(<FixturePod as ComponentTrait>::component_id());

    // GATES G3 gate 5, mirrored from `reflect_on.rs` so the twin comparison G7a runs
    // stays "the marker and nothing else": the print exists on both sides of the pair.
    println!(
        "bin={} reflect_feature={} linkage={}",
        env!("CARGO_BIN_NAME"),
        if cfg!(feature = "reflect") { "on" } else { "off" },
        if cfg!(feature = "reflect") { "present" } else { "absent" },
    );
}

/// The one deliberate difference from `reflect_off_twin` (GATES D10's positive control).
#[inline(never)]
fn positive_control_marker() -> u64 {
    0x5eed
}
