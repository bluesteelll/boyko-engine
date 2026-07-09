//! `ScratchColumn` — the Stage-0 transient-scratch primitive, tested in
//! isolation.
//!
//! Covers: address-stability of the column base across a grow past the first
//! commit step (the property `std::Vec` lacks); `clear` = `len = 0` with NO free
//! (committed pages stay resident, base unchanged); `row_ptr(i)` for distinct
//! `i` yield distinct addresses at the correct TYPED stride; `push` /
//! `extend_from_slice` / `as_slice` round-trip; the typed `row_ptr` returns
//! `*mut T` (no `u8` cast at the call site). The Miri-TB suite (single-threaded
//! UB-clean + the parallel distinct-index test) lives in `scratch_column_miri`.
//!
//! Component-id allocation: 110 (`F32` scalar) / 111 (`Body16`) — in the free
//! band below `MAX_COMPONENTS = 512`, clear of the dense tests' 103..=105.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

// ── component types ─────────────────────────────────────────────────────────

/// A scalar `f32` scratch element (a single solver lane).
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(transparent)]
struct F32(f32);

const F32_ID: ComponentId = ComponentId(110);

impl Component for F32 {
    fn component_id() -> ComponentId {
        F32_ID
    }
}

/// A 16-byte POD body-state scratch element (the solver's per-element shape).
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Body16 {
    px: f32,
    py: f32,
    vx: f32,
    vy: f32,
}

const BODY_ID: ComponentId = ComponentId(111);

impl Component for Body16 {
    fn component_id() -> ComponentId {
        BODY_ID
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn register() {
    component_registry::register_layout::<F32>(F32_ID.0);
    component_registry::register_layout::<Body16>(BODY_ID.0);
}

fn f32_column(reserve_rows: usize) -> ScratchColumn<F32> {
    register();
    ScratchColumn::new(F32_ID, reserve_rows)
}

fn body_column(reserve_rows: usize) -> ScratchColumn<Body16> {
    register();
    ScratchColumn::new(BODY_ID, reserve_rows)
}

// ── push / extend / as_slice round-trip ─────────────────────────────────────

#[test]
fn push_extend_as_slice_round_trip() {
    let mut col = f32_column(64);
    {
        let mut build = col.build_view();
        assert!(build.is_empty());
        assert_eq!(build.push(F32(1.0)), 0);
        assert_eq!(build.push(F32(2.0)), 1);
        build.extend_from_slice(&[F32(3.0), F32(4.0), F32(5.0)]);
        assert_eq!(build.len(), 5);
        assert_eq!(
            build.as_slice(),
            &[F32(1.0), F32(2.0), F32(3.0), F32(4.0), F32(5.0)]
        );
        // Mutate through the whole-buffer slice (the single-threaded surface).
        for v in build.as_mut_slice() {
            v.0 *= 10.0;
        }
        assert_eq!(
            build.as_slice(),
            &[F32(10.0), F32(20.0), F32(30.0), F32(40.0), F32(50.0)]
        );
    }
    assert_eq!(col.len(), 5);
}

// ── clear() = len-0, no free, base unchanged ────────────────────────────────

#[test]
fn clear_resets_len_without_freeing_or_moving_base() {
    let mut col = body_column(256);
    {
        let mut build = col.build_view();
        for i in 0..32u32 {
            build.push(Body16 { px: i as f32, py: 0.0, vx: 0.0, vy: 0.0 });
        }
        assert_eq!(build.len(), 32);
    }

    // Base BEFORE clear (via the solve view's index-0 pointer).
    let base_before = col.solve_view().as_ptr_for_test();

    {
        let mut build = col.build_view();
        build.clear();
        assert_eq!(build.len(), 0, "clear sets len = 0");
        assert!(build.is_empty());
    }

    // Base AFTER clear — clear must NOT free the reservation (committed pages
    // stay resident), so re-pushing reuses the SAME base address.
    {
        let mut build = col.build_view();
        build.push(Body16 { px: 99.0, py: 0.0, vx: 0.0, vy: 0.0 });
    }
    let base_after = col.solve_view().as_ptr_for_test();
    assert_eq!(
        base_before, base_after,
        "clear() keeps the committed reservation: base address unchanged"
    );

    // The re-pushed row is readable at index 0.
    assert_eq!(col.as_slice_for_test()[0].px, 99.0);
}

// ── address stability across a grow past the first commit step ──────────────

#[test]
fn column_base_is_address_stable_across_grow() {
    // reserve_rows large enough to never hit the ceiling, but the initial
    // committed capacity is far smaller, so filling past the first commit step
    // forces at least one in-place `grow_rows`. The VM-reserved base must NOT
    // move (the property `std::Vec` lacks that caused SP4).
    let reserve = 1 << 16; // 65_536 rows; first commit covers far fewer.
    let mut col = body_column(reserve);

    // Capture the base via the index-0 pointer after seeding one element.
    {
        let mut build = col.build_view();
        build.push(Body16 { px: 0.0, py: 0.0, vx: 0.0, vy: 0.0 });
    }
    // SAFETY: index 0 is live (just pushed); reading the pointer (not
    // dereferencing) is sound. Captured as an address only.
    let base_before = unsafe { col.solve_view().row_ptr(0) };

    // Push enough rows to cross at least one grow boundary. 20_000 * 16 B =
    // 320 KB, well past the initial commit step.
    {
        let mut build = col.build_view();
        for i in 1..20_000u32 {
            build.push(Body16 { px: i as f32, py: 0.0, vx: 0.0, vy: 0.0 });
        }
    }
    assert!(col.len() >= 20_000);

    // SAFETY: index 0 is still live and never moves; reading its pointer is sound.
    let base_after = unsafe { col.solve_view().row_ptr(0) };
    assert_eq!(
        base_before, base_after,
        "ScratchColumn base must be address-stable across grow (in-place commit)"
    );

    // Spot-check that early rows are still readable (not relocated).
    let slice = col.as_slice_for_test();
    assert_eq!(slice[0].px, 0.0);
    assert_eq!(slice[12_345].px, 12_345.0);
}

// ── distinct row_ptr addresses at the correct typed stride ──────────────────

#[test]
fn row_ptr_distinct_addresses_with_typed_stride() {
    let mut col = body_column(64);
    {
        let mut build = col.build_view();
        for i in 0..8u32 {
            build.push(Body16 { px: i as f32, py: 0.0, vx: 0.0, vy: 0.0 });
        }
    }

    let view = col.solve_view();
    assert_eq!(view.len(), 8);

    // Each row_ptr is a TYPED `*mut Body16`; consecutive pointers differ by
    // exactly one element (the typed stride = size_of::<Body16>()), not by 1
    // byte — i.e. `add(1)` on a `*mut T`, no `u8` cast at the call site.
    let stride = std::mem::size_of::<Body16>();
    let mut prev: Option<usize> = None;
    for i in 0..8usize {
        // SAFETY: `i < len` and `i` is live (all 8 pushed); `row_ptr`'s contract
        // holds. The returned pointer is a `*mut Body16` — no cast here proves it.
        let p: *mut Body16 = unsafe { view.row_ptr(i) };
        let addr = p as usize;
        if let Some(prev_addr) = prev {
            assert_eq!(
                addr - prev_addr,
                stride,
                "consecutive typed row_ptr differ by exactly the element stride"
            );
        }
        prev = Some(addr);
    }

    // All 8 addresses are pairwise distinct.
    let mut addrs: Vec<usize> = (0..8)
        // SAFETY: each `i < len` and live; pointer read only.
        .map(|i| unsafe { view.row_ptr(i) } as usize)
        .collect();
    addrs.sort_unstable();
    addrs.dedup();
    assert_eq!(addrs.len(), 8, "all row_ptr addresses are distinct");
}

// ── typed row_ptr returns *mut T (no u8 cast) ───────────────────────────────

#[test]
fn row_ptr_is_typed_mut_t() {
    let mut col = f32_column(16);
    {
        let mut build = col.build_view();
        build.push(F32(7.0));
    }
    let view = col.solve_view();
    // SAFETY: index 0 is live; single-threaded write through the typed pointer.
    unsafe {
        // The binding's type is `*mut F32` with NO cast — proving the typed
        // surface. Write then read back.
        let p: *mut F32 = view.row_ptr(0);
        (*p).0 = 42.0;
        assert_eq!((*p).0, 42.0);
    }
    assert_eq!(col.as_slice_for_test()[0], F32(42.0));
}

// ── test-only read helpers on the column / view ─────────────────────────────
//
// These exist purely so the tests can read the column without re-deriving the
// pointer math; they exercise the public `solve_view` / `build_view` surface.

trait ColumnReadTestExt<T: Copy> {
    fn as_slice_for_test(&mut self) -> Vec<T>;
}

impl<T: Copy> ColumnReadTestExt<T> for ScratchColumn<T> {
    fn as_slice_for_test(&mut self) -> Vec<T> {
        self.build_view().as_slice().to_vec()
    }
}

trait SolveViewAddrTestExt {
    fn as_ptr_for_test(&self) -> usize;
}

impl<T: Copy> SolveViewAddrTestExt
    for boyko_ecs::ecs::core::component::scratch::ScratchSolveView<'_, T>
{
    fn as_ptr_for_test(&self) -> usize {
        if self.is_empty() {
            0
        } else {
            // SAFETY: non-empty ⇒ index 0 is in-bounds; pointer read only.
            unsafe { self.row_ptr(0) as usize }
        }
    }
}
