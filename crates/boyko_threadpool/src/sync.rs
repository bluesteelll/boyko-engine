//! loom-aware synchronization shim (Phase 9.1, D1).
//!
//! This module re-exports the small set of synchronization primitives the
//! pool's own protocols use (`pending` counter, `idle` bitset, `shutdown`
//! flag, scope waker), routed through one of two backends selected at
//! compile time:
//!
//! - **`#[cfg(loom)]`** → `loom::sync::*` / `loom::thread::*`, so the loom
//!   model-checker (in `tests/loom_pool.rs`, `#![cfg(loom)]`) observes the
//!   *real* `Acquire`/`Release`/`AcqRel` orderings of the production
//!   `ScopeShared` / idle-bitset methods rather than a re-implementation.
//! - **`#[cfg(not(loom))]`** (the shipped artifact) → `core::sync::atomic::*`
//!   / `std::sync::*` / `std::thread::*`. Each `crate::sync::X` is then a
//!   transparent compile-time alias for the `core`/`std` item — byte-identical
//!   codegen, no indirection (Phase 9.1 §6 zero-native-cost contract).
//!
//! ## Scope of the shim (H1)
//!
//! Only the primitives that actually appear in the pool's *own* synchronization
//! protocols — the surface the loom models drive — are shimmed: the three
//! atomics in use (`AtomicU64`, `AtomicUsize`, `AtomicBool`) plus `Ordering`;
//! `fence`; `Mutex` / `Condvar`; and `thread::{self, Thread}` (the
//! `ScopeShared.waker` `Thread` + `current()` / `unpark()` on the wakeup
//! happens-before, and the loom `thread::park` used by the M1 join model).
//!
//! `fence` joined the shim with KE16 W-a: `worker::publish_fence` — the
//! `fence(SeqCst)` prologue of every wake decision — is production code, and
//! the loom M2 model drives it through `loom_exports::publish_fence`, so the
//! producer's half of the wake protocol's store-buffer litmus must be visible
//! to loom rather than modelled by a copy.
//!
//! Deliberately **not** shimmed:
//! - **`Arc`** — `loom::sync::Arc` does not implement `Arc<[T]>: From<Vec<T>>`,
//!   which `ThreadPool`'s `Arc<[Stealer]>` / `Arc<[CachePadded<…>]>` registries
//!   rely on, so a shimmed `Arc` would break the `--cfg loom` *lib* build. None
//!   of the loom models (M1/M2/M3) need a loom-instrumented `Arc` — they drive
//!   the atomic protocols over a toy queue, never `ThreadPoolBuilder::build`
//!   (which is crossbeam-coupled and loom-opaque, like the deque). Production
//!   therefore keeps `std::sync::Arc` directly; native codegen is unchanged.
//! - **`UnsafeCell`** — the crate holds none. The only interior cells are the
//!   thread-local `std::cell::Cell`s in `tls.rs`, which are per-thread and not
//!   a loom target. A future loom toy-queue that needs a cell must use
//!   `loom::cell::UnsafeCell` via its `.with()` / `.with_mut()` API (not a raw
//!   deref) — that would not be a pure `use`-swap and is out of scope here.
//! - **`park_timeout`** — loom has no equivalent; `scope.rs` keeps the
//!   fully-qualified `std::thread::park_timeout` (only ever reached on the
//!   native / Miri path, never under loom).
//! - **`JoinHandle`** / `thread::Builder` — only the crossbeam-coupled
//!   `ThreadPoolBuilder::build` / `ThreadPool::drop` paths need them (loom has
//!   no equivalent); they stay on `std::thread`. The per-worker
//!   `WorkerHandle.thread` is consequently a `std::thread::Thread` (it comes
//!   from `JoinHandle::thread()`), distinct from the shimmed `ScopeShared.waker`
//!   `Thread`; the two are separate fields and never assigned across.
//!
//!   KE16 W-d′ adds one relation between them without changing that: under
//!   `cfg(not(loom))` [`WakeHandle`] IS `std::thread::Thread`, so
//!   `WorkerHandle.thread` is a `WakeHandle` BY IDENTITY and
//!   `ScopeShared.joiner_wake` can point at it with no cast and no allocation.
//!   Under `cfg(loom)` the two types genuinely differ (`WakeHandle` is a
//!   counting newtype over `loom::thread::Thread`) and no production
//!   `WakeHandle` is ever constructed — the models own theirs.

#[cfg(loom)]
#[allow(unused_imports)]
pub(crate) use loom::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering, fence,
};
#[cfg(loom)]
#[allow(unused_imports)]
pub(crate) use loom::sync::{Condvar, Mutex};
#[cfg(loom)]
#[allow(unused_imports)]
pub(crate) use loom::thread::{self, Thread};

#[cfg(not(loom))]
#[allow(unused_imports)]
pub(crate) use core::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering, fence,
};
// `Mutex` re-export for the builder's one-shot bootstrap handshake, which parks a worker on a
// `Condvar` until the pool publishes itself — a genuine blocking wait, not a memo.
//
// 2026-07 audit: `ScopeShared`'s panic-payload slot used to be the shim's other user, justified
// as "cold-path only (panics are rare)". Half of that was true — the WRITE only happens on a
// panic, but `Scope::drop` read it through an unconditional `lock()` on EVERY scope teardown,
// and `ScopeShared::new` constructed a fresh `Mutex` per scope. Under the parallel scheduler a
// scope is created and torn down per system run, so a "panics are rare" lock sat on a
// per-system-run path. It is now an `AtomicPtr` CAS-once slot: the no-panic path reads null.
#[cfg(not(loom))]
#[allow(unused_imports)]
#[allow(clippy::disallowed_types)]
pub(crate) use std::sync::{Condvar, Mutex};
#[cfg(not(loom))]
#[allow(unused_imports)]
pub(crate) use std::thread::{self, Thread};

/// KE16 W-d′ — the type of a count-gated wake target.
///
/// A per-worker handle OWNED BY `PoolInner` (`WorkerHandle.thread`), pointed at
/// from `ScopeShared.joiner_wake` and unparked by the last completer AFTER its
/// decrement. Its whole reason to exist is the lifetime: `ScopeShared.waker`
/// lives inside the allocation the decrement releases, so a gated wake must aim
/// at something the joiner cannot free.
///
/// Native: a transparent alias of the handle `WorkerHandle` already holds, so
/// the pointer is `&inner.workers[wid].thread` with no cast and no new
/// allocation.
///
/// `pub` in a private module: `loom_exports` re-exports it for the M1c model
/// (a `pub use` of a `pub(crate)` item is E0364/E0365). Nothing outside that
/// `cfg(loom)` module can name it.
#[cfg(not(loom))]
pub type WakeHandle = std::thread::Thread;

/// KE16 W-d′ — the loom form of the count-gated wake target: a counting newtype
/// over `loom::thread::Thread`.
///
/// The M1c model cannot build a `PoolInner` (crossbeam-coupled, see the shim's
/// scope note above), so it owns the target itself — a `Box<WakeHandle>` that
/// outlives the completers — and counts the unparks the REAL `complete_task`
/// issues through it. That is what makes "exactly one wake, by the last
/// completer" an assertion over production code rather than over a copy of it.
/// No production `WakeHandle` is ever constructed under `cfg(loom)`
/// (`PoolInner::joiner_wake_target` returns null there).
#[cfg(loom)]
pub struct WakeHandle {
    inner: loom::thread::Thread,
    unparks: loom::sync::atomic::AtomicUsize,
}

#[cfg(loom)]
impl WakeHandle {
    /// Wrap a loom thread handle as a countable wake target.
    pub fn new(inner: loom::thread::Thread) -> Self {
        Self {
            inner,
            unparks: loom::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Count this wake, then issue loom's REAL `unpark` — the edge M1c's join
    /// loop parks against.
    ///
    /// `AcqRel` on the counter so the increment is ordered against the
    /// [`unparks`](Self::unparks) read the model takes after its joins.
    pub fn unpark(&self) {
        self.unparks.fetch_add(1, Ordering::AcqRel);
        self.inner.unpark();
    }

    /// How many wakes have been issued through this target.
    pub fn unparks(&self) -> usize {
        self.unparks.load(Ordering::Acquire)
    }
}
