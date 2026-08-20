//! §3.6's own example: a two-component color. The error lands on the TUPLE — neither the key nor
//! any one component is the thing that is wrong.
use aether::aether;

aether! {
    material gold { base: (1.0, 0.72), metallic: 1.0 }
}

fn main() {}
