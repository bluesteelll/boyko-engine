//! CORE C5 gates 1, 2 and 4 — `[T; N]`-of-`Prim` element access.
//!
//! 1. element round-trip over `[f32;4]`, `[u8;60]`, `[u32;1]`, `[i8;3]` — the flagship
//!    shape (`GpuTransform3D`/`TrsPacked`), the inline-string shape
//!    (`UiName { bytes: [u8; CAP] }`), the degenerate one and a signed one;
//! 2. `i == len` and `i == usize::MAX` answer `None`/`false`, in a **release** test;
//! 4. `stride == size_of::<T>()`, pinned against `offset_of!`-derived element spacing
//!    in a fixture whose array fields are **followed by padding**, so a `stride` taken
//!    from neighbouring field offsets is a demonstrably different number.
//!
//! Gate 3 (allocation delta) is not here: it lives with the instrument it extends, in
//! [`c4_prim_zero_alloc.rs`](../c4_prim_zero_alloc.rs) — one `#[global_allocator]` per
//! binary, and §3.3's correction of 2026-08-21 put the instrument at C4.
//!
//! Run BOTH profiles — gate 2 is a different gate in each, not a repetition:
//!
//! ```text
//! cargo test -p boyko-reflect --test c5_array
//! cargo test -p boyko-reflect --release --test c5_array
//! ```
//!
//! # Why every gate writes ALL elements before reading ANY
//!
//! C5's RED mutation is `stride = size_of::<T>() - 1`, and for a byte-wide element that
//! is **stride 0**: every element aliases element 0. A per-element write-then-read pair
//! would then agree with itself perfectly — the same self-consistency defect C4 gate 1
//! had to split into two tests. Writing the whole array first and reading it back
//! afterwards is what makes a collapsed stride visible, and it is why the samples differ
//! per element.

use boyko_reflect::array::{array_get, array_set};
use boyko_reflect::scalar::{Scalar, ScalarKind};
use boyko_reflect::type_info::ArrayInfo;

// ───────────────────────────────── the fixture ──────────────────────────────

/// The four array shapes C5 gate 1 names, plus a trailing `u32` that forces **one
/// padding byte after `i3`** — the fixture property gate 4 leans on.
///
/// Ordered by descending element alignment so the padding is where it is wanted
/// (between `i3` and `tail`) rather than scattered.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Arrays {
    /// The flagship shape: `GpuTransform3D`/`TrsPacked` is `[f32;4]`×3.
    f4: [f32; 4],
    /// The degenerate shape: one element, so `len - 1 == 0`.
    u1: [u32; 1],
    /// The inline-string shape: `UiName { bytes: [u8; CAP] }`.
    b60: [u8; 60],
    /// The signed shape — and the one gate 4's padding clause is about.
    i3: [i8; 3],
    /// Forces alignment-4 padding after `i3`. Not itself under test.
    tail: u32,
}

fn sample_struct() -> Arrays {
    Arrays {
        f4: [-1.0, -2.0, -3.0, -4.0],
        u1: [0xDEAD_BEEF],
        b60: [0xCC; 60],
        i3: [-1, -2, -3],
        tail: 0x1234_5678,
    }
}

/// The single site every descriptor's `stride` comes from.
///
/// **This is C5's RED MUTATION site**: `size_of::<T>() - 1` here is the derive bug where
/// `stride` came from the wrong `size_of`, and it must red gate 1 (on elements `1..n`)
/// and gate 4 (on the stride identity).
fn descriptor_stride<T>() -> usize {
    size_of::<T>()
}

/// One array under test: where it starts, what describes it, and what to write.
struct ArrCase {
    name: &'static str,
    offset: usize,
    info: ArrayInfo,
    samples: Vec<Scalar>,
}

fn cases() -> [ArrCase; 4] {
    use std::mem::offset_of;
    [
        ArrCase {
            name: "f4",
            offset: offset_of!(Arrays, f4),
            info: ArrayInfo {
                elem: ScalarKind::F32,
                stride: descriptor_stride::<f32>(),
                len: 4,
            },
            samples: expected_f4().iter().copied().map(Scalar::from).collect(),
        },
        ArrCase {
            name: "u1",
            offset: offset_of!(Arrays, u1),
            info: ArrayInfo {
                elem: ScalarKind::U32,
                stride: descriptor_stride::<u32>(),
                len: 1,
            },
            samples: expected_u1().iter().copied().map(Scalar::from).collect(),
        },
        ArrCase {
            name: "b60",
            offset: offset_of!(Arrays, b60),
            info: ArrayInfo {
                elem: ScalarKind::U8,
                stride: descriptor_stride::<u8>(),
                len: 60,
            },
            samples: expected_b60().iter().copied().map(Scalar::from).collect(),
        },
        ArrCase {
            name: "i3",
            offset: offset_of!(Arrays, i3),
            info: ArrayInfo {
                elem: ScalarKind::I8,
                stride: descriptor_stride::<i8>(),
                len: 3,
            },
            samples: expected_i3().iter().copied().map(Scalar::from).collect(),
        },
    ]
}

/// Distinct per element — see the module header: equal samples would make a collapsed
/// stride invisible.
fn expected_f4() -> [f32; 4] {
    [0.5, 1.5, 2.5, -0.0]
}
fn expected_u1() -> [u32; 1] {
    [0x0BAD_F00D]
}
fn expected_b60() -> [u8; 60] {
    let mut out = [0u8; 60];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    out
}
fn expected_i3() -> [i8; 3] {
    [i8::MIN, 0, 127]
}

// ───────────────────────────────── gate 1 ───────────────────────────────────

/// CORE C5 gate 1 — every element of every shape round-trips through
/// `array_set`/`array_get`, **whole array written before any element is read**.
#[test]
fn every_element_round_trips_through_the_by_index_accessors() {
    for case in cases() {
        let mut value = sample_struct();
        let base = (&raw mut value).cast::<u8>();
        // SAFETY (both loops): `base` is a live, initialized, correctly aligned `Arrays`
        // this frame owns exclusively; `case.offset` is that type's own `offset_of!` for
        // the array field, so `base.add(offset)` is element 0 of a real `[T; N]` whose
        // `elem`/`stride`/`len` `case.info` describes truthfully, with inherited
        // provenance and no live reference into it across the calls.
        for (i, sample) in case.samples.iter().enumerate() {
            let wrote = unsafe { array_set(base.add(case.offset), &case.info, i, *sample) };
            assert!(wrote, "{}[{i}]: a matching-kind, in-bounds set must succeed", case.name);
        }
        for (i, sample) in case.samples.iter().enumerate() {
            let read = unsafe { array_get(base.add(case.offset), &case.info, i) };
            assert_eq!(
                read,
                Some(*sample),
                "{}[{i}]: element did not round-trip (stride = {}, len = {})",
                case.name,
                case.info.stride,
                case.info.len
            );
        }
    }
}

/// Gate 1's *"and it landed in the right element"* half, read back through the **typed
/// fields**: the round-trip above compares `Scalar`s, and a stride that mis-addresses
/// every element consistently would still agree with itself.
///
/// All four arrays are written into ONE fixture and the whole struct is compared against
/// a literal, so an element that spilled into a neighbouring array is visible too.
#[test]
fn every_element_lands_at_its_own_address() {
    let mut value = sample_struct();
    let base = (&raw mut value).cast::<u8>();

    for case in cases() {
        for (i, sample) in case.samples.iter().enumerate() {
            // SAFETY: as `every_element_round_trips_through_the_by_index_accessors`.
            let wrote = unsafe { array_set(base.add(case.offset), &case.info, i, *sample) };
            assert!(wrote, "{}[{i}]: set refused", case.name);
        }
    }

    assert_eq!(
        value,
        Arrays {
            f4: expected_f4(),
            u1: expected_u1(),
            b60: expected_b60(),
            i3: expected_i3(),
            tail: 0x1234_5678,
        },
        "an element write landed at the wrong address (wrong stride, wrong offset, or a \
         spill into a neighbouring field)"
    );
}

/// A mismatched element kind is refused by the same release check `prim::set_*` carries
/// (D11), and the array is left alone — the by-index writer adds a bounds check, it does
/// not replace the kind check.
#[test]
fn a_mismatched_element_kind_is_refused_and_writes_nothing() {
    for case in cases() {
        let mut value = sample_struct();
        let before = value;
        let base = (&raw mut value).cast::<u8>();
        // A kind no case uses as its element, so it mismatches all four.
        let wrong = Scalar::from(-12_345i64);
        // SAFETY: as gate 1 -- the accessor is free to refuse, which is what is under test.
        let wrote = unsafe { array_set(base.add(case.offset), &case.info, 0, wrong) };
        assert!(!wrote, "{}: array_set accepted an I64 into a {:?} array", case.name, case.info.elem);
        assert_eq!(value, before, "{}: array_set REFUSED and still wrote", case.name);
    }
}

// ───────────────────────────────── gate 2 ───────────────────────────────────

/// CORE C5 gate 2 — `i == len` and `i == usize::MAX` are refusals, not reads.
///
/// `usize::MAX` is the load-bearing index and the reason the bounds check is ordered
/// **before** the `index * stride` multiply: that product overflows. **Measured** (C5's
/// third RED, the ordering swapped): the debug leg reds with *"attempt to multiply with
/// overflow"* raised inside `array.rs` — a panic out of a refuse-don't-fail library —
/// and the release leg stays green, because the wrapped product is discarded by a check
/// still keyed on `index`. This gate therefore runs in **both** profiles and is only
/// load-bearing in the debug one; the release leg's liveness is observed at runtime
/// below so a filtered-out release run cannot pass as one.
#[test]
fn an_out_of_bounds_index_is_refused_in_both_directions() {
    for case in cases() {
        let mut value = sample_struct();
        let before = value;
        let base = (&raw mut value).cast::<u8>();
        let sample = case.samples[0];

        for index in [case.info.len, case.info.len + 1, usize::MAX] {
            // SAFETY: as gate 1. The accessor's contract permits any `index`; refusing
            // an out-of-range one without deriving a pointer is exactly what is tested.
            let read = unsafe { array_get(base.add(case.offset), &case.info, index) };
            assert_eq!(
                read, None,
                "{}: array_get READ at index {index} of a len-{} array",
                case.name, case.info.len
            );
            // SAFETY: as above, write side.
            let wrote = unsafe { array_set(base.add(case.offset), &case.info, index, sample) };
            assert!(
                !wrote,
                "{}: array_set ACCEPTED index {index} of a len-{} array",
                case.name, case.info.len
            );
        }
        assert_eq!(value, before, "{}: an out-of-bounds refusal still changed bytes", case.name);
    }
}

/// Gate 2's non-vacuity clause, release half: proof that the release leg of this file
/// actually ran, observed at runtime (a `debug_assert!` that did NOT execute), not
/// restated from `cfg!`.
#[test]
#[cfg(not(debug_assertions))]
fn release_leg_is_live_and_debug_assert_is_gone() {
    let mut fired = false;
    debug_assert!(raise(&mut fired));
    println!("C5 gate 2: release leg live; debug_assert! executed = {fired}");
    assert!(
        !fired,
        "a `debug_assert!` RAN in a release-profile test -- the wrapping-multiply half of \
         gate 2's argument is being measured in the wrong build"
    );
}

/// The debug twin. Together the two make `running N` differ between profiles, so a
/// filtered-out release leg cannot pass as one.
#[test]
#[cfg(debug_assertions)]
fn debug_leg_is_live_and_debug_assert_still_fires() {
    let mut fired = false;
    debug_assert!(raise(&mut fired));
    println!("C5: debug leg live; debug_assert! executed = {fired}");
    assert!(fired, "`debug_assert!` did not run in a debug-profile test");
}

/// Sets the flag and reports success — the probe body for the two leg tests.
fn raise(flag: &mut bool) -> bool {
    *flag = true;
    true
}

// ───────────────────────────────── gate 4 ───────────────────────────────────

/// CORE C5 gate 4 — `stride` is `size_of::<T>()`, and that is pinned against the
/// **measured** spacing of real elements rather than against a second spelling of
/// `size_of`.
///
/// The three claims are separate on purpose:
/// * `info.stride == size_of::<T>()` — the descriptor's own identity;
/// * `info.stride == addr_of(arr[i+1]) - addr_of(arr[i])` — the array's real layout,
///   read off actual element addresses, which is what a wrong `size_of` breaks;
/// * `info.stride * info.len == size_of::<[T; N]>()` — the extent the safety contract
///   of `array_get` promises the caller.
#[test]
fn stride_is_size_of_t_and_matches_measured_element_spacing() {
    let value = sample_struct();

    let f4 = cases()[0].info;
    assert_eq!(f4.stride, size_of::<f32>(), "f4: descriptor stride is not size_of::<f32>()");
    assert_eq!(f4.stride, spacing(&value.f4[0], &value.f4[1]), "f4: measured spacing");
    assert_eq!(f4.stride * f4.len, size_of::<[f32; 4]>(), "f4: stride*len is not the extent");

    let b60 = cases()[2].info;
    assert_eq!(b60.stride, size_of::<u8>(), "b60: descriptor stride is not size_of::<u8>()");
    assert_eq!(b60.stride, spacing(&value.b60[0], &value.b60[1]), "b60: measured spacing");
    assert_eq!(b60.stride * b60.len, size_of::<[u8; 60]>(), "b60: stride*len is not the extent");

    let i3 = cases()[3].info;
    assert_eq!(i3.stride, size_of::<i8>(), "i3: descriptor stride is not size_of::<i8>()");
    assert_eq!(i3.stride, spacing(&value.i3[0], &value.i3[1]), "i3: measured spacing");
    assert_eq!(i3.stride * i3.len, size_of::<[i8; 3]>(), "i3: stride*len is not the extent");

    // The degenerate array has no second element, so only the two claims that do not
    // need one apply — recorded rather than silently skipped.
    let u1 = cases()[1].info;
    assert_eq!(u1.stride, size_of::<u32>(), "u1: descriptor stride is not size_of::<u32>()");
    assert_eq!(u1.stride * u1.len, size_of::<[u32; 1]>(), "u1: stride*len is not the extent");
    println!("C5 gate 4: u1 is len 1 -- measured spacing is not defined and is not asserted");
}

/// Gate 4's **non-vacuity clause**: the fixture really does put padding after an array,
/// so "spacing derived from the next field's offset" is a *different number* from
/// `size_of::<T>()`, and a `stride` taken from there would be caught.
///
/// Without this the gate is a tautology on a tightly packed struct.
#[test]
fn the_fixture_really_does_pad_after_an_array() {
    use std::mem::offset_of;
    let neighbour_spacing = offset_of!(Arrays, tail) - offset_of!(Arrays, i3);
    let extent = size_of::<[i8; 3]>();
    println!(
        "C5 gate 4 non-vacuity: offset_of!(tail) - offset_of!(i3) = {neighbour_spacing}, \
         size_of::<[i8;3]>() = {extent}, size_of::<Arrays>() = {}",
        size_of::<Arrays>()
    );
    assert!(
        neighbour_spacing > extent,
        "the C5 fixture has NO padding after `i3`, so gate 4 cannot distinguish a stride \
         taken from the array's real layout from one taken from neighbouring field \
         offsets -- re-order the fixture or widen the trailing field"
    );
}

/// Distance in bytes between two elements of the same array, read off their real
/// addresses. `usize` arithmetic on addresses, not pointer offsets — no provenance is
/// involved and nothing is dereferenced.
fn spacing<T>(a: &T, b: &T) -> usize {
    (std::ptr::from_ref(b) as usize) - (std::ptr::from_ref(a) as usize)
}

// ───────────────────────── the case table's own precondition ────────────────

/// The four shapes gate 1 names are all present, and their descriptors agree with the
/// fixture's real field types — the non-vacuity clause for gates 1 and 2.
#[test]
fn the_case_table_covers_the_four_named_shapes() {
    let cases = cases();
    let shapes: Vec<(ScalarKind, usize)> =
        cases.iter().map(|c| (c.info.elem, c.info.len)).collect();
    println!("C5 gate 1 shapes: {shapes:?}");
    assert_eq!(
        shapes,
        vec![
            (ScalarKind::F32, 4),
            (ScalarKind::U32, 1),
            (ScalarKind::U8, 60),
            (ScalarKind::I8, 3),
        ],
        "the C5 case table no longer covers [f32;4] (flagship), [u32;1] (degenerate), \
         [u8;60] (inline string) and [i8;3] (signed)"
    );
    for case in &cases {
        assert_eq!(
            case.samples.len(),
            case.info.len,
            "{}: the sample list and the descriptor disagree about `len`",
            case.name
        );
    }
}
