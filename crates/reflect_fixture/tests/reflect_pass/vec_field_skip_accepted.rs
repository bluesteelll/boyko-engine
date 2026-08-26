//! CORE C9 / D35 — `#[reflect(skip)]` is the way OUT of the `Opaque`-field row, and it
//! bakes D14's descriptor exactly.
//!
//! This file is `reflect_compile_fail/vec_field_rejected.rs` plus one attribute. A refusal
//! defined in terms of an escape hatch that does not exist is a refusal with no way out,
//! and until this rung the hatch did not exist: five sites scheduled `#[reflect(skip)]` at
//! C9 (one of them landed code in `boyko_macros/src/reflect.rs`, one of them BLOCKS
//! `REFLECTION-PLAN-BOUNDARY.md`'s rung B2) and none of them was a **Lands** list.
//! MEASURED before this rung: the attribute parsed as inert — `parse_reflect_no_default`
//! scans type-level attributes exclusively, so a field attribute was never read at all, and
//! a `Vec<u32>` field carrying the skip baked the same `Opaque` descriptor as one without.
//!
//! # `skip` does not shorten the walk, and that is the whole D14 contract
//!
//! `fields.len()` stays at the DECLARED count, the field keeps its index, its name and its
//! `offset_of!`, and all four accessor slots are `None`. By-index access must not depend on
//! which fields were skipped. So the assertions below are the same ones
//! `tests/c7_derive_bake.rs`'s index-faithfulness gate makes about an unclassifiable field
//! — which is exactly why that gate's subject could be migrated by adding this attribute
//! and nothing else.
//!
//! A `t.pass()` fixture is **run**, not merely compiled (trybuild's `check_pass` executes
//! the binary and requires success), so `main` below is the assertion.

use boyko_macros::Component;
use boyko_reflect::{Reflect, ValueKind};

/// The subject: `reflect_compile_fail/vec_field_rejected.rs` plus `#[reflect(skip)]`.
#[derive(Component, Default)]
#[component(reflect)]
pub struct HasSkippedVecField {
    /// Describable — index 0.
    pub tag: u32,
    /// Not describable, and explicitly opted out — index 1.
    #[reflect(skip)]
    pub items: Vec<u32>,
    /// Describable — index 2, and the field whose NAME an index-shifting walk would move.
    pub other: u32,
}

fn main() {
    let ti = <HasSkippedVecField as Reflect>::TYPE_INFO;

    assert_eq!(
        ti.fields.len(),
        3,
        "`#[reflect(skip)]` OMITTED a field: `fields.len()` must equal the DECLARED count \
         unconditionally (D14), or by-index access depends on which fields were skipped"
    );

    assert_eq!(ti.fields[0].name, "tag");
    assert_eq!(ti.fields[1].name, "items");
    assert_eq!(
        ti.fields[2].name, "other",
        "field #2 is `other` -- a walk that dropped the skipped field would put `other` at \
         index 1 with its name and offset still RIGHT, which is the whole defect"
    );

    assert_eq!(
        ti.fields[1].kind,
        ValueKind::Opaque,
        "a skipped field bakes `Opaque` -- the same descriptor the refusal's subject would \
         have baked, minus the refusal"
    );
    assert!(
        ti.fields[1].get.is_none()
            && ti.fields[1].set.is_none()
            && ti.fields[1].nested.is_none()
            && ti.fields[1].array.is_none(),
        "an `Opaque` field has NO accessor to call, therefore no code path"
    );
    assert_eq!(
        ti.fields[1].offset,
        core::mem::offset_of!(HasSkippedVecField, items),
        "a skipped field keeps its real offset: `skip` means *these bytes are not \
         describable*, never *pretend this field is not there*"
    );
}
