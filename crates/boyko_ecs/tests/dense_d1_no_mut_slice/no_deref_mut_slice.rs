// SP4 fix (Dense plan Decision 8): `DenseSolveView` does NOT implement
// `DerefMut<Target = [T]>` (nor `Deref`), so it cannot be coerced to a
// whole-buffer mutable slice. The SP4 reborrow is un-typeable.
//
// Expected diagnostic: `DenseSolveView` cannot be dereferenced / no
// `DerefMut` impl, so `&mut *view` is not a slice.

use boyko_ecs::ecs::core::component::dense::DenseSolveView;

fn must_not_compile(view: DenseSolveView<'_>) {
    let mut view = view;
    // No `DerefMut<Target = [u8]>` impl exists for the solve view.
    let _whole: &mut [u8] = &mut *view;
}

fn main() {}
