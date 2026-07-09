//! Thread-local reentrancy depth for deferred-hook draining (Phase 14a,
//! plan §8 P1 / Q-A1 / fixes F1 + F2).
//!
//! # Why thread-local instead of a world field
//!
//! The depth counter previously lived in `EcsMaster::hook_drain_depth` and the
//! RAII guard cached a `NonNull<EcsMaster>` minted from `&mut *self`. Under
//! Tree Borrows (`MIRIFLAGS=-Zmiri-tree-borrows`, mandated by the project),
//! the bracketed method body's own `self.`-reborrows are foreign accesses that
//! transition the guard's cached child tag to `Frozen`; the guard's `Drop`
//! then wrote the decrement through that frozen tag → UB (F2).
//!
//! Moving the counter to a per-thread [`Cell`] removes the cached pointer
//! entirely: the guard reads/writes only TLS, never any field of `EcsMaster`,
//! so no `&mut *self` reborrow can freeze it. All hook firing + draining runs
//! on the single-threaded apply window / direct-API caller thread (SAFETY-4),
//! so a per-thread counter is the correct granularity.
//!
//! This mirrors the established `IN_SYSTEM_RUN` thread-local + `InSystemRunGuard`
//! pattern (`crates/boyko_threadpool/src/tls.rs:37,142-166`) — the exact
//! precedent, and TB-clean.

use core::cell::Cell;
use core::marker::PhantomData;

thread_local! {
    /// Reentrancy depth for deferred-hook draining (Q-A1). Thread-local because
    /// all hook firing + draining happens on the single-threaded apply window /
    /// direct-API caller thread (SAFETY-4); a per-thread counter cannot be
    /// frozen by any `&mut EcsMaster` reborrow (fixes F2's Tree Borrows UB).
    static HOOK_DRAIN_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// RAII depth bracket for the deferred-hook reentrancy counter.
///
/// [`enter`](Self::enter) bumps the thread-local depth; `Drop` decrements it on
/// EVERY exit path (`Ok` / `Err` / panic), so the depth can never leak — a
/// leaked increment would silently disable the drain for the rest of the
/// process. Mirrors the codebase's `InSystemRunGuard` (tls.rs:142-166) and
/// `CursorSync` (command_queue.rs) RAII discipline.
///
/// Holds NO `NonNull<EcsMaster>` (and no reference at all): the only state it
/// touches is the [`HOOK_DRAIN_DEPTH`] thread-local. Because the guard never
/// reads or writes any field of `EcsMaster`, a bracketed method body's own
/// `self.`-reborrows cannot freeze the guard's tag — this is the F2 fix.
///
/// Wired into the five bracket sites in Wave 4: the three direct-API methods
/// (`create_entity` / `create_entity_at` / `delete_entity`) and the two
/// schedule `system.apply` sites; plus `drain_deferred_hook_queue` brackets its
/// own walk (the F1 fix).
///
/// # Not `Send` / not `Sync`
///
/// The `PhantomData<*const ()>` field makes the guard `!Send + !Sync`: it is a
/// per-thread drop guard tied to TLS and must never cross a thread boundary
/// (it is always a stack local within a single-threaded deferred scope). The
/// raw-pointer marker enforces this at compile time without any runtime cost
/// (the guard remains a ZST).
pub(crate) struct DeferredScopeGuard(PhantomData<*const ()>);

impl DeferredScopeGuard {
    /// Enters a deferred-hook scope: bumps the thread-local depth and returns
    /// the bracket guard. The depth is restored by `Drop` on every exit path.
    #[inline]
    pub(crate) fn enter() -> Self {
        HOOK_DRAIN_DEPTH.with(|d| d.set(d.get() + 1));
        DeferredScopeGuard(PhantomData)
    }
}

impl Drop for DeferredScopeGuard {
    #[inline]
    fn drop(&mut self) {
        HOOK_DRAIN_DEPTH.with(|d| {
            let v = d.get();
            debug_assert!(v > 0, "deferred scope depth underflow");
            d.set(v - 1);
        });
    }
}

/// Returns the current thread's deferred-hook drain depth.
///
/// `drain_deferred_hook_queue` gates on `== 0` (Q-A1: only the outermost owner
/// drains). The read is a single thread-local load, off the per-entity
/// structural-op hot path — it runs once per direct-API call / per
/// `system.apply`, not per entity.
#[inline]
pub(crate) fn hook_drain_depth() -> u32 {
    HOOK_DRAIN_DEPTH.with(|d| d.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_round_trips_through_guard() {
        assert_eq!(hook_drain_depth(), 0);
        {
            let _g = DeferredScopeGuard::enter();
            assert_eq!(hook_drain_depth(), 1);
            {
                let _inner = DeferredScopeGuard::enter();
                assert_eq!(hook_drain_depth(), 2);
            }
            assert_eq!(hook_drain_depth(), 1);
        }
        assert_eq!(hook_drain_depth(), 0);
    }
}
