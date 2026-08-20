//! §5.4: flattening CONCATENATES the state path, so two chart positions can collapse onto one
//! generated name. Aether names both positions and the name they share, rather than letting
//! rustc report a duplicate enum variant on generated tokens.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial A;
        state A {
            initial BC;
            state BC {}
        }
        state AB {
            initial C;
            state C {}
        }
    }
}

fn main() {}
