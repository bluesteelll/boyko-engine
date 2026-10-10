//! §3.3: scheduling clauses without a `plugin` header have nowhere to hold the registration.
use aether::aether;

aether! {
    system tick() on update {}
}

fn main() {}
