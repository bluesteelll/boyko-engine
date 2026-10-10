//! `P0`'s `on E` is shadowed for every leaf by `A`'s own `on E`, so no leaf's inheritance walk
//! ever reaches it — and a target resolved only along that walk would never be looked up. A
//! chart that names a state which does not exist is broken whether or not anything reaches it.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial P0;
        state P0 {
            initial A;
            on E => Nowhere;
            state A {
                on E => Top;
            }
        }
        state Top {}
    }
}

fn main() {}
