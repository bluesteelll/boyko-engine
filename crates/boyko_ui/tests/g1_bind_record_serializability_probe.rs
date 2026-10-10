//! **Gaia rung G1's probe: what Serializability class do the bind-record
//! components actually carry?**
//!
//! G1's gate ([`docs/gaia/CAMPAIGN.md`](../../../docs/gaia/CAMPAIGN.md) §Rung ladder)
//! ends with *"plus a probe pinning `BindText`'s actual Serializability class"*.
//! That is a MEASUREMENT, not a design choice, so it lands identically under either
//! answer to ballot **F1** (the bake route: the rejected EG2 reflection seam vs
//! macro-time GK-4). It is landed here, ahead of everything else in G1, because the
//! rest of G1 is blocked on F1 and this is not.
//!
//! # Why the answer matters
//!
//! [`docs/gaia/DECISIONS.md`](../../../docs/gaia/DECISIONS.md) §UI bindings item 1
//! rules that *"a binding bakes into a **POD bind-record component**"*, and G1's own
//! gate is `bake_one(…) == save_world(hand-built world)` **byte-for-byte**. Both
//! sentences quietly assume the bind records take the blit path. If they do not, the
//! baker cannot emit their bytes directly — it has to drive the same per-element
//! `serialize_fn` the saver drives, and "byte-for-byte" is a claim about an encoded
//! run rather than a memcpy of a struct.
//!
//! These tests are **not** `#[ignore]`d and assert nothing aspirational. They pin
//! today's answer so a later change to a bind record's field set — adding a `bool`,
//! widening `TemplateId`, dropping the `Entity` — cannot move the class under G1
//! without a test naming the move.
//!
//! # The measured answer (2026-08-30, this checkout)
//!
//! Printed by the run itself; the assertions below are the pins. The short form:
//! **`BindText` is NOT `PlainOldBytes`.** It cannot be, and the reason is structural
//! rather than incidental — the `Serializability::PlainOldBytes` doc requires every
//! field to be transitively in `{integers, floats, raw pointers}` with **"NO `bool`,
//! `char`, enum, niche type, or `Entity`"**, and `BindText` carries **two**
//! disqualifying fields at once:
//!
//! * `source: Entity` — and it is not merely a niche problem. An `Entity` in a baked
//!   file is the one field that MUST be remapped at load
//!   (`LoadEntityMap` / the loud `UnmappedEntity`), which a blit cannot do. Gaia's
//!   own grammar rule `link` exists for exactly this field.
//! * `template: TemplateId` — a `#[repr(u8)]` enum, whose bytes are not all-bits-valid
//!   on an untrusted load (the C3 argument).
//!
//! `BindValue` carries the `Entity` but no enum; `UiTextBuffer` (the SINK, not a bind
//! record) is all-integer and is the useful contrast.
//!
//! # What this hands the F1 decision
//!
//! Whichever bake route wins, the baker must run the entity-remap-bearing encode path
//! for `BindText`, not a struct memcpy. That is a fact about the type, so it survives
//! F1 either way — and it is the first concrete instance of the general shape: **a
//! bind record binds a source ENTITY, so the "POD bind-record" phrasing in the
//! ratified decision line is loose exactly where the format is hardest.** Recorded
//! here rather than editing that line, because narrowing a ratified ruling is the
//! owner's, not this rung's.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    Serializability, get_serialize_info,
};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;

use boyko_ui::binding::components::{BindText, BindValue, UiTextBuffer};

/// Forces every id to be minted and its `SERIALIZE` row installed, then reports the
/// registry's classification for `id`.
fn class_of(id: boyko_ecs::ecs::identifiers::primitives::ComponentId) -> Serializability {
    get_serialize_info(id.0)
        .map(|i| i.serializability)
        .expect("every registered component owns a SERIALIZE row")
}

/// Registers the three types by touching their ids inside a live world, which is
/// what installs the registry rows the probe reads.
fn registered_world() -> EcsMaster {
    let world = EcsMaster::new();
    let _ = BindText::component_id();
    let _ = BindValue::component_id();
    let _ = UiTextBuffer::component_id();
    world
}

/// The probe. Prints the class of each bind-record type and its sink, so
/// `cargo test -p boyko-ui --test g1_bind_record_serializability_probe -- --nocapture`
/// answers G1's question directly instead of through a source read.
#[test]
fn report_bind_record_serializability_classes() {
    let _world = registered_world();
    for (name, id) in [
        ("BindText", BindText::component_id()),
        ("BindValue", BindValue::component_id()),
        ("UiTextBuffer", UiTextBuffer::component_id()),
    ] {
        let info = get_serialize_info(id.0).expect("SERIALIZE row");
        println!(
            "{name:<13} class={:?}  serialize_fn={}  deserialize_fn={}  map_entities_fn={}  \
             stable_name={}",
            info.serializability,
            info.serialize_fn.is_some(),
            info.deserialize_fn.is_some(),
            info.map_entities_fn.is_some(),
            info.stable_name,
        );
    }
}

/// **G1's actual question, pinned.** `BindText` is not blittable.
///
/// If this ever reds because the class became `PlainOldBytes`, something removed the
/// `Entity` and the enum — in which case G1's byte-for-byte gate got simpler and the
/// "POD bind-record" wording became literally true. Either way the change must be
/// deliberate, which is what a pin is for.
#[test]
fn bindtext_is_not_plain_old_bytes() {
    let _world = registered_world();
    let class = class_of(BindText::component_id());
    assert_ne!(
        class, Serializability::PlainOldBytes,
        "BindText classified {class:?}. It carries `source: Entity` (which a baked \
         file MUST remap at load — a blit cannot) and `template: TemplateId`, a \
         #[repr(u8)] enum whose bytes are not all-bits-valid on an untrusted load. \
         G1's `bake_one(..) == save_world(..)` byte-for-byte gate therefore compares \
         an ENCODED RUN for this type, not a struct memcpy — and DECISIONS §UI \
         bindings item 1's phrase 'a POD bind-record component' is loose here"
    );
}

/// `BindValue` measured beside it — same `Entity`, no enum. Pinned separately so a
/// future change that makes one blittable and not the other is visible.
#[test]
fn bindvalue_class_is_pinned() {
    let _world = registered_world();
    let class = class_of(BindValue::component_id());
    assert_ne!(
        class, Serializability::PlainOldBytes,
        "BindValue classified {class:?}; it carries `source: Entity`, which is \
         disqualifying for the blit path on its own — the enum is not the only reason \
         BindText is not POB"
    );
}

/// The contrast case, and the reason the two assertions above are not vacuous: the
/// SINK is all-integer, so if the derive were classifying everything conservatively
/// this would be `Ignore`/`SerializeViaFn` too and the probe would prove nothing.
#[test]
fn the_sink_is_the_control_and_is_blittable() {
    let _world = registered_world();
    let class = class_of(UiTextBuffer::component_id());
    assert_eq!(
        class,
        Serializability::PlainOldBytes,
        "control: UiTextBuffer is `[u8; 247] + u8 + [u8; 8]` — all integers, no \
         Entity, no enum — so it takes the blit path. If this reds, the derive is not \
         classifying by field type and the two assertions above measure nothing"
    );
}
