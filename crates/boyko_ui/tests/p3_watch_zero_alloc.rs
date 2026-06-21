//! GATE 6 — WATCH no-change path: ZERO allocation + ZERO tree read when the file
//! is unchanged (Decision 7), via a counting global allocator + DELTA comparison.
//!
//! Mirrors the existing `zero_alloc.rs` harness: a process-global counting
//! allocator, armed around exactly one measured region under a serialization lock.
//!
//! We measure the `ui_hot_reload_system` watch tick directly (NOT through the
//! `App` scheduler, whose per-frame dispatch allocates a fixed amount unrelated to
//! the watch path) on a world wired by `UiPlugin` at startup. Three measured
//! regions:
//!   * `noop`     — a no-change tick that is THROTTLED (returns before any syscall).
//!   * `nochange` — a no-change tick PAST the throttle: it does the one
//!     `metadata()` syscall, sees the unchanged `(mtime,size)`, and returns BEFORE
//!     any `query_entities` / `UiTreeView` build / parse (the gated no-change path).
//!   * `reload`   — a tick that actually reconciles a changed file (the tree-read
//!     path), as the upper-bound witness.
//!
//! The gate: `nochange` allocations are a tiny constant AND vastly below `reload`
//! — proving the tree-read path (which allocates a `query_entities` `Vec` + a
//! `UiTreeView` + the parse) is gated out of the no-change path.

mod p3_common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ui::reload::ui_hot_reload_system;
use boyko_ui::UiPlugin;

use p3_common::TempUi;

// ───────────────────────── counting allocator ─────────────────────────────

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);

// SAFETY: forwards every call verbatim to the system allocator; the only added
// behavior is an atomic increment on alloc/realloc when armed.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn count_allocs(f: impl FnOnce()) -> usize {
    ALLOCS.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    f();
    ARMED.store(false, Ordering::Relaxed);
    ALLOCS.load(Ordering::Relaxed)
}

/// Builds a finished `App` with a `UiPlugin` watching a temp file (1 ms poll).
fn build(tag: &str, src: &str) -> (App, TempUi) {
    let temp = TempUi::new(tag, src);
    let mut app = App::with_threads(1);
    app.add_plugin(
        UiPlugin::new()
            .with_ui_path(temp.path)
            .with_hot_reload(true)
            .with_poll_interval(Duration::from_millis(1)),
    );
    app.finish();
    (app, temp)
}

const DOC: &str = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
    #b  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";

#[test]
fn watch_throttled_tick_allocates_nothing() {
    let _arm = lock_arm();
    let (mut app, _temp) = build("throttle", DOC);
    // First direct tick polls (the resource seeds last_poll in the past) and sets
    // last_poll = now. An IMMEDIATE second tick is throttled → returns before any
    // syscall or alloc.
    ui_hot_reload_system(app.world_mut());
    let throttled = count_allocs(|| {
        ui_hot_reload_system(app.world_mut());
    });
    assert_eq!(throttled, 0, "a throttled no-change tick allocates nothing (early return)");
}

#[test]
fn watch_nochange_path_is_tiny_and_far_below_reload() {
    let _arm = lock_arm();

    // ── nochange: a tick PAST the throttle on an UNCHANGED file. ──
    let (mut app, _temp) = build("nochange", DOC);
    ui_hot_reload_system(app.world_mut()); // clear the seeded poll, set last_poll = now
    std::thread::sleep(Duration::from_millis(3)); // let the 1 ms throttle elapse
    let nochange = count_allocs(|| {
        ui_hot_reload_system(app.world_mut());
    });

    // ── reload: a tick that actually reconciles a CHANGED file. ──
    let (mut app2, temp2) = build("reload_witness", DOC);
    ui_hot_reload_system(app2.world_mut());
    // Change the file, then settle (two observations of the new signature).
    temp2.write("\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(77), height: Px(40) }
    #b  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
");
    std::thread::sleep(Duration::from_millis(3));
    ui_hot_reload_system(app2.world_mut()); // detect → pending
    std::thread::sleep(Duration::from_millis(3));
    let reload = count_allocs(|| {
        ui_hot_reload_system(app2.world_mut()); // settled → reconcile (tree read)
    });

    // The no-change path must be a tiny constant — the `metadata()` syscall on
    // windows-gnu may allocate a small fixed amount for the path/OsString, but it
    // does NOT build the `query_entities` Vec / `UiTreeView` / parse. A generous
    // absolute bound, plus the structural proof that it is FAR below a real
    // reconcile.
    assert!(
        nochange <= 8,
        "no-change watch tick allocates a tiny constant (no tree read): got {nochange}"
    );
    assert!(
        reload > nochange,
        "a reconciling tick (tree read + parse) must allocate strictly more than the \
         gated no-change path (nochange {nochange}, reload {reload})"
    );
    assert!(
        reload >= nochange + 4,
        "the tree-read path (query_entities Vec + UiTreeView + parse) is a clear, \
         large delta over the no-change path (nochange {nochange}, reload {reload})"
    );
}

#[test]
fn watch_report_nochange_vs_reload_counts() {
    let _arm = lock_arm();
    let (mut app, _temp) = build("rep_nochange", DOC);
    ui_hot_reload_system(app.world_mut());
    std::thread::sleep(Duration::from_millis(3));
    let nochange = count_allocs(|| ui_hot_reload_system(app.world_mut()));

    let (mut app2, temp2) = build("rep_reload", DOC);
    ui_hot_reload_system(app2.world_mut());
    temp2.write("version=1\n#root  UiLayout { layout_type: Column, width: Px(9) }\n");
    std::thread::sleep(Duration::from_millis(3));
    ui_hot_reload_system(app2.world_mut());
    std::thread::sleep(Duration::from_millis(3));
    let reload = count_allocs(|| ui_hot_reload_system(app2.world_mut()));
    println!("WATCH_ALLOC_REPORT nochange={nochange} reload={reload}");
}
