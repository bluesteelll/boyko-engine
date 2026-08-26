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
use proc_macro2::{Literal, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Ident, Index, Member, Type};

// ─────────────────────────── CORE C9: the refusal table ─────────────────────

/// **The refusal table (CORE C9 / D36): `(rule name, diagnostic message)`, read AT the
/// refusal sites.**
///
/// Each row's second element **is** the bytes a refusal prints: every site below
/// interpolates `REFUSALS[IDX_…].1` into its `quote_spanned!`, so a rule with no row has
/// literally nothing to say and does not compile. That is what makes the table a live
/// datum rather than a declaration — the shape D36 chose after the originally-specified
/// `&[&str]` was measured unconstructible on two counts: `boyko_macros` is
/// `[lib] proc-macro = true`, so **no test can import this const** (only a source-text
/// scan can read it — D31 measured the same obstacle about this same crate), and nothing
/// in a derive *iterates* a list of rule names to decide anything, so a `&[&str]` the
/// derive merely declared would have been computed and never read.
///
/// # The census over it, and the two directions it closes
///
/// `tests/reflect_refusal_census.rs` scans this source text and asserts a **bijection**
/// between these rows and the `.rs` fixtures in
/// `crates/reflect_fixture/tests/reflect_compile_fail/`, keyed by the rule name, plus
/// that every blessed `.stderr` carries its own row's message bytes. A rule added here
/// without its fixture reds the census (D20 item 2's first direction); a refusal site
/// added to this file without a row here does not **compile**, because there is no
/// `IDX_…` to index and no message to emit (D36's own direction, which the struck
/// equality-of-counts form could not see at all).
///
/// # One row is `message-only`, and that is not a loophole
///
/// [`IDX_MISSING_DEFAULT`]'s refusal is a **trait bound**, not a `compile_error!`: a proc
/// macro cannot see trait impls, so D20's requirement is carried by
/// `boyko_reflect`'s `ReflectDefault` and its `#[diagnostic::on_unimplemented]`. Its row
/// exists precisely so a census keyed on `REFUSALS` is not blind to it — the defect D20
/// was written to close — and the census asserts its bytes are **byte-identical** to that
/// attribute's `message = "…"`. The duplication is unavoidable: `boyko_macros` must never
/// gain an edge to `boyko_reflect` (CORE D17), so the two strings cannot share a const,
/// and a census clause is the only thing that can keep them equal.
pub(crate) const REFUSALS: &[(&str, &str)] = &[
    (
        "bitset_storage_rejected",
        "`#[component(reflect)]` cannot be combined with `storage = \"bitset\"`: a bitset \
         enable tag has no ComponentPool and no per-row bytes -- the bit IS the datum, so \
         \"read the field at offset N\" describes nothing. Drop `reflect` to keep the tag, \
         or drop `storage = \"bitset\"` to keep the descriptor.",
    ),
    (
        "vec_field_rejected",
        "this field's type is not one the v1 reflection model can describe, so its bytes \
         would be silently absent from a descriptor whose wire is shared with the shipped \
         `boyko_serialize` -- and a silent omission there is unacceptable. Give the field \
         a describable type (a primitive, a `[Prim; N]`, or a nested reflected struct), or \
         opt it out explicitly with `#[reflect(skip)]`.",
    ),
    (
        "fieldless_enum_without_repr_rejected",
        "a reflected fieldless enum needs a guaranteed discriminant width, and this one has \
         no `#[repr(Int)]`: add `#[repr(u8)]` (or another integer repr), or drop `reflect` \
         from this item.",
    ),
    (
        "data_carrying_enum_rejected",
        "a data-carrying enum has no v1 reflection descriptor: the walk would bake \
         `fields: &[]`, asserting that a type with payload variants has no fields. Drop \
         `reflect` from this item, or make it a fieldless `#[repr(Int)]` enum.",
    ),
    (
        "union_rejected",
        "a union has no v1 reflection descriptor: the walk would bake `fields: &[]`, \
         asserting that a type whose members overlap has no fields. Drop `reflect` from \
         this item, or reflect a struct that wraps it behind a describable field.",
    ),
    (
        "missing_default_rejected",
        "`#[component(reflect)]` bakes `default_in_place` from `Default`, and `{Self}` does not implement it",
    ),
];

/// `storage = "bitset"` together with `reflect` — spanned at the **`reflect` key** (D37).
pub(crate) const IDX_BITSET_STORAGE: usize = 0;

/// A field the v1 kind table classifies `Opaque`, without `#[reflect(skip)]` (D15/D34).
pub(crate) const IDX_OPAQUE_FIELD: usize = 1;

/// A fieldless enum with no `#[repr(Int)]` — no guaranteed discriminant width (FIX Mi3).
pub(crate) const IDX_FIELDLESS_ENUM_WITHOUT_REPR: usize = 2;

/// A data-carrying enum as the component ITSELF (D38).
pub(crate) const IDX_DATA_CARRYING_ENUM: usize = 3;

/// A union as the component ITSELF (D38).
pub(crate) const IDX_UNION: usize = 4;

/// D20's `Default` requirement — the one **message-only** row (see [`REFUSALS`]).
pub(crate) const IDX_MISSING_DEFAULT: usize = 5;

/// `&str` equality in a `const` context — `PartialEq for str` is not `const fn`.
const fn same_str(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// **Each `IDX_…` names the row it indexes, pinned at const-eval.**
///
/// Without this, the index constants and the table would be two lists ordered by
/// convention: inserting a row anywhere but the end would silently re-point every index
/// after it, so a `Vec` field would refuse with a union's message and every gate below
/// would stay green — the fixtures would still fail to compile, and the `.stderr` clause
/// is the only one that could notice, one bless too late.
///
/// It is also what keeps [`IDX_MISSING_DEFAULT`] from being a dead datum. That row is
/// **message-only** — its refusal is a trait bound, so no `quote_spanned!` site indexes it
/// — and a `const` nothing reads is precisely the shape D36 struck the original `REFUSALS`
/// for. Here it is read.
const _: () = {
    assert!(REFUSALS.len() == 6, "a REFUSALS row was added or removed without its IDX_ pin");
    assert!(same_str(REFUSALS[IDX_BITSET_STORAGE].0, "bitset_storage_rejected"));
    assert!(same_str(REFUSALS[IDX_OPAQUE_FIELD].0, "vec_field_rejected"));
    assert!(same_str(
        REFUSALS[IDX_FIELDLESS_ENUM_WITHOUT_REPR].0,
        "fieldless_enum_without_repr_rejected"
    ));
    assert!(same_str(REFUSALS[IDX_DATA_CARRYING_ENUM].0, "data_carrying_enum_rejected"));
    assert!(same_str(REFUSALS[IDX_UNION].0, "union_rejected"));
    assert!(same_str(REFUSALS[IDX_MISSING_DEFAULT].0, "missing_default_rejected"));
};

/// A refusal's message as a string literal carrying `span`.
///
/// The span goes on the **literal** as well as on the `quote_spanned!` block, because the
/// two carets are produced by different mechanisms and only one of them is `quote`'s to
/// give: `quote`'s `ToTokens for str` mints its `Literal` at `Span::call_site()` — which
/// is the derive attribute, i.e. exactly the caret C9 forbids — while the invocation span
/// is what `compile_error!` itself reports at. Setting both makes the caret independent of
/// which one rustc reads.
///
/// Deliberately takes the message as an argument rather than an index: **every refusal
/// site spells `REFUSALS[IDX_…].1` itself** (D36), so that the row's bytes are visibly the
/// diagnostic's bytes at the site, and a site whose row does not exist fails to compile
/// here rather than reding a fixture count somewhere else.
fn spanned_message(message: &str, span: Span) -> Literal {
    let mut lit = Literal::string(message);
    lit.set_span(span);
    lit
}

/// Pushes [`IDX_OPAQUE_FIELD`]'s refusal, **spanned at the field's TYPE** (D34).
///
/// The type, not the whole `syn::Field`. MEASURED while blessing this corpus: a
/// `Field::span()` starts at the field's first **attribute**, and a documented field's
/// first attribute is its `///` line — so the caret landed under the doc comment, pointing
/// at prose instead of at the offending token. The type is what the kind table declined,
/// it identifies the field unambiguously in the source, and it is the one span that reads
/// the same for a named field and for a tuple field (which has no ident to point at).
fn push_opaque_field_refusal(refusals: &mut Vec<TokenStream2>, field: &syn::Field) {
    let span = field.ty.span();
    let msg = spanned_message(REFUSALS[IDX_OPAQUE_FIELD].1, span);
    refusals.push(quote_spanned! { span =>
        #[cfg(feature = "reflect")]
        ::core::compile_error!(#msg);
    });
}

/// [`parse_reflect_skip`] with its error routed into `refusals` instead of aborting.
///
/// A malformed `#[reflect(...)]` on one field must not stop the walk: the remaining fields
/// may carry refusals of their own, and reporting one attribute typo at a time is the
/// behaviour C7's own `parse_reflect_no_default` was written to avoid at the type level.
/// The error joins the refusal list, so it is emitted through the same
/// `#[cfg(feature = "reflect")]`-gated path — a key that only the reflect emission reads
/// cannot be a hard error in a build where that emission does not exist.
fn parse_skip_or_record(attrs: &[syn::Attribute], refusals: &mut Vec<TokenStream2>) -> bool {
    match parse_reflect_skip(attrs) {
        Ok(v) => v,
        Err(e) => {
            let err = e.to_compile_error();
            refusals.push(quote! {
                #[cfg(feature = "reflect")]
                const _: () = { #err };
            });
            false
        }
    }
}

/// Parses the optional type-level `#[reflect(...)]` attribute (CORE C7 / D20).
///
/// `reflect` is registered as a derive helper attribute on `#[derive(Component)]`
/// (`lib.rs`'s `attributes(...)` list). Without that registration
/// `#[reflect(no_default)]` is a hard *"cannot find attribute `reflect` in this scope"*
/// resolved **before** the derive runs — the derive cannot inspect what does not resolve.
///
/// The only key accepted at the **type** level is the bare `no_default` (D20's documented
/// way out of the `Default` requirement, and the string `ReflectDefault`'s
/// `on_unimplemented` label names). D14's *field*-level `#[reflect(skip)]` is parsed by
/// [`parse_reflect_skip`] (CORE C9 / D35); the two vocabularies are disjoint on purpose,
/// so `#[reflect(skip)]` on a type and `#[reflect(no_default)]` on a field are both
/// "unknown key" errors naming the keys that position does accept.
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

/// Parses a **field's** `#[reflect(...)]` attribute (CORE C9 / D35).
///
/// The one key is the bare `skip`, D14's opt-out and the documented way out of
/// [`IDX_OPAQUE_FIELD`]'s refusal. A refusal defined in terms of an escape hatch that does
/// not exist is a refusal with no way out, which is why the hatch lands in the same rung
/// as the refusal rather than in the five places that scheduled it and none that built it.
///
/// **Skipping does not shorten the walk.** D14 forbids omission outright: a skipped field
/// keeps its index, its name and its `offset_of!`, and bakes `ValueKind::Opaque` with all
/// four accessor slots `None` — the same descriptor an unclassifiable field would have
/// baked, minus the refusal. By-index access must not depend on which fields were skipped,
/// so `skip` means *"I accept that these bytes are not describable"*, never *"pretend this
/// field is not there"*.
///
/// Returns `true` iff `skip` was supplied on this field.
fn parse_reflect_skip(attrs: &[syn::Attribute]) -> Result<bool, syn::Error> {
    let mut skip = false;
    for attr in attrs {
        if !attr.path().is_ident("reflect") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                if skip {
                    return Err(
                        meta.error("duplicate #[reflect(...)] key; skip may be set at most once")
                    );
                }
                skip = true;
                return Ok(());
            }
            Err(meta.error("unknown #[reflect(...)] key on a field; valid keys: skip"))
        })?;
    }
    Ok(skip)
}

/// True when the item carries an **integer** `#[repr(...)]` — the guaranteed discriminant
/// width [`IDX_FIELDLESS_ENUM_WITHOUT_REPR`] demands (analysis FIX Mi3).
///
/// `#[repr(C)]` is deliberately NOT accepted. A `repr(C)` enum's discriminant is the
/// platform's `int`, which is a target-dependent width rather than a guaranteed one, and
/// the whole reason this refusal exists is that C10 will bake the discriminant's **byte**
/// into a descriptor a serializer reads.
fn has_integer_repr(attrs: &[syn::Attribute]) -> bool {
    let mut found = false;
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        // A malformed or unrecognized `repr` is not this function's error to report --
        // rustc rejects it on its own, and swallowing the parse keeps the derive from
        // adding a second, worse diagnostic to a broken attribute.
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(id) = meta.path.get_ident()
                && matches!(
                    id.to_string().as_str(),
                    "u8" | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "usize"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "isize"
                )
            {
                found = true;
            }
            // Consume any `= …` / `(…)` payload so an unrelated key does not abort the walk.
            if meta.input.peek(syn::Token![=]) || meta.input.peek(syn::token::Paren) {
                let _ = meta.value().and_then(|v| v.parse::<proc_macro2::TokenTree>());
            }
            Ok(())
        });
    }
    found
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
/// descriptor slots that kind makes live — plus whether the field **fell through to
/// `Opaque` un-skipped**, which is [`IDX_OPAQUE_FIELD`]'s subject (CORE C9).
///
/// The flag is returned rather than refused here because the caller owns the span it
/// would be refused at (the field, not the type) and owns the decision to suppress the
/// whole emission once any refusal fires.
fn field_info(
    ty_name: &Ident,
    member: &Member,
    field_ty: &Type,
    skip: bool,
) -> (TokenStream2, bool) {
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

    // D14/D35: `#[reflect(skip)]` short-circuits CLASSIFICATION, not the walk. The field
    // keeps its index, its name and its offset and bakes `Opaque` with no accessors --
    // whatever its type would otherwise have classified as. Deciding it here rather than
    // after the kind table is what makes the opt-out mean *"these bytes are not
    // describable"* uniformly, instead of meaning different things for a `Vec` and a `u32`.
    if skip {
        return (
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
            },
            false,
        );
    }

    if let Some(kind) = scalar_kind(field_ty) {
        let kind_ident = format_ident!("{}", kind);
        let (get, set) = prim_accessors(kind);
        return (quote! {
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
        }, false);
    }

    // `[T; N]` where `T` is a `Prim`, and only that (CORE D19: arrays of arrays are v2,
    // so `[[f32; 4]; 4]` falls through to `Opaque` rather than being silently flattened).
    if let Type::Array(arr) = field_ty
        && let Some(kind) = scalar_kind(&arr.elem)
    {
        let kind_ident = format_ident!("{}", kind);
        let elem = &arr.elem;
        let len = &arr.len;
        return (quote! {
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
        }, false);
    }

    if is_nested_path(field_ty) {
        return (quote! {
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
        }, false);
    }

    // The fallthrough, and since CORE C9 it is a REFUSAL rather than a silent descriptor.
    // Every standard indirection lands here -- `Vec`, `Box`, `Option<T>`, `PhantomData<T>`,
    // `&T`, a raw pointer, a data-carrying enum in a field -- because none of them is a
    // `Prim`, a `[Prim; N]` or an argument-free path.
    //
    // **NO TOKENS.** Until this repair the arm built the same `Opaque` `FieldInfo` the
    // `skip` return above builds, under a comment claiming that kept "the un-refused shape
    // of the descriptor visible at exactly one site". The claim was inverted: the shape was
    // at TWO sites and this was the dead one. Every input reaching here also pushes
    // [`IDX_OPAQUE_FIELD`]'s refusal, and `codegen` returns the refusals and discards the
    // whole `fields` vector unconditionally, so those tokens were computed and never read.
    // MEASURED: corrupting them (wrong name, offset and kind) left every gate green. Before
    // C9 the arm WAS live -- it baked `Padded._pd` -- and C9's `#[reflect(skip)]` migration
    // of that field is what killed it.
    //
    // The one live copy is the `skip` return, and `reflect_pass/vec_field_skip_accepted.rs`
    // asserts its name, index, offset, kind and all four accessor slots. If a later rung
    // makes this arm reachable the missing element does not survive as a hole: `quote!`'s
    // `#(#fields),*` drops an empty stream, so the array is one element SHORT of
    // `#field_count` and the `static __REFLECT_FIELDS: [FieldInfo; #field_count]`
    // annotation refuses it -- MEASURED: `error[E0308]: expected an array with a size of
    // 2, found one with a size of 1`. D14's index-faithfulness invariant catches the change
    // in the type system, which is the loud direction.
    (TokenStream2::new(), true)
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
    bitset_reflect_key: Option<Span>,
) -> (TokenStream2, TokenStream2, bool) {
    // -- CORE C9: the refusals, collected before anything is emitted ------------
    //
    // They are collected rather than returned at the first one because a type can be
    // wrong in more than one way at once, and reporting only the first would make the
    // second a surprise on the next build. They are emitted INSIDE the same
    // `#[cfg(feature = "reflect")]` block as the emission they guard (D33): a refusal that
    // fired with the feature off would refuse a program that compiles to nothing, which
    // contradicts D1's zero-cost-when-off premise and makes the corpus's feature-off leg
    // unsatisfiable by construction.
    let mut refusals: Vec<TokenStream2> = Vec::new();

    // D37 -- spanned at the `reflect` KEY, not at the `storage` key and not at the user's
    // type name. Three census-gated documents once specified three different carets for
    // this one refusal; the tie-break is that `storage = "bitset"` is legitimate on its
    // own and `reflect` is the token that is wrong.
    if let Some(span) = bitset_reflect_key {
        let msg = spanned_message(REFUSALS[IDX_BITSET_STORAGE].1, span);
        refusals.push(quote_spanned! { span =>
            #[cfg(feature = "reflect")]
            ::core::compile_error!(#msg);
        });
    }

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
                        let skip = parse_skip_or_record(&f.attrs, &mut refusals);
                        let (tokens, opaque) = field_info(name, &member, &f.ty, skip);
                        if opaque {
                            push_opaque_field_refusal(&mut refusals, f);
                        }
                        tokens
                    })
                    .collect(),
            ),
            Fields::Unnamed(unnamed) => (
                Ident::new("TupleStruct", Span::call_site()),
                unnamed
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let skip = parse_skip_or_record(&f.attrs, &mut refusals);
                        let (tokens, opaque) =
                            field_info(name, &Member::Unnamed(Index::from(i)), &f.ty, skip);
                        if opaque {
                            push_opaque_field_refusal(&mut refusals, f);
                        }
                        tokens
                    })
                    .collect(),
            ),
            Fields::Unit => (Ident::new("Struct", Span::call_site()), Vec::new()),
        },
        // D38 -- the two ITEM-level shapes the derive used to ACCEPT, baking
        // `TypeKind::Opaque` with `fields: &[]` for both. For a fieldless `#[repr(Int)]`
        // enum that empty list is TRUE and the item stays accepted (its `kind: Opaque` is
        // C10's to replace); for a data-carrying enum and for a union it is a coherent lie
        // about a type that has members, and `validate` cannot see it because "has no
        // fields" is structurally well-formed.
        Data::Enum(e) => {
            let span = e.enum_token.span;
            if e.variants.iter().any(|v| !matches!(v.fields, Fields::Unit)) {
                let msg = spanned_message(REFUSALS[IDX_DATA_CARRYING_ENUM].1, span);
                refusals.push(quote_spanned! { span =>
                    #[cfg(feature = "reflect")]
                    ::core::compile_error!(#msg);
                });
            } else if !has_integer_repr(&input.attrs) {
                let msg = spanned_message(REFUSALS[IDX_FIELDLESS_ENUM_WITHOUT_REPR].1, span);
                refusals.push(quote_spanned! { span =>
                    #[cfg(feature = "reflect")]
                    ::core::compile_error!(#msg);
                });
            }
            (Ident::new("Opaque", Span::call_site()), Vec::new())
        }
        Data::Union(u) => {
            let span = u.union_token.span;
            let msg = spanned_message(REFUSALS[IDX_UNION].1, span);
            refusals.push(quote_spanned! { span =>
                #[cfg(feature = "reflect")]
                ::core::compile_error!(#msg);
            });
            (Ident::new("Opaque", Span::call_site()), Vec::new())
        }
    };

    // A refused item emits its refusals and NOTHING ELSE. Emitting the descriptor beside
    // them would add rustc's own follow-on errors -- a union has no `Default`, so D20's
    // witness would fire too -- and a `.stderr` that pins two errors pins the second one's
    // rendering as well, which is drift this corpus does not need. The witness is dropped
    // with it, for the same reason.
    if !refusals.is_empty() {
        return (quote! { #(#refusals)* }, TokenStream2::new(), true);
    }

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

    (items, witness, false)
}
