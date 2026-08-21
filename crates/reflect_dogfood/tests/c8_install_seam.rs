//! **CORE C8 gate 1 (D28) — the install seam reads back the RIGHT descriptor, at the
//! RIGHT id, across a crate boundary.**
//!
//! # Why this reads an ADDRESS and not `is_some()`
//!
//! C8's gate 1 was originally *"`type_info_of(T::component_id().0)` is `Some`"*. That
//! form is blind to **both** of the seam's characteristic failures, and D28 records why:
//!
//! * `REFLECT` is write-once, **first writer wins**
//!   (`boyko_reflect/src/registry.rs:83-94`). So an install that publishes the *wrong*
//!   descriptor — a sibling's `TYPE_INFO` — is never corrected, and an `is_some()` gate
//!   stays green permanently.
//! * `install_type_info(0, …)` is indistinguishable from `install_type_info(raw, …)` for
//!   any subject that happens to hold id 0. That is C7's *"baked zeros into offsets where
//!   every subject was one field wide"* red wearing new clothes, and it is why this gate
//!   requires **two** subjects and asserts their ids differ **before** using them.
//!
//! # Why the subjects are `reflect_dogfood`'s and not this file's
//!
//! This gate discharges an obligation C7 scheduled **at C8** and C8's own gate list did
//! not carry: `tests/c7_cross_crate_address.rs:36-38` states that the one-address-per-type
//! property *"goes live at **CORE C8**'s install seam and at **ECS EG8**, both of which
//! read a descriptor from a crate other than the one that defined it"*. The seam is where
//! a descriptor emitted by the **defining** crate is published into a table read by
//! everyone else, so the gate must cross that boundary: `ProbeLeaf` / `ProbeRoot` are
//! defined in `src/address.rs` and read here, and the reference address is taken through
//! the defining crate's own `#[inline(never)]` reader, never through this crate's
//! materialization of the same const.
//!
//! # The invocation is part of the gate
//!
//! ```text
//! cargo test -p reflect-dogfood --features reflect-dogfood/reflect --test c8_install_seam
//! ```
//!
//! The install is `#[cfg(feature = "reflect")]` evaluated in the **expanding** crate (D2)
//! and no package's `default` enables `reflect`, so a plain `cargo test -p reflect-dogfood`
//! compiles this file to nothing and exits 0 — a vacuous pass on the green side *and* on
//! every red side. The output must read `running [1-9]`.
#![cfg(feature = "reflect")]

use boyko_ecs::ecs::core::component::component::Component;
use boyko_reflect::{Reflect, type_info_of};
use reflect_dogfood::address::{
    ProbeLeaf, ProbeRoot, probe_leaf_type_info_in_defining_crate,
    probe_root_type_info_in_defining_crate,
};

/// **The gate.** For each subject: the descriptor the registry hands back for
/// `T::component_id()` is *the same object* the defining crate emitted — compared by
/// address, not by contents.
///
/// The two subjects are not redundancy. Red (ii) of D28 — `install_type_info(0, …)` — is
/// invisible to any one-subject gate whose subject happens to be the first type this
/// binary registers, because slot 0 is exactly where that subject belongs anyway. With two
/// subjects one of them necessarily holds a non-zero id and reads `None`.
#[test]
fn the_install_seam_publishes_each_types_own_descriptor_at_its_own_id() {
    let leaf_id = <ProbeLeaf as Component>::component_id().0;
    let root_id = <ProbeRoot as Component>::component_id().0;

    // Instrument precondition, stated before it is used: two subjects that shared an id
    // would make the whole gate one subject wearing two names, and the literal-`0`
    // mutation would be invisible again.
    assert_ne!(
        leaf_id, root_id,
        "instrument precondition: ProbeLeaf and ProbeRoot must hold DISTINCT component \
         ids, or this gate degenerates to a one-subject gate and cannot see an install \
         that ignores its id argument"
    );

    for (id, defining, what) in [
        (leaf_id, probe_leaf_type_info_in_defining_crate(), "ProbeLeaf"),
        (root_id, probe_root_type_info_in_defining_crate(), "ProbeRoot"),
    ] {
        let installed = type_info_of(id).unwrap_or_else(|| {
            panic!(
                "`{what}` has component id {id} and NOTHING is installed there. Either the \
                 derive emitted no install slot in `component_id()` (C8's whole subject), \
                 or the slot was emitted with the wrong id -- e.g. a literal `0`, in which \
                 case every subject piles into slot 0, the first writer wins, and every \
                 later subject reads None exactly like this."
            )
        });
        assert!(
            std::ptr::eq(installed, defining),
            "`{what}` (id {id}) is registered as {:p}, but the crate that DEFINES it \
             emitted {:p}. The install seam published a descriptor that is not this \
             type's -- and `REFLECT` is write-once (first writer wins), so the wrong \
             descriptor will NEVER be corrected: every editor read of this id is a \
             coherent lie for the rest of the process.",
            std::ptr::from_ref(installed),
            std::ptr::from_ref(defining),
        );
    }
}

/// The seam is **idempotent at the source level too**: `component_id()` memoizes through
/// a `OnceLock`, so the second call runs no install at all and the table is unchanged.
///
/// This is not a restatement of `registry.rs`'s write-once unit test, which installs twice
/// by hand. Here the question is whether the *emitted* slot can be re-entered — the answer
/// is that it sits inside `get_or_init`, and a slot placed outside it would still pass the
/// gate above while paying an install on every id lookup.
#[test]
fn a_second_component_id_touch_changes_nothing() {
    let first = type_info_of(<ProbeLeaf as Component>::component_id().0);
    let second = type_info_of(<ProbeLeaf as Component>::component_id().0);
    match (first, second) {
        (Some(a), Some(b)) => assert!(
            std::ptr::eq(a, b),
            "two touches of ProbeLeaf::component_id() produced two different registered \
             descriptors ({:p} then {:p})",
            std::ptr::from_ref(a),
            std::ptr::from_ref(b),
        ),
        _ => panic!("ProbeLeaf's descriptor is not installed; see the gate above"),
    }
}

/// A subject the derive was told to reflect is reachable through the registry **without**
/// the caller naming `Reflect` at all — which is the seam's purpose: an editor holds a
/// `ComponentId`, not a type.
///
/// The `<T as Reflect>::TYPE_INFO` on the right-hand side is the oracle, not the route.
#[test]
fn the_registry_route_and_the_trait_route_reach_one_object() {
    let by_id = type_info_of(<ProbeRoot as Component>::component_id().0)
        .expect("ProbeRoot is annotated, so the seam installed its descriptor");
    let by_trait = <ProbeRoot as Reflect>::TYPE_INFO;
    assert!(
        std::ptr::eq(by_id, by_trait),
        "the id route reaches {:p} and the trait route reaches {:p} -- an editor holding \
         only a ComponentId would inspect a different object than a compiler-visible \
         `<T as Reflect>` read of the same type",
        std::ptr::from_ref(by_id),
        std::ptr::from_ref(by_trait),
    );
    assert_eq!(
        by_id.type_name,
        std::any::type_name::<ProbeRoot>(),
        "the descriptor reached by id names a different type than the id's own"
    );
}
