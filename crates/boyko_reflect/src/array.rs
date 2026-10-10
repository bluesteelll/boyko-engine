//! `[T; N]`-of-`Prim` element access (CORE C5, decisions D12/D19): the by-index
//! reader and writer an [`ArrayInfo`] describes.
//!
//! An array field is **not** readable through [`FieldInfo::get`] — that slot is `None`
//! for every non-`Prim` kind, and [`validate`] reds on an `Array` that carries one
//! ([`Violation::ArrayWithScalarAccessor`]). Elements are reached here instead, by
//! index, through the descriptor's `offset + stride + count`, all three of them
//! `const`-baked. Nothing is materialized: there is no `Vec<Scalar>` of elements and
//! no iterator that builds one, which is why §3.3's *array read* row claims **0
//! allocations** and why C5 gate 3 measures it rather than asserting it.
//!
//! # Scope: `T` is a `Prim`, and only that (D19)
//!
//! `[[f32; 4]; 4]` is an array *of arrays* and needs either a 2-D descriptor or a
//! recursive [`ArrayInfo`]; both are v2, and the recursive form was rejected outright
//! because it moves an unbounded descend into the *descriptor*, where §3.1's
//! acyclicity argument — which is about Rust types, not about `&'static` graphs — does
//! not reach. The named exclusion is `boyko_render`'s `csm_config.rs`
//! `view_proj: [[f32;4];4]`, which C9 refuses rather than silently flattens.
//! `GpuTransform3D`/`TrsPacked` — the case D12 exists for — is `[f32;4]`×3 and is
//! fully covered.
//!
//! # The bounds check is a RELEASE check, and it runs BEFORE the multiply
//!
//! Same reasoning as D11's kind check: the legitimate `--release --features reflect`
//! editor build has no `debug_assert!` in it, and a stale `(ComponentId, field, index)`
//! triple after a hot-reload is exactly the input that arrives there. So `index <
//! len` is an ordinary branch that answers `None`/`false`.
//!
//! **It is also ordered first for a second reason, and that reason is MEASURED rather
//! than reasoned.** The offset is `index * stride`, and for a stale `usize::MAX` index
//! that product overflows. Moving the check below the multiply was run as C5's third
//! RED (2026-08-21, this worktree): the **debug** leg reds with *"attempt to multiply
//! with overflow"* raised at `array.rs:63` — a **panic out of a library whose whole
//! contract is to refuse rather than fail**, and the exact shape a maintainer "fixes"
//! with `#[should_panic]` — while the **release** leg stays **green**, because the
//! wrapped product is discarded by a check that is still keyed on `index`.
//!
//! So the cost of the wrong order is a debug-only panic, *not* a release wild read —
//! recorded that way because the first draft of this paragraph claimed the wild read
//! and the red refuted it. (`usize::MAX * stride` wraps to `2^64 - stride`, which is
//! above any real extent, so even an offset-keyed check refuses it; there is no input
//! in this shape that reaches a wild pointer.) Ordering the check first is what makes
//! `usize::MAX` a plain `None` in **both** profiles, which is what C5 gate 2 asserts in
//! each.
//!
//! [`FieldInfo::get`]: crate::type_info::FieldInfo::get
//! [`validate`]: crate::type_info::validate
//! [`Violation::ArrayWithScalarAccessor`]: crate::type_info::Violation::ArrayWithScalarAccessor

use crate::prim;
use crate::scalar::Scalar;
use crate::type_info::ArrayInfo;

/// Reads element `index` of the array `info` describes, as one [`Scalar`].
///
/// Returns `None` — **before computing any offset** — when `index >= info.len`.
/// There is no other `None`: `info.elem` is a `ScalarKind`, so the element reader
/// always produces a value.
///
/// # Safety
///
/// `p` must point at **element 0** of a live, initialized `[T; N]` whose layout `info`
/// describes truthfully: `info.elem` is `T`'s scalar kind, `info.stride` is
/// `size_of::<T>()`, and `info.len` is `N`. The pointer must be `align_of::<T>()`-
/// aligned and valid for reads of `stride * len` bytes, with provenance covering the
/// whole array — the contract `offset_of!`-derived field arithmetic already needs. The
/// array must not be concurrently written.
///
/// A descriptor that lies about `stride` or `len` makes this call out-of-bounds; that
/// is why C5 gate 4 pins `stride` against `offset_of!`-derived element spacing instead
/// of trusting the number.
pub unsafe fn array_get(p: *const u8, info: &ArrayInfo, index: usize) -> Option<Scalar> {
    if index >= info.len {
        return None;
    }
    // SAFETY: the bounds check above proved `index <= len - 1`, so `index * stride <=
    // (len - 1) * stride < stride * len`, which the caller guarantees is the array's
    // readable extent — the derived pointer is in bounds, element-aligned (a multiple of
    // `stride == size_of::<T>()` from an `align_of::<T>()`-aligned base) and inherits the
    // caller's provenance. `getter_for(info.elem)` is the reader baked for `T`, which the
    // caller guarantees `info.elem` names.
    let elem = unsafe { p.add(index * info.stride) };
    let get = prim::getter_for(info.elem);
    // SAFETY: as above -- `elem` satisfies exactly the per-kind reader's contract.
    Some(unsafe { get(elem) })
}

/// Writes `v` into element `index`, returning `false` — **before touching memory** —
/// when `index >= info.len` or when `v`'s kind is not `info.elem`.
///
/// The two refusals are deliberately indistinguishable in the return type: both are
/// "this write did not happen", and neither allocates a diagnostic on the way out
/// (§3.3). The kind half is the per-kind setter's own release check (D11), so a
/// non-canonical payload is refused by the same branch.
///
/// # Safety
///
/// As [`array_get`], with write permission: the array must be writable for
/// `stride * len` bytes and no reference into it may be live across this call.
pub unsafe fn array_set(p: *mut u8, info: &ArrayInfo, index: usize, v: Scalar) -> bool {
    if index >= info.len {
        return false;
    }
    // SAFETY: as `array_get`, with write permission -- the bounds check has already
    // proved the element is in the array's extent, and the caller guarantees exclusive
    // access for the duration of the call.
    let elem = unsafe { p.add(index * info.stride) };
    let set = prim::setter_for(info.elem);
    // SAFETY: as above; the setter refuses a mismatched kind before storing, so a wrong
    // `v` costs a `false` rather than a corrupted element.
    unsafe { set(elem, v) }
}
