//! A-1 — Miri proof for the zero-sized-resource heap-corruption fix.
//!
//! Run under:
//!
//! ```powershell
//! cargo +nightly miri test -p boyko-ecs --test miri_zst_resource
//! ```
//!
//! `-Zmiri-tree-borrows` is the workspace default (`.cargo/config.toml`). No
//! `-Zmiri-ignore-leaks` is needed: this file never constructs an
//! `Arc<ThreadPool>` (no OS worker threads ⇒ nothing for Miri to flag as
//! "leaked" at process exit) and never calls `Schedule::run`, so it sidesteps
//! the Phase-9 `Scope::spawn` Tree-Borrows deferral documented in
//! `miri_phase9.rs` / `miri_phase16.rs`.
//!
//! # The bug this proves fixed (C-NEW-ZST)
//!
//! Inserting a zero-sized resource (`struct Marker;`) routed a dangling-but-
//! aligned pointer (`Box::into_raw` of a ZST returns `NonNull::dangling()`,
//! never a real allocation) into a slot whose cached `Layout` has `size == 0`.
//! Both manual `std::alloc::dealloc` sites in `resources.rs` — the replace-path
//! free and the `Drop` walk — then called `dealloc(dangling_ptr, size_0_layout)`,
//! freeing memory the allocator never handed out. On the platform allocator that
//! surfaced as `STATUS_HEAP_CORRUPTION` at process exit; under Miri it is the
//! "deallocating <ptr>, which is dangling / not allocated" invalid-free detector.
//!
//! The fix guards both `dealloc` sites with `if layout.size() != 0`. Under Miri
//! this file MUST be clean post-fix. Pre-fix, the `replace`-path assertions and
//! the world `drop` would each trip Miri's invalid-deallocation check.
//!
//! # `#![cfg(miri)]`
//!
//! This file is a Miri-only deliverable (per the A-1 brief). Under a regular
//! `cargo test` the whole module compiles to nothing, so it adds zero runtime
//! cost to the normal suite — the equivalent non-Miri coverage already lives in
//! the in-module `resources.rs` tests (`zst_resource_round_trips_without_dealloc`,
//! `zst_resource_with_drop_runs_drop_glue`).
#![cfg(miri)]

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Resource;

// ── Resource types ─────────────────────────────────────────────────────────

/// Plain zero-sized resource (no `Drop`). `size_of == 0` ⇒ both `dealloc`
/// guards must skip. `#[derive(Resource)]` mints a per-type `ResourceId`.
#[derive(Resource)]
struct ZstPlain;

/// Drop-counter for the ZST-with-Drop resource. A `static` is fine here:
/// `#![cfg(miri)]` runs single-threaded under Miri and the test that reads it
/// owns the only world that inserts/drops the type, so there is no cross-test
/// interleave on this counter within a Miri run.
static ZST_DROP_RUNS: AtomicUsize = AtomicUsize::new(0);

/// Zero-sized resource WITH a `Drop` impl. Proves the fix skips only the
/// `dealloc` (the memory free) while STILL running the drop glue — a ZST's
/// `drop_fn` is a real side effect that must fire on replace and on teardown.
#[derive(Resource)]
struct ZstDropper;

impl Drop for ZstDropper {
    fn drop(&mut self) {
        ZST_DROP_RUNS.fetch_add(1, Ordering::Relaxed);
    }
}

/// A genuinely sized resource (`size_of == 4 != 0`) — the no-regression
/// control. Its pointer IS a real allocation, so both `dealloc` guards take the
/// `size() != 0` branch and free it normally. Miri must accept that path too.
#[derive(Resource)]
struct NonZst(u32);

// ── A-1 tests ──────────────────────────────────────────────────────────────

/// Full ZST-resource lifecycle on a real `EcsMaster`, single-threaded, no
/// schedule: insert → read → REPLACE (hits the replace-path `dealloc` guard) →
/// remove → DROP the world (hits the `Drop`-path `dealloc` guard). Pre-fix this
/// would trip Miri's invalid-deallocation detector twice (on the replace and on
/// the world drop). Post-fix it MUST be clean.
#[test]
fn zst_plain_full_lifecycle_through_ecs_master_is_miri_clean() {
    let mut world = EcsMaster::new();

    // insert — `Box::into_raw` of a ZST yields a dangling pointer; the slot
    // caches a size-0 layout.
    world.insert_resource(ZstPlain);
    assert!(world.contains_resource::<ZstPlain>(), "ZST present after insert");

    // read via the shared facade — a ZST has no bytes, but the borrow path
    // still dereferences the slot pointer to form `&ZstPlain`. Binding it
    // exercises the read without touching memory the allocator never gave us.
    let _r: &ZstPlain = world.resource::<ZstPlain>();
    assert!(
        world.try_resource::<ZstPlain>().is_some(),
        "try_resource returns Some for a present ZST"
    );

    // REPLACE — the second insert hits the R4 replace path: it reads the old
    // slot out, drops the (no-op) old value, and reaches the manual `dealloc`
    // site that the fix guards with `layout.size() != 0`. THE pre-fix crash #1.
    world.insert_resource(ZstPlain);
    assert!(world.contains_resource::<ZstPlain>(), "ZST present after replace");

    // remove — `Box::from_raw` on the dangling ZST pointer is sound; it
    // reclaims the (zero-sized) value WITHOUT a manual dealloc.
    let taken = world.remove_resource::<ZstPlain>();
    assert!(taken.is_some(), "remove returns Some for a present ZST");
    assert!(!world.contains_resource::<ZstPlain>(), "slot empty after remove");

    // Re-insert so the surviving value rides the world Drop — THE pre-fix
    // crash #2 (the `Drop`-walk `dealloc` site).
    world.insert_resource(ZstPlain);
    drop(world);
}

/// A ZST WITH a `Drop` impl: the fix skips the `dealloc` but the drop glue must
/// still run exactly once per logical destruction — once on the replace path,
/// once on the world teardown. Confirms the guard narrows ONLY the free, not the
/// drop, and that doing so is Miri-clean (no use-after-free of the dropped ZST).
#[test]
fn zst_with_drop_runs_glue_without_dealloc_miri_clean() {
    let before = ZST_DROP_RUNS.load(Ordering::Relaxed);

    let mut world = EcsMaster::new();
    world.insert_resource(ZstDropper); // baseline — no drop yet
    world.insert_resource(ZstDropper); // replace — drops the prior value (no dealloc)

    assert_eq!(
        ZST_DROP_RUNS.load(Ordering::Relaxed) - before,
        1,
        "the ZST replace path runs the old value's drop glue exactly once"
    );

    drop(world); // teardown — drops the surviving value (no dealloc)

    assert_eq!(
        ZST_DROP_RUNS.load(Ordering::Relaxed) - before,
        2,
        "after world drop, total ZST drops == 2 (replaced + final)"
    );
}

/// No-regression control: a genuinely sized resource still round-trips through
/// the SAME insert/replace/remove/drop lifecycle. Here every `dealloc` guard
/// takes the `size() != 0` branch and frees a real allocation, so this exercises
/// the normal (non-ZST) alloc/dealloc path under Miri. If the fix had instead
/// skipped a non-zero free, Miri would report a leak (with the default
/// leak-checking) or the round-tripped value would be wrong.
#[test]
fn non_zst_resource_round_trip_is_miri_clean() {
    let mut world = EcsMaster::new();

    world.insert_resource(NonZst(7));
    assert_eq!(world.resource::<NonZst>().0, 7, "value round-trips after insert");

    // Replace — old (real) allocation is freed via the `size() != 0` branch.
    world.insert_resource(NonZst(11));
    assert_eq!(world.resource::<NonZst>().0, 11, "new value readable after replace");

    // Remove — `Box::from_raw` reclaims and frees the real allocation.
    let taken = world.remove_resource::<NonZst>();
    assert_eq!(taken.map(|v| v.0), Some(11), "remove returns the live value");
    assert!(!world.contains_resource::<NonZst>(), "slot empty after remove");

    // Re-insert so a real allocation rides the world Drop (the `Drop`-walk
    // `size() != 0` free path).
    world.insert_resource(NonZst(99));
    drop(world);
}
