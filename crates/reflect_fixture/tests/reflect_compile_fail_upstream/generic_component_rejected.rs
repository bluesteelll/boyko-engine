//! **NOT one of C9's refusals — an upstream regression pin (D34).**
//!
//! C9's matrix carried a *"generic type parameters"* row whose stated hazard was a
//! per-impl `static TYPE_INFO` collapsing across monomorphizations. MEASURED on
//! rustc 1.97.1 with **no `reflect` opt-in in the input at all**: a generic
//! `#[derive(Component)]` struct already fails, because the derive emits `impl #name` and
//! `impl … Component for #name` from the bare ident and threads no generics
//! (`crates/boyko_macros/src/component.rs`'s `impl #name` / `impl … for #name`). With
//! `#[component(reflect)]` added the count goes up and the reflect seam is still never
//! entered — so the row's hazard is **unreachable**, and a refusal for a case rustc
//! already refuses is *a fixture whose red cannot fire*: delete C9's refusal and the
//! program still does not compile.
//!
//! The row was therefore struck from `REFUSALS`, and this file is what remains: a pin on
//! the diagnostic the tree produces today. **The obligation returns** the day the derive
//! threads generics — this file is what will red and say so, and §6 defers generics to v2.
//!
//! # Why it lives outside the census directory, and runs in one leg only
//!
//! It pins **rustc's and `#[derive(Component)]`'s** prose rather than C9's, so counting it
//! against `REFUSALS` would make the census claim a rule C9 does not author. And its
//! output DIFFERS between the two feature legs — the reflect emission adds its own
//! obligations on top of the same underlying failure — so one blessed file cannot serve
//! both. It runs in the feature-ON leg, where the input is the one C9's struck row named.

use boyko_macros::Component;

/// The subject: one type parameter, which the derive does not thread.
#[derive(Component, Default)]
#[component(reflect)]
pub struct GenericComponent<T: Default + 'static> {
    /// The generic payload.
    pub value: T,
}

fn main() {}
