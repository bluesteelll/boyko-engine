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
//! `Mutex` / `Condvar`; and `thread::{self, Thread}` (the `ScopeShared.waker`
//! `Thread` + `current()` / `unpark()` on the wakeup happens-before, and the
//! loom `thread::park` used by the M1 join model).
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
//! - **`fence`** — no production code uses `core::sync::atomic::fence`.
//! - **`park_timeout`** — loom has no equivalent; `scope.rs` keeps the
//!   fully-qualified `std::thread::park_timeout` (only ever reached on the
//!   native / Miri path, never under loom).
//! - **`JoinHandle`** / `thread::Builder` — only the crossbeam-coupled
//!   `ThreadPoolBuilder::build` / `ThreadPool::drop` paths need them (loom has
//!   no equivalent); they stay on `std::thread`. The per-worker
//!   `WorkerHandle.thread` is consequently a `std::thread::Thread` (it comes
//!   from `JoinHandle::thread()`), distinct from the shimmed `ScopeShared.waker`
//!   `Thread`; the two are separate fields and never assigned across.

#[cfg(loom)]
#[allow(unused_imports)]
pub(crate) use loom::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[cfg(loom)]
#[allow(unused_imports)]
pub(crate) use loom::sync::{Condvar, Mutex};
#[cfg(loom)]
#[allow(unused_imports)]
pub(crate) use loom::thread::{self, Thread};

#[cfg(not(loom))]
#[allow(unused_imports)]
pub(crate) use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[cfg(not(loom))]
#[allow(unused_imports)]
pub(crate) use std::sync::{Condvar, Mutex};
#[cfg(not(loom))]
#[allow(unused_imports)]
pub(crate) use std::thread::{self, Thread};
