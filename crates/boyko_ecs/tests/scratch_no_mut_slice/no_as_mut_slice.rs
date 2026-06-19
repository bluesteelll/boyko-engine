// SP4 fix: `ScratchSolveView` exposes per-element `row_ptr(i) -> *mut T` ONLY —
// there is NO `as_mut_slice`/whole-buffer `&mut` method. A worker therefore
// cannot reborrow the whole column (the structural SP4 fix).
//
// Expected diagnostic: no method `as_mut_slice` found for `ScratchSolveView`.

use boyko_ecs::ecs::core::component::scratch::ScratchSolveView;

fn must_not_compile(view: ScratchSolveView<'_, f32>) {
    let mut view = view;
    // The build view has `as_mut_slice`; the solve view deliberately does not.
    let _whole: &mut [f32] = view.as_mut_slice();
}

fn main() {}
