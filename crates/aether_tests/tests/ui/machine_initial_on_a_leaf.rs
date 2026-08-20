//! `initial` on a childless state can never retarget anything — silently ignoring it would let
//! the author believe a nested chart exists.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial Idle;
        state Idle {
            initial Running;
        }
    }
}

fn main() {}
