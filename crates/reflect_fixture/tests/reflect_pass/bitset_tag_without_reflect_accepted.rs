//! CORE C9 / D37 — **C8 gate 4's clauses, migrated.** The refusal took C8's subject with
//! it, and these are the halves that survive on shapes that still compile.
//!
//! `tests/c8_bitset_suppression.rs` gated D29's *silent* suppression: a
//! `#[component(reflect, storage = "bitset")]` tag compiled and published nothing. C9 makes
//! that combination a spanned `compile_error!`
//! (`reflect_compile_fail/bitset_storage_rejected.rs`), so the file's subject can no longer
//! be declared and its two tests went with it. That file predicted this itself — *"If this
//! file ever stops compiling, the message arrived early and C9's row is the place to record
//! it."*
//!
//! # What is asserted here, and why each half is not decoration
//!
//! * **The positive control** — a reflected table component installs a descriptor. C8's
//!   own header explains why it exists: the tag clause below is an ABSENCE, and an absence
//!   is green in a world where the install seam does not exist for anything.
//! * **The tag installs nothing** — still true, now for a structural reason rather than a
//!   suppressed one: with no `reflect` key there is no emission to suppress.
//! * **The tag is still a bitset enable tag** — C8's second test, and the one that pins
//!   *"suppression did not break componenthood"*. D29's term lived in one binding beside
//!   six neighbours that carry the same condition, so a term landing in a NEIGHBOURING
//!   binding was the natural mis-edit; that defect leaves the `type_info_of` clause green
//!   and shows up only here, as an enable tag silently promoted to signature storage.
//!
//! A `t.pass()` fixture is **run** (trybuild's `check_pass` executes the binary), so these
//! are assertions rather than a compile.

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::core::component::component_registry::{StorageKind, storage_kind};
use boyko_macros::Component;
use boyko_reflect::type_info_of;

/// A bitset enable tag — legitimate on its own. `reflect` is the token C9 refuses beside
/// it, not this key.
#[derive(Component, Default)]
#[component(storage = "bitset")]
pub struct PlainBitsetTag;

/// The positive control: the reflect opt-in WITHOUT the `storage` key.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(C)]
pub struct InstalledSubject {
    /// One POD field, so the control has a real (non-ZST) descriptor.
    pub value: u32,
}

fn main() {
    let control_id = <InstalledSubject as ComponentTrait>::component_id().0;
    assert!(
        type_info_of(control_id).is_some(),
        "INSTRUMENT DEAD: the positive control carries `#[component(reflect)]` and its id \
         ({control_id}) has no descriptor installed, so the tag clause below would read \
         `None` in a world where the install seam does not exist at all"
    );

    let tag_id = <PlainBitsetTag as ComponentTrait>::component_id().0;
    assert!(
        type_info_of(tag_id).is_none(),
        "a `storage = \"bitset\"` enable tag (id {tag_id}) has a TypeInfo installed, and it \
         never asked for one"
    );

    assert_eq!(
        storage_kind(tag_id),
        StorageKind::Bitset,
        "`PlainBitsetTag` (id {tag_id}) carries `storage = \"bitset\"` and does not read \
         back as one. The C9 edit that removed D29's `!hooks.storage_bitset` term has taken \
         a NEIGHBOURING binding's term with it: the funnel that stopped emitting the \
         seventh slot must still emit the first six"
    );
    assert_eq!(
        storage_kind(control_id),
        StorageKind::Table,
        "the positive control is supposed to differ from the tag in the `storage` key and \
         in nothing else, so if it is not table-storage the pair is no longer a controlled \
         comparison"
    );
}
