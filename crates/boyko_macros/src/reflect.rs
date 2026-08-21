//! `#[component(reflect)]` — the field walk and `offset_of!` baking (CORE C7).
//!
//! The `#[derive(Component)]` half of the editor-only reflection layer: for an annotated
//! type it emits a **free `static TypeInfo`** describing the type's fields plus the
//! `impl boyko_reflect::Reflect` that points at it, all behind
//! `#[cfg(feature = "reflect")]` evaluated in the crate the derive expanded into
//! (CORE D2). This crate emits tokens naming `boyko_reflect` and **does not depend on
//! it** (CORE D17).
//!
//! # No install call is emitted at this rung
//!
//! The static exists and is inert; CORE C8 wires it into `component_id()`. Splitting
//! them keeps *"the derive computes the right offsets"* separable from *"the funnel
//! appends correctly"*, which are two different failures.
//!
//! # The walk is index-faithful and total (CORE D14)
//!
//! `fields.len()` equals the type's **declared** field count, unconditionally. A field
//! the v1 kind table cannot classify — a `PhantomData<T>`, an opaque handle, an array of
//! arrays — bakes `ValueKind::Opaque` with **every** accessor slot `None`, never a
//! shorter list: a shorter list would make by-index access depend on which fields were
//! skipped. CORE C9 turns an un-skipped `Opaque` into the spanned refusal D15 requires;
//! C7 owes only that each field's index is its declaration position.
//!
//! # Classification is SYNTACTIC, and the soundness proof is not
//!
//! A proc macro sees tokens, not types. The kind table below matches shapes, and the
//! shapes it cannot decide fall to `Opaque` or to a trait bound that fails to compile —
//! never to a silent guess. What keeps a *wrong* descriptor from becoming a wild pointer
//! chase is `boyko_reflect::validate`'s two structural checks (CORE D21), which enumerate
//! no type name at all and therefore cannot drift against this list.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, Index, Member, Type};

/// Parses the optional type-level `#[reflect(...)]` attribute (CORE C7 / D20).
///
/// `reflect` is registered as a derive helper attribute on `#[derive(Component)]`
/// (`lib.rs`'s `attributes(...)` list). Without that registration
/// `#[reflect(no_default)]` is a hard *"cannot find attribute `reflect` in this scope"*
/// resolved **before** the derive runs — the derive cannot inspect what does not resolve.
///
/// The only key C7 accepts is the bare `no_default` (D20's documented way out of the
/// `Default` requirement, and the string `ReflectDefault`'s `on_unimplemented` label
/// names). D14's *field*-level `#[reflect(skip)]` lands at C9 and is parsed there; a
/// field-level `#[reflect(...)]` is inert here rather than an error, because the
/// registration is what makes it resolve and C9 owns what it means.
///
/// Returns `true` iff `no_default` was supplied. Consulted only when
/// `#[component(reflect)]` is present.
pub(crate) fn parse_reflect_no_default(attrs: &[syn::Attribute]) -> Result<bool, TokenStream> {
    let mut no_default = false;
    for attr in attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("no_default") {
                if no_default {
                    return Err(meta.error(
                        "duplicate #[reflect(...)] key; no_default may be set at most once",
                    ));
                }
                no_default = true;
                return Ok(());
            }
            Err(meta.error("unknown #[reflect(...)] key; valid keys: no_default"))
        });
        if let Err(e) = result {
            return Err(e.to_compile_error().into());
        }
    }
    Ok(no_default)
}

/// The `boyko_reflect::ScalarKind` variant a syntactic field type maps to, or `None`
/// when the type is not one of the eleven primitives C7 classifies.
///
/// **Syntactic by nature.** This matches a *bare, single-segment, argument-free* path —
/// `f32`, never `mymod::f32` and never `Wrapper<f32>`. A `type u32 = SomethingElse;`
/// alias in scope would be misclassified; that is the same exposure every other
/// syntactic decision in this derive already carries (`storage = "bitset"`'s ZST check,
/// the `Entity`-field scan), and asking the type system is not available to a macro.
///
/// **`ScalarKind::EntityId` is deliberately absent.** It is the one kind whose name is an
/// ordinary identifier a user crate may also define, so matching on the name would
/// install `prim::get_entity_id` on a same-named foreign type and read its first bytes as
/// a slot index — the silent-garbage class the model exists to refuse — and no C7 gate
/// covers it. An `EntityId` field therefore falls to the `Nested` arm and fails to
/// compile with a trait bound naming the type, which is the safe direction; C9's refusal
/// matrix is the rung that owns the diagnostic.
fn scalar_kind(ty: &Type) -> Option<&'static str> {
    let Type::Path(tp) = ty else {
        return None;
    };
    if tp.qself.is_some() || tp.path.segments.len() != 1 {
        return None;
    }
    let seg = &tp.path.segments[0];
    if !matches!(seg.arguments, syn::PathArguments::None) {
        return None;
    }
    Some(match seg.ident.to_string().as_str() {
        "bool" => "Bool",
        "u8" => "U8",
        "u16" => "U16",
        "u32" => "U32",
        "u64" => "U64",
        "i8" => "I8",
        "i16" => "I16",
        "i32" => "I32",
        "i64" => "I64",
        "f32" => "F32",
        "f64" => "F64",
        _ => return None,
    })
}

/// The `prim::get_*` / `prim::set_*` pair installed for a [`scalar_kind`] name.
///
/// The mapping is the inverse of [`scalar_kind`]'s table and is exhaustive over its
/// outputs; anything else is a bug in this file rather than a user error, hence the
/// `unreachable!` rather than a diagnostic.
fn prim_accessors(kind: &str) -> (Ident, Ident) {
    let suffix = match kind {
        "Bool" => "bool",
        "U8" => "u8",
        "U16" => "u16",
        "U32" => "u32",
        "U64" => "u64",
        "I8" => "i8",
        "I16" => "i16",
        "I32" => "i32",
        "I64" => "i64",
        "F32" => "f32",
        "F64" => "f64",
        other => unreachable!("`{other}` is not a kind `scalar_kind` produces"),
    };
    (format_ident!("get_{}", suffix), format_ident!("set_{}", suffix))
}

/// True when `ty` is a bare path with **no generic arguments anywhere** — the syntactic
/// proxy for *"a field whose type is itself reflectable"* (`ValueKind::Nested`).
///
/// A generic argument is what separates the standard indirections the taxonomy calls
/// `Opaque` (`Vec<T>`, `Box<T>`, `Option<T>`, `PhantomData<T>`) from a plain nested
/// struct, and a non-path type (reference, raw pointer, tuple, slice, array of
/// non-`Prim`) is never nested-by-value in the first place. The proxy cannot be exact —
/// no syntactic list can name a user-defined indirection such as
/// `struct MyBox<T>(*mut T)`, which is precisely why `validate`'s structural checks, not
/// this function, are the soundness proof (CORE D21).
///
/// A bare path that is *not* reflectable fails to compile at the
/// `<T as Reflect>::TYPE_INFO` it produces, spanned at the field's type. That is the safe
/// direction: a wrong `Nested` is a compile error, where a wrong `Opaque` would be a
/// silent omission from a wire format shared with the shipped `boyko_serialize` (D15).
fn is_nested_path(ty: &Type) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    if tp.qself.is_some() {
        return false;
    }
    tp.path.segments.iter().all(|s| matches!(s.arguments, syn::PathArguments::None))
}

/// One field's `FieldInfo` literal: name, `offset_of!` offset, kind, and exactly the
/// descriptor slots that kind makes live.
fn field_info(ty_name: &Ident, member: &Member, field_ty: &Type) -> TokenStream2 {
    let name_str = match member {
        Member::Named(id) => id.to_string(),
        Member::Unnamed(idx) => idx.index.to_string(),
    };
    // A tuple struct's field name is its decimal index (taxonomy §3), so by-name and
    // by-position coincide for it -- the reorder stability the design advertises does NOT
    // hold for tuple structs, which is why named-field structs are recommended for
    // anything serialized. Stated in `#[component(reflect)]`'s rustdoc, because a derive
    // diagnostic for it is not expressible: a tuple struct is ACCEPTED, so there is no
    // `compile_error!` to carry the text, and a non-fatal proc-macro warning needs the
    // nightly-only `proc_macro::Diagnostic`.
    let offset = quote! { ::core::mem::offset_of!(#ty_name, #member) };
    let type_id_fn = quote! { __reflect_type_id_of::<#field_ty> };

    if let Some(kind) = scalar_kind(field_ty) {
        let kind_ident = format_ident!("{}", kind);
        let (get, set) = prim_accessors(kind);
        return quote! {
            ::boyko_reflect::FieldInfo {
                name: #name_str,
                offset: #offset,
                type_id_fn: #type_id_fn,
                kind: ::boyko_reflect::ValueKind::Prim(
                    ::boyko_reflect::ScalarKind::#kind_ident
                ),
                get: ::core::option::Option::Some(::boyko_reflect::prim::#get),
                set: ::core::option::Option::Some(::boyko_reflect::prim::#set),
                nested: ::core::option::Option::None,
                enum_info: ::core::option::Option::None,
                array: ::core::option::Option::None,
            }
        };
    }

    // `[T; N]` where `T` is a `Prim`, and only that (CORE D19: arrays of arrays are v2,
    // so `[[f32; 4]; 4]` falls through to `Opaque` rather than being silently flattened).
    if let Type::Array(arr) = field_ty
        && let Some(kind) = scalar_kind(&arr.elem)
    {
        let kind_ident = format_ident!("{}", kind);
        let elem = &arr.elem;
        let len = &arr.len;
        return quote! {
            ::boyko_reflect::FieldInfo {
                name: #name_str,
                offset: #offset,
                type_id_fn: #type_id_fn,
                kind: ::boyko_reflect::ValueKind::Array,
                get: ::core::option::Option::None,
                set: ::core::option::Option::None,
                nested: ::core::option::Option::None,
                enum_info: ::core::option::Option::None,
                array: ::core::option::Option::Some(::boyko_reflect::ArrayInfo {
                    elem: ::boyko_reflect::ScalarKind::#kind_ident,
                    // The element's OWN `size_of`, not a spacing guessed from the field:
                    // a stride from the wrong size reads every element but the first from
                    // the wrong address.
                    stride: ::core::mem::size_of::<#elem>(),
                    len: #len,
                }),
            }
        };
    }

    if is_nested_path(field_ty) {
        return quote! {
            ::boyko_reflect::FieldInfo {
                name: #name_str,
                offset: #offset,
                type_id_fn: #type_id_fn,
                kind: ::boyko_reflect::ValueKind::Nested,
                get: ::core::option::Option::None,
                set: ::core::option::Option::None,
                // Derive-time recursion is depth 1 (§3.1): a POINTER to the inner type's
                // own static, never a flattened path table -- so there is no proc-macro
                // recursion and no ordering requirement between the two expansions.
                nested: ::core::option::Option::Some(
                    <#field_ty as ::boyko_reflect::Reflect>::TYPE_INFO
                ),
                enum_info: ::core::option::Option::None,
                array: ::core::option::Option::None,
            }
        };
    }

    quote! {
        ::boyko_reflect::FieldInfo {
            name: #name_str,
            offset: #offset,
            type_id_fn: #type_id_fn,
            kind: ::boyko_reflect::ValueKind::Opaque,
            get: ::core::option::Option::None,
            set: ::core::option::Option::None,
            nested: ::core::option::Option::None,
            enum_info: ::core::option::Option::None,
            array: ::core::option::Option::None,
        }
    }
}

/// The `#[component(reflect)]` emission: `(items, witness)`.
///
/// `items` is the `#[cfg(feature = "reflect")] const _: () = { … };` block carrying the
/// descriptor and the `impl Reflect`; `witness` is D20's `ReflectDefault` bound
/// assertion, empty when `#[reflect(no_default)]` was supplied. They are returned
/// separately only because the caller splices them at two points in its `quote!`.
///
/// # Why a free `static` inside an anonymous `const` block (CORE D22)
///
/// `impl T { static X: … }` is *"error: associated `static` items are not allowed"* on
/// the pinned toolchain, so the originally-specified `&T::__REFLECT_TYPE_INFO` was not
/// expressible. The other escape — an associated `const` — compiles and is **wrong**,
/// but *not* for the reason this campaign wrote down at five sites: it said a `const` is
/// "const-promoted afresh at each `&`-site". **Measured 2026-08-21, and false on this
/// emission.** The expansion below contains exactly **one** `&__REFLECT_TYPE_INFO`, so
/// within a crate a `const` descriptor's address is perfectly stable; substituting
/// `const` for both `static`s here leaves all sixteen of `reflect_fixture`'s
/// `c7_derive_bake` tests green, both `ptr::eq` clauses included.
///
/// The divergence is at the **crate boundary**. `&` on a `const` in a const-initializer
/// produces an anonymous allocation created by const-evaluation, and each crate that
/// *evaluates* the associated const interns its own copy of it — so the defining crate
/// gets one descriptor and every consumer gets another (measured: `0x7ff739cff5c8`
/// upstream, `0x7ff739cf26a0` downstream, one type). A graph read entirely from one side
/// stays internally consistent, which is why no single-crate check can see this.
///
/// `boyko_reflect::validate`'s acyclicity walk identifies types **by address**, so two
/// addresses for one type degrade both its cycle test and its memoization while it goes
/// on returning `Ok`. One stable address per type is a CORE C6 obligation, not a style
/// choice, and it goes live at C8's install seam and at ECS EG8 — both of which read a
/// descriptor from a crate other than the one defining it. The gate is
/// `reflect_dogfood/tests/c7_cross_crate_address.rs`, the only place in the workspace
/// where an annotated type is *defined in a library* and read from a consumer.
///
/// # Why `type_name` is `module_path!()`-derived
///
/// `std::any::type_name` is **not const** on the pinned toolchain (measured:
/// *"`std::any::type_name` is not yet stable as a const fn"*), and `TypeInfo.type_name`
/// is a baked `&'static str` field. `concat!(module_path!(), "::", stringify!(T))` is the
/// shape every hand-baked static in this campaign already writes, and the slot is
/// diagnostics only — never a save key (CORE D8).
pub(crate) fn codegen(
    input: &DeriveInput,
    name: &Ident,
    no_default: bool,
) -> (TokenStream2, TokenStream2) {
    // A non-struct shape is `TypeKind::Opaque` with an empty field list -- *"a type the
    // model cannot describe"*, which is exactly what that arm means. Enums are CORE
    // C10's (`TypeKind::Enum` plus an `EnumInfo`), and describing one as a fieldless
    // STRUCT would be a lie the model already has a word for.
    let (type_kind, fields): (Ident, Vec<TokenStream2>) = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => (
                Ident::new("Struct", Span::call_site()),
                named
                    .named
                    .iter()
                    .map(|f| {
                        let member =
                            Member::Named(f.ident.clone().expect("invariant: named field"));
                        field_info(name, &member, &f.ty)
                    })
                    .collect(),
            ),
            Fields::Unnamed(unnamed) => (
                Ident::new("TupleStruct", Span::call_site()),
                unnamed
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(i, f)| field_info(name, &Member::Unnamed(Index::from(i)), &f.ty))
                    .collect(),
            ),
            Fields::Unit => (Ident::new("Struct", Span::call_site()), Vec::new()),
        },
        Data::Enum(_) | Data::Union(_) => (Ident::new("Opaque", Span::call_site()), Vec::new()),
    };
    let field_count = fields.len();

    // CORE D20: `Some(__reflect_default_in_place)` plus a named, spanned bound
    // assertion, or `None` plus no witness at all.
    //
    // **The helper is GENERIC over `ReflectDefault`, and that was measured rather than
    // preferred.** Written monomorphically -- `ptr::write(p as *mut #name, <#name as
    // Default>::default())` -- a type with no `Default` produced TWO errors, and the
    // FIRST one was the bare *"the trait bound `T: Default` is not satisfied"* from this
    // helper's own body, with D20's named message second. D20's promise is that such a
    // type *"fails with `ReflectDefault`'s message"*; leading with the anonymous one
    // keeps the promise only in the sense that the good message is somewhere in the
    // output. Taking the bound through `ReflectDefault` (whose supertrait supplies
    // `Default` inside the body) leaves exactly one obligation per site, and BOTH errors
    // then carry the named message. Measured on the pinned toolchain, all three counts
    // (2 = raw + named; 2 = named + named) taken from an actual compile.
    let (default_items, default_slot, witness) = if no_default {
        (TokenStream2::new(), quote! { ::core::option::Option::None }, TokenStream2::new())
    } else {
        (
            quote! {
                /// Writes `T::default()` into uninitialized bytes.
                ///
                /// # Safety
                ///
                /// `p` must be writable for `size_of::<T>()`, aligned to
                /// `align_of::<T>()`, and must NOT already hold an initialized value
                /// whose drop glue is owed -- `TypeInfo::default_in_place`'s contract.
                unsafe fn __reflect_default_in_place<
                    T: ::boyko_reflect::ReflectDefault,
                >(p: *mut u8) {
                    // SAFETY: the caller guarantees `p` is a writable, correctly aligned
                    // destination of exactly this type holding no initialized value, so
                    // the raw write initializes it without dropping anything. No
                    // intermediate reference to the destination is ever formed.
                    unsafe {
                        ::core::ptr::write(
                            p as *mut T,
                            <T as ::core::default::Default>::default(),
                        )
                    }
                }
            },
            quote! {
                ::core::option::Option::Some(
                    __reflect_default_in_place::<#name> as unsafe fn(*mut u8)
                )
            },
            // The turbofish argument carries the user's own type-name span, so a missing
            // `Default` is reported at their item with `ReflectDefault`'s message rather
            // than as an `E0277` inside an expansion they never wrote -- which is the
            // whole reason D20 exists.
            quote! {
                #[cfg(feature = "reflect")]
                const _: fn() = || {
                    fn __assert_reflect_default<T: ::boyko_reflect::ReflectDefault>() {}
                    __assert_reflect_default::<#name>();
                };
            },
        )
    };

    let items = quote! {
        #[cfg(feature = "reflect")]
        const _: () = {
            /// `TypeId::of` is not `const`, so every `type_id_fn` slot is a monomorphized
            /// fn item coerced to `fn() -> TypeId`.
            fn __reflect_type_id_of<T: ?Sized + 'static>() -> ::core::any::TypeId {
                ::core::any::TypeId::of::<T>()
            }

            #default_items

            /// Runs this type's drop glue in place.
            ///
            /// # Safety
            ///
            /// `p` must hold a live, initialized value of this type that the caller owns
            /// and will not read again.
            unsafe fn __reflect_drop_in_place(p: *mut u8) {
                // SAFETY: the caller guarantees `p` holds a live, initialized, owned
                // value of exactly this type which is never read again.
                unsafe { ::core::ptr::drop_in_place(p as *mut #name) }
            }

            static __REFLECT_FIELDS: [::boyko_reflect::FieldInfo; #field_count] = [
                #(#fields),*
            ];

            static __REFLECT_TYPE_INFO: ::boyko_reflect::TypeInfo =
                ::boyko_reflect::TypeInfo {
                    type_name: ::core::concat!(
                        ::core::module_path!(), "::", ::core::stringify!(#name)
                    ),
                    type_id_fn: __reflect_type_id_of::<#name>,
                    size: ::core::mem::size_of::<#name>(),
                    align: ::core::mem::align_of::<#name>(),
                    fields: &__REFLECT_FIELDS,
                    kind: ::boyko_reflect::TypeKind::#type_kind,
                    enum_info: ::core::option::Option::None,
                    default_in_place: #default_slot,
                    // `needs_drop` is a `const fn`, so the slot is decided by the type
                    // system at const-eval rather than by a syntactic guess about which
                    // shapes own glue -- which is exactly the "all POD" assumption C7's
                    // second RED exists to make observable.
                    drop_in_place: if ::core::mem::needs_drop::<#name>() {
                        ::core::option::Option::Some(
                            __reflect_drop_in_place as unsafe fn(*mut u8)
                        )
                    } else {
                        ::core::option::Option::None
                    },
                };

            impl ::boyko_reflect::Reflect for #name {
                const TYPE_INFO: &'static ::boyko_reflect::TypeInfo = &__REFLECT_TYPE_INFO;
            }
        };
    };

    (items, witness)
}
