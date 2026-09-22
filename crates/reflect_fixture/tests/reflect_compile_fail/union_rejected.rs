//! CORE C9 / D38 — a union as the component ITSELF, refused at the union item.
//!
//! The same arm as the data-carrying enum, the same lie — and **no document had a row for
//! it**. GATES G5's corpus carried `union_rejected` from the start; CORE's refusal matrix
//! carried nothing, so the plan that owned the derive would have shipped a derive that
//! accepts a union and bakes `fields: &[]` for it, with `validate` returning `Ok`.
//!
//! MEASURED 2026-08-26 with C9's union refusal removed from the derive and the subject
//! below unchanged otherwise: `#[component(reflect)]` on a two-field union bakes
//! `kind=Opaque fields=0 size=4 align=4` — the coherent lie this row refuses.
//!
//! ⚠️ **The same claim, made earlier in this header without the opt-out below, was not
//! measurable on this subject.** A union cannot `#[derive(Default)]`, so with the union
//! refusal removed the emission path reaches D20's `ReflectDefault` bound and the file
//! fails with *two* `error[E0277]: … `Overlap` does not implement it` — C7's refusal, not
//! C9's. Whatever union produced those four numbers, it was not this one until
//! `#[reflect(no_default)]` was put on it.
//!
//! # Why the subject carries `#[reflect(no_default)]`, and why it is load-bearing
//!
//! Without it this row is in exactly the class
//! `the_upstream_pins_are_not_counted_as_refusals` excludes `generic_component_rejected`
//! and `repr_packed_rejected` for: *deleting C9's refusal leaves the program non-compiling
//! anyway*, so the prescribed red — delete the refusal, watch the fixture compile — is
//! unobservable. The red was not dead (trybuild reports a **mismatch**, so it still fires)
//! but it was measuring D20's bound rather than D38's arm, and a fixture filed under one
//! rule while pinning another is how a rule quietly stops being tested.
//!
//! With the opt-out, deleting the `Data::Union` arm makes this file **compile** —
//! MEASURED, and that is the red this fixture is for.
//!
//! The audit's lens found the original gap by asking the reachability question in the
//! OPPOSITE direction from D34's: not *"is this row's input already refused upstream?"*
//! but *"is this accepted shape in any row?"*.

use boyko_macros::Component;

/// The subject: two overlapping members, so `fields: &[]` would be a claim about a type
/// that has some. `no_default` is not decoration — see the header.
#[derive(Component, Clone, Copy)]
#[component(reflect)]
#[reflect(no_default)]
#[repr(C)]
pub union Overlap {
    /// Read as an integer.
    pub bits: u32,
    /// The same bytes, read as a float.
    pub value: f32,
}

fn main() {}
