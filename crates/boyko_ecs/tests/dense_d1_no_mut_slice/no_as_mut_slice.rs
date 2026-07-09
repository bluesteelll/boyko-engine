// SP4 fix (Dense plan Decision 8): `DenseSolveView` exposes per-slot `row_ptr`
// ONLY — there is NO `as_mut_slice`/`as_mut_bytes`/whole-buffer `&mut` method.
// A worker therefore cannot reborrow the whole column (the structural SP4 fix).
//
// Expected diagnostic: no method `as_mut_bytes` (nor `as_mut_slice`) found for
// `DenseSolveView`.

use boyko_ecs::ecs::core::component::dense::DenseSolveView;

fn must_not_compile(view: DenseSolveView<'_>) {
    let mut view = view;
    // The build view has `as_mut_bytes`; the solve view deliberately does not.
    let _whole: &mut [u8] = view.as_mut_bytes();
}

fn main() {}
