//! CORE C9 / FIX Mi3 — **the ACCEPTANCE half of the `#[repr(Int)]` enum rule**, which
//! until now was claimed in three places and gated in none.
//!
//! `fieldless_enum_without_repr_rejected.rs` pins the refusal. Nothing pinned the twin:
//! `has_integer_repr` returning **true** was reached by no test in the tree.
//!
//! * `data_carrying_enum_rejected.rs` carries `#[repr(u8)]`, but the payload branch fires
//!   first and the function is never called;
//! * `fieldless_enum_without_repr_rejected.rs` has no repr, so it exercises only the
//!   `false` result;
//! * and C9 moved `c7_derive_bake.rs`'s only accepted-enum subject into the refusal corpus
//!   (D38), which is what left the accepted side with no subject at all.
//!
//! MEASURED before this fixture existed: force `has_integer_repr` to `return false`
//! unconditionally and **everything stays green** — the refusal corpus (3 passed), the
//! census (8 passed) and `c7_derive_bake` (16 passed). An over-broad enum refusal would
//! have shipped green while three documents went on claiming the opposite: this file's
//! twin header (*"a fieldless enum that DOES carry `#[repr(u8)]` is **accepted**"*),
//! `c7_derive_bake.rs`'s struck-arm note (*"the arm still runs for a fieldless
//! `#[repr(Int)]` enum, which stays ACCEPTED"*), and `REFUSALS`'s own remedy text, which
//! tells the user to *"add `#[repr(u8)]`"*.
//!
//! # The RED this fixture exists to produce
//!
//! `has_integer_repr` → `false`: the subject below stops compiling with
//! `fieldless_enum_without_repr_rejected`'s message, and the `t.pass()` glob reds. A
//! remedy a refusal names must be a remedy that works, and this is the only thing in the
//! tree that says so.
//!
//! # Why `TypeKind::Opaque` is the RIGHT assertion here, and is not a lie
//!
//! `Opaque` with `fields: &[]` for a fieldless enum is *incomplete, not false* — the type
//! genuinely has no fields — which is exactly why §5 lets C9 land ahead of C10 while the
//! two D38 shapes are refused. C10 replaces this `kind` with `TypeKind::Enum` plus an
//! `EnumInfo`, and when it does, this assertion is what makes the change visible instead
//! of silent.
//!
//! A `t.pass()` fixture is **run** (trybuild's `check_pass` builds the binary and requires
//! it to exit successfully), so `main` below asserts the descriptor rather than merely
//! observing that the file compiles.

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_macros::Component;
use boyko_reflect::{Reflect, TypeKind, type_info_of};

/// The subject: `fieldless_enum_without_repr_rejected.rs`'s `NoRepr` plus the one
/// attribute its message tells the user to add.
#[derive(Component, Default)]
#[component(reflect)]
#[repr(u8)]
pub enum Phase {
    /// The `Default` variant.
    #[default]
    Idle,
    /// Second.
    Running,
    /// Third.
    Done,
}

fn main() {
    let ti = <Phase as Reflect>::TYPE_INFO;

    assert_eq!(
        ti.kind,
        TypeKind::Opaque,
        "a fieldless `#[repr(u8)]` enum bakes `TypeKind::Opaque` until C10 replaces it with \
         `TypeKind::Enum`. If this reads `Enum`, C10 landed and this fixture is the place \
         to state the new contract -- not a failure"
    );
    assert_eq!(
        ti.fields.len(),
        0,
        "`fields: &[]` is TRUE for a fieldless enum, which is the whole reason this shape \
         stays accepted while the data-carrying twin is refused"
    );
    assert_eq!(
        ti.size,
        1,
        "`#[repr(u8)]` is the guaranteed discriminant width the refusal demands; a size \
         other than 1 means the repr the derive accepted is not the repr it got"
    );

    assert!(
        ti.default_in_place.is_some(),
        "the enum derives `Default`, so D20's slot must be baked -- an accepted item whose \
         `default_in_place` were `None` would mean the witness and the slot disagreed"
    );

    let id = <Phase as ComponentTrait>::component_id().0;
    assert!(
        type_info_of(id).is_some(),
        "an ACCEPTED item must reach C8's install seam. A refusal suppresses the whole \
         emission, so a `None` here is the acceptance half failing one step later than the \
         compile"
    );
}
