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

use boyko_macros::Component;

/// Copy of `reflect_on`'s `FixturePod` (see the header for why this file is a copy).
#[derive(Component)]
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

    #[cfg(feature = "reflect")]
    reflect_linkage();

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

/// Same linkage as `reflect_on` (this file is its copy plus the marker).
#[cfg(feature = "reflect")]
#[inline(never)]
fn reflect_linkage() {
    let install: fn(usize, &'static boyko_reflect::TypeInfo) = boyko_reflect::install_type_info;
    core::hint::black_box(install);
}
