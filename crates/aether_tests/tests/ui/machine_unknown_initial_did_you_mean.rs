//! §3.5: an unknown `initial` target lists the declared states and suggests the near-miss.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial Playing;
        state Playing {
            initial Runing;
            state Running {}
            state Paused {}
        }
    }
}

fn main() {}
