//! The by-id structural seam on [`EcsMaster`] (Reflection plan ECS, EG2 §4).
//!
//! Four public items — `add_component_by_id`, `remove_component_by_id`,
//! `mark_component_changed` (here) and `EnableTagId::try_from_component_id`
//! (in `component_registry/tags.rs`) — that let a caller who cannot name the
//! Rust type perform the structural op a scene loader, a prefab instantiator,
//! an undo stack and a network apply all need. **They name no reflection
//! type**: the vocabulary is [`ComponentId`] + `&[u8]`, exactly like the
//! `add_tag` / `remove_tag` pair they are modelled on.
//!
//! # Why a new module rather than an append to `component_api.rs`
//!
//! ⚠️ **EVERY `component_api.rs` CITATION FIGURE BELOW IS AS OF `f7c46c76`, and none of them is
//! true in the tree this file ships in.** The stamp is the repair, not a caveat: these numbers
//! justify a DECISION taken at that commit, and a ledger re-derived against a moving tree states a
//! number that the very act of writing it changes — which is how the last three revisions of this
//! header went wrong. The shipping-tree recount is at the END of this section, stamped the same
//! way, and the ONE figure here that is a shipping-tree measurement rather than an `f7c46c76` one
//! is labelled where it sits. Until EG2-R round 5 the header carried NO as-of qualifier at all: a
//! reviewer defended it as honest because *"it says at HEAD f7c46c76"*, and `grep -n HEAD` on this
//! file returned nothing — exit 1, measured.
//!
//! At `f7c46c76`, `component_api.rs` was cited by line at **40** places across **FIVE** documents
//! — `FEATURE_MAP.md` 4, `SYSTEMS.md` 4, `REFLECTION-ANALYSIS.md` 5,
//! `REFLECTION-PLAN-BOUNDARY.md` 3, `REFLECTION-PLAN-ECS.md` 24 — at **19**
//! distinct anchors. An append shifts every one; a new module shifts none.
//! Only THREE of the five `docs/REFLECTION-*.md` cite it at all (`-CORE` and
//! `-GATES` cite it zero times). RECOUNTED at EG2-R round 4 over both citation
//! forms — `component_api.rs:N`, and the `(N)` table form under a `**File:**`
//! heading, which is how `FEATURE_MAP` and `SYSTEMS` cite. The figures this
//! header used to give — "five reflection plans" and "twelve places" — matched
//! no countable population and are struck.
//!
//! ⚠️ **"31 stale" and "37 citations" were struck by the same act, and THAT strike is WITHDRAWN
//! at round 5.** Those two count a different, perfectly definite population — **every** `:N`
//! token, bare continuations included — which is what the `OPEN-QUESTIONS` twins count, and four
//! of that ledger's five per-document figures reproduce exactly under it (`FEATURE_MAP` 4 of 4,
//! `SYSTEMS` 4 of 4, `REFLECTION-ANALYSIS` 5 of 9, `REFLECTION-PLAN-BOUNDARY` 4 of 5). The two
//! ledgers count two populations and neither refutes the other; say which one you mean. Same
//! correction applies to the waived anchors below: FOUR is right for the prefixed form and the
//! twins' SIX is right for all tokens — they are the same three anchors, each cited twice, and one
//! line cites all three at once.
//!
//! ⚠️ **EG2-R FORK A spent part of that saving and the ledger must say so.**
//! Splitting `dense_insert_and_fire` into `dense_insert_only` +
//! `dense_fire_add_insert` (the phase order a dense `#[require]` caller needs is
//! unreachable through the fused form) grew `component_api.rs` from 793 to 823
//! lines, all 30 above its old `:121`. **33** of the 40 therefore moved by +30
//! and were repaired; **7** did not (`:49` ×3, `:76` ×1, `:94` ×3); the old
//! `:105` (`store.mark_arch_present`) moved INTO `dense_insert_only` at `:132`.
//! FOUR of the 33 are `~`-waived at THREE anchors (`:238~` twice, `:422~` and
//! `:713-716~` once each), and a waived anchor rots SILENTLY on a line shift.
//! **The saving this module bought was real and it is now smaller: spend the
//! next one deliberately.** FIVE anchors into `crates/boyko_ecs/tests/seam_by_id.rs`
//! (`:895`, `:942`, `:1308`, `:1314`, `:2217`) are cited TWELVE times across the
//! EN/RU twins, and NO census sees them: `docs/OPEN-QUESTIONS.md` is not `GATED`.
//! ⚠️ **That one is a SHIPPING-TREE figure, not an `f7c46c76` one** — at `f7c46c76` the twins do
//! not name this file at all (`git show f7c46c76:docs/OPEN-QUESTIONS.md | grep -c seam_by_id` → 0,
//! and 0 for the RU twin). It counts citations that USE an anchor and excludes the two inventory
//! sentences that merely LIST the five, one per twin; counting those instead gives 22. Naming the
//! population is the whole of the difference.
//!
//! **Round-5 recount — same population, same two forms, measured in the shipping tree (this
//! landing applied, uncommitted): 54 citations across SEVEN documents at 23 distinct anchors.**
//! `REFLECTION-PLAN-ECS.md` went 24 → 30 and each `OPEN-QUESTIONS` twin joined with 4; the seven
//! unmoved citations are now `:49` ×6, `:76` ×1 and `:94` ×4, and the waived `:238~` is cited ×3
//! rather than twice — a growth this ledger's own copy in `REFLECTION-PLAN-ECS.md` is part of, and
//! the sentence asserting *"`:238~` twice"* is itself that anchor's third citation.
//! **A number that describes the diff it lives inside is not maintainable; it is stamped. Re-take
//! it rather than quoting it.**

use std::cell::UnsafeCell;

use crate::ecs::core::commands::migration_helpers::{
    AttachSrc, merged_archetype_id_dyn, migrate_entity_attach_ids_with_bytes,
    migrate_entity_detach_ids, without_ids_archetype_id,
};
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component_registry::{
    self, MAX_COMPONENTS, ResidencyKind, StorageKind,
};
use crate::ecs::core::component::hooks::scope::DeferredScopeGuard;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Outcome of [`EcsMaster::add_component_by_id`] (§4 S1).
///
/// An editor's "Add Component" must REFUSE rather than clobber a present value;
/// `add_tag`'s in-place-replace semantics are right for a tag (zero bytes to
/// lose) and wrong for data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddOutcome {
    /// The component was attached; bytes, ticks and hooks all landed.
    ///
    /// Named `Attached`, not `Added`, and that is load-bearing — but the rule
    /// is **effective visibility**, not crate membership, and getting that
    /// backwards would misdirect the next author. rustc's `trimmed_def_paths`
    /// prints a short path only while it is unambiguous among the paths a
    /// diagnostic can REACH. This crate ALREADY contains a second item named
    /// `Added` — `InPlaceOutcome::Added` in `relationship/mod.rs` — and the four
    /// `.stderr` were green with it the whole time, because that enum is
    /// PRIVATE to its module. A `pub` variant on a `pub` enum re-exported from a
    /// `pub` module is what collides.
    ///
    /// MEASURED: with this variant named `Added`,
    /// `cargo test -p boyko-ecs --test compile_fail_chunk
    /// --test enable_filter_compile_fail --test query_change_detection_compile_fail`
    /// exits 101 with 4 mismatches, each a one-token diff
    /// (`Added::<BitsetTag>` -> `boyko_ecs::ecs::core::iters::query::Added::<BitsetTag>`).
    /// The measurement stands; only the stated rule was wrong.
    Attached,
    /// The entity already hosts this component; NOTHING was written.
    AlreadyPresent,
    /// The call was refused before any mutation. See [`RejectReason`].
    Rejected(RejectReason),
}

/// Why [`EcsMaster::add_component_by_id`] refused (§4 S1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RejectReason {
    /// The entity handle is stale or its slot is null.
    EntityDead,
    /// `bytes.len() != component_registry::get_layout(id.0).size`.
    WrongByteLen,
    /// `storage_kind(id)` is neither `Table` nor `Dense` (or `id` is unregistered).
    NotTableOrDense,
    /// `residency_class(id) == ResidencyKind::Gpu`.
    GpuResident,
}

impl EcsMaster {
    /// Attaches the data component `id` to `entity` from raw `bytes` (§4 S1).
    ///
    /// The by-id twin of [`EcsMaster::add_tag`](Self::add_tag), generalised off
    /// the ZST restriction: a scene loader, a prefab instantiator, an undo stack
    /// and a network apply all attach a component whose Rust type they cannot
    /// name. Names no reflection type.
    ///
    /// # Arm order (gated)
    ///
    /// Liveness → storage kind → residency → byte length → presence → mutate.
    /// The kind and residency refusals precede the length check **on purpose**:
    /// a caller who passes a bitset id with a wrong-length slice must be told
    /// `NotTableOrDense`, and a refusal placed after `merged_archetype_id_dyn`
    /// would abort the process inside `residency_conflict_panic` instead of
    /// returning. The length check is a plain **release** `if`, never a
    /// `debug_assert!` — a short slice reaching the pool memcpy is a
    /// buffer overread in a shipping build.
    ///
    /// Returns [`AddOutcome::AlreadyPresent`] without writing anything when the
    /// entity already hosts `id` (data is NOT clobbered — that is the one
    /// semantic this pair does not inherit from `add_tag`).
    ///
    /// # Ownership — the bytes are MOVED, not borrowed
    ///
    /// The row the world writes is the row the world later drops (`drop_fn` on
    /// despawn / world drop). `bytes` is therefore a **move**, not a view: a
    /// caller that keeps its own live value and passes a byte view of it
    /// **double-drops**, through a `pub fn` carrying no `unsafe`. Wrap the
    /// source value in [`std::mem::ManuallyDrop`] (or `mem::forget` it) before
    /// forming the slice. For a `Copy` payload there is nothing to transfer and
    /// nothing to do.
    ///
    /// # `#[require]`
    ///
    /// Honoured, identically to a typed `insert`, **for both storage kinds**:
    /// the transitive required closure of `id` is expanded, each missing member
    /// is constructed through its capture-free `RequiredCtor`, and
    /// on_add / on_insert fire for the constructed ids as well as for `id`. A
    /// required id the entity already carries is skipped (present ⇒ skip; the
    /// existing value wins).
    ///
    /// For a DENSE `id` the phase order is reproduced, not merely the call
    /// order: the dense store write lands BEFORE the required columns' on_add
    /// (so a hook reading `DeferredEcsMaster::get_component` sees the dense
    /// value — `get_component_raw` has a dense arm), and the dense on_add /
    /// on_insert fire AFTER them, exactly as `migrate_entity_insert`'s Phase 1
    /// and POST-dense blocks do. Both facts are MEASURED against the typed path
    /// by `tests/seam_by_id.rs`'s `g13b`.
    ///
    /// ## Three divergences, each deliberate or filed
    ///
    /// 1. **`AlreadyPresent` does not repair a broken require.** A typed
    ///    `insert` of an already-present component re-expands its closure and
    ///    re-constructs a missing member; this seam returns
    ///    [`AddOutcome::AlreadyPresent`] and writes NOTHING, which is the one
    ///    semantic it deliberately does not inherit from `add_tag` (§4: an
    ///    editor's "Add Component" must refuse, not mutate). Repair is
    ///    `remove_component_by_id(needs)` then `add_component_by_id(needs)`.
    /// 2. **A `#[require]` target that is DENSE mints a phantom per-archetype
    ///    column** (the expansion pushes required ids unfiltered) and trips
    ///    `migrate_entity_attach_ids_with_bytes`'s `target.has_component_id`
    ///    debug-assert — the loud failure the filed gap deserves, deliberately
    ///    left in place. Shared with the typed path, not introduced here.
    /// 3. **A required id that is `Gpu`-resident** reaches
    ///    `residency_conflict_panic` inside the archetype resolver, because
    ///    arm 3 checks only the caller's own `id`. Shared with the typed path.
    pub fn add_component_by_id(
        &mut self,
        entity: Entity,
        id: ComponentId,
        bytes: &[u8],
    ) -> AddOutcome {
        // ARM 1 — liveness. Unlike `add_tag`'s silent no-op this REPORTS: a
        // by-id caller holds an id it cannot type-check, so "nothing happened"
        // must be distinguishable from "the entity was already gone".
        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return AddOutcome::Rejected(RejectReason::EntityDead);
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return AddOutcome::Rejected(RejectReason::EntityDead);
            }
            *slot
        };

        // ARM 2 — storage kind. The bounds check is MANDATORY and comes first:
        // `storage_kind` `debug_assert!`s on an out-of-range id, so without it
        // the "total" contract of this fn is false in debug.
        if id.0 >= MAX_COMPONENTS {
            return AddOutcome::Rejected(RejectReason::NotTableOrDense);
        }
        let kind = component_registry::storage_kind(id.0);
        if !matches!(kind, StorageKind::Table | StorageKind::Dense) {
            return AddOutcome::Rejected(RejectReason::NotTableOrDense);
        }
        // An id with no registered layout has no byte size to validate against
        // and no pool to write into — refused in the same channel as a
        // non-table/dense kind (there is no "unregistered" reason: from the
        // caller's side it is the same "this id is not attachable" fact).
        let Some(layout) = component_registry::get_layout(id.0) else {
            return AddOutcome::Rejected(RejectReason::NotTableOrDense);
        };

        // ARM 3 — residency. A `Gpu` column lives in device memory; a host byte
        // slice cannot be memcpy'd into it. Refused BEFORE the archetype
        // resolver, which would reach `residency_conflict_panic`.
        if component_registry::residency_class(id.0) == ResidencyKind::Gpu {
            return AddOutcome::Rejected(RejectReason::GpuResident);
        }

        // ARM 4 — byte length. A plain release `if`: this is the ONLY check
        // standing between a caller-supplied slice and
        // `write_at_unchecked_initialized`'s `copy_nonoverlapping` of
        // `layout.size` bytes.
        if bytes.len() != layout.size {
            return AddOutcome::Rejected(RejectReason::WrongByteLen);
        }

        // ARM 5 — presence, PER STORAGE KIND. A dense id is filtered out of
        // every archetype signature by construction, so the signature probe
        // answers `false` for a component the entity HAS; the dense membership
        // oracle is the global store.
        //
        // SAFETY (U1, U2, U11, F1) — mirrors `add_tag`: `archetype_ptr` is
        //   stable, interior-mutable (`SharedReadWrite`, F4-rooted) slab
        //   provenance; it survives sibling structural writes under TB/SB
        //   (the whole slab element is `UnsafeCell`-wrapped). Non-null +
        //   generation-matched above ⇒ the slot is live. Shared reads only
        //   (`id`, signature word); no `&mut` is taken.
        let (source_archetype_id, in_signature) = unsafe {
            let archetype = &*inland.archetype_ptr();
            (archetype.id(), archetype.has_component_id(id))
        };
        let present = match kind {
            StorageKind::Dense => self.dense_contains(entity, id),
            _ => in_signature,
        };
        if present {
            return AddOutcome::AlreadyPresent;
        }

        // ARM 6 — mutate. The RAII depth bracket opens HERE, after every
        // refusal and after the presence test (`remove_tag`'s bracket order,
        // not `add_tag`'s): a call that mutates nothing must not enter a
        // deferred scope at all.
        let scope = DeferredScopeGuard::enter();

        // `#[require]` is decided BEFORE the storage-kind split, and that is the
        // whole point: the typed path expands the required closure regardless of
        // the inserted id's storage kind (`merged_archetype_id` filters dense out
        // of `bundle_ids` but runs `for_each_required_id_excluding`
        // unconditionally). A split ABOVE this test is what made a dense
        // caller's whole expansion vanish (gate 13b).
        //
        // The expansion happens in the CALLER, and `merged_archetype_id_dyn` is
        // untouched: it is PRE-EXISTING and SHARED with `add_tag`, whose
        // `migrate_entity_attach_ids` sibling has no constructor pass. Expanding
        // inside the resolver would hand that path a target archetype containing
        // pools that never get a committed row, breaking the
        // `row == target.current_index == dst_pool.count()` lockstep its Step 3
        // asserts.
        //
        // 0%-GATE: a require-free id — the overwhelming majority, and every id in
        // a table-only world — never enters the cold helper and never touches its
        // scratch. `any_requires` is one memoized `get_required_plan` read plus an
        // `is_empty()`.
        if component_registry::any_requires(&[id]) {
            self.add_by_id_expanding_requires(entity, inland, source_archetype_id, kind, id, bytes);
        } else {
            match kind {
                StorageKind::Dense => {
                    // No migration (the dense payoff): the entity keeps its
                    // archetype and the store records the membership. Neither a
                    // presence nor a length check happens inside — which is why
                    // arms 4 and 5 precede it.
                    self.dense_insert_and_fire(entity, source_archetype_id, id, bytes);
                }
                _ => {
                    let target_archetype_id =
                        merged_archetype_id_dyn(self, source_archetype_id, &[id]);
                    // `id ∉ source` (arm 5) ⇒ the union is strictly larger ⇒ a
                    // distinct exact-mask match.
                    debug_assert_ne!(
                        target_archetype_id, source_archetype_id,
                        "merged_archetype_id_dyn returned the source for a strictly-growing union"
                    );
                    migrate_entity_attach_ids_with_bytes(
                        self,
                        entity,
                        source_archetype_id,
                        target_archetype_id,
                        &[id],
                        &[AttachSrc::Bytes(bytes)],
                    );
                }
            }
        }

        // Direct API: drop the bracket, then drain — at depth 0 the drain runs;
        // reached at depth >= 1 it no-ops and the outermost owner drains (Q-A1).
        drop(scope);
        self.drain_deferred_hook_queue();
        AddOutcome::Attached
    }

    /// The `#[require]`-bearing half of [`Self::add_component_by_id`] (§4 S1,
    /// EG2-R FORK A). Split into its own `#[cold] #[inline(never)]` fn for a
    /// MEASURED reason, not a stylistic one: the two scratch arrays below are
    /// 12,288 B, and while they lived in `add_component_by_id`'s body LLVM
    /// hoisted them into that fn's PROLOGUE — `.seh_stackalloc 12440` plus a
    /// `___chkstk_ms` page-probe loop on EVERY call, including `EntityDead` /
    /// `WrongByteLen` / `NotTableOrDense` / `AlreadyPresent`, all of which return
    /// before the arrays are used. Its siblings in the same object pay 64
    /// (`add_tag`) and 72 (`remove_component_by_id`). Verified by re-reading
    /// `.seh_stackalloc` out of `--emit asm -C codegen-units=1`, before and
    /// after; the binary witness is the ABSENCE of the `___chkstk_ms` line from
    /// `add_component_by_id`'s prologue (Windows emits the page probe only for
    /// frames >= 4096, so its absence IS `.seh_stackalloc < 4096`).
    ///
    /// Reproduces the typed path's PHASE order for a dense caller id, which is
    /// not the same as its call order — see the dense arm below.
    #[cold]
    #[inline(never)]
    fn add_by_id_expanding_requires(
        &mut self,
        entity: Entity,
        inland: EntityInland,
        source_archetype_id: ArchetypeId,
        kind: StorageKind,
        id: ComponentId,
        bytes: &[u8],
    ) {
        // A DENSE caller id owns no archetype column and must NOT enter the
        // migration set: `merged_archetype_id_dyn` does not filter its `extra`,
        // so passing it would mint a phantom per-archetype column — the exact
        // trap `merged_archetype_id`'s own dense filter exists to avoid. The
        // caller's id therefore seeds `ids[0]` only when it is table-stored.
        let caller_is_table = !matches!(kind, StorageKind::Dense);

        // Stack scratch. The width is `MAX_COMPONENTS`, which is NUMERICALLY the
        // same 512 as `migration_helpers.rs`'s private `MAX_MIGRATION_COLUMNS`
        // (`const MAX_MIGRATION_COLUMNS: usize = MAX_COMPONENTS;`) — and the
        // REASON it is right is the archetype bound, not the registry bound: the
        // attach set must fit ONE archetype. A narrower constant would silently
        // TRUNCATE a valid (if absurd) registry in release — the `debug_assert!`
        // below is a debug-only tripwire, not a release guard, so the width has
        // to be sound on its own. Raising the sibling constant's visibility just
        // to say so would be a sixth kernel item for nothing.
        let mut ids = [id; MAX_COMPONENTS];
        let mut srcs = [AttachSrc::Bytes(bytes); MAX_COMPONENTS];
        let mut n = usize::from(caller_is_table);

        component_registry::for_each_required_id_excluding(&[id], |req_id| {
            // PRESENT ⇒ SKIP, decided on `component_ids()`, NOT on
            // `has_component_id`. `component_ids()` is the RETAINED list and it
            // KEEPS dense/bitset ids, while the mask filters them out;
            // `migrate_entity_insert`'s Step 2b tests the retained list for
            // exactly this reason. Using the filtered mask here would silently
            // re-construct a dense id the entity already has.
            //
            // SAFETY (U1, U2, U11, F1): same as arm 5's presence read —
            //   `archetype_ptr` is stable, interior-mutable (`SharedReadWrite`,
            //   F4-rooted) slab provenance; non-null + generation-matched by the
            //   caller ⇒ live; a shared read of the retained id list only, no
            //   `&mut` is taken.
            let already = unsafe {
                let archetype = &*inland.archetype_ptr();
                archetype.component_ids().contains(&req_id)
            };
            if already {
                return;
            }
            debug_assert!(
                n < MAX_COMPONENTS,
                "required closure exceeded the archetype bound"
            );
            ids[n] = req_id;
            srcs[n] = AttachSrc::Ctor(
                component_registry::required_ctor_for(&[id], req_id).expect(
                    "invariant: req_id came from this id's own required \
                     closure, so a ctor exists for it",
                ),
            );
            n += 1;
        });

        if n == 0 {
            // A DENSE caller whose entire required closure is already present:
            // there is no table work at all, so there is no migration and no
            // target archetype. The typed twin collapses to
            // `InsertCommand::apply_replace_in_place`, whose dense branch marks
            // the SOURCE present — which is what the fused helper does here.
            debug_assert!(!caller_is_table, "a table caller always seeds ids[0]");
            self.dense_insert_and_fire(entity, source_archetype_id, id, bytes);
            return;
        }

        // `ids[0] / srcs[0]` are the caller's own id and bytes whenever it is
        // table-stored, so `n == 1` (every required id already present) is the
        // ordinary single-id attach and needs no special case.
        let target_archetype_id = merged_archetype_id_dyn(self, source_archetype_id, &ids[..n]);
        debug_assert_ne!(
            target_archetype_id, source_archetype_id,
            "merged_archetype_id_dyn returned the source for a strictly-growing union"
        );

        // TYPED PHASE ORDER, and the two halves are NOT adjacent by accident.
        // `migrate_entity_insert` writes the dense store inside its Phase-1
        // closure — before ANY hook fires, and marking the TARGET archetype
        // present — and fires the dense hooks in its POST block, AFTER the
        // table/required fires. Both facts are observable and both were MEASURED
        // on the typed path:
        //   * order: required-table on_add fires first, dense on_add second;
        //   * visibility: the required column's on_add reads the dense value as
        //     PRESENT (`get_component_raw` has a dense arm).
        // Calling the fused `dense_insert_and_fire` on either side of the
        // migration reproduces exactly one of the two and breaks the other
        // (gate 13b asserts both).
        if !caller_is_table {
            self.dense_insert_only(entity, target_archetype_id, id, bytes);
        }
        migrate_entity_attach_ids_with_bytes(
            self,
            entity,
            source_archetype_id,
            target_archetype_id,
            &ids[..n],
            &srcs[..n],
        );
        if !caller_is_table {
            self.dense_fire_add_insert(entity, id);
        }
    }

    /// Detaches the component `id` from `entity` (§4 S2). Returns `false` when
    /// the entity is dead or does not host `id` — the by-id twin of
    /// [`EcsMaster::remove_tag`](Self::remove_tag).
    ///
    /// # No reason channel
    ///
    /// `false` conflates dead / absent / not-detachable **on purpose**: the
    /// twin it mirrors returns nothing at all, and inventing a residency
    /// refusal for a `bool`-returning fn would be a channel with no reader (a
    /// `Gpu` archetype is all-`Gpu` by the `GPU_RESIDENT ⇔ all-components-Gpu`
    /// semantic, so a detach cannot mix residencies). A `Bitset` id — an enable
    /// tag — is likewise `false`: it is toggled, never detached
    /// (`disable_id` is its verb).
    ///
    /// **Copies nothing.** It calls the existing `migrate_entity_detach_ids`,
    /// which already carries the retained-id guard; there is no detach sibling
    /// in this rung.
    ///
    /// # Entity-targeted observers do NOT fire (deferred)
    ///
    /// A typed `remove` fires `on_replace` / `on_remove` **entity-targeted**
    /// observers (`migrate_entity_remove`) and re-raises the sticky
    /// `HAS_ENTITY_OBSERVER` bit on the destination. This fn does not:
    /// `migrate_entity_detach_ids` contains neither call. That helper is
    /// PRE-EXISTING and SHARED with [`EcsMaster::remove_tag`](Self::remove_tag),
    /// so fixing it changes `remove_tag` too — a kernel change outside this
    /// rung's approved diff, filed in `docs/OPEN-QUESTIONS.md` and gated
    /// RED-by-design by `g17` in `tests/seam_by_id.rs`. MEASURED: typed remove
    /// fires 1, by-id remove fires 0, and the by-id call still returns `true`.
    /// The attach direction DOES fire them — its helper
    /// (`migrate_entity_attach_ids_with_bytes`) is this rung's own sibling with
    /// no other caller.
    pub fn remove_component_by_id(&mut self, entity: Entity, id: ComponentId) -> bool {
        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return false;
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return false;
            }
            *slot
        };

        if id.0 >= MAX_COMPONENTS {
            return false;
        }
        match component_registry::storage_kind(id.0) {
            // Dense: `dense_remove_and_fire` probes presence WITHOUT creating a
            // store, fires pre-tombstone, and returns `false` for an absent
            // member — its return value IS this fn's return value. Routing the
            // dense case through the signature probe instead would return a
            // silent `false` for every dense component, indistinguishable from
            // "absent".
            //
            // The DRAIN is the SEAM's job, and it is the half with observable
            // consequences: `dense_remove_and_fire`'s other caller is
            // `RemoveCommand::apply`, which already runs at depth >= 1 inside
            // the command queue and correctly owns nothing. Without a drain
            // here the hook's deferred command is left STRANDED — not lost, but
            // applied by whichever self-draining direct API runs next, i.e.
            // mis-attributed in time to an unrelated operation (gate 15).
            //
            // The BRACKET is uniformity, not mechanism, and saying otherwise
            // would misdirect the next author. It has no observable effect
            // during the fire, and that is structural rather than incidental: a
            // hook receives a `DeferredEcsMaster`, whose entire surface is reads
            // plus `DeferredCommands`, every method of which is one
            // `deferred_hook_queue.push` — there is no route from a hook body to
            // `&mut EcsMaster` and therefore none to `drain_deferred_hook_queue`,
            // the ONLY reader of the depth (VERIFIED: one non-test reader of
            // `hook_drain_depth()` in the crate). It is kept because all 13
            // `DeferredScopeGuard::enter()` sites in this crate's non-test source
            // bracket (`entity_api.rs` x5, this file x3, `tag_api.rs` x2,
            // `schedule.rs` x2, plus `drain_deferred_hook_queue`'s own), because
            // ARM 6's "a call that mutates nothing must not enter a deferred
            // scope" rule is only expressible if a bracket exists to place, and
            // because the guard is a ZST plus two TLS `Cell` ops. `g15b` pins
            // that it is there.
            StorageKind::Dense => {
                // Presence FIRST, so a call that mutates nothing never enters a
                // deferred scope — this fn's own arm-6 rule and `remove_tag`'s
                // bracket order.
                if !self.dense_contains(entity, id) {
                    return false;
                }
                let scope = DeferredScopeGuard::enter();
                let removed = self.dense_remove_and_fire(entity, id);
                debug_assert!(
                    removed,
                    "dense_contains / dense_remove_and_fire disagree on presence"
                );
                drop(scope);
                self.drain_deferred_hook_queue();
                removed
            }
            StorageKind::Table => {
                // SAFETY: same rationale as `add_component_by_id`'s presence
                //   read and `remove_tag`'s — shared reads through stable,
                //   interior-mutable (`SharedReadWrite`, F4-rooted) slab
                //   provenance; non-null + generation-matched ⇒ live; no `&mut`.
                let (source_archetype_id, present) = unsafe {
                    let archetype = &*inland.archetype_ptr();
                    (archetype.id(), archetype.has_component_id(id))
                };
                if !present {
                    return false; // W1: absent ⇒ silent no-op, decided on the signature
                }

                let scope = DeferredScopeGuard::enter();
                let target_archetype_id = without_ids_archetype_id(self, source_archetype_id, &[id]);
                // `id ∈ source` ⇒ `kept` is strictly smaller ⇒ a distinct
                // exact-mask match (the EMPTY archetype when it was the last).
                debug_assert_ne!(
                    target_archetype_id, source_archetype_id,
                    "without_ids_archetype_id returned the source for a strictly-shrinking set"
                );
                migrate_entity_detach_ids(
                    self,
                    entity,
                    source_archetype_id,
                    target_archetype_id,
                    &[id],
                );
                drop(scope);
                self.drain_deferred_hook_queue();
                true
            }
            // `Bitset` (an enable tag) and any future kind: not detachable.
            _ => false,
        }
    }

    /// Stamps `id`'s changed tick on `entity` with the world's current tick
    /// (§4 S3) — the write twin of
    /// [`EcsMaster::get_component_changed_tick`](Self::get_component_changed_tick),
    /// which any by-id writer (`set_field`, scene apply, replication) needs
    /// because a raw-pointer write touches no tick.
    ///
    /// Returns `false` for a dead / stale entity, an out-of-range or
    /// non-table/dense id, or a component the entity does not host.
    ///
    /// ⚠️ The twin is ASYMMETRIC: the READ has no dense arm (it resolves
    /// `pools.get_pool(id)?` and a dense id owns no per-archetype pool), while
    /// this write has one. A dense write is therefore observable only through a
    /// `Changed<T>` query under `Schedule::run` — see `tests/seam_by_id.rs`.
    ///
    /// Stamps `changed` ONLY. `added` is preserved, which is why
    /// `stamp_slot_ticks` / `fill_ticks` are not the mechanism: they write both.
    pub fn mark_component_changed(&mut self, entity: Entity, id: ComponentId) -> bool {
        let current_tick = self.current_tick();
        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return false;
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return false;
            }
            *slot
        };
        if id.0 >= MAX_COMPONENTS {
            return false;
        }

        match component_registry::storage_kind(id.0) {
            StorageKind::Table => {
                let archetype_ptr = inland.archetype_ptr();
                // Projected rather than struct-wide, for two reasons — neither
                // of which is "the struct-wide form is UB".
                // `add_component_by_id` / `remove_component_by_id` / `add_tag` /
                // `remove_tag` all form `&*archetype_ptr`, and Miri under Tree
                // Borrows is CLEAN over gate 1 end to end — including the
                // sibling `current_index` write through a same-cell-derived
                // pointer and the `Box`-of-slab dealloc at world drop:
                //   MIRIFLAGS=-Zmiri-tree-borrows cargo miri test -p boyko-ecs \
                //     --test seam_by_id g1_table_add_...
                //   -> test result: ok. 1 passed; 0 failed
                // (the two `memory leaked` lines that invocation also prints are
                // the pre-existing process-lifetime `BundleColumnCache::
                // resolve_and_cache` memo, unrelated to this fn — read the
                // verdict off `test result:`, never off the exit code).
                //
                // 1. This fn is the WRITE twin of `get_component_changed_tick`
                //    and shares its prologue in shape; the twins must not
                //    diverge on the borrow form.
                // 2. `addr_of!((*p).component_pools)` is strictly narrower than
                //    a struct-wide `&Archetype`: it never covers
                //    `current_index`, so it cannot interact with
                //    BUG-MIGRATE-TB-1's cell at all. That is a DEFENSIVE
                //    NARROWING, not a correctness requirement.
                //
                // SAFETY (U1, U2, U4, F1): `archetype_ptr` is stable,
                //   interior-mutable (`SharedReadWrite`, F4-rooted) slab
                //   provenance — non-null + generation-matched above ⇒ the slot
                //   is live; `&mut self` gives exclusive access.
                //   `addr_of!((*p).component_pools)` reads only the cold pool
                //   table (never `current_index`), so the shared
                //   `&ComponentPoolBundle` narrows nothing a sibling migration
                //   writes — no freeze of the slab cell.
                let pools = unsafe { &*core::ptr::addr_of!((*archetype_ptr).component_pools) };
                let Some(pool) = pools.get_pool(id) else {
                    return false;
                };
                let row = inland.unit_index() as usize;
                if row >= pool.count() {
                    return false;
                }
                // SAFETY: `row < pool.count() <= committed_rows` (checked
                //   above), so the tick slot lies in the committed prefix of
                //   the pool's `changed` tick sub-region; `&mut self` ⇒
                //   exclusive access, no concurrent reader/writer in the
                //   single-threaded direct-API context (Phase 9 SCH3).
                unsafe { pool.write_changed_tick(row, current_tick) };
                true
            }
            StorageKind::Dense => {
                let Some(store) = self.dense_registry.store(id) else {
                    return false;
                };
                let Some(slot) = store.slot_of(entity.id()) else {
                    return false;
                };
                // SAFETY: `slot` came from `slot_of`, which only maps LIVE
                //   slots below the column's high-water mark, so `[slot]` lies
                //   in the committed prefix of the `changed` tick sub-region.
                //   The base pointer is address-stable for the store's lifetime
                //   (write-once vm-reservation base). `&mut self` ⇒ exclusive
                //   world access; the per-slot `UnsafeCell<Tick>` is the same
                //   cell `Mut<Dense>`'s deref guard writes, which is what makes
                //   this observable to a `Changed<T>` query.
                unsafe {
                    let cell: *const UnsafeCell<Tick> =
                        store.changed_ticks_ptr().add(slot as usize);
                    *(*cell).get() = current_tick;
                }
                true
            }
            // `Bitset` (an enable tag) owns no tick storage at all.
            _ => false,
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// GATE 15b — the three ARM-6 brackets are THERE.
//
// The depth they bump is `pub(crate)` and only readable from inside the crate,
// which is why this gate is a unit `mod` and not a row in `tests/seam_by_id.rs`
// — the same reason gate 10 lives inside `commands/migration_helpers.rs`.
//
// This gate exists because the bracket has NO other observable effect: no hook
// can reach `drain_deferred_hook_queue`, the depth's only reader (a hook holds a
// `DeferredEcsMaster`, whose whole mutating surface is
// `deferred_hook_queue.push`). A property that is stated in a comment and gated
// nowhere is the shape this campaign keeps finding; this is the check for the
// statement in `remove_component_by_id`'s dense arm.
//
// RED: delete `let scope = DeferredScopeGuard::enter();` and the matching
// `drop(scope);` from the arm under test.
// ARTIFACT: `assertion `left == right` failed ... left: 0  right: 1`.
// ═════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod bracket_gate {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
    use crate::ecs::core::component::hooks::scope::hook_drain_depth;
    use crate::ecs::core::component::hooks::{ComponentHooks, HookContext};

    const SEQ: Ordering = Ordering::SeqCst;

    // ── Fixed ids: grep-verified-free sub-block [356, 360) ───────────────────
    //
    // The shared lib-test process registers ComponentIds process-globally, so
    // these must be disjoint from EVERY other `#[cfg(test)]` module in the lib.
    // The highest fixed id in use below 400 is 354 (`iters/query`), and gate
    // 10's Step-6 module claims 335-339, so 356-359 is free.
    const ANCHOR: ComponentId = ComponentId(356);
    const TABLE_ADD: ComponentId = ComponentId(357);
    const DENSE_VAL: ComponentId = ComponentId(358);
    const TABLE_REM: ComponentId = ComponentId(359);

    /// `u32::MAX` sentinel, so "the hook never fired" is distinguishable from
    /// "the hook fired at depth 0" — the two outcomes a deleted bracket and a
    /// deleted fire would otherwise share.
    const NEVER: u32 = u32::MAX;

    static ADD_DEPTH: AtomicU32 = AtomicU32::new(NEVER);
    static DENSE_REMOVE_DEPTH: AtomicU32 = AtomicU32::new(NEVER);
    static TABLE_REMOVE_DEPTH: AtomicU32 = AtomicU32::new(NEVER);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Anchor(u32);
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TableAdd(u32);
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DenseVal(u32);
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TableRem(u32);

    unsafe fn add_probe(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
        ADD_DEPTH.store(hook_drain_depth(), SEQ);
    }
    unsafe fn dense_remove_probe(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
        DENSE_REMOVE_DEPTH.store(hook_drain_depth(), SEQ);
    }
    unsafe fn table_remove_probe(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
        TABLE_REMOVE_DEPTH.store(hook_drain_depth(), SEQ);
    }

    /// Registers layouts, the dense storage kind and the three probes. Hooks go
    /// in BEFORE any archetype is minted for these ids — the registry REFUSES
    /// with `AlreadyArchetyped` otherwise, and the `Result` is `.expect`ed so
    /// that refusal cannot be swallowed into a vacuous pass.
    fn register() {
        component_registry::register_layout::<Anchor>(ANCHOR.0);
        component_registry::register_layout::<TableAdd>(TABLE_ADD.0);
        component_registry::register_layout::<DenseVal>(DENSE_VAL.0);
        component_registry::register_layout::<TableRem>(TABLE_REM.0);
        component_registry::set_storage_kind(DENSE_VAL.0, StorageKind::Dense);
        component_registry::register_hooks_by_id(
            TABLE_ADD,
            ComponentHooks {
                on_add: Some(add_probe),
                ..Default::default()
            },
        )
        .expect("fresh id, never archetyped");
        component_registry::register_hooks_by_id(
            DENSE_VAL,
            ComponentHooks {
                on_remove: Some(dense_remove_probe),
                ..Default::default()
            },
        )
        .expect("fresh id, never archetyped");
        component_registry::register_hooks_by_id(
            TABLE_REM,
            ComponentHooks {
                on_remove: Some(table_remove_probe),
                ..Default::default()
            },
        )
        .expect("fresh id, never archetyped");
    }

    /// Byte view of a `#[repr(C)]` POD local for the by-id attach.
    ///
    /// # Safety
    /// `T` must be the `#[repr(C)]` type registered for the id it is paired
    /// with; the slice is consumed before `v` goes out of scope.
    unsafe fn pod_bytes<T: Copy>(v: &T) -> &[u8] {
        // SAFETY: caller contract — `T` is `#[repr(C)]` POD and the slice
        //   borrows `v` for strictly less than `v`'s lifetime.
        unsafe {
            core::slice::from_raw_parts((v as *const T).cast::<u8>(), core::mem::size_of::<T>())
        }
    }

    #[test]
    fn g15b_the_three_arm6_brackets_are_entered_before_their_fires() {
        register();
        let mut ecs = EcsMaster::new();

        let src = ecs.create_archetype(&[ANCHOR, TABLE_REM]);
        let a = Anchor(1);
        let r = TableRem(2);
        // SAFETY: both locals are the registered `#[repr(C)]` types for their
        //   ids and outlive the borrows.
        let (ab, rb) = unsafe { (pod_bytes(&a), pod_bytes(&r)) };
        let e = ecs
            .create_entity(src, &[(ANCHOR, ab), (TABLE_REM, rb)])
            .expect("spawn must succeed");

        // ── Bracket 1: `add_component_by_id`'s ARM 6. ───────────────────────
        let ta = TableAdd(3);
        // SAFETY: registered `#[repr(C)]` type for `TABLE_ADD`.
        let tab = unsafe { pod_bytes(&ta) };
        assert_eq!(
            ecs.add_component_by_id(e, TABLE_ADD, tab),
            AddOutcome::Attached,
            "precondition: the table attach lands"
        );
        assert_eq!(
            ADD_DEPTH.load(SEQ),
            1,
            "add_component_by_id must fire its on_add INSIDE the deferred scope \
             it opened (u32::MAX here would mean the hook never fired at all)"
        );

        // ── Bracket 2: `remove_component_by_id`'s DENSE arm. ────────────────
        let dv = DenseVal(4);
        // SAFETY: registered `#[repr(C)]` type for `DENSE_VAL`.
        let dvb = unsafe { pod_bytes(&dv) };
        assert_eq!(
            ecs.add_component_by_id(e, DENSE_VAL, dvb),
            AddOutcome::Attached,
            "precondition: the dense membership is established"
        );
        assert!(
            ecs.remove_component_by_id(e, DENSE_VAL),
            "precondition: a present dense member detaches"
        );
        assert_eq!(
            DENSE_REMOVE_DEPTH.load(SEQ),
            1,
            "the dense remove arm must fire its on_remove INSIDE the deferred \
             scope it opened"
        );

        // ── Bracket 3: `remove_component_by_id`'s TABLE arm. ────────────────
        assert!(
            ecs.remove_component_by_id(e, TABLE_REM),
            "precondition: a present table component detaches"
        );
        assert_eq!(
            TABLE_REMOVE_DEPTH.load(SEQ),
            1,
            "the table remove arm must fire its on_remove INSIDE the deferred \
             scope it opened"
        );

        // The brackets are BALANCED: every `enter()` was matched by a `drop`,
        // so the caller's thread is back at depth 0. A leaked increment would
        // silently disable the drain for the rest of the process.
        assert_eq!(
            hook_drain_depth(),
            0,
            "every ARM-6 bracket is dropped before the fn returns"
        );
    }
}
