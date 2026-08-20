//! §4's duplicate-name rule at the boundary rung A5 measured: a `scene` and a `material` of one
//! name both expand to a bare `pub fn`, and rustc's E0428 for that shape puts BOTH of its labels
//! on the `aether!` token, naming no user token at all. Aether therefore owns it — across kinds
//! as well as within one — and this golden is what pins both spans on the user's own idents.
use aether::aether;

aether! {
    material lab { base: (0.0, 0.0, 0.0) }

    scene lab {
        entity { }
    }
}

fn main() {}
