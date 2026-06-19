// SP4 fix: `ScratchSolveView` does NOT implement `DerefMut<Target = [T]>` (nor
// `Deref`), so it cannot be coerced to a whole-buffer mutable slice. The SP4
// reborrow is un-typeable.
//
// Expected diagnostic: `ScratchSolveView` cannot be dereferenced / no `DerefMut`
// impl, so `&mut *view` is not a slice.

use boyko_ecs::ecs::core::component::scratch::ScratchSolveView;

fn must_not_compile(view: ScratchSolveView<'_, f32>) {
    let mut view = view;
    // No `DerefMut<Target = [f32]>` impl exists for the solve view.
    let _whole: &mut [f32] = &mut *view;
}

fn main() {}
