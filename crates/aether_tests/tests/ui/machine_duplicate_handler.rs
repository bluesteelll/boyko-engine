//! §3.5: two handlers for one event in one state — the SECOND `on` errs, a note marks the first.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial A;
        state A {
            on E => A;
            on E => A;
        }
    }
}

fn main() {}
