//! A planned construct names its rung — a misspelling and a not-yet-shipped construct are
//! different failures and deserve different messages.
use aether::aether;

aether! {
    material gold { }
}

fn main() {}
