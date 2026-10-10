//! The degenerate case of the flattening-collision check: two siblings spelled the same.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial Idle;
        state Idle {}
        state Idle {}
    }
}

fn main() {}
