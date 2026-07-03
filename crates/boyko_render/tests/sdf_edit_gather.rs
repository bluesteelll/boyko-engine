//! Host plan R7 — the SDF instance path's headless gather + encode contract.
//!
//! `collect_sdf_edits` folds every `SdfPrimitive` entity into the reused `SdfEditStaging`
//! scratch ONCE. The host runs it explicitly AFTER `App::finish` (which drains all
//! startup spawns) so it is order-proof against the user's `add_startup_system`
//! registration (the P0 fix — `SdfPlugin` no longer registers the gather itself). These
//! tests mirror that: `finish_and_gather` calls `finish()` then `run_system(collect_...)`
//! WITHOUT a device (no GPU). They assert the gathered edits are byte-equal to the
//! spawned ones, the clamp to `MAX_SDF_EDITS` holds, an empty scene stays inert (the
//! 0%-gate that keeps the marcher's edit list at the empty boot seed), and the frame loop
//! adds no SDF-path allocation after boot.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;

use boyko_rhi_vulkan::compute::{EDITLIST_BUFFER_WORDS, encode_edit_list};

use boyko_render::sdf_edit::{
    MAX_SDF_EDITS, SdfEdit, SdfEditStaging, SdfPlugin, SdfPrimitive, collect_sdf_edits, sdf_op,
};

/// Views a `#[repr(C)]`/`#[repr(transparent)]` POD as raw bytes for the spawn path.
///
/// # Safety
/// `T` is a fixed-layout POD (`SdfPrimitive` is `#[repr(transparent)]` over the
/// `#[repr(C, align(16))]` `SdfEdit`), so its byte image is a valid pool serialization.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we read its `size_of::<T>()` bytes read-only. `T` is
    // a fixed-layout POD matching the pool's stored layout; the slice borrows `value`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Spawns one `SdfPrimitive` (a bare component — no `#[require]`, no filter) through the
/// raw `EcsMaster` archetype/create path.
fn spawn_sdf_primitive(world: &mut EcsMaster, edit: SdfEdit) {
    let arch = world.create_archetype(&[SdfPrimitive::component_id()]);
    let prim = SdfPrimitive(edit);
    world
        .create_entity(arch, &[(SdfPrimitive::component_id(), as_bytes(&prim))])
        .expect("the SdfPrimitive archetype accepts its one column");
}

/// Builds an `App` with `SdfPlugin` (inserts `SdfEditStaging` only — the gather is NOT a
/// startup system). `SdfPlugin` registers NO process-global hooks, so co-located tests
/// are safe.
fn sdf_app() -> App {
    let mut app = App::new();
    app.add_plugins(SdfPlugin);
    app
}

/// Drains startup + runs the gather ONCE, exactly as the host does after `finish()`:
/// `collect_sdf_edits` observes the World only after every spawn is applied (the P0
/// order-proof site). Mirrors `runner::run_windowed`'s post-`finish` gather call.
fn finish_and_gather(app: &mut App) {
    app.finish();
    app.world_mut().run_system(collect_sdf_edits);
}

/// Three distinct spawned edits → count 3, `edits()` byte-equal to the spawned bytes, and
/// `encode_edit_list` writes word 0 == 3.
#[test]
fn three_primitives_gather_byte_equal_and_encode_count_three() {
    let spawned = [
        SdfEdit::sphere([1.0, 2.0, 3.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::box_shape([-1.0, 0.0, 2.0], [0.4, 0.6, 0.8], sdf_op::SUBTRACT, 0.1),
        SdfEdit::sphere([0.0, -3.0, 1.5], 1.25, sdf_op::INTERSECT, 0.0),
    ];

    let mut app = sdf_app();
    for e in spawned {
        spawn_sdf_primitive(app.world_mut(), e);
    }
    // The host gathers AFTER finish() drains the spawns (the P0 order-proof site).
    finish_and_gather(&mut app);

    let staging = app.world().resource::<SdfEditStaging>();
    assert!(staging.is_dirty(), "a non-empty gather marks the staging dirty");
    let edits = staging.edits();
    assert_eq!(edits.len(), 3, "three spawned primitives gather to count 3");

    // Byte-equal: the gather is a pure copy of the SdfEdit bytes (no repack).
    for (got, want) in edits.iter().zip(spawned.iter()) {
        assert_eq!(as_bytes(got), as_bytes(want), "gathered edit is byte-equal to the spawned one");
    }

    // The encode packs word 0 == edit_count (== 3) at the front of the SSBO layout.
    let mut buf = vec![0u32; EDITLIST_BUFFER_WORDS];
    encode_edit_list(&mut buf, edits);
    assert_eq!(buf[0], 3, "encode_edit_list writes word 0 = edit_count = 3");
}

/// 20 spawned primitives clamp to `MAX_SDF_EDITS` (16) — the excess is silently dropped,
/// matching the shader's edit-count clamp.
#[test]
fn twenty_primitives_clamp_to_max() {
    let mut app = sdf_app();
    for i in 0..20u32 {
        spawn_sdf_primitive(
            app.world_mut(),
            SdfEdit::sphere([i as f32, 0.0, 0.0], 0.3, sdf_op::UNION, 0.0),
        );
    }
    finish_and_gather(&mut app);

    let staging = app.world().resource::<SdfEditStaging>();
    assert_eq!(staging.edits().len(), MAX_SDF_EDITS, "20 primitives clamp to MAX_SDF_EDITS (16)");
    assert!(staging.is_dirty(), "a full-and-clamped gather is still dirty");

    // The encoded count also clamps to 16 (the packed edit array is exactly 16 wide).
    let mut buf = vec![0u32; EDITLIST_BUFFER_WORDS];
    encode_edit_list(&mut buf, staging.edits());
    assert_eq!(buf[0], MAX_SDF_EDITS as u32, "the encoded edit_count clamps to 16");
}

/// An empty scene (0 `SdfPrimitive`) → count 0, dirty FALSE, and the encoded header is
/// byte-identical to the empty boot seed (word 0 == 0) — the 0%-gate proof that a
/// SDF-less scene leaves the marcher's edit list at today's empty seed.
#[test]
fn empty_scene_is_inert_and_header_matches_empty_seed() {
    let mut app = sdf_app();
    finish_and_gather(&mut app);

    let staging = app.world().resource::<SdfEditStaging>();
    assert_eq!(staging.edits().len(), 0, "no primitives gather to count 0");
    assert!(!staging.is_dirty(), "an empty gather leaves the staging NOT dirty (no upload)");

    // The 0%-gate: encoding the empty gather is byte-identical to encoding the empty
    // seed the boot writes (`encode_edit_list(buf, &[])`), the golden empty-list anchor.
    let empty: [SdfEdit; 0] = [];
    let mut gathered_buf = vec![0u32; EDITLIST_BUFFER_WORDS];
    let mut seed_buf = vec![0u32; EDITLIST_BUFFER_WORDS];
    encode_edit_list(&mut gathered_buf, staging.edits());
    encode_edit_list(&mut seed_buf, &empty);
    assert_eq!(gathered_buf, seed_buf, "the empty gather encodes byte-identically to the empty seed");
    assert_eq!(gathered_buf[0], 0, "the empty edit-list header carries edit_count 0");
}

// ── zero_alloc: the SDF gather runs ONCE at startup; the frame loop touches it never. ──
//
// The counting allocator is process-global; the tests in this binary run in PARALLEL
// threads, so a global flag would fold sibling allocations into this window. Counting is
// therefore THREAD-LOCAL (only the measuring thread counts), robust to concurrent siblings.

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ACQUISITIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

#[inline]
fn note_alloc() {
    let _ = COUNTING.try_with(|c| {
        if c.get() {
            let _ = ACQUISITIONS.try_with(|a| a.set(a.get() + 1));
        }
    });
}

// SAFETY: pure delegation to `System` with a thread-local counter side-effect; the
// layout/pointer contracts are forwarded unchanged.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_alloc();
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_alloc();
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_alloc();
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded verbatim to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// After boot, N `App::update` frames add ZERO heap allocation from the SDF path: the
/// gather ran once at boot (the host's post-`finish()` `run_system`), the staging is a
/// fixed inline `[SdfEdit; MAX_SDF_EDITS]` (no `Vec`), and the Main schedule has no
/// per-frame SDF system. `SdfEditStaging::edits()` (the host's per-frame read) is a pure
/// slice, no allocation.
#[test]
fn sdf_path_is_alloc_free_per_frame_after_boot() {
    let mut app = sdf_app();
    for i in 0..3u32 {
        spawn_sdf_primitive(
            app.world_mut(),
            SdfEdit::sphere([i as f32, 1.0, -1.0], 0.5, sdf_op::UNION, 0.0),
        );
    }
    finish_and_gather(&mut app);

    // Warm the frame path outside the counting window (first frames may lazily size
    // ECS/schedule scratch that has nothing to do with the SDF path).
    for _ in 0..4 {
        app.update();
    }

    ACQUISITIONS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    for _ in 0..4 {
        app.update();
        // The host's per-frame SDF read is a pure slice over the inline staging — no alloc.
        let _ = app.world().resource::<SdfEditStaging>().edits().len();
    }
    COUNTING.with(|c| c.set(false));

    assert_eq!(
        ACQUISITIONS.with(|a| a.get()),
        0,
        "the SDF path allocates nothing per frame after boot (startup-once gather, inline staging)"
    );
}
