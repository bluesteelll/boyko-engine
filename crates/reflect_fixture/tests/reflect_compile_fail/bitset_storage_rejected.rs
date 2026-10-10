//! CORE C9 — `storage = "bitset"` together with `reflect`, refused at the `reflect` KEY.
//!
//! A bitset enable tag has no `ComponentPool` and no per-row bytes: the bit IS the datum,
//! so *"read the field at offset N"* describes nothing and a descriptor filed under the
//! tag's id would be a coherent lie an inspector cannot detect.
//!
//! D37 chose the caret. `storage = "bitset"` is legitimate on its own — the accepting
//! twin `reflect_pass/bitset_tag_without_reflect_accepted.rs` is exactly this type minus
//! one key — so `reflect` is the token that is wrong.

use boyko_macros::Component;

/// The subject, migrated from `tests/c8_bitset_suppression.rs`'s `C8BitsetTag`, which
/// predicted this: *"If this file ever stops compiling, the message arrived early and
/// C9's row is the place to record it."*
#[derive(Component, Default)]
#[component(reflect, storage = "bitset")]
pub struct BitsetTag;

fn main() {}
