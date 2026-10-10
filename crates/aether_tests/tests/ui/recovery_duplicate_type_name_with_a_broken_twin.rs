//! W2's type-half probe — the same collision on the half §7.1 deliberately does NOT own.
//!
//! §4's duplicate rule covers the fn-producing constructs only: a duplicated TYPE carries a
//! derive, so rustc reports it against the user's own item and a second Aether check could only be
//! worse (duplicated checks drift). The recovery stub does not change that decision, but it does
//! change what rustc sees — the stub now stands in for the broken `component Foo`, so the
//! duplicate exists in the expansion and E0428 fires.
//!
//! That is the point of pinning it: the stub is emitted under `quote_spanned!(name.span())`, so
//! rustc's primary label lands on the user's `Foo` in the broken construct and its "first defined
//! here" note on the `Foo` of the `tag`. BOTH labels are user tokens. The failure mode this file
//! rules out is the same one the fn half was measured for — a duplicate whose every label points
//! at the `aether!` token, where the reader learns nothing.
use aether::aether;

aether! {
    tag Foo;

    component Foo { hp f32 }
}

fn main() {}
