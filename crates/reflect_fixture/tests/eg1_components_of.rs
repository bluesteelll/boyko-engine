//! **ECS EG1 — `components_of_into`: table + dense, kind-tagged.**
//!
//! Gates 1, 1b, 2, 2b, 4 and 5 of `docs/REFLECTION-PLAN-ECS.md`'s EG1, plus the three
//! numbers that rung owes. Gate 3 (zero allocations) is a **separate binary**
//! (`ecs_alloc.rs`) because a `#[global_allocator]` is one per binary and that one is
//! `#![cfg(not(miri))]`; gate 6 (Miri Tree-Borrows) is a **third** binary
//! (`eg1_miri_tb.rs`) for the same reason in the other direction.
//!
//! # Why this lives in `reflect_fixture` and not in `boyko_reflect`
//!
//! Stated once by the plan's §8 so no rung re-decides it: every test that constructs a
//! `#[component(reflect)]` component lives here. `boyko_reflect` carries no
//! `boyko-macros` edge and declares no `reflect` feature "now or ever", so the derive's
//! consumer-side `#[cfg(feature = "reflect")]` could not even be evaluated there.
//!
//! ⚠️ **The engine's own `Transform` / `GpuTransform3D` / `EmitterActive` are NOT named
//! here.** The shapes below are local stand-ins with the same storage kinds; the real
//! types appear in exactly one rung, EG8, in `crates/reflect_dogfood/`. Naming an engine
//! type in this package would be a gate that cannot be built in the package that runs it
//! — this package must stay FFI-free because it is the one the CI Miri row names.
//!
//! # The invocation is part of the gate
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect --test eg1_components_of
//! ```
//!
//! The output must read `running [1-9]`; a plain `cargo test -p reflect-fixture` compiles
//! this file to nothing and exits 0.
#![cfg(feature = "reflect")]

use std::time::Instant;

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::core::component::component_registry::{
    MAX_COMPONENTS, ResidencyKind, StorageKind, classify_component_residency, get_layout,
    storage_kind,
};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::Component;
use boyko_reflect::ecs::{IdEntry, IdKind, Refusal, components_of_into, display_name};

// ═════════════════════════════════════════ the fixture shapes ═══════════════════════════

/// The `Table` citizen: signature storage, host bytes, `#[component(reflect)]`.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(C)]
struct Eg1Table {
    /// Offset 0.
    x: f32,
    /// Offset 4.
    y: f32,
}

/// The `TableOpaque` citizen: signature storage, host bytes, **no** `#[component(reflect)]`
/// — so `type_info_of` returns `None` and an inspector can show its name and nothing else.
#[derive(Component, Default)]
#[repr(C)]
struct Eg1Opaque {
    /// Offset 0.
    v: u32,
}

/// The `Dense` citizen — the `GpuTransform3D` **shape**, not the type: three `[f32; 4]`
/// lanes twice over, `#[repr(C)]`, 96 bytes. Dense storage is excluded from every
/// archetype signature *mask* while its id is RETAINED in `component_ids()`, which is the
/// whole reason source 1 needs a filter.
#[derive(Component, Default)]
#[component(reflect, storage = "dense")]
#[repr(C)]
struct Eg1Dense {
    /// Byte 0.
    prev_pos: [f32; 4],
    /// Byte 16.
    prev_rot: [f32; 4],
    /// Byte 32.
    prev_scale: [f32; 4],
    /// Byte 48.
    curr_pos: [f32; 4],
    /// Byte 64.
    curr_rot: [f32; 4],
    /// Byte 80.
    curr_scale: [f32; 4],
}

/// A `ResidencyKind::Gpu` citizen that DOES carry `#[component(reflect)]`.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(C)]
struct Eg1GpuAnnotated {
    /// Offset 0.
    lane: [f32; 4],
}

/// A `ResidencyKind::Gpu` citizen that does NOT carry `#[component(reflect)]`.
///
/// Its existence is gate 2b's whole point: §3's matrix has no `Table + Gpu` row for the
/// un-annotated case, so without this shape the classification order would be a property
/// of which fixture the implementer happened to write.
#[derive(Component, Default)]
#[repr(C)]
struct Eg1GpuPlain {
    /// Offset 0.
    lane: [f32; 4],
}

// ═════════════════════════════════════════ helpers ══════════════════════════════════════

/// The sentinel a buffer is pre-filled with, so "unmodified" is checkable byte for byte.
/// `usize::MAX` is not a mintable `ComponentId` (`MAX_COMPONENTS` is 512), and `Bitset` is
/// the one kind EG1 cannot produce, so neither half can be written by accident.
const SENTINEL: IdEntry = IdEntry { id: ComponentId(usize::MAX), kind: IdKind::Bitset };

/// Ids of the local shapes, minted on first touch.
fn id_of<C: ComponentTrait>() -> ComponentId {
    <C as ComponentTrait>::component_id()
}

/// Classifies both GPU shapes `Gpu`, idempotently.
///
/// Residency is a **process-global**, write-once classification and this binary's tests
/// run in parallel, so every test that spawns a GPU shape calls this first: a same-class
/// re-classification is an explicit no-op in `set_residency_class`, and doing it here
/// rather than in a `static` initialiser keeps the call visible at the site that depends
/// on it.
fn classify_the_gpu_shapes() {
    classify_component_residency(id_of::<Eg1GpuAnnotated>().get(), ResidencyKind::Gpu);
    classify_component_residency(id_of::<Eg1GpuPlain>().get(), ResidencyKind::Gpu);
}

/// Spawns an entity carrying the table shape **and** the dense shape.
///
/// ⚠️ **The spawn ORDER is load-bearing and this is the polluting one.** The archetype
/// minted here keeps `component_ids() == [table, dense]` while its signature mask is
/// `{table}` only, so any later table-only entity dedups into it. Gate 1b spawns exactly
/// that entity afterwards.
fn spawn_table_and_dense(ecs: &mut EcsMaster) -> Entity {
    let archetype = ecs.get_or_create_archetype(&[id_of::<Eg1Table>(), id_of::<Eg1Dense>()]);
    ecs.spawn_two(archetype, Eg1Table::default(), Eg1Dense::default())
        .expect("invariant: a fresh archetype accepts its own two-component push")
}

/// A byte view of one fully-initialised stack local, in the shape `create_entity` takes.
///
/// The construct `spawn_one` / `spawn_two` use internally; it is spelled out here only
/// because the kernel provides no three-arity typed spawn and the by-id data attach that
/// would extend an existing entity **does not exist yet** — it is EG2's `S1`, the first
/// item of the owner-gated seam.
fn bytes_of<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live, fully-initialised `T` on the caller's stack, so
    //   `size_of::<T>()` bytes starting at its address are readable and initialised. The
    //   slice is `T`-lifetime-bound, and every caller below consumes it inside the same
    //   `create_entity` call, before the local can be moved or dropped. `T` is read as
    //   opaque bytes and never reinterpreted.
    unsafe { core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns an entity carrying the table shape, the opaque shape **and** the dense shape —
/// three components spanning BOTH enumeration sources.
///
/// ⚠️ **`add_tag` is not the route to a three-component subject, and that is a MEASURED
/// kernel finding rather than a preference.** `EcsMaster::add_tag` on an entity that
/// carries a **dense** component panics in RELEASE, and so does `remove_tag`: both
/// `migrate_entity_attach_ids` and `migrate_entity_detach_ids` walk the source archetype's
/// **retained** `component_ids()` and ask for a per-archetype pool for every id in it —
/// but a dense id is retained in that list and structurally has no pool, so the walk hits
/// `.expect("invariant: source hosts its own component id")`. Measured in this worktree
/// with no reflection code on the stack. It is `boyko_ecs`'s to fix, not this campaign's;
/// EG1 routes around it, and EG2/EG6 build `add_component_by_id` / `remove_component_by_id`
/// on exactly those two helpers, so the finding is theirs to carry.
///
/// The id list is passed **canonically sorted** — the detach path debug-asserts that
/// property of `component_ids()`, while `get_or_create_archetype` stores the caller's order
/// verbatim.
fn spawn_table_opaque_and_dense(ecs: &mut EcsMaster) -> Entity {
    let (table, opaque, dense) = (id_of::<Eg1Table>(), id_of::<Eg1Opaque>(), id_of::<Eg1Dense>());
    let mut ids = [table, opaque, dense];
    ids.sort_unstable();
    let archetype = ecs.get_or_create_archetype(&ids);

    // No `mem::forget` after the copy, deliberately: all three shapes are plain
    // `f32`/`u32` aggregates, so `needs_drop` is false and their `ComponentLayout::drop_fn`
    // is `None`. There is no ownership to transfer and nothing for a second drop to free.
    let (t, o, d) = (Eg1Table::default(), Eg1Opaque::default(), Eg1Dense::default());
    ecs.create_entity(
        archetype,
        &[(table, bytes_of(&t)), (opaque, bytes_of(&o)), (dense, bytes_of(&d))],
    )
    .expect("invariant: a fresh archetype accepts a push that supplies every signature id")
}

/// Spawns an entity carrying only the table shape.
fn spawn_table_only(ecs: &mut EcsMaster) -> Entity {
    let archetype = ecs.get_or_create_archetype(&[id_of::<Eg1Table>()]);
    ecs.spawn_one(archetype, Eg1Table::default())
        .expect("invariant: the table-only archetype accepts a one-component push")
}

/// Enumerates into a 32-slot stack buffer and returns the filled prefix as a `Vec` for
/// assertion convenience.
///
/// The `Vec` is the TEST's, never the glue's: `components_of_into` writes into the caller's
/// slice and allocates nothing, which `ecs_alloc.rs` measures under a counting allocator.
fn enumerate(ecs: &EcsMaster, entity: Entity) -> Vec<IdEntry> {
    let mut buf = [SENTINEL; 32];
    let n = components_of_into(ecs, entity, &mut buf)
        .expect("invariant: the fixture entities are live and 32 slots hold every fixture");
    buf[..n].to_vec()
}

/// The kind reported for `id`, or `None` if `id` was not enumerated at all.
fn kind_of(entries: &[IdEntry], id: ComponentId) -> Option<IdKind> {
    entries.iter().find(|e| e.id == id).map(|e| e.kind)
}

/// Asserts no id appears twice — the half of gate 1 an "enumerates both" assertion cannot
/// express, and the half a second source of the same datum would break.
fn assert_no_duplicate_ids(entries: &[IdEntry]) {
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            assert_ne!(
                a.id, b.id,
                "component id {} was enumerated twice ({:?} then {:?}) -- two sources are \
                 reporting the same datum, which is the defect class the source-1 filter \
                 exists to prevent",
                a.id.get(),
                a.kind,
                b.kind
            );
        }
    }
}

// ═════════════════════════════════════════ gate 1 ═══════════════════════════════════════

/// **EG1 gate 1 — the B.3 assertion.** An entity carrying the reflect table shape and the
/// dense shape enumerates **exactly two** entries, `{(table, Table), (dense, Dense)}`, with
/// no id appearing twice.
///
/// ⚠️ *"Enumerates both"* — the assertion this rung was originally written around — is
/// satisfied by the **defective** implementation: an unfiltered source 1 emits the dense id
/// as well, so the buffer holds three entries for a two-component entity and the weaker
/// assertion still passes. That is also what made **R1** (delete source 2) a red that could
/// not fire. The exact count and the no-duplicate walk are what arm it.
#[test]
fn dense_component_is_enumerated() {
    let mut ecs = EcsMaster::new();
    let table = id_of::<Eg1Table>();
    let dense = id_of::<Eg1Dense>();

    assert_eq!(
        storage_kind(dense.get()),
        StorageKind::Dense,
        "the dense shape must actually be dense-classified, or this gate is a table \
         component wearing a dense name and source 2 is never exercised"
    );

    let entity = spawn_table_and_dense(&mut ecs);
    let entries = enumerate(&ecs, entity);

    assert_eq!(
        entries.len(),
        2,
        "a two-component entity enumerated {} entries: {entries:?}. Three means source 1 \
         is unfiltered and re-reported the dense id it retains; one means source 2 never \
         ran",
        entries.len()
    );
    assert_no_duplicate_ids(&entries);
    assert_eq!(
        kind_of(&entries, table),
        Some(IdKind::Table),
        "the reflect table shape must classify Table: it is in the signature, it has host \
         bytes, and `type_info_of` returned its descriptor"
    );
    assert_eq!(
        kind_of(&entries, dense),
        Some(IdKind::Dense),
        "the dense shape must classify Dense, and it can only be reached through source 2 \
         -- source 1 filters its id out"
    );
}

// ═════════════════════════════════════════ gate 1b ══════════════════════════════════════

/// **EG1 gate 1b — the order control.** A **table-only** entity spawned AFTER a
/// table+dense one enumerates exactly one entry.
///
/// This is the gate that catches the real defect rather than a cosmetic one. Archetype
/// dedup keys on the FILTERED signature mask, so both entities share one archetype whose
/// `component_ids()` is `[table, dense]`. An unfiltered source 1 therefore reports the
/// dense component on an entity that does not carry it — while `dense_contains` says
/// `false` — which is a *"wrong answer that looks like an answer"* (D15), not a duplicate.
/// The spawn order below is the one a fresh test binary takes.
#[test]
fn a_table_only_entity_spawned_after_a_dense_one_reports_only_its_own() {
    let mut ecs = EcsMaster::new();
    let table = id_of::<Eg1Table>();
    let dense = id_of::<Eg1Dense>();

    // ORDER IS THE FIXTURE: the polluting archetype first.
    let polluter = spawn_table_and_dense(&mut ecs);
    let table_only = spawn_table_only(&mut ecs);

    assert_eq!(
        ecs.entity_archetype_id(polluter),
        ecs.entity_archetype_id(table_only),
        "the two entities must share ONE archetype, or the trap this gate exists for is \
         not armed: dedup keys on the filtered mask, and both masks are {{table}}"
    );
    assert!(
        !ecs.dense_contains(table_only, dense),
        "the table-only entity must not be a member of the dense store -- the independent \
         answer this gate measures the enumeration against"
    );

    let entries = enumerate(&ecs, table_only);
    assert_eq!(
        entries.len(),
        1,
        "a table-only entity enumerated {} entries: {entries:?}. Two means source 1 read \
         the archetype's RETAINED id list unfiltered and reported a component this entity \
         does not carry",
        entries.len()
    );
    assert_eq!(kind_of(&entries, table), Some(IdKind::Table));
    assert_eq!(
        kind_of(&entries, dense),
        None,
        "the dense id must not appear at all on an entity `dense_contains` refuses"
    );
}

// ═════════════════════════════════════════ gate 2 ═══════════════════════════════════════

/// **EG1 gate 2 — `IdKind` classification is exhaustive over the fixture.**
///
/// Four of the five kinds; `Bitset` is source 3's and belongs to EG3.
///
/// ⚠️ The `TableGpu` row gets a **GPU-pure entity of its own**. A `ResidencyKind::Gpu` id
/// alongside any non-Gpu component is a **release-present panic** at archetype mint
/// (`saw_gpu && saw_non_gpu`), not a refusal — so the original "on the same entity as the
/// other three" form of this gate could not be built at all.
#[test]
fn id_kinds_are_exhaustive_over_the_fixture() {
    classify_the_gpu_shapes();

    let mut ecs = EcsMaster::new();
    let table = id_of::<Eg1Table>();
    let opaque = id_of::<Eg1Opaque>();
    let dense = id_of::<Eg1Dense>();

    // ── Table, TableOpaque, and the dynamic tag ────────────────────────────────────
    let tag = ecs.register_tag("editor_marker");
    let tag_id = ComponentId::from(tag);
    let archetype = ecs.get_or_create_archetype(&[table, opaque]);
    let cpu = ecs
        .spawn_two(archetype, Eg1Table::default(), Eg1Opaque::default())
        .expect("invariant: a fresh archetype accepts its own two-component push");
    ecs.add_tag(cpu, tag);

    let entries = enumerate(&ecs, cpu);
    assert_no_duplicate_ids(&entries);
    assert_eq!(entries.len(), 3, "table + opaque + dynamic tag, got {entries:?}");
    assert_eq!(
        kind_of(&entries, table),
        Some(IdKind::Table),
        "`#[component(reflect)]` + Table + Cpu is the one row that renders fields"
    );
    assert_eq!(
        kind_of(&entries, opaque),
        Some(IdKind::TableOpaque),
        "a plain `#[derive(Component)]` installed no TypeInfo, so it is name-only"
    );
    assert_eq!(
        kind_of(&entries, tag_id),
        Some(IdKind::TableOpaque),
        "a dynamic tag is Table-kind with no TypeInfo -- it merges into TableOpaque \
         because the only discriminator, `DynamicTagMarker`'s TypeId, is unnameable \
         outside `boyko_ecs`"
    );
    assert_eq!(
        display_name(tag_id),
        "editor_marker",
        "a dynamic tag's `ComponentLayout::type_name` IS the interned user name -- the \
         reason TableOpaque is a legible row for it and not a blank one"
    );

    // ── Dense, on the entity that carries one ──────────────────────────────────────
    let dense_entity = spawn_table_and_dense(&mut ecs);
    assert_eq!(kind_of(&enumerate(&ecs, dense_entity), dense), Some(IdKind::Dense));

    // ── TableGpu, on a GPU-PURE entity of its own ──────────────────────────────────
    let gpu = id_of::<Eg1GpuAnnotated>();
    assert_eq!(
        boyko_ecs::ecs::core::component::component_registry::residency_class(gpu.get()),
        ResidencyKind::Gpu,
        "the GPU shape must actually be Gpu-classified, or the TableGpu arm is never \
         reached and this row proves nothing"
    );
    let gpu_archetype = ecs.get_or_create_archetype(&[gpu]);
    let gpu_entity = ecs
        .spawn_one(gpu_archetype, Eg1GpuAnnotated::default())
        .expect("invariant: a GPU-PURE archetype accepts its own push");
    let gpu_entries = enumerate(&ecs, gpu_entity);
    assert_eq!(gpu_entries.len(), 1, "the GPU-pure entity carries exactly one component");
    assert_eq!(
        kind_of(&gpu_entries, gpu),
        Some(IdKind::TableGpu),
        "a Gpu-classed id is enumerated and marked unreadable -- it is NOT hidden, because \
         an inspector that omits a component the entity has is lying about what is there"
    );
}

// ═════════════════════════════════════════ gate 2b ══════════════════════════════════════

/// **EG1 gate 2b — the classification PRECEDENCE.** `residency_class` is consulted
/// **before** `type_info_of`.
///
/// Asserted on **two** Gpu fixtures, one annotated with `#[component(reflect)]` and one
/// not, so the order is a property of the code rather than of which fixture happened to be
/// written. Both must classify `TableGpu`: the GPU answer is a classification test, not a
/// `TypeInfo` test, and §3's matrix has no `Table + Gpu` row for the un-annotated case.
///
/// Both live on ONE entity, which is legal precisely because both are `Gpu` — the archetype
/// is GPU-pure.
#[test]
fn residency_is_consulted_before_type_info() {
    classify_the_gpu_shapes();

    let mut ecs = EcsMaster::new();
    let annotated = id_of::<Eg1GpuAnnotated>();
    let plain = id_of::<Eg1GpuPlain>();

    assert!(
        boyko_reflect::type_info_of(annotated.get()).is_some(),
        "the annotated GPU shape must have a descriptor installed, or this gate degenerates \
         into two copies of the un-annotated case and the precedence is never exercised"
    );
    assert!(
        boyko_reflect::type_info_of(plain.get()).is_none(),
        "the plain GPU shape must have NO descriptor -- it is the half that would classify \
         TableOpaque if `type_info_of` were consulted first"
    );

    let archetype = ecs.get_or_create_archetype(&[annotated, plain]);
    let entity = ecs
        .spawn_two(archetype, Eg1GpuAnnotated::default(), Eg1GpuPlain::default())
        .expect("invariant: an archetype of two Gpu ids is GPU-PURE and accepts its push");

    let entries = enumerate(&ecs, entity);
    assert_eq!(entries.len(), 2, "got {entries:?}");
    assert_eq!(
        kind_of(&entries, annotated),
        Some(IdKind::TableGpu),
        "an annotated Gpu id classifies TableGpu: residency wins over the descriptor"
    );
    assert_eq!(
        kind_of(&entries, plain),
        Some(IdKind::TableGpu),
        "an un-annotated Gpu id classifies TableGpu too -- if this said TableOpaque the \
         descriptor test would be running first and the GPU row would be reachable only \
         for types that opted into reflection"
    );
}

// ═════════════════════════════════════════ gate 4 ═══════════════════════════════════════

/// **EG1 gate 4 — `Err(BufferTooSmall)`, and the buffer is UNMODIFIED.**
///
/// A one-slot buffer against a three-component entity. The refusal is the easy half; the
/// assertion that carries weight is that not one entry was written, so a caller cannot
/// mistake a partial fill for an answer. That is why `IdEntry` derives `PartialEq`.
///
/// The subject spans **both** sources — two signature ids and one dense id — so the
/// overflow is not decidable from source 1 alone.
#[test]
fn a_short_buffer_refuses_and_writes_nothing() {
    let mut ecs = EcsMaster::new();

    let entity = spawn_table_opaque_and_dense(&mut ecs);

    // Three: the table shape, the opaque shape, and the dense shape.
    assert_eq!(enumerate(&ecs, entity).len(), 3, "the subject must be a THREE-component entity");

    let mut one_slot = [SENTINEL; 1];
    assert_eq!(
        components_of_into(&ecs, entity, &mut one_slot),
        Err(Refusal::BufferTooSmall),
        "a short buffer must refuse rather than return a short count -- a truncated list \
         is a wrong answer that looks like an answer"
    );
    assert_eq!(
        one_slot,
        [SENTINEL; 1],
        "the refusal wrote into the caller's buffer. A caller that ignored the Err and read \
         slot 0 would now see a real component id and could not tell a partial fill from an \
         answer"
    );

    // A buffer that is exactly big enough is NOT a refusal — the boundary, from the other
    // side, so `>` has not been written as `>=`.
    let mut exact = [SENTINEL; 3];
    assert_eq!(
        components_of_into(&ecs, entity, &mut exact),
        Ok(3),
        "an exactly-sized buffer is sufficient, not too small"
    );
}

// ═══════════════════════════ the other refusal EG1 lands ════════════════════════════════

/// **Not on EG1's numbered gate list, and it should be.** `Refusal::EntityDead` is the other
/// arm `components_of_into` can return at this rung — the refusal taxonomy as a whole is
/// EG4's, but this one variant is *reachable from EG1's own code today*, and a datum that
/// lands without a gate is how this campaign has repeatedly shipped an unexercised path.
///
/// The distinction it carries is the one `Option` could not: a dead entity is not "an
/// entity with no components", and an inspector holding a stale row after a despawn must
/// show those apart.
#[test]
fn a_stale_entity_handle_refuses_rather_than_answering_empty() {
    let mut ecs = EcsMaster::new();
    let entity = spawn_table_only(&mut ecs);
    assert_eq!(enumerate(&ecs, entity).len(), 1, "live first, so the refusal below is the death");

    assert!(ecs.delete_entity(entity), "the fixture entity must actually have been despawned");

    let mut buf = [SENTINEL; 8];
    assert_eq!(
        components_of_into(&ecs, entity, &mut buf),
        Err(Refusal::EntityDead),
        "a stale handle must refuse. `Ok(0)` would tell an inspector the entity is alive and \
         carries nothing, which is a different row from the one it must draw"
    );
    assert_eq!(buf, [SENTINEL; 8], "a refusal writes nothing into the caller's buffer");
}

// ═══════════════════════════ `display_name`'s two ungated rules ═════════════════════════

/// An in-range id this process never registered. `MAX_COMPONENTS` is 512 and this binary mints
/// fewer than ten ids, but the test below **asserts** that rather than assuming it — an id that
/// turned out to be registered would make the gate measure a named component instead.
const NEVER_REGISTERED: ComponentId = ComponentId(500);

/// The exact text `display_name` owes for an unregistered id, pinned here **from outside the
/// crate** because the constant that carries it is private.
const UNREGISTERED_TEXT: &str = "<unregistered ComponentId>";

/// **Not on EG1's numbered gate list, and it should be — the same omission as the refusal above,
/// one function away.** `UNREGISTERED`'s own `///` forbids the empty string in as many words:
/// *"a blank cell in an inspector reads as 'this component has no name', which is a different and
/// wronger statement"*. MEASURED at the EG1 verification: emptying that constant left **every**
/// gate in the campaign green — the rung's own seven, the allocator binary's three, and the whole
/// `-p boyko-reflect --all-targets` sweep.
///
/// A documented rule with no gate is a rule the next edit deletes for free, and this file already
/// carries the proof that the shape is gateable: the refusal test above pins the sibling
/// distinction — a dead entity is not an entity with no components — in the same binary, in the
/// same rung.
#[test]
fn an_unregistered_id_is_named_rather_than_left_blank() {
    assert!(
        get_layout(NEVER_REGISTERED.get()).is_none(),
        "id {} IS registered in this binary, so this gate is measuring a named component and not \
         the unregistered path at all -- pick a higher one",
        NEVER_REGISTERED.get()
    );

    let name = display_name(NEVER_REGISTERED);
    assert!(
        !name.is_empty(),
        "`display_name` answered with the empty string. An inspector renders that as a blank \
         cell, which reads as `this component has no name` -- a different and wronger statement \
         than `this process never registered this id`"
    );
    assert_eq!(
        name, UNREGISTERED_TEXT,
        "the sentinel text moved. It is part of what an inspector shows a human, so it is pinned \
         from outside the crate; re-pin it here deliberately, in the same change that moves it"
    );
}

/// **`display_name` is total, and it was not.** `pub fn display_name(id: ComponentId)` accepts any
/// id and its `///` states **no bound**, but `get_layout` opens with
/// `debug_assert!(component_id < MAX_COMPONENTS, …)` before its release-path `None`.
///
/// MEASURED at the EG1 verification, before the bound moved into the glue: this call **panicked in
/// debug** and returned the sentinel in release — a debug/release divergence in a landed public
/// API, reached with the exact `ComponentId` this file already fills its buffers with.
///
/// ⚠️ This test is deliberately **not** release-only. Bounding the doc instead of the function
/// would have made the rule a statement about release behaviour, and such a rule is *ungatable in
/// debug* — the assert fires before it can be observed. Fixing the function instead lets one
/// assertion cover both profiles, which is the reason that fix was the one taken.
#[test]
fn display_name_answers_at_any_id_rather_than_aborting() {
    let past_the_registry = SENTINEL.id.get();
    assert!(
        past_the_registry >= MAX_COMPONENTS,
        "the sentinel id is inside the registry's range, so this gate no longer crosses the \
         bound it exists to cross"
    );
    assert_eq!(
        display_name(SENTINEL.id),
        UNREGISTERED_TEXT,
        "an out-of-range id must be ANSWERED, not asserted on. This is the same id the buffers \
         above are pre-filled with, so a caller holding a stale handle reaches it by accident"
    );
}

// ═════════════════════════ gate 5 + the numbers the rung owes ═══════════════════════════

/// **EG1 gate 5 and the rung's *Measured and reported* list.**
///
/// `size_of::<IdEntry>()` is pinned by a `const _: () = assert!(...)` in
/// `boyko_reflect::ecs` — a compile-time gate, so this test only reports the number it
/// pins. The other two numbers are measured here.
///
/// ⚠️ `dense_ids().len()` is **asserted**, not only printed. The plan's §7 promised the
/// assertion and the rung's own list had reduced it to a print; EG1 is the rung on which
/// that number stops being vacuous, because this is the first binary in which a dense
/// component is actually registered.
#[test]
fn the_numbers_this_rung_owes() {
    let mut ecs = EcsMaster::new();
    let entity = spawn_table_and_dense(&mut ecs);

    // ── dense_ids().len() ───────────────────────────────────────────────────────────
    let dense_len = ecs.dense_registry().dense_ids().len();
    println!("EG1: dense_ids().len() on the fixture = {dense_len}");
    assert!(
        dense_len >= 1,
        "dense_ids() is EMPTY in this binary, so source 2 iterates zero elements and every \
         assertion about it holds for every possible implementation -- the tautology shape \
         this number exists to refuse"
    );

    // ── size_of::<IdEntry>() ────────────────────────────────────────────────────────
    println!("EG1: size_of::<IdEntry>() = {}", size_of::<IdEntry>());
    assert_eq!(size_of::<IdEntry>(), 16, "pinned by a const assert in `boyko_reflect::ecs`");

    // ── components_of_into wall clock, per entity ───────────────────────────────────
    //
    // Under Miri this loop is interpreted, so the count drops to something that finishes;
    // the number printed there is not a wall clock of anything and is not asserted on.
    //
    // ⚠ INDICATIVE, NOT PINNED — and the print now SAYS so, because a number quoted in a
    // rung report is read as a measurement. This is a bare `Instant` over 20k reps, no
    // warm-up, no repetition, on a shared machine: the release figure moved 15.3 / 18.1 /
    // 20.9 / 21.0 ns/entity across four consecutive runs at the EG1 verification, against
    // 28.2 recorded by the rung itself. The only ASSERTION here is `sink == 2 * reps` — that
    // every iteration really enumerated the two components. Pinning a wall clock needs a
    // criterion bench, which is the GATES plan's to schedule and not this binary's.
    let reps: usize = if cfg!(miri) { 4 } else { 20_000 };
    let mut buf = [SENTINEL; 32];
    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..reps {
        sink += components_of_into(&ecs, entity, &mut buf).expect("live entity");
    }
    let elapsed = start.elapsed();
    assert_eq!(sink, 2 * reps, "every iteration enumerated the same two components");
    println!(
        "EG1: components_of_into = {:.1} ns/entity [INDICATIVE, not pinned] ({} reps, \
         sources 1+2, {} table id(s) scanned + {dense_len} dense id(s) probed)",
        elapsed.as_nanos() as f64 / reps as f64,
        reps,
        ecs.archetype_master()
            .get_archetype(ecs.entity_archetype_id(entity).expect("live"))
            .expect("live")
            .component_ids()
            .len()
    );
}
