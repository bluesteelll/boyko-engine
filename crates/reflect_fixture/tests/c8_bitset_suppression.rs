//! **CORE C8 gate 4 (D29) — `storage = "bitset"` suppresses the reflect emission, and
//! the suppression is SILENT at this rung.**
//!
//! # What is being suppressed, and why here rather than at C9
//!
//! A `storage = "bitset"` enable tag has **no `ComponentPool` and no per-row bytes**. Up
//! to C7 the reflect emission was inert — a `static` nothing referenced — so a descriptor
//! baked for a bitset tag described nothing and reached nothing, and the combination
//! merely *compiled*: measured at the C8 audit, `#[component(reflect, storage =
//! "bitset")]` produced `size=0 align=1 fields=0 kind=Struct` and no gate moved.
//!
//! C8 is the rung that makes the emission **do** something: it publishes into a
//! `ComponentId`-keyed table that an editor reads by id. A descriptor registered for an id
//! with no bytes behind it is the "coherent lie" shape — an inspector would offer a
//! zero-field view of a tag whose state is a bit in a bitset, and nothing would red. So
//! the suppression lands with the seam, not after it (D29).
//!
//! The reflect emission was the **one** binding in its neighbourhood of `component.rs`
//! with no `storage_bitset` term; its six neighbours — `entities_items`,
//! `serialize_items`, `clone_install`, `relationship_install`, `serialize_install` and
//! `bundle_items` — all carry one. This gate is the term's witness.
//!
//! *(Cited by BINDING NAME rather than by line. The C8 audit's own anchors for these six
//! had already drifted by ~15 lines, and landing this rung moved them again — the four
//! `REFLECTION-PLAN-*.md` documents are not in `internal_docs_anchors.rs`'s `GATED_DOCS`,
//! so nothing reds when they rot. A binding name is checkable by grep and does not move.)*
//!
//! # Why the positive control is not decoration
//!
//! C8's gate 4 as first written asserted one thing about one subject: the bitset tag's
//! `type_info_of` is `None`. That clause is **green today**, green before this rung, and
//! green if the entire C8 install seam were deleted — because nothing is installed for
//! *anything* in that world. A gate whose subject is an absence needs a presence beside
//! it, or it is this campaign's "gate that cannot fail" again. [`C8InstalledSubject`] is
//! that presence: it differs from the tag in the `storage` key and in nothing else that
//! matters, and the gate reads both in one pass.
//!
//! # The invocation is part of the gate
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect --test c8_bitset_suppression
//! ```
//!
//! Feature-off this file compiles to nothing and exits 0 — a vacuous pass on the green
//! side and on every red side. The output must read `running [1-9]`.
#![cfg(feature = "reflect")]

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::core::component::component_registry::{StorageKind, storage_kind};
use boyko_macros::Component;
use boyko_reflect::type_info_of;

/// The subject: a bitset enable tag that ALSO asks for reflection.
///
/// Fieldless because `storage = "bitset"` requires it (`component.rs`'s
/// `reject_non_zst_bitset_tag` guard), which is also why the pre-suppression descriptor
/// was so convincingly empty — a zero-field
/// `Struct` is exactly what a correct walk of a unit struct produces, so nothing about the
/// descriptor's *contents* could ever have flagged it. The defect was never in the bake;
/// it was in there being a row at all.
#[derive(Component, Default)]
#[component(reflect, storage = "bitset")]
pub struct C8BitsetTag;

/// The positive control: the same opt-in **without** the `storage` key.
///
/// Its only job is to prove the instrument can see an install at all, so that
/// [`C8BitsetTag`]'s `None` is a statement about the suppression rather than about a seam
/// that is missing for everyone.
#[derive(Component, Default)]
#[repr(C)]
#[component(reflect)]
pub struct C8InstalledSubject {
    /// One POD field, so the control has a real (non-ZST) descriptor.
    pub value: u32,
}

/// **The gate.** The tag registers no descriptor; the control does — read in one pass, so
/// neither half can be satisfied by a world in which the seam does not exist.
#[test]
fn a_bitset_tag_installs_no_descriptor_while_its_unkeyed_twin_does() {
    let control_id = <C8InstalledSubject as ComponentTrait>::component_id().0;
    assert!(
        type_info_of(control_id).is_some(),
        "INSTRUMENT DEAD: the positive control `C8InstalledSubject` carries \
         `#[component(reflect)]` and its id ({control_id}) has no descriptor installed, so \
         the bitset clause below would read `None` in a world where the C8 install seam \
         does not exist at all -- which is the whole failure mode this control is here to \
         exclude"
    );

    let tag_id = <C8BitsetTag as ComponentTrait>::component_id().0;
    assert!(
        type_info_of(tag_id).is_none(),
        "a `storage = \"bitset\"` enable tag (id {tag_id}) has a TypeInfo installed. A \
         bitset tag has no ComponentPool and no per-row bytes, so an editor reading this \
         id by id would be handed a zero-field descriptor for something whose state is a \
         bit in a bitset -- a coherent lie, in the exact shape C7's follow-up caught in \
         the non-struct arm. The emission's condition must carry the `!hooks.storage_bitset` \
         term its six neighbours in `component.rs` already carry."
    );
}

/// The suppression is **silent at C8**, and it suppresses the reflect emission and
/// **nothing else**: the tag is still installed, through the same funnel, as a bitset
/// enable tag.
///
/// This is the half that pins the rung boundary. C9 owns the spanned `compile_error!` (its
/// refusal-matrix table row) and, with it, ECS D5's release `assert!` inside
/// `install_type_info`. If this file ever stops compiling, the message arrived early and
/// C9's row is the place to record it — not a silent re-interpretation of C8.
///
/// # This clause replaces one that could not fail
///
/// As landed, the test asserted `assert_ne!(tag_id, control_id)`. `register_new::<Self>()`
/// mints one id per type, so that is true of any two distinct types in any world — it
/// stayed green under RED 4 (drop the `!hooks.storage_bitset` term) and under a deletion of
/// the whole install seam, i.e. under **every** red C8 defines. Gate 4's `running 2` was one
/// real clause and one compile.
///
/// The claim worth making is the one the doc line above already made in prose and no
/// assertion carried: **suppression did not break componenthood.** D29's term went into the
/// one binding in `component.rs` that lacked it, beside six neighbours that have it — and a
/// term landing in a *neighbouring* binding is the natural mis-edit. That defect leaves the
/// clause above green (nothing is installed for the tag either way) and is visible only
/// here: it would leave `C8BitsetTag` classified `Table`, an enable tag silently promoted to
/// signature storage.
///
/// The distinct-id property the old assertion named is *also* carried, and non-tautologically:
/// two different `StorageKind`s cannot be read out of one slot.
#[test]
fn the_suppressed_tag_is_still_installed_as_a_bitset_enable_tag() {
    let tag_id = <C8BitsetTag as ComponentTrait>::component_id().0;
    let control_id = <C8InstalledSubject as ComponentTrait>::component_id().0;

    assert_eq!(
        storage_kind(tag_id),
        StorageKind::Bitset,
        "`C8BitsetTag` (id {tag_id}) carries `storage = \"bitset\"` and reads back as {:?}. \
         The reflect suppression has taken the storage install with it: the funnel that \
         stopped emitting the seventh slot must still emit the first six, or C8 has not \
         suppressed an emission -- it has broken the component. C9 owns REFUSING this \
         combination; C8 owns making it inert, which is not the same as making it wrong.",
        storage_kind(tag_id)
    );
    assert_eq!(
        storage_kind(control_id),
        StorageKind::Table,
        "the positive control `C8InstalledSubject` (id {control_id}) reads back as {:?}, not \
         `Table` -- it is supposed to differ from the tag in the `storage` key and in \
         nothing else, so if it is not table-storage the pair is no longer a controlled \
         comparison",
        storage_kind(control_id)
    );
}
