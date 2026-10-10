//! **NOT one of C9's refusals — an upstream regression pin (D34).**
//!
//! C9's matrix carried a `#[repr(packed)]` row whose stated reason — *taking `&field` on a
//! packed type is UB* — is true of the accessors (`crates/boyko_reflect/src/prim.rs`'s
//! scalar reads take a shared reborrow, which requires alignment). It is not, however, a
//! reason C9 can act on, and the measurement runs both ways:
//!
//! * `#[repr(packed)]` with a field needing align > 1, under a **plain**
//!   `#[derive(Component)]` with no `reflect` anywhere → `error[E0793]: reference to field
//!   of packed struct is unaligned`, from the derive's own emission. Already refused.
//! * `#[repr(packed)]` with **every** field of align 1 → compiles, installs, and is
//!   **sound**: the struct is align 1, so every `base + offset` read is aligned by
//!   construction.
//!
//! The *reachable* set of the row is exactly its *harmless* set, and its unsound set is
//! refused by a diagnostic C9 neither authors nor controls. Struck from `REFUSALS`; the
//! obligation **returns** the day `#[derive(Component)]` stops taking a reference to a
//! field, and this file is what will red and say so.
//!
//! See `generic_component_rejected.rs`'s header for why both pins sit outside the census
//! directory and run in the feature-ON leg only.

use boyko_macros::Component;

/// The subject: a packed struct whose second field needs align 4.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(packed)]
pub struct PackedComponent {
    /// Align 1.
    pub flag: u8,
    /// Align 4 in an align-1 struct — the unaligned reference.
    pub value: u32,
}

fn main() {}
