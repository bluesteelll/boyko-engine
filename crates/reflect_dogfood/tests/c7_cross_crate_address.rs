//! **CORE C7 / D22 — one stable address per type, observed ACROSS A CRATE BOUNDARY.**
//!
//! # Why this file exists, and why the existing clause was not enough
//!
//! `reflect_fixture`'s `c7_derive_bake.rs` already carries
//! `a_types_descriptor_has_exactly_one_address_within_this_crate` (renamed here from
//! `a_types_descriptor_has_exactly_one_address`), whose doc used to say the emission must be a
//! free `static` because a `const` "is const-promoted afresh at each `&`-site". Applying
//! exactly that substitution to the derive — `static __REFLECT_FIELDS` → `const` and
//! `static __REFLECT_TYPE_INFO` → `const` in `boyko_macros/src/reflect.rs` — leaves all
//! sixteen of that file's tests **green**, both `ptr::eq` clauses included. Measured, not
//! predicted.
//!
//! The property is real and violable; that gate's **subject set** could not see it, and
//! the stated reason is why it looked sufficient. The derive's expansion contains exactly
//! **one** `&__REFLECT_TYPE_INFO`, so a `const` is *not* re-promoted per `&`-site here:
//! within a crate its address is perfectly stable, and every same-crate `ptr::eq` is
//! blind. The divergence appears at the **crate boundary**, where an associated `const`'s
//! value is re-materialized by whichever crate evaluates it — the defining crate gets one
//! copy of the promoted allocation and every consumer gets its own.
//!
//! `reflect_dogfood` is the only package in the workspace that can host the subject: it
//! has a `src/lib.rs` (so an annotated type can be *defined* somewhere a consumer can
//! reach), a `boyko-macros` edge, and a `reflect` feature. `reflect_fixture` has no
//! library target at all, so every one of its annotated types is a private item of the
//! very binary that reads it.
//!
//! # Why this is urgent rather than tidy
//!
//! C7 is the first rung whose output can break a **C6** obligation. C6's Check B
//! identifies types **by address** (`ptr::from_ref` + `ptr::eq` over its `path` and `done`
//! arrays, `boyko_reflect/src/type_info.rs:626,641-651`). Two addresses for one type means
//! Check B's cycle detection and its memoization both silently stop detecting, while
//! `validate` goes on returning `Ok`.
//!
//! It goes live at **CORE C8**'s install seam and at **ECS EG8** — both read a descriptor
//! from a crate other than the one defining it — so a defect introduced by this rung would
//! first surface two rungs downstream of the change that caused it.
//!
//! # The invocation is part of the gate
//!
//! ```text
//! cargo test -p reflect-dogfood --features reflect-dogfood/reflect --test c7_cross_crate_address
//! ```
//!
//! The derive's emission is `#[cfg(feature = "reflect")]` evaluated in the **expanding**
//! crate (D2) and no package's `default` enables `reflect`, so a plain
//! `cargo test -p reflect-dogfood` compiles this file to nothing and exits 0 — a vacuous
//! pass on the green side *and* on every red side. The output must read `running [1-9]`.
#![cfg(feature = "reflect")]

use boyko_reflect::Reflect;
use boyko_reflect::type_info::{ValueKind, validate};
use reflect_dogfood::address::{
    ProbeLeaf, ProbeRoot, probe_leaf_type_info_in_defining_crate,
    probe_root_type_info_in_defining_crate,
};

/// **The gate.** One type, one descriptor address — read once in the crate that defines
/// it and once here, in a crate that merely consumes it.
///
/// This is the clause the `static` → `const` substitution in `boyko_macros/src/reflect.rs`
/// reds, and the only one in the campaign that can.
#[test]
fn one_type_has_one_descriptor_address_across_the_crate_boundary() {
    for (defining, consuming, what) in [
        (
            probe_leaf_type_info_in_defining_crate(),
            <ProbeLeaf as Reflect>::TYPE_INFO,
            "ProbeLeaf",
        ),
        (
            probe_root_type_info_in_defining_crate(),
            <ProbeRoot as Reflect>::TYPE_INFO,
            "ProbeRoot",
        ),
    ] {
        assert!(
            std::ptr::eq(defining, consuming),
            "`{what}` has TWO descriptor addresses: {:p} in the crate that DEFINES it, \
             {:p} in this consumer. The emission is no longer a free `static` -- an \
             associated `const` is re-materialized by whichever crate evaluates it, so \
             each consumer interns its own copy of the promoted allocation. C6's Check B \
             identifies types by address (`ptr::eq` over its `path`/`done` arrays), so \
             this makes its cycle detection and its memoization silently stop detecting \
             while `validate` keeps returning Ok -- and it goes live at C8's install seam \
             and at ECS EG8, both of which read a descriptor from a crate other than the \
             one defining it.",
            std::ptr::from_ref(defining),
            std::ptr::from_ref(consuming),
        );
    }
}

/// The divergence in the exact shape C6's Check B compares: a `Nested` **edge** reached
/// through the **defining** crate against the child's descriptor materialized **here**.
///
/// # The root must be read through the defining crate, and that was measured
///
/// The obvious form of this clause — `<ProbeRoot as Reflect>::TYPE_INFO.fields[0].nested`
/// against `<ProbeLeaf as Reflect>::TYPE_INFO`, both read here — **cannot fail**, and it
/// was written that way first and observed green under the `const` mutation. A `const`
/// descriptor is re-materialized *whole* by the crate that evaluates it: the consumer's
/// copy of `ProbeRoot`'s field table nests the consumer's copy of `ProbeLeaf`, so a graph
/// read entirely from one side is internally consistent no matter how many copies exist.
///
/// The boundary has to be crossed by the two operands, not merely be somewhere in the
/// history of one of them. So the edge comes from [`probe_root_type_info_in_defining_crate`]
/// — the upstream materialization — and the child from this crate's own read. That pairing
/// is what C8's install seam produces: `validate` is handed a descriptor the *defining*
/// crate emitted, then walks into types the consumer also names for itself.
#[test]
fn a_nested_edge_reached_upstream_points_at_the_downstream_descriptor() {
    let root_here = <ProbeRoot as Reflect>::TYPE_INFO;
    assert_eq!(root_here.fields.len(), 2, "ProbeRoot declares `leaf` and `tag`");
    assert_eq!(root_here.fields[0].name, "leaf");
    assert_eq!(
        root_here.fields[0].kind,
        ValueKind::Nested,
        "`leaf` is a bare path, so it nests"
    );

    let root_upstream = probe_root_type_info_in_defining_crate();
    let edge =
        root_upstream.fields[0].nested.expect("a Nested field carries its child's descriptor");
    assert!(
        std::ptr::eq(edge, <ProbeLeaf as Reflect>::TYPE_INFO),
        "`ProbeRoot.leaf`'s edge, reached through the crate that DEFINES it, is {:p}, but \
         this crate's `ProbeLeaf::TYPE_INFO` is {:p}. C6's Check B compares exactly these \
         two pointers with `ptr::eq` over its `path`/`done` arrays, so a walk that met \
         `ProbeLeaf` through the upstream edge would not recognize it when it met the \
         same type through a downstream reference -- which is the graph C8's install seam \
         and ECS EG8 hand it.",
        std::ptr::from_ref(edge),
        std::ptr::from_ref(<ProbeLeaf as Reflect>::TYPE_INFO),
    );
}

/// The subjects are coherent, so a red above is about ADDRESSES and not about a
/// descriptor this file introduced being malformed in some other way.
#[test]
fn the_cross_crate_probe_descriptors_are_coherent() {
    for info in [<ProbeRoot as Reflect>::TYPE_INFO, <ProbeLeaf as Reflect>::TYPE_INFO] {
        if let Err(problems) = validate(info) {
            let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
            panic!("`{}` is INCOHERENT:\n  {}", info.type_name, lines.join("\n  "));
        }
    }
}
