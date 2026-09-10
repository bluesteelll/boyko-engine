//! [`run_check_ticks_scan`] — cold-path wraparound clamp for per-row ticks.
//!
//! See Phase 10 plan §2.7 WRAP1-WRAP5, §4.6, §9.6 (Round 2 W3 + Round 3
//! W-NEW-1 / W-NEW-1b API corrections), and §10.6 for cost analysis.
//!
//! # Why a cold-path scan exists
//!
//! `Tick::is_newer_than` interprets `wrapping_sub` differences as the
//! true elapsed-tick count under wraparound. The interpretation is
//! correct iff every stored tick stays within [`MAX_CHANGE_AGE`] of the
//! world's current tick. Without intervention, a tick stored
//! `MAX_CHANGE_AGE + 1` frames ago would underflow the comparison and
//! flip the semantic from "older than" to "newer than".
//!
//! [`Schedule::run`] calls [`EcsMaster::should_run_check_ticks`] every
//! frame; when it returns `true` (every ~100 days at 60 FPS per §9.3
//! analysis), it invokes [`run_check_ticks_scan`] to walk every live
//! per-slot tick in the world — both storage kinds — and clamp any whose
//! age has exceeded `MAX_CHANGE_AGE`.
//!
//! # Scope walked: TWO storages, because there are two (Dense plan D4)
//!
//! * every live `(archetype, component, row)` triple in table storage;
//! * every LIVE slot of every [`DenseStore`] in the world's
//!   [`DenseRegistry`].
//!
//! The second arm is not an extension of the first — it is the only route
//! to a dense component's ticks at all. A dense component owns no
//! per-archetype `ComponentPool` (its one column lives in the world-global
//! registry), so the archetype walk's `get_pool_mut` answers `None` for it
//! and skips it. Until the dense arm existed this module's scope read
//! "every live `(archetype, component, row)` triple", which was a complete
//! description of the world before dense storage and a description of half
//! of it afterwards: a dormant dense slot's tick aged without limit and
//! `Tick::is_newer_than` eventually INVERTED on it, reporting an untouched
//! component as `Changed` to a query that never wrote it — a silent wrong
//! answer, with no error and no counter.
//!
//! # Live rows only (Round 2 W3 for tables; the same rule for dense)
//!
//! Only the first `pool.count()` rows in each `ComponentPool` are
//! scanned. Unused slots above `count()` remain at `Tick::ZERO` and have
//! no semantic meaning (no system reads them — the column's data buffer
//! is untouched beyond `count()`). A dense store's high-water mark
//! (`len()`) covers TOMBSTONED slots as well, so the dense arm filters on
//! [`DenseStore::is_live`] — the same oracle `for_each_live` and the saver
//! use.
//!
//! [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
//! [`DenseStore`]: crate::ecs::core::component::dense::DenseStore
//! [`DenseStore::is_live`]: crate::ecs::core::component::dense::DenseStore
//! [`DenseRegistry`]: crate::ecs::core::component::dense::DenseRegistry
//! [`EcsMaster::should_run_check_ticks`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::should_run_check_ticks
//! [`MAX_CHANGE_AGE`]: super::MAX_CHANGE_AGE

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

/// Walks every live per-row `added`/`changed` tick and clamps anything
/// older than [`MAX_CHANGE_AGE`] against `world.current_tick()`.
///
/// Runs on the dispatcher inside the apply window (no workers live; the
/// `&mut EcsMaster` borrow is exclusive — plan §2.7 WRAP4 / §8.1 atomic
/// discipline). Cost: O(live_stored_ticks). At the design point
/// (100 k entities × 50 components × 2 ticks/row) ~10 M operations ≈ 3 ms
/// cold. Frequency: every `CHECK_TICK_THRESHOLD` ticks ≈ ~100 days at
/// 60 FPS (plan §9.3 / §10.6).
///
/// After the scan, `Schedule::run` writes
/// `world.set_last_check_tick(world.current_tick())` to reset the
/// wraparound counter (plan §2.7 WRAP1).
///
/// # Iteration shape (Round 2 W3 + Round 3 W-NEW-1 / W-NEW-1b)
///
/// * Archetypes via [`ArchetypeMaster::iter_archetypes_mut`] — the new
///   one-liner mirror of the existing `iter_archetypes()`.
/// * Per archetype, every declared `ComponentId` (cloned out of the
///   archetype's id slice before the pool reborrow so the iterator and
///   the pool reference don't both borrow the archetype mutably).
/// * Per pool, rows `0..pool.count()` only — never the full buffer
///   (Round 2 W3 correctness + cost reduction).
/// * Then every dense store via `DenseRegistry::dense_ids()`, slots
///   `0..store.len()` filtered by `is_live` (see the module header).
///
/// # Two arms, ONE policy
///
/// Both arms clamp the SAME two tick arrays (`added` AND `changed`) with
/// the same [`Tick::check_tick`] call and the same live-rows-only rule.
/// The divergence between storage kinds IS the defect class the dense arm
/// closes, so a second policy here would re-open it in a new shape.
///
/// # No reporting surface, deliberately
///
/// The scan returns `()` and moves no counter — it never had one, and the
/// dense arm does not invent one. (The sibling dense fix in
/// `serialize::load_writer` DID add a per-arm counter pair, because its
/// caller surfaces a load report where "nothing opted in" and "the pass
/// cannot see dense storage" were indistinguishable.) Here the gate against
/// that blindness is the pair of tests at the bottom of this file: the
/// table fixture is the control that passes either way, and the dense
/// fixture is red without this arm. A frame-path counter incremented once
/// per ~100 days would be read by nobody.
///
/// [`MAX_CHANGE_AGE`]: super::MAX_CHANGE_AGE
/// [`Tick::check_tick`]: super::Tick::check_tick
/// [`ArchetypeMaster::iter_archetypes_mut`]: crate::ecs::core::archetype::archetype_master::ArchetypeMaster::iter_archetypes_mut
#[cold]
#[inline(never)]
pub(crate) fn run_check_ticks_scan(world: &mut EcsMaster) {
    let current = world.current_tick();

    let archetype_master = world.archetype_master_mut();
    for archetype in archetype_master.iter_archetypes_mut() {
        // KE6 — the archetype-level `ArchAdded` stamp ages out on exactly the
        // same axis as the per-row ticks below, and for a stamp the risk is
        // worse than for a row: a dormant archetype (one that took a spawn
        // burst and was never touched again) never revisits its stamp, so
        // without this clamp its `is_newer_than` verdict would silently flip
        // from "very old" to "newer than now" once the age crossed
        // MAX_CHANGE_AGE. Clamped before the per-row walk so the archetype's
        // summary can never be older than the rows it summarises.
        archetype.clamp_arch_added(current);

        // The archetype's component id slice borrows `archetype` immutably,
        // and the subsequent `component_pools_mut()` borrows it mutably.
        // Materialise the id list onto the stack (cold path; allocation
        // budget is dominated by the per-row clamp work) to break the
        // borrow.
        let component_ids: Vec<_> = archetype.component_ids().to_vec();

        let pools = archetype.component_pools_mut();
        for component_id in component_ids {
            let Some(pool) = pools.get_pool_mut(component_id) else {
                continue;
            };
            // Round 2 W3 — live rows only. Anything at `>= count()` is
            // either unused buffer space (Tick::ZERO) or a slot that
            // `swap_remove` has logically released; clamping is harmless
            // but pointless.
            let live_count = pool.count();
            for i in 0..live_count {
                // SAFETY (W3 + STORE3 + SCH3 — plan §2.7 WRAP4, §8.1):
                //   `world: &mut EcsMaster` is the dispatcher's exclusive
                //   borrow inside the apply window; no worker holds a
                //   cell-mediated borrow on any tick column at this
                //   moment. `i < pool.count() <= committed_rows` (Phase X.I
                //   committed-prefix invariant, maintained by `grow_rows` /
                //   the removal paths), so both tick accesses stay inside
                //   the committed prefix of the pool's tick sub-regions.
                let added = unsafe { pool.read_added_tick(i) };
                let clamped = added.check_tick(current);
                if clamped != added {
                    // SAFETY: same conditions as the read above; the
                    //   write target is the slot we just read.
                    unsafe {
                        pool.write_added_tick(i, clamped);
                    }
                }

                // SAFETY: as above for the `changed_ticks` column.
                let changed = unsafe { pool.read_changed_tick(i) };
                let clamped = changed.check_tick(current);
                if clamped != changed {
                    // SAFETY: same conditions; `changed_ticks` slot
                    //   parallel to the one just read.
                    unsafe {
                        pool.write_changed_tick(i, clamped);
                    }
                }
            }
        }
    }

    // ── Dense arm (Dense plan D4 ticks) ─────────────────────────────────────
    //
    // NOT a second visit of anything the loop above touched, and the proof is
    // structural rather than a convention: a dense id gets NO per-archetype
    // `ComponentPool` at ANY of the three mint funnels — `Archetype::create_by_ids`,
    // `register_component` and `register_component_inplace` all screen on
    // `is_signature_storage` BEFORE `add_pool`. A spawn-built archetype does
    // still NAME the dense id in `component_ids()` (retained since Dense plan
    // D0), so the loop above reaches the id and then drops it at
    // `get_pool_mut(cid) == None`; a loaded archetype does not even name it.
    // Under either shape the dense column is unreachable from an archetype, so
    // this arm is purely ADDITIVE — no store can be clamped twice, and a double
    // clamp would in any case be idempotent (`check_tick` is a floor, not a
    // shift).
    //
    // The id list is FIXED for the duration of the scan (nothing here creates a
    // store), so it is re-read per turn under a short shared borrow instead of
    // being copied out: unlike the archetype arm — whose `to_vec` exists to
    // break the borrow between one archetype's id slice and its own pool bundle
    // — the ids and the stores are reachable through separate borrows of the
    // world, so the cold path can stay allocation-free.
    let dense_count = world.dense_registry().dense_ids().len();
    for i in 0..dense_count {
        let component_id = world.dense_registry().dense_ids()[i];
        let Some(store) = world.dense_registry_mut().store_existing_mut(component_id) else {
            continue;
        };

        // `len()` is the column HIGH-WATER MARK, not the live count: a dense
        // store tombstones on remove (`DenseStore::remove` runs the registered
        // `drop_fn` and clears the live bit), leaving the slot's bytes logically
        // uninitialised below the mark. Its ticks are then meaningless — no
        // query can reach them, since `Added`/`Changed`/`Mut` resolve a slot
        // through the membership map, which no longer answers for that entity —
        // so the arm filters on `is_live`, the same oracle `for_each_live` and
        // the saver use.
        let slot_count = store.len();
        for slot in 0..slot_count {
            if !store.is_live(slot) {
                continue;
            }

            // SAFETY (D4 + the WRAP4 / §8.1 discipline of the arm above):
            //   `world: &mut EcsMaster` is the dispatcher's exclusive borrow
            //   inside the apply window, so no worker holds a cell-mediated
            //   borrow of this column's tick sub-regions. `slot < store.len()
            //   == column.count() <= committed_rows` (the Phase X.I
            //   committed-prefix invariant the dense column inherits from
            //   `ComponentPool`), so both `added_ticks_ptr()[slot]` and
            //   `changed_ticks_ptr()[slot]` lie inside the committed prefix.
            //   Both bases are the column's address-stable write-once
            //   reservation pointers — their provenance is the reservation, not
            //   this `&mut` reborrow — so writing back through
            //   `UnsafeCell::get()` is sound; `Tick` is `Copy` and each slot is
            //   a distinct memory location.
            unsafe {
                let added_cell = &*store.added_ticks_ptr().add(slot);
                let added = *added_cell.get();
                let clamped = added.check_tick(current);
                if clamped != added {
                    *added_cell.get() = clamped;
                }

                let changed_cell = &*store.changed_ticks_ptr().add(slot);
                let changed = *changed_cell.get();
                let clamped = changed.check_tick(current);
                if clamped != changed {
                    *changed_cell.get() = clamped;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::slice;
    use std::sync::Once;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::ecs::core::archetype::archetype::Archetype;
    use crate::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::entity::entity::Entity;
    use crate::ecs::core::iters::query::{Changed, Query};
    use crate::ecs::core::system::{IntoSystem, System};
    use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

    /// Creates a **component-free** archetype holding one row, and returns its
    /// id.
    ///
    /// ⚠ The component-free part is deliberate and was arrived at by
    /// measurement. The first version of these tests declared a probe component
    /// with a hand-picked `ComponentId`, following the convention in
    /// `enable_tag_api::tests`. That convention has a failure mode the filtered
    /// run cannot show: ids are picked by hand across ~111 sites in this crate,
    /// the registry refuses a second type on an occupied slot, and the panic
    /// surfaces only when the whole lib binary runs (measured — id 331 was
    /// already `archetype::tests::ResComp`). Since the property under test is
    /// **archetype-level**, the probe needs no component at all, and dropping it
    /// removes the collision class rather than moving it to a different number
    /// that a sibling rung might claim next.
    fn empty_archetype_with_one_row(world: &mut EcsMaster) -> ArchetypeId {
        let e = world.spawn_empty();
        world
            .entity_archetype_id(e)
            .expect("a freshly spawned entity is live")
    }

    fn stamp_of(world: &EcsMaster, arch_id: ArchetypeId) -> Tick {
        world
            .archetype_master()
            .get_archetype(arch_id)
            .expect("archetype resolves")
            .arch_added()
    }

    fn set_stamp(world: &mut EcsMaster, arch_id: ArchetypeId, tick: Tick) {
        let arch: &mut Archetype = world
            .archetype_master_mut()
            .get_archetype_mut(arch_id)
            .expect("archetype resolves");
        arch.stamp_arch_added(tick);
    }

    /// **KE6, the clamp half of the oracle.** `run_check_ticks_scan` must pull
    /// an aged-out `ArchAdded` stamp back to the oldest still-valid tick,
    /// exactly as it already does for the per-row `added`/`changed` columns.
    ///
    /// # Why the stamp needs this more than a row does
    ///
    /// A per-row tick belongs to a row that is, by construction, part of a live
    /// archetype somebody is iterating. The `ArchAdded` stamp of a **dormant**
    /// archetype — one that took a spawn burst and was never touched again — is
    /// never rewritten by anything. It is therefore the value most likely to
    /// age past `MAX_CHANGE_AGE`, and `Tick::is_newer_than` reads a wrapped
    /// difference as an elapsed count: an unclamped stamp does not degrade
    /// gracefully, it **inverts**, turning a permanent skip into a permanent
    /// non-skip or the reverse.
    ///
    /// The arrangement inverts the usual one for convenience: rather than
    /// advancing the world tick past `MAX_CHANGE_AGE` (4 billion bumps), the
    /// stamp is written *ahead* of a fresh world's `current_tick == 0`, which
    /// is the same wrapped-age condition (`0.wrapping_sub(1000)` is enormous).
    ///
    /// Red without the `archetype.clamp_arch_added(current)` line in
    /// `run_check_ticks_scan`: the stamp stays at 1000.
    #[test]
    fn check_ticks_clamps_the_arch_added_stamp() {
        let mut world = EcsMaster::new();
        let arch_id = empty_archetype_with_one_row(&mut world);

        // An "ancient" stamp, expressed as a tick far AHEAD of the world's
        // current 0 — `current.wrapping_sub(stamp)` is then huge, which is the
        // condition `check_tick` clamps on.
        const AHEAD: u32 = 1_000;
        set_stamp(&mut world, arch_id, Tick::new(AHEAD));

        let current = world.current_tick();
        assert_eq!(current, Tick::ZERO, "a fresh world starts at tick 0");
        assert!(
            current.get().wrapping_sub(AHEAD) > MAX_CHANGE_AGE,
            "precondition: the stamp's wrapped age must exceed MAX_CHANGE_AGE, \
             or the scan has nothing to clamp and this test is vacuous",
        );

        run_check_ticks_scan(&mut world);

        let clamped = stamp_of(&world, arch_id);
        assert_eq!(
            clamped,
            Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE)),
            "the aged-out stamp must be pulled to the oldest still-valid tick",
        );
        assert_ne!(
            clamped,
            Tick::new(AHEAD),
            "…and must not have been left untouched",
        );
    }

    /// The over-correction guard: a stamp that is NOT aged out must survive the
    /// scan byte-identical. Without it, `clamp_arch_added` could
    /// unconditionally overwrite the stamp and still pass the test above.
    ///
    /// ⚠ The stamp is written **explicitly**, not left to the spawn. An earlier
    /// draft relied on the spawn's own stamp and was measured **vacuous**: a
    /// fresh world sits at `current_tick == 0` and an unmaintained stamp also
    /// reads `Tick::ZERO`, so `assert_eq!(after, before)` held against an
    /// implementation that stamped nothing at all — it passed on the red run
    /// that failed every other test in this pair. `u32::MAX - 10` is a value no
    /// code path could have produced, and its wrapped age against `current == 0`
    /// is 11, comfortably inside `MAX_CHANGE_AGE`, so the scan must return it
    /// untouched.
    #[test]
    fn check_ticks_leaves_a_fresh_arch_added_stamp_alone() {
        let mut world = EcsMaster::new();
        let arch_id = empty_archetype_with_one_row(&mut world);

        const FRESH: u32 = u32::MAX - 10;
        set_stamp(&mut world, arch_id, Tick::new(FRESH));

        let current = world.current_tick();
        assert!(
            current.get().wrapping_sub(FRESH) <= MAX_CHANGE_AGE,
            "precondition: the stamp must be INSIDE the valid window, or this \
             guard is testing the clamp path instead of the pass-through one",
        );

        run_check_ticks_scan(&mut world);

        let after = stamp_of(&world, arch_id);
        assert_eq!(
            after,
            Tick::new(FRESH),
            "a stamp within MAX_CHANGE_AGE of `current` is returned unchanged",
        );
        assert_ne!(
            after,
            Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE)),
            "…and specifically was NOT pulled to the clamp value",
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // The DENSE arm — the second storage kind that keeps its own per-slot ticks
    // ════════════════════════════════════════════════════════════════════════

    /// Hand-picked ids for the two fixtures below, grep-verified free across the
    /// whole crate (`423..=459` carries no `ComponentId(n)` literal and no bare
    /// id argument).
    ///
    /// ⚠ Hand-picked rather than minted, which is the opposite of the reasoning
    /// in [`empty_archetype_with_one_row`] and does not contradict it. That doc
    /// records the collision class of picking a number a sibling rung already
    /// claims; `register_new` has the mirror-image failure, because it walks a
    /// process-global counter up from 0 through exactly the low range this
    /// crate's hand-picked ids occupy, and a mint that lands on an occupied slot
    /// panics the whole test BINARY rather than this test. A dense fixture needs
    /// a real registered id (the storage-kind classification is keyed by it), so
    /// the choice is between the two hazards, and a verified-free high number is
    /// the one whose failure is visible to a grep.
    const CLAMP_TABLE_ID: ComponentId = ComponentId(440);
    const CLAMP_DENSE_ID: ComponentId = ComponentId(441);

    /// The clock value the fixtures are aged against: three ticks short of the
    /// `u32` wrap.
    ///
    /// Far enough past `MAX_CHANGE_AGE` that a stamp of `Tick::ZERO` is aged out
    /// (the scan's precondition), and close enough to the wrap that
    /// [`a_dormant_dense_slot_is_not_changed_after_the_tick_wraps`] can roll the
    /// counter over to 0 while staying inside a single `CHECK_TICK_THRESHOLD`
    /// interval of this clamp — i.e. modelling ONE missed clamp, not several.
    const AGED_CURRENT: u32 = u32::MAX - 2;

    /// A plain TABLE component — the CONTROL. Its per-row ticks live in the
    /// archetype's `ComponentPool` and have been clamped since Wave D.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ClampTable {
        v: u32,
    }

    /// A DENSE component — the subject. Its per-slot ticks live in the
    /// world-global `DenseStore` column (Dense plan D4) and are read by
    /// `Added` / `Changed` / `Mut`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ClampDense {
        v: u32,
    }

    impl Component for ClampTable {
        fn component_id() -> ComponentId {
            CLAMP_TABLE_ID
        }
    }

    impl Component for ClampDense {
        fn component_id() -> ComponentId {
            CLAMP_DENSE_ID
        }

        const STORAGE_IS_DENSE: bool = true;
    }

    /// Registers both fixtures once per process — the hand-written mirror of
    /// what `#[derive(Component)]` emits (`boyko_macros` is a dev-dependency, so
    /// the derive cannot be used from `src/`). `install_dense_storage_kind` is
    /// the half that classifies the id: without it `ClampDense` would be minted
    /// as ordinary table storage and the archetype would grow a column for it,
    /// which is precisely the premise the double-visit proof rests on.
    fn register_clamp_components() {
        static REGISTERED: Once = Once::new();
        REGISTERED.call_once(|| {
            component_registry::register_layout::<ClampTable>(CLAMP_TABLE_ID.0);
            component_registry::register_layout::<ClampDense>(CLAMP_DENSE_ID.0);
            component_registry::install_dense_storage_kind::<ClampDense>(CLAMP_DENSE_ID.0);
        });
    }

    /// Reinterprets a fixture as the `&[u8]` blob the direct `create_entity` API
    /// takes.
    fn as_bytes<T: Copy>(value: &T) -> &[u8] {
        // SAFETY: both call sites pass a `#[repr(C)]` struct of a single `u32`,
        //   so all `size_of::<T>()` bytes are initialised (no padding, no uninit
        //   niche) and every bit pattern is a valid `u8`. The returned slice's
        //   lifetime is tied to `value`'s borrow, so the bytes cannot be moved
        //   or dropped while it is live, and it is read-only.
        unsafe { slice::from_raw_parts(std::ptr::from_ref(value).cast::<u8>(), size_of::<T>()) }
    }

    /// Spawns ONE entity carrying a table component AND a dense component, both
    /// stamped at the world's current tick, and returns it.
    fn spawn_table_plus_dense(world: &mut EcsMaster) -> Entity {
        register_clamp_components();
        let archetype = world.get_or_create_archetype(&[CLAMP_TABLE_ID, CLAMP_DENSE_ID]);
        let table = ClampTable { v: 7 };
        let dense = ClampDense { v: 9 };
        world
            .create_entity(
                archetype,
                &[
                    (CLAMP_TABLE_ID, as_bytes(&table)),
                    (CLAMP_DENSE_ID, as_bytes(&dense)),
                ],
            )
            .expect("invariant: the archetype was just minted and both ids are registered")
    }

    /// The `(added, changed)` pair of the fixture's single TABLE row.
    fn table_ticks(world: &mut EcsMaster, archetype: ArchetypeId) -> (Tick, Tick) {
        let pool = world
            .archetype_master_mut()
            .get_archetype_mut(archetype)
            .expect("invariant: the archetype resolves")
            .component_pools_mut()
            .get_pool_mut(CLAMP_TABLE_ID)
            .expect("invariant: a signature-storage component owns a per-archetype pool");
        debug_assert_eq!(pool.count(), 1, "the fixture spawns exactly one row");
        // SAFETY: row 0 is the fixture's single live row (`count() == 1`), so it
        //   lies inside the committed prefix of both tick sub-regions, and the
        //   `&mut EcsMaster` borrow is exclusive — no worker holds a
        //   cell-mediated borrow of this column.
        unsafe { (pool.read_added_tick(0), pool.read_changed_tick(0)) }
    }

    /// The `(added, changed)` pair of the fixture's single DENSE slot.
    fn dense_ticks(world: &EcsMaster, entity: Entity) -> (Tick, Tick) {
        let store = world
            .dense_registry()
            .store(CLAMP_DENSE_ID)
            .expect("invariant: the dense insert created the store");
        let slot = store
            .slot_of(entity.id())
            .expect("invariant: the entity is a live member of the dense store")
            as usize;
        // SAFETY: `slot` came out of the store's own membership map, so it is
        //   live and `< len() <= committed_rows` — inside the committed prefix
        //   of both tick sub-regions. The `&EcsMaster` borrow means no writer is
        //   in flight, and `Tick` is `Copy`.
        unsafe {
            (
                *(*store.added_ticks_ptr().add(slot)).get(),
                *(*store.changed_ticks_ptr().add(slot)).get(),
            )
        }
    }

    /// Builds a one-shot system WITHOUT running it, so the caller can install an
    /// explicit `(last_run, this_run]` window before the single dispatch. Pins
    /// `In = ()` / `Out = ()` so the `Marker` parameter infers from the closure.
    fn into_unit_system<F, M>(body: F) -> F::System
    where
        F: IntoSystem<(), (), M>,
    {
        F::into_system(body)
    }

    /// **The dense arm, mechanism half.** `run_check_ticks_scan` must clamp a
    /// dense store's per-slot `added` / `changed` ticks exactly as it clamps a
    /// table column's per-row ones.
    ///
    /// The table fixture is the CONTROL: it passes with or without the dense
    /// arm, so a red here is the dense storage kind and not the harness.
    ///
    /// Red before the dense arm existed: the dense slot's ticks stay at
    /// `Tick::ZERO` while the table row's move to `current - MAX_CHANGE_AGE`.
    #[test]
    fn check_ticks_clamps_dense_slot_ticks() {
        let mut world = EcsMaster::new();
        let entity = spawn_table_plus_dense(&mut world);
        let archetype = world
            .entity_archetype_id(entity)
            .expect("a freshly spawned entity is live");

        // The premise the dense arm rests on, ASSERTED rather than assumed — and
        // the two halves point in opposite directions on purpose. A SPAWN-built
        // archetype RETAINS the dense id in `component_ids()` (so the table walk
        // does reach the id) but owns NO per-archetype `ComponentPool` for it
        // (so `get_pool_mut` drops it). That second fact is what makes the dense
        // pass purely ADDITIVE: no store can be visited by both arms.
        assert!(
            world
                .archetype_master()
                .get_archetype(archetype)
                .expect("invariant: the archetype resolves")
                .component_ids()
                .contains(&CLAMP_DENSE_ID),
            "premise: a spawn-built archetype RETAINS a dense id in component_ids()",
        );
        assert!(
            world
                .archetype_master_mut()
                .get_archetype_mut(archetype)
                .expect("invariant: the archetype resolves")
                .component_pools_mut()
                .get_pool_mut(CLAMP_DENSE_ID)
                .is_none(),
            "premise: a dense id owns NO per-archetype ComponentPool, so the \
             archetype arm skips it and the dense pass cannot double-visit it",
        );

        assert_eq!(
            table_ticks(&mut world, archetype),
            (Tick::ZERO, Tick::ZERO),
            "the fixture is spawned into a fresh world, so both table ticks are 0",
        );
        assert_eq!(
            dense_ticks(&world, entity),
            (Tick::ZERO, Tick::ZERO),
            "…and so are both dense ticks",
        );

        // Drive the clock rather than wait for it: one store IS the ~100-day
        // dormancy the scan exists for (§9.3), with no 4-billion-frame loop.
        world.change_tick.store(AGED_CURRENT, Ordering::Relaxed);
        let current = world.current_tick();
        assert!(
            current.get().wrapping_sub(Tick::ZERO.get()) > MAX_CHANGE_AGE,
            "precondition: both stamps must be aged out, or the scan has nothing \
             to clamp and this test is vacuous",
        );

        run_check_ticks_scan(&mut world);

        let expected = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
        assert_eq!(
            table_ticks(&mut world, archetype),
            (expected, expected),
            "CONTROL: the TABLE row's added/changed ticks are pulled to the \
             oldest still-valid tick — this half passed before the dense arm",
        );
        assert_eq!(
            dense_ticks(&world, entity),
            (expected, expected),
            "the DENSE slot's added/changed ticks must be clamped by the SAME \
             policy: a dense store keeps its own per-slot ticks and they are \
             live inputs to Added/Changed/Mut",
        );
    }

    /// **The dense arm, path half.** After the clamp, a real `Changed<Dense>`
    /// query must NOT match a dormant dense component once the world's tick
    /// counter wraps.
    ///
    /// # Why the query and not the tick
    ///
    /// The tick is the mechanism; the query is the path a user stands on. The
    /// clamp's whole purpose is that `Tick::is_newer_than` reads a wrapped
    /// difference as an elapsed count, so an unclamped stamp does not degrade —
    /// it INVERTS. Here the inversion is exact and visible: an unclamped stamp
    /// still reads `Tick::ZERO`, the world's counter wraps to 0, and the dormant
    /// slot's age becomes 0 — "changed this very frame", to a system that never
    /// touched it. No error, no counter, no log line.
    ///
    /// Red before the dense arm existed: `DENSE_HITS == 1`.
    #[test]
    fn a_dormant_dense_slot_is_not_changed_after_the_tick_wraps() {
        let mut world = EcsMaster::new();
        let entity = spawn_table_plus_dense(&mut world);
        assert_eq!(
            dense_ticks(&world, entity),
            (Tick::ZERO, Tick::ZERO),
            "the dormant slot's stamps start at 0 and nothing below touches them",
        );

        world.change_tick.store(AGED_CURRENT, Ordering::Relaxed);
        run_check_ticks_scan(&mut world);
        world.set_last_check_tick(world.current_tick());

        // Three more ticks pass and the counter WRAPS to 0 — one tick past the
        // last clamp's window, inside a single CHECK_TICK_THRESHOLD interval.
        world.change_tick.store(0, Ordering::Relaxed);

        static DENSE_HITS: AtomicUsize = AtomicUsize::new(0);
        static TABLE_HITS: AtomicUsize = AtomicUsize::new(0);
        DENSE_HITS.store(0, Ordering::Relaxed);
        TABLE_HITS.store(0, Ordering::Relaxed);

        let mut system = into_unit_system(
            |dense: Query<&ClampDense, Changed<ClampDense>>,
             table: Query<&ClampTable, Changed<ClampTable>>| {
                for _ in &dense {
                    DENSE_HITS.fetch_add(1, Ordering::Relaxed);
                }
                for _ in &table {
                    TABLE_HITS.fetch_add(1, Ordering::Relaxed);
                }
            },
        );
        system.initialize(&mut world);

        // The window a scheduler installs on the wrap frame — one tick wide,
        // `(u32::MAX, 0]`. Set through the scheduler's OWN call
        // (`Schedule::run` does `set_change_ticks(prev_this_run, this_run)`)
        // rather than by running a `Schedule`, because the frames between the
        // clamp and the wrap number in the billions — the same reason the scan
        // itself is tested against a driven clock.
        system.set_change_ticks(Tick::new(u32::MAX), Tick::ZERO);
        world.run_system_once(&mut system);

        assert_eq!(
            DENSE_HITS.load(Ordering::Relaxed),
            0,
            "a dormant DENSE component must not report itself Changed after the \
             wrap — with an unclamped slot its stamp still reads 0, which equals \
             `this_run`, and the query answers 'changed this frame'",
        );
        assert_eq!(
            TABLE_HITS.load(Ordering::Relaxed),
            0,
            "CONTROL: the TABLE row was clamped and does not match",
        );

        // Anti-vacuity: the SAME query path must be able to answer "changed", or
        // a query that silently matched nothing (unmatched archetype, unresolved
        // dense store) would read as a pass above. A window opening one tick
        // before the clamped stamp contains it, so both rows must match.
        let clamped = AGED_CURRENT.wrapping_sub(MAX_CHANGE_AGE);
        DENSE_HITS.store(0, Ordering::Relaxed);
        TABLE_HITS.store(0, Ordering::Relaxed);
        system.set_change_ticks(Tick::new(clamped.wrapping_sub(1)), Tick::ZERO);
        world.run_system_once(&mut system);

        assert_eq!(
            DENSE_HITS.load(Ordering::Relaxed),
            1,
            "liveness: a window containing the slot's stamp MUST match it, or \
             the zero above is a query that sees nothing rather than a clamp",
        );
        assert_eq!(
            TABLE_HITS.load(Ordering::Relaxed),
            1,
            "liveness: the same for the table control",
        );
    }
}
