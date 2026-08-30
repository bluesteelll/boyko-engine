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
//! `(archetype, component, row)` triple and clamp any tick whose age has
//! exceeded `MAX_CHANGE_AGE`.
//!
//! # Scope walked (Round 2 W3 — live rows only)
//!
//! Only the first `pool.count()` rows in each `ComponentPool` are
//! scanned. Unused slots above `count()` remain at `Tick::ZERO` and have
//! no semantic meaning (no system reads them — the column's data buffer
//! is untouched beyond `count()`).
//!
//! [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
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
///
/// [`MAX_CHANGE_AGE`]: super::MAX_CHANGE_AGE
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::archetype::archetype::Archetype;
    use crate::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
    use crate::ecs::identifiers::primitives::ArchetypeId;

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
}
