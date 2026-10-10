//! §2's case convention for the FN-producing construct, diagnosed at the NAME's own span with a
//! rename suggestion — the mirror of `lowercase_component.rs`, which refuses the opposite case for
//! the TYPE-producing ones.
use aether::aether;

aether! {
    material Gold { base: (1.0, 0.72, 0.30), metallic: 1.0 }
}

fn main() {}
