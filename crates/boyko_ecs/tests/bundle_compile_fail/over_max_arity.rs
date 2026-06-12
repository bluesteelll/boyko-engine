// Phase 22 D7 (review M1) — `#[derive(Bundle)]` on a struct with MORE than
// `MAX_BUNDLE_ARITY` (16) fields must be rejected at expansion time with the
// dedicated arity diagnostic (`boyko_macros/src/lib.rs`). The macro
// early-returns the error before emitting any impl, so the field types are
// never trait-checked — plain `u32` fields keep the snapshot pinned to the
// single load-bearing message (mirrors the `unit_struct.rs` early-return
// precedent: no trailing E0277 noise).

use boyko_macros::Bundle;

#[derive(Bundle)]
struct SeventeenFields {
    f01: u32,
    f02: u32,
    f03: u32,
    f04: u32,
    f05: u32,
    f06: u32,
    f07: u32,
    f08: u32,
    f09: u32,
    f10: u32,
    f11: u32,
    f12: u32,
    f13: u32,
    f14: u32,
    f15: u32,
    f16: u32,
    f17: u32,
}

fn main() {}
