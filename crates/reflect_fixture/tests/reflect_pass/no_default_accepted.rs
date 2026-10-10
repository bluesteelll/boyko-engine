//! CORE C9 gate 6 / D20 — `#[reflect(no_default)]` is the way OUT of the `Default`
//! requirement, and `default_in_place: None` is a real state rather than a hole.
//!
//! This file is `reflect_compile_fail/missing_default_rejected.rs` plus one attribute. The
//! opt-out suppresses BOTH the `default_in_place` slot and the `ReflectDefault` witness, so
//! a green here says the two moved together — a suppression that dropped only the slot
//! would still fail to compile, and one that dropped only the witness would bake a slot
//! calling `Default` on a type that has none.
//!
//! `None` has a real consumer: `add_default` answering `Err(Refusal::NoDefault)` — an
//! inspector's "Add Component" greying the button out rather than fabricating a value.
//!
//! A `t.pass()` fixture is **run** (trybuild's `check_pass` executes the binary and
//! requires success), which is what lets `main` assert the slot rather than merely
//! observing that the file compiles.

use boyko_macros::Component;
use boyko_reflect::Reflect;

/// The subject: no `Default` impl anywhere, opted out.
#[derive(Component)]
#[component(reflect)]
#[reflect(no_default)]
pub struct NoDefaultOptedOut {
    /// A `Prim`, so nothing else about this type is refusable.
    pub value: u32,
}

fn main() {
    let ti = <NoDefaultOptedOut as Reflect>::TYPE_INFO;
    assert!(
        ti.default_in_place.is_none(),
        "`#[reflect(no_default)]` must bake `default_in_place: None`. A `Some` here would \
         mean the opt-out suppressed the WITNESS but not the slot, leaving a fn pointer \
         that calls `Default` on a type that does not implement it"
    );
    assert_eq!(ti.fields.len(), 1, "the opt-out changes the default slot and nothing else");
    assert_eq!(ti.fields[0].name, "value");
}
