//! Two materials of one name, and the reason Aether owns this one instead of deferring.
//!
//! MEASURED with real rustc: the bare expansion is two `pub fn twice()`s, so rustc reports E0428
//! and puts BOTH of its labels on the `aether!` token — no user token is named anywhere. A
//! material emits no derive and no trait bound, so unlike `component`×`component` there is no
//! second, localized error to rescue it. §7.1's "defer when the downstream layer already lands
//! well" therefore does not apply here, and this golden is what pins BOTH spans on user tokens.
use aether::aether;

aether! {
    material twice { base: (0.0, 0.0, 0.0) }
    material twice { base: (1.0, 1.0, 1.0) }
}

fn main() {}
