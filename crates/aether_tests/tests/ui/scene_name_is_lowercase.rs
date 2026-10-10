//! §2's case rule for the value-producing constructs: a scene expands to a SPAWN FN, so an
//! UpperCamelCase name reads like a type at the `add_startup_system` call site.
use aether::aether;

aether! {
    scene Lab {
        entity { }
    }
}

fn main() {}
