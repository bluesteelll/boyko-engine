//! A pool build registers every crossbeam-epoch participant it creates -- its builder and each of
//! its workers -- inside `ThreadPoolBuilder::build`, so no frame downstream counts a thread's first
//! epoch touch (ci-fix-1 F2: master's push CI run 38182709148 counted crossbeam-epoch's first-pin
//! objects inside one measured physics step of `boyko_physics/tests/narrowphase_carry.rs`).
//!
//! crossbeam-deque's `Stealer::steal` pins before it tests for emptiness, and a thread's first pin
//! registers it. The objects that registration allocates, crossbeam-epoch 0.9.21 on x86_64:
//!
//! * **2304 B, align 128**: the thread's `Local`, once per thread;
//! * **640 B, align 128**: the default collector's `Global`, once per process;
//! * **2072 B, align 8**: the global queue's sentinel node, once per process (later also a sealed
//!   bag node, when a thread's deferred garbage spills).
//!
//! 640/128 is not crossbeam's alone: `PoolInner::workers` is 640/128 at W = 4 and
//! `PoolInner::injector_local` at W = 2. Only arm 1 (W = 1) reads the process-first pair, and no
//! window below contains a pool build.
//!
//! Each arm runs on a FRESH thread, so its builder has no prior `Local`. One test in this binary,
//! so the process-wide counters see no sibling test.
//!
//! Arm 3 drives arm 2's pool from a thread that did not build it, after
//! `ThreadPool::register_thread`.
//!
//! RED-first (ci-fix-1 C2): on the tree before the fix, arm 1's first post-build assertion reads
//! "build() registered 0 of 2 epoch participants".

#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

/// Indices of the three signatures in the counter arrays.
const LOCAL: usize = 0;
const GLOBAL: usize = 1;
const SENTINEL: usize = 2;
/// `(size, align)` per index.
const SIGNATURES: [(usize, usize); 3] = [(2304, 128), (640, 128), (2072, 8)];

/// Per-signature acquisitions on the arm's own thread (the builder) and on every other thread.
static ON_SUBJECT: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static ON_OTHERS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// Open while a window runs: every epoch-signature acquisition is logged.
static WINDOW_OPEN: AtomicBool = AtomicBool::new(false);
const LOG_CAP: usize = 256;
static LOG: [AtomicU64; LOG_CAP] = [const { AtomicU64::new(0) }; LOG_CAP];
static LOG_LEN: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Set on the arm's own thread. Const-initialised and destructor-free, so the allocator can
    /// read it at any point of a thread's life without allocating.
    static SUBJECT: Cell<bool> = const { Cell::new(false) };
}

struct Counting;

// SAFETY: every call forwards verbatim to the platform `System` allocator with the same layout
// (and pointer, for `dealloc` / `realloc`); the wrapper only bumps atomics and reads a
// destructor-free thread-local through `try_with`, neither of which allocates, so the allocator
// contract is exactly `System`'s.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size(), layout.align());
        // SAFETY: forwarded verbatim to the system allocator (same layout).
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` / `layout` came from `System.alloc` or `System.realloc` above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size, layout.align());
        // SAFETY: `ptr` / `layout` came from this allocator; `new_size` forwarded verbatim.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL_ALLOC: Counting = Counting;

fn note(size: usize, align: usize) {
    let Some(sig) = SIGNATURES.iter().position(|&s| s == (size, align)) else {
        return;
    };
    let subject = SUBJECT.try_with(Cell::get).unwrap_or(false);
    // Relaxed: every read below happens after a join (`build`'s boot wait, a scope's join) that
    // orders the worker's increment before it.
    if subject {
        ON_SUBJECT[sig].fetch_add(1, Ordering::Relaxed);
    } else {
        ON_OTHERS[sig].fetch_add(1, Ordering::Relaxed);
    }
    if WINDOW_OPEN.load(Ordering::Relaxed) {
        let i = LOG_LEN.fetch_add(1, Ordering::Relaxed);
        if i < LOG_CAP {
            LOG[i].store(((subject as u64) << 63) | ((align as u64) << 32) | size as u64, Ordering::Relaxed);
        }
    }
}

/// A snapshot of both counter arrays.
#[derive(Clone, Copy, Debug)]
struct Sigs {
    subject: [u64; 3],
    others: [u64; 3],
}

impl Sigs {
    fn now() -> Sigs {
        Sigs {
            subject: core::array::from_fn(|i| ON_SUBJECT[i].load(Ordering::Relaxed)),
            others: core::array::from_fn(|i| ON_OTHERS[i].load(Ordering::Relaxed)),
        }
    }

    fn since(self, base: Sigs) -> Sigs {
        Sigs {
            subject: core::array::from_fn(|i| self.subject[i] - base.subject[i]),
            others: core::array::from_fn(|i| self.others[i] - base.others[i]),
        }
    }

    fn total(&self) -> u64 {
        self.subject.iter().sum::<u64>() + self.others.iter().sum::<u64>()
    }
}

/// Runs `f` inside a window and returns the epoch-signature acquisitions it made, every thread, as
/// `(this thread?, size, align)` entries.
fn window<R>(f: impl FnOnce() -> R) -> (Sigs, Vec<(bool, u64, u64)>, R) {
    LOG_LEN.store(0, Ordering::Relaxed);
    let before = Sigs::now();
    WINDOW_OPEN.store(true, Ordering::Relaxed);
    let r = f();
    WINDOW_OPEN.store(false, Ordering::Relaxed);
    let made = Sigs::now().since(before);
    let n = LOG_LEN.load(Ordering::Relaxed).min(LOG_CAP);
    let log = (0..n)
        .map(|i| {
            let e = LOG[i].load(Ordering::Relaxed);
            (e >> 63 == 1, e & 0xFFFF_FFFF, (e >> 32) & 0x7FFF_FFFF)
        })
        .collect();
    (made, log, r)
}

/// Bounds every spin below; past it the construction runs what is left and its witness reds.
const WAIT: Duration = Duration::from_secs(10);

/// `boyko_ui/tests/zero_alloc.rs`'s driver-touch construction: a leader task pushes `children`
/// onto its worker's own deque and spins until they have run, while the calling thread's join
/// finds the injector empty and steals. Returns how many children the calling thread ran.
fn driver_touch(pool: &ThreadPool, children: usize) -> usize {
    let driver = std::thread::current().id();
    let pushed = AtomicBool::new(false);
    let done = AtomicUsize::new(0);
    let on_driver = AtomicUsize::new(0);
    let (p, pushed, done, on_driver) = (pool, &pushed, &done, &on_driver);
    p.install(|s| {
        s.spawn(move || {
            p.scope(|inner| {
                for _ in 0..children {
                    inner.spawn(move || {
                        if std::thread::current().id() == driver {
                            on_driver.fetch_add(1, Ordering::Relaxed);
                        }
                        done.fetch_add(1, Ordering::Release);
                    });
                }
                pushed.store(true, Ordering::Release);
                let deadline = Instant::now() + WAIT;
                while done.load(Ordering::Acquire) < children && Instant::now() < deadline {
                    std::hint::spin_loop();
                }
            });
        });
        // Acquire: pairs with the leader's Release store, after its pushes.
        let deadline = Instant::now() + WAIT;
        while !pushed.load(Ordering::Acquire) && Instant::now() < deadline {
            std::hint::spin_loop();
        }
    });
    on_driver.load(Ordering::Relaxed)
}

/// `e1_engine_ui_alloc_census.rs`'s `first_touch_round`: a leader pushes `8 x workers` children
/// with a little work each onto its worker's deque and spins (never joins) until others have run
/// them, so every idle worker takes at least one by a sibling steal.
fn first_touch_round(p: &ThreadPool, workers: usize) {
    let children = 8 * workers;
    p.install(|s| {
        s.spawn(move || {
            let done = AtomicUsize::new(0);
            p.scope(|inner| {
                for _ in 0..children {
                    let done = &done;
                    inner.spawn(move || {
                        let mut x = 0u64;
                        for i in 0..20_000u64 {
                            x = black_box(x.wrapping_add(i));
                        }
                        done.fetch_add(1, Ordering::Release);
                    });
                }
                let deadline = Instant::now() + WAIT;
                while done.load(Ordering::Acquire) < children && Instant::now() < deadline {
                    std::hint::spin_loop();
                }
            });
        });
    });
}

/// Arm 1, W = 1: the process's first registration happens inside `build`, on the builder.
fn arm_1() {
    SUBJECT.with(|s| s.set(true));
    let before = Sigs::now();
    assert_eq!(
        (before.subject[GLOBAL] + before.others[GLOBAL], before.subject[LOCAL]),
        (0, 0),
        "precondition: something pinned before arm 1 ({before:?}), so the process-first objects \
         cannot be attributed"
    );
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let built = Sigs::now().since(before);
    assert_eq!(
        built.subject[LOCAL] + built.others[LOCAL],
        2,
        "build() registered {} of 2 epoch participants (builder + 1 worker): {built:?}",
        built.subject[LOCAL] + built.others[LOCAL]
    );
    assert_eq!(built.subject[LOCAL], 1, "the builder's Local must be registered on the builder: {built:?}");
    assert_eq!(built.others[LOCAL], 1, "the worker's Local must be registered inside build(): {built:?}");
    assert_eq!(
        (built.subject[GLOBAL], built.subject[SENTINEL]),
        (1, 1),
        "the process-first Global and queue sentinel must be the builder's, registered before any \
         worker exists: {built:?}"
    );
    let (made, log, stolen) = window(|| driver_touch(&pool, 8));
    assert!(stolen >= 1, "witness: the driver stole none of its worker's 8 children, so the window proves nothing");
    assert_eq!(
        made.total(),
        0,
        "a window holding the driver's steal of its worker's deque counted epoch-sized \
         acquisitions (this thread?, size, align): {log:?}"
    );
    println!("[arm 1] W1 build: {built:?}; driver-steal window: 0 epoch acquisitions, {stolen} of 8 stolen");
}

/// Arm 2, W = 4: one `Local` per worker and the builder's, all inside `build`.
fn arm_2() -> Arc<ThreadPool> {
    SUBJECT.with(|s| s.set(true));
    const W: usize = 4;
    let before = Sigs::now();
    let pool = ThreadPoolBuilder::new().num_threads(W).build();
    let built = Sigs::now().since(before);
    assert_eq!(
        (built.subject[LOCAL], built.others[LOCAL]),
        (1, W as u64),
        "build() registered {} of {} epoch participants (builder + {W} workers): {built:?}",
        built.subject[LOCAL] + built.others[LOCAL],
        W + 1
    );
    let (made, log, ()) = window(|| {
        for _ in 0..64 {
            first_touch_round(&pool, W);
        }
    });
    assert_eq!(
        made.total(),
        0,
        "64 rounds of sibling steals on a {W}-worker pool counted epoch-sized acquisitions \
         (this thread?, size, align): {log:?}"
    );
    println!("[arm 2] W{W} build: {built:?}; 64 sibling-steal rounds: 0 epoch acquisitions");
    pool
}

/// Arm 3: a thread that did not build the pool registers through `register_thread` and then
/// steals from it without a single epoch-sized acquisition.
fn arm_3(pool: &ThreadPool) {
    SUBJECT.with(|s| s.set(true));
    let before = Sigs::now();
    pool.register_thread();
    let registered = Sigs::now().since(before);
    assert_eq!(
        registered.subject[LOCAL], 1,
        "register_thread() must register the calling thread's Local: {registered:?}"
    );
    pool.register_thread();
    assert_eq!(
        Sigs::now().since(before).total(),
        registered.total(),
        "a second register_thread() must allocate nothing"
    );
    let (made, log, stolen) = window(|| {
        let mut stolen = 0;
        for _ in 0..64 {
            stolen += driver_touch(pool, 8);
            if stolen > 0 {
                break;
            }
        }
        stolen
    });
    assert!(stolen >= 1, "witness: the driver stole no child in 64 rounds, so the window proves nothing");
    assert_eq!(
        made.total(),
        0,
        "a registered non-builder driver's steals counted epoch-sized acquisitions          (this thread?, size, align): {log:?}"
    );
    println!("[arm 3] register_thread(): {registered:?}; driver-steal window: 0 epoch acquisitions");
}

#[test]
fn a_build_registers_every_epoch_participant_and_no_window_sees_one() {
    let arm = |name: &'static str, f: Box<dyn FnOnce() + Send>| {
        if let Err(p) = std::thread::Builder::new().name(name.into()).spawn(f).expect("spawn").join() {
            std::panic::resume_unwind(p);
        }
    };
    arm("arm-1", Box::new(arm_1));
    let (tx, rx) = std::sync::mpsc::channel();
    arm("arm-2", Box::new(move || tx.send(arm_2()).expect("send")));
    let pool = rx.recv().expect("arm 2 built a pool");
    arm("arm-3", Box::new(move || arm_3(&pool)));
}
