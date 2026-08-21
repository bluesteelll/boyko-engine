//! The two traits `#[component(reflect)]`'s expansion names (CORE C7, decision D22).
//!
//! Both were specified before they existed. [`Reflect`] appeared in the plan set only at
//! C7's first emission bullet and C8's install (`<Self as boyko_reflect::Reflect>::
//! TYPE_INFO`) — two *uses*, no `Lands`; [`ReflectDefault`] existed only inside D20's
//! fenced prose sketch, introduced by *"where `boyko_reflect` declares"*, while C9's gate
//! 5 already blessed a `.stderr` against its message and C9's third RED already deleted
//! that attribute *from* it. D22 assigns both to C7, in this crate, in the same commit as
//! the derive that emits them.

use crate::type_info::TypeInfo;

/// A type whose reflection descriptor is baked at compile time.
///
/// The one item `#[component(reflect)]`'s expansion implements, and the one item C8's
/// install slot reads (`install_type_info(raw, <Self as Reflect>::TYPE_INFO)`).
///
/// # The associated const is a `&'static`, and the referent is a free `static`
///
/// The derive emits a **free `static`** inside a generated `const _: () = { … };` block
/// and points this const at it by reference. The two alternatives are both wrong, and
/// not interchangeably so:
///
/// * `static` is **not permitted as an associated item** — `impl T { static X: u32 = 7; }`
///   is *"error: associated `static` items are not allowed"* on the pinned toolchain — so
///   a `T::__REFLECT_TYPE_INFO` naming one is not expressible;
/// * an associated **`const`** would compile, and would give the type **one descriptor
///   per crate that reads it**. [`validate`](crate::validate)'s acyclicity walk
///   identifies types **by address** (`ptr::eq` over its `path` and `done` arrays), so
///   two addresses for one type degrade both its cycle test and its memoization into a
///   walk that recognizes nothing — while it goes on returning `Ok`. One stable address
///   per type is a C6 obligation, not a style choice.
///
/// **The second bullet's mechanism, corrected (measured 2026-08-21).** This doc — and
/// four others — used to say a `const` is "const-promoted afresh at each `&`-site". The
/// derive's expansion contains exactly *one* `&__REFLECT_TYPE_INFO`, so that is not what
/// happens: within a crate a `const` descriptor's address is stable, and every same-crate
/// `ptr::eq` check in the campaign is blind to the substitution (`reflect_fixture`'s
/// sixteen `c7_derive_bake` tests all stay green under it). What actually diverges is the
/// **crate boundary**: `&` on a `const` in a const-initializer yields an anonymous
/// const-evaluated allocation, and each crate that evaluates the associated const interns
/// its own copy. The property is therefore only observable from a *consumer* of the
/// defining crate, which is what `reflect_dogfood/tests/c7_cross_crate_address.rs` is.
///
/// # Implementing by hand
///
/// Nothing stops a hand-written impl, and C3/C6's fixtures are exactly that shape minus
/// the trait. What such an impl owes is what the derive owes: a descriptor that
/// [`validate`](crate::validate) accepts, whose `size`/`align`/`offset`s are the
/// compiler's own, and whose referent is a `static` rather than a promoted `const`.
pub trait Reflect {
    /// This type's descriptor. Address-stable for the process's lifetime.
    const TYPE_INFO: &'static TypeInfo;
}

/// `Default`, wearing a readable refusal (CORE D20).
///
/// `#[component(reflect)]` bakes [`TypeInfo::default_in_place`] from `Default`, and a
/// proc macro **cannot detect a missing trait impl** — so the requirement cannot be a
/// spanned `compile_error!` and would otherwise surface as an `E0277` pointing into an
/// expansion the user never wrote. The derive emits a witness
///
/// ```ignore
/// const _: fn() = || {
///     fn __assert_reflect_default<T: ::boyko_reflect::ReflectDefault>() {}
///     __assert_reflect_default::<MyType>();
/// };
/// ```
///
/// spanned at the type name, so the refusal reads as the message below and points at the
/// user's own item. `#[reflect(no_default)]` bakes `default_in_place: None` instead and
/// emits no witness — `None` is a real state with a real consumer (`add_default`
/// answering `Err(Refusal::NoDefault)`), not a hole.
///
/// `#[diagnostic::on_unimplemented]` is this tree's existing answer for the class:
/// `boyko_ecs`'s `query/chunked_data.rs` and `query/filter.rs` both carry one, the second
/// with a blessed `compile_fail` fixture pinning its message.
///
/// [`TypeInfo::default_in_place`]: crate::TypeInfo::default_in_place
#[diagnostic::on_unimplemented(
    message = "`#[component(reflect)]` bakes `default_in_place` from `Default`, and `{Self}` does not implement it",
    label = "add `#[derive(Default)]`, write an impl, or opt out with `#[reflect(no_default)]`"
)]
pub trait ReflectDefault: Default {}

impl<T: Default> ReflectDefault for T {}
