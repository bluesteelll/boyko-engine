//! CORE C9 gate 5(a) — **a positive CONTROL, labelled as one.**
//!
//! `storage = "dense"` is NOT refused. A dense component has real per-row bytes at a stable
//! address and is the one non-table kind that is fully readable; its *enumeration* problem
//! belongs to `docs/REFLECTION-PLAN-ECS.md` (analysis B.3), and refusing it would make the
//! design decline the one flagship component it can fully read.
//!
//! # What this control excludes, and what it cannot
//!
//! It excludes *"the refusals ate the dense case too"* — a bitset refusal mis-scoped to
//! match any `storage` key, which is a one-token mis-edit away from the real one. That is
//! real value, and it is why C9 keeps it.
//!
//! ⚠️ But it was MEASURED green **before any C9 code existed**: `storage_kind(id) == Dense`,
//! `type_info_of(id).is_some()`, two `Prim(F32)` fields at offsets 0 and 4. Under C9's other
//! reds it cannot move. So it carries its own RED — *mis-scope the bitset refusal to match
//! any `storage` key* — and that is the mutation that makes it a gate rather than a
//! description.

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::core::component::component_registry::{StorageKind, storage_kind};
use boyko_macros::Component;
use boyko_reflect::{Reflect, ScalarKind, ValueKind, type_info_of};

/// The subject: dense storage AND reflection, which is a legitimate pair.
#[derive(Component, Default)]
#[component(reflect, storage = "dense")]
#[repr(C)]
pub struct DenseReflected {
    /// Offset 0.
    pub x: f32,
    /// Offset 4.
    pub y: f32,
}

fn main() {
    let id = <DenseReflected as ComponentTrait>::component_id().0;

    assert_eq!(
        storage_kind(id),
        StorageKind::Dense,
        "the subject must actually be dense-classified, or this control is a table \
         component wearing a dense name and proves nothing about the `storage` key"
    );
    assert!(
        type_info_of(id).is_some(),
        "a `storage = \"dense\"` component with `#[component(reflect)]` installed NO \
         descriptor. The bitset refusal has been mis-scoped to the `storage` key rather \
         than to the `bitset` value, and the design now declines the one non-table kind it \
         can fully read"
    );

    let ti = <DenseReflected as Reflect>::TYPE_INFO;
    assert_eq!(ti.fields.len(), 2);
    assert_eq!(ti.fields[0].kind, ValueKind::Prim(ScalarKind::F32));
    assert_eq!(ti.fields[0].offset, 0);
    assert_eq!(ti.fields[1].kind, ValueKind::Prim(ScalarKind::F32));
    assert_eq!(ti.fields[1].offset, 4);
}
