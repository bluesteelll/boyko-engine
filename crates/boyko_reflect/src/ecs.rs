//! `boyko_reflect::ecs` — the ECS glue: everything that stands between a `ComponentId`
//! and a byte (`docs/REFLECTION-PLAN-ECS.md` §0).
//!
//! Rung **EG1** lands the enumeration half: [`components_of_into`] over **source 1**
//! (the archetype's id list, filtered) and **source 2** (the dense registry), each entry
//! tagged with the [`IdKind`] that decides which of the plan's five inspector row states
//! applies, plus [`display_name`]. Source 3 (bitset presence) is EG3's and is deliberately
//! absent rather than stubbed — see the marker inside [`components_of_into`]'s body.
//!
//! Nothing here allocates. Nothing here forms a `&mut` into ECS storage. Every fn takes
//! `&EcsMaster`.
//!
//! # Source 1 is `component_ids()` **filtered**, and the filter is the rung
//!
//! `Archetype::component_ids()` is **not** the archetype signature. It is the raw id list
//! handed to whichever call first minted that archetype, and it **retains** `Bitset` and
//! `Dense` ids; only the signature *mask* is filtered. Every consumer inside `boyko_ecs`
//! re-filters through the `pub` `is_signature_storage`, and this module does the same.
//!
//! Reading it unfiltered is not a duplicate-entry bug, it is a **false positive whose
//! direction depends on spawn order**: archetype dedup keys on the filtered mask, so a
//! table-only entity spawned after a table+dense one shares that archetype and would be
//! reported as carrying the dense component while `dense_contains` says `false`. That is
//! the plan's D15 *"wrong answer that looks like an answer"*, and it is the order a fresh
//! test binary takes. The plan records the measurement as **D23**; the gate that arms it
//! is `crates/reflect_fixture/tests/eg1_components_of.rs`.
//!
//! # The route is the safe accessor (plan D2)
//!
//! `entity_archetype_id` → `archetype_master().get_archetype(id)` → `component_ids()`.
//! `Archetype`'s fields are `pub(crate)`, so the raw-projection form the analysis
//! prescribed is not expressible from this crate at all (plan F8). What sanctions the
//! whole-struct `&Archetype` read is the kernel's own precedent
//! (`ecs_master/component_api.rs`): the BUG-MIGRATE-TB-1 hazard *"DOES NOT APPLY"* where
//! no sibling structural migration and no slab dealloc can interleave with the read — and
//! here the reference is derived from `&EcsMaster` and dropped before the function
//! returns. Whether Tree Borrows agrees is a measurement, not a claim; it is
//! `crates/reflect_fixture/tests/eg1_miri_tb.rs`.

use boyko_ecs::ecs::core::component::component_registry::{
    MAX_COMPONENTS, ResidencyKind, get_layout, is_signature_storage, residency_class,
    storage_kind,
};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::{EcsMaster, Entity};

use crate::registry::type_info_of;

/// Why an access could not be served (plan §2 / D6).
///
/// `#[repr(u8)]`, `Copy`, one byte — never a bare `Option`, because `None` conflates
/// states the inspector must show apart: an id that is absent, an id whose type opted out
/// of `#[component(reflect)]`, and an id that is GPU-resident are three different rows,
/// and reporting the wrong one is a correctness failure of a tool whose entire job is to
/// be trusted about what is there.
///
/// EG1 can return exactly two of these — [`EntityDead`](Refusal::EntityDead) and
/// [`BufferTooSmall`](Refusal::BufferTooSmall). The rest are the read / write / presence /
/// structural rungs', and they land here rather than one-per-rung so that the enum a
/// caller matches on never grows a variant under it.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// Stale generation, never registered, or despawned.
    EntityDead,
    /// The entity does not carry this id.
    ComponentAbsent,
    /// No `TypeInfo`: the type opted out of `#[component(reflect)]`, or the id is a
    /// dynamic tag (which can never have one).
    NotReflectable,
    /// `ResidencyKind::Gpu` — enumerated, deliberately unreadable.
    NoHostBytes,
    /// `StorageKind::Bitset` — no per-row bytes; use the presence view instead.
    PresenceOnly,
    /// The field index is past the type's field count.
    FieldOutOfRange,
    /// By-name lookup missed.
    NoSuchField,
    /// Field kind vs. `Scalar` tag — the load-bearing RELEASE check.
    KindMismatch,
    /// [`components_of_into`]'s out slice could not hold the whole answer.
    BufferTooSmall,
    /// `TypeInfo::default_in_place` is `None` — the type has no `Default`, or it carries
    /// `#[reflect(no_default)]`. Inspectable, not synthesizable.
    NoDefault,
    /// A v1 scope edge, named per site.
    Unsupported,
}

/// How the id is stored, i.e. which of the plan's five inspector row states applies.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdKind {
    /// In the signature, host bytes, `TypeInfo` present — render fields.
    Table,
    /// In the signature, host bytes, no `TypeInfo` — render the name only. Deliberately
    /// merges "opted out of `#[component(reflect)]`" with "dynamic tag": the only
    /// discriminator is `type_id == TypeId::of::<DynamicTagMarker>()`, and that marker is
    /// private and unnameable outside `boyko_ecs`.
    TableOpaque,
    /// In the signature, `ResidencyKind::Gpu` — no host bytes to show.
    TableGpu,
    /// Not in the signature mask, host bytes, `TypeInfo` — render fields.
    Dense,
    /// Not in the signature mask, no bytes — render a boolean toggle. Produced by
    /// source 3, which is EG3's.
    Bitset,
}

/// One enumerated component: the id, and the row state that decides how to render it.
///
/// `PartialEq`/`Eq` are load-bearing rather than convenience: [`components_of_into`]
/// promises that a [`Refusal::BufferTooSmall`] leaves the caller's buffer **unmodified**,
/// and the only way to assert that is to compare entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdEntry {
    /// The component id.
    pub id: ComponentId,
    /// Which storage backend it lives in, resolved to a row state.
    pub kind: IdKind,
}

// EG1 gate 5 — the footprint is PINNED, not observed. `ComponentId` is
// `#[repr(transparent)]` over `usize` and `IdKind` is `#[repr(u8)]`, so on the engine's
// 64-bit target one entry is 8 bytes of id + 1 byte of kind rounded to the id's
// alignment. A caller sizes a stack buffer from this number (the upper bound is
// `MAX_COMPONENTS`), so a silent growth is a silent stack cost.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    size_of::<IdEntry>() == 16,
    "IdEntry's footprint moved off its measured pin (16 bytes) -- re-measure and re-pin \
     deliberately, in the same change that moved it"
);

/// The string an inspector shows for an id whose layout was never registered.
///
/// Not an empty string: a blank cell in an inspector reads as "this component has no
/// name", which is a different and wronger statement than "this process never registered
/// this id".
///
/// That paragraph is a **rule**, so it is gated rather than trusted:
/// `crates/reflect_fixture/tests/eg1_components_of.rs` pins this exact text from the
/// outside, on an id no process registers. It landed ungated at EG1 and the emptying — the
/// one thing the paragraph forbids — left every gate in the campaign green (D25).
const UNREGISTERED: &str = "<unregistered ComponentId>";

/// Enumerates every component `entity` carries, tagging each id with its [`IdKind`].
///
/// Returns the number of entries written into `out`, or a [`Refusal`].
///
/// # Complete, or it refuses (plan D15)
///
/// There is no short count. If the answer does not fit, this returns
/// [`Refusal::BufferTooSmall`] and **`out` is left byte-for-byte unmodified** — a
/// truncated component list is a wrong answer that looks like an answer, and a caller
/// that saw a partial fill could not tell the two apart. The upper bound is
/// `MAX_COMPONENTS` (512), so a caller can size one stack array at startup and never
/// meet the error; the error exists so that undersizing is loud.
///
/// The size question is answered by walking the sources **before** anything is written,
/// through the same visitor that fills the buffer — one predicate, not two, so the count
/// and the fill cannot drift apart.
///
/// # Allocations
///
/// Zero, asserted rather than claimed:
/// `crates/reflect_fixture/tests/ecs_alloc.rs` measures the call under a counting
/// allocator.
pub fn components_of_into(
    ecs: &EcsMaster,
    entity: Entity,
    out: &mut [IdEntry],
) -> Result<usize, Refusal> {
    let mut needed: usize = 0;
    visit_components(ecs, entity, |_| needed += 1)?;
    if needed > out.len() {
        return Err(Refusal::BufferTooSmall);
    }

    let mut written: usize = 0;
    visit_components(ecs, entity, |entry| {
        out[written] = entry;
        written += 1;
    })?;
    debug_assert_eq!(
        written, needed,
        "invariant: the counting pass and the filling pass walk the same sources under \
         the same &EcsMaster borrow, so they cannot disagree"
    );
    Ok(written)
}

/// The name an inspector shows for `id` — `ComponentLayout::type_name`.
///
/// For a derived component this is the Rust path; for a **dynamic tag** it is the
/// interned user name, which is the only name that citizen has.
///
/// # This is DIAGNOSTICS ONLY, and two rules ride on that
///
/// 1. **It must never reach a save file.** The persisted key is
///    `get_serialize_info(id).stable_name` (`docs/REFLECTION-PLAN-BOUNDARY.md`), which is
///    a stability contract; `type_name` is not one — it moves when a type is renamed or
///    re-homed, and a save keyed on it silently stops loading.
/// 2. **It must never be fed back into `register_enable_tag`.** A *derived*
///    `#[component(storage = "bitset")]` type never interns its name in `TAG_NAMES`, so
///    that call **mints a brand-new dynamic tag** and toggles the new tag's bit while the
///    real one stays as it was — returning `Ok(())`. The plan records this as F27; the
///    bitset write path goes through the checked `EnableTagId` constructor instead
///    (EG2 / EG3), and EG3 gate 7 asserts this crate never made the by-name call.
///
/// Returns [`UNREGISTERED`]'s text for an id this process never registered.
///
/// # The bound is checked HERE, and that is the whole reason this is not one line
///
/// `pub fn display_name(id: ComponentId)` accepts any id, and the sentence above states no
/// bound — so an id at or past `MAX_COMPONENTS` must answer, not abort. `get_layout` opens
/// with `debug_assert!(component_id < MAX_COMPONENTS, …)` and only *then* falls through to
/// its release-path `None`, so delegating the range question to it made this function's
/// contract true in release and a **panic in debug** — measured at EG1's verification on
/// `ComponentId(usize::MAX)`, which is the sentinel the rung's own test file fills buffers
/// with (D25).
///
/// # Why this DEPARTS from `type_info_of`'s discipline, one crate-module away
///
/// [`crate::type_info_of`] keeps the `debug_assert!` + release guard pair (CORE F6's shape,
/// copied from the kernel) and **states the bound in its own `///`** — *"or the id is out of
/// bounds"* — with a `#[cfg(debug_assertions)]` test for the panic and a
/// `#[cfg(not(debug_assertions))]` test for the `None`. That is a coherent contract, and
/// copying it here was the alternative. It is rejected on two concrete grounds, not on taste:
///
/// 1. **`type_info_of` returns `Option`; this returns `&'static str`.** There, `None` is the
///    value that means *"you asked wrong"*, and the `debug_assert!` sharpens it into a
///    development-time signal. Here there is no second value to sharpen — an id past the
///    registry IS an id this process never registered, which is exactly what the sentinel
///    says — so the assert converts an answerable question into an abort and buys nothing.
/// 2. **The out-of-range id is reachable from this module's OWN contract.**
///    [`components_of_into`] promises that a [`Refusal::BufferTooSmall`] leaves the caller's
///    buffer *unmodified*, i.e. still holding whatever the caller pre-filled. A caller that
///    ignored the `Err` and read a slot back gets its own filler — the rung's test file fills
///    with `ComponentId(usize::MAX)` — and an inspector that aborts on that is strictly worse
///    than one that renders `<unregistered ComponentId>`.
///
/// The remaining reason is mechanical: a rule about release behaviour is **ungatable in
/// debug**, because the assert fires before the rule can be observed, so the doc-bounding
/// branch also costs a second, cfg-split test to say what one assertion says here.
pub fn display_name(id: ComponentId) -> &'static str {
    if id.get() >= MAX_COMPONENTS {
        return UNREGISTERED;
    }
    get_layout(id.get()).map_or(UNREGISTERED, |layout| layout.type_name)
}

/// Walks every source that can name a component of `entity`, calling `visit` once per
/// component, in source order.
///
/// The single source of truth about *which* ids are enumerated and *what kind* each one
/// is. [`components_of_into`] runs it twice — once to size the answer, once to write it —
/// because the buffer must not be touched until the whole answer is known to fit.
/// Generic over the closure, so both passes monomorphise into direct calls.
fn visit_components<F: FnMut(IdEntry)>(
    ecs: &EcsMaster,
    entity: Entity,
    mut visit: F,
) -> Result<(), Refusal> {
    // ── source 1: the archetype's id list, FILTERED (D2 + D23) ─────────────────────
    //
    // The `&Archetype` is confined to this block and dies before source 2 runs.
    {
        let archetype_id = ecs.entity_archetype_id(entity).ok_or(Refusal::EntityDead)?;
        let archetype = ecs
            .archetype_master()
            .get_archetype(archetype_id)
            .ok_or(Refusal::EntityDead)?;

        for &id in archetype.component_ids() {
            // NOT a tidy-up: `component_ids()` retains Bitset and Dense ids, and an
            // unfiltered read reports them on entities that do not carry them, because
            // archetype dedup keys on the filtered mask. See this module's header.
            if !is_signature_storage(storage_kind(id.get())) {
                continue;
            }
            visit(IdEntry { id, kind: classify_signature_id(id) });
        }
    }

    // ── source 2: the dense registry (D14) ─────────────────────────────────────────
    //
    // `dense_ids()` is registration-ordered and small (one id per dense component type
    // in the process), and `dense_contains` is an O(1) membership probe — both `pub`.
    for &id in ecs.dense_registry().dense_ids() {
        if ecs.dense_contains(entity, id) {
            visit(IdEntry { id, kind: IdKind::Dense });
        }
    }

    // ── source 3: bitset presence — EG3 ────────────────────────────────────────────
    //
    // Deliberately ABSENT, not stubbed. A stub that returns "no bitset components" is
    // indistinguishable at the call site from an entity that carries none, so it would
    // ship the wrong answer under a name that claims to be the right one. EG3 lands the
    // `0..MAX_COMPONENTS` scan on `storage_kind` (plan D3) here.

    Ok(())
}

/// Resolves the row state of a **signature-storage** id.
///
/// # The order is the decision (plan D7, pinned by D23)
///
/// `residency_class` is consulted **before** `type_info_of`, so a `Gpu` id classifies
/// [`IdKind::TableGpu`] whether or not it carries `#[component(reflect)]`. The GPU answer
/// is a *classification* test, not a `TypeInfo` test and not a null-column test: a null
/// column also means *absent*, so a null test would conflate two rows and produce exactly
/// the "lists a component and then shows nothing" confusion the refusal exists to
/// prevent. It is also decidable with no device, which is why the gate runs in CI.
fn classify_signature_id(id: ComponentId) -> IdKind {
    if residency_class(id.get()) == ResidencyKind::Gpu {
        return IdKind::TableGpu;
    }
    if type_info_of(id.get()).is_some() {
        IdKind::Table
    } else {
        IdKind::TableOpaque
    }
}
