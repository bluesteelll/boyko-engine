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
                //   moment. `i < pool.count() <= pool.added_ticks.len()`
                //   by the parallel-array invariant in `ComponentPool::new`
                //   / `swap_remove_unit` (plan §2.2 STORE1 + STORE5).
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
