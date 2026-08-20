//! Generated names are the snake_case COLLAPSE of the flattened state path, and that collapse
//! is lossy: `AB` and `A_b` are two distinct enum variants that mint one `__aether_m__a_b__e`.
//! rustc would report the duplicate on generated tokens; Aether reports it on the two states.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial AB;
        state AB {
            on E => AB;
        }
        state A_b {
            on E => A_b;
        }
    }
}

fn main() {}
