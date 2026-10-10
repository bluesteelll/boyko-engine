//! The 3.4 rule: the empty participant context is deliberately not defaulted.
use aether::aether;

aether! {
    event Damage {
        victim: entity,
        amount: f32,
    }
}

fn main() {}
