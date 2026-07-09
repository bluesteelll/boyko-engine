//! `#[derive(Relationship)]` / `#[derive(RelationshipTarget)]` implementations,
//! plus the shared relationship-role parsing/types read by the `Component` derive.
//!
//! The two relationship derives are ADDITIVE over `#[derive(Component)]`: they emit
//! ONLY the `impl Relationship` / `impl RelationshipTarget` trait block, while the
//! hook wiring + entity-remap metadata are folded into the `Component` derive via
//! [`RelationshipRole`] (re-exported to [`crate::component`]).

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Path, Type, Visibility, parse_macro_input};

use crate::common::FieldAccess;
use crate::component::ComponentHookPaths;

// ── Relations v1 derive support (`Relationship` / `RelationshipTarget`) ──────────
//
// The two relationship derives are ADDITIVE over `#[derive(Component)]`: the
// component itself is declared with `#[derive(Component)]`, and `#[derive(Relationship)]`
// / `#[derive(RelationshipTarget)]` add ONLY the `impl Relationship` /
// `impl RelationshipTarget` trait block. The hook wiring + (source) the entity-remap
// clone/serialize metadata are folded into the SAME `register_hooks` / `component_id()`
// the `Component` derive already builds (composed, not a separate impl block — see
// `RelationshipRole` below, read by `component_macro`). This mirrors the in-crate
// `ChildOf` / `Children` HAND-MIRROR (`hierarchy/mod.rs`), which the dev-dep cycle
// forces to be written by hand but which is byte-for-byte the derive output.

/// Which side of a relation a `#[derive(Component)]` carries (Relations v1,
/// Decision 4), parsed from `#[relationship(...)]` / `#[relationship_target(...)]`.
/// `None` (no such attribute) is the plain-component path — the relationship overrides
/// in `component_macro` all no-op.
pub(crate) enum RelationshipRole {
    /// The source-of-truth foreign-key side (`#[relationship(target = T)]`). The
    /// `component_macro` path reads the spec's foreign-key field to drive the auto
    /// entity-remap codegen (B11).
    Source(RelationshipSourceSpec),
    /// The reverse-index side (`#[relationship_target(source = S, …)]`). The
    /// `component_macro` path needs only the DISCRIMINANT (to pick the cascade-hook /
    /// `Ignore`-clone / no-serialize overrides); the spec's fields are validated by
    /// construction in `parse_relationship_target` (private-field + mandatory
    /// `retain_empty` checks fire for `#[derive(Component)]` too) and re-read by the
    /// `RelationshipTarget` derive, so the carried payload is intentionally not read
    /// here — keeping it documents the parse and keeps the two roles symmetric.
    ///
    /// Boxed: the target spec carries a `syn::Type` (the collection field type) and is
    /// far larger than the source spec; an unboxed variant skews the enum size
    /// (`clippy::large_enum_variant`). This is a transient parse value (one per derive
    /// invocation), so the indirection is free.
    #[allow(dead_code)]
    Target(Box<RelationshipTargetSpec>),
}

/// Parsed `#[relationship(target = <Type> [, allow_self_referential])]` (the SOURCE
/// side). The foreign-key field is selected by the same rule as Bevy's
/// `relationship_field()` (single field, or the `#[relationship]`-annotated field of a
/// multi-field struct).
pub(crate) struct RelationshipSourceSpec {
    /// The `RelationshipTarget` type (`type Target = …`). Required.
    target: Path,
    /// The selector of the foreign-key `Entity` field — `self.#field` in `target()`.
    field: FieldAccess,
    /// `true` iff the bare `allow_self_referential` flag was supplied (sets
    /// `const ALLOW_SELF_REFERENTIAL = true`).
    allow_self_referential: bool,
}

impl RelationshipSourceSpec {
    /// Clones the foreign-key field selector for the auto entity-remap codegen.
    pub(crate) fn field_access(&self) -> FieldAccess {
        match &self.field {
            FieldAccess::Named(id) => FieldAccess::Named(id.clone()),
            FieldAccess::Index(i) => FieldAccess::Index(*i),
        }
    }
}

/// Parsed `#[relationship_target(source = <Type> [, linked_despawn] [, retain_empty])]`
/// (the TARGET side). The single field is the `Collection`; it must be private.
pub(crate) struct RelationshipTargetSpec {
    /// The `Relationship` source type (`type Source = …`). Required.
    source: Path,
    /// The collection field type (`type Collection = …`).
    field_ty: Type,
    /// The collection field selector — `&self.#field` in `collection()`.
    field: FieldAccess,
    /// `true` iff the bare `linked_despawn` flag was supplied
    /// (`const LINKED_DESPAWN = true`).
    linked_despawn: bool,
    /// `true` iff the bare `retain_empty` flag was supplied
    /// (`const RETAIN_EMPTY = true`). v1: MUST be `true` (W1) — `false` is rejected.
    retain_empty: bool,
}

impl RelationshipRole {
    /// Emits the `const HAS_HOOKS = true;` + `register_hooks` body wiring the GENERIC
    /// monomorphized hooks for this relation side (Decision 4). The fn pointers are
    /// the trait methods `<Self as Relationship>::on_insert` etc., which the runtime
    /// trait defaults forward to `generic_hooks::relationship_on_insert::<Self>` —
    /// each monomorphizes to one bare `HookFn` per relation type (no `dyn`). A SOURCE
    /// wires `on_insert` (link) + `on_replace` (unlink); a TARGET wires ONLY
    /// `on_replace` (the cascade — never `on_add`/`on_insert`, B7).
    pub(crate) fn hook_items_codegen(&self) -> TokenStream2 {
        let assigns = match self {
            RelationshipRole::Source(_) => quote! {
                hooks.on_insert = ::std::option::Option::Some(
                    <Self as ::boyko_ecs::ecs::core::relationship::Relationship>::on_insert
                        as ::boyko_ecs::ecs::core::component::hooks::HookFn,
                );
                hooks.on_replace = ::std::option::Option::Some(
                    <Self as ::boyko_ecs::ecs::core::relationship::Relationship>::on_replace
                        as ::boyko_ecs::ecs::core::component::hooks::HookFn,
                );
            },
            RelationshipRole::Target(_) => quote! {
                hooks.on_replace = ::std::option::Option::Some(
                    <Self as ::boyko_ecs::ecs::core::relationship::RelationshipTarget>::on_replace
                        as ::boyko_ecs::ecs::core::component::hooks::HookFn,
                );
            },
        };
        quote! {
            const HAS_HOOKS: bool = true;

            #[inline]
            fn register_hooks(
                hooks: &mut boyko_ecs::ecs::core::component::hooks::ComponentHooks,
            ) {
                #assigns
            }
        }
    }

    /// Rejects a user `#[component(on_insert=…)]` / `#[component(on_replace=…)]`
    /// alongside the relationship attribute (R5 `relationship_hook_collision`): the
    /// relationship OWNS those slots, so a user hook would be silently dropped or
    /// double-install. The other two slots (`on_add` / `on_remove`) are free — a
    /// relationship does not wire them, so they compose without conflict.
    pub(crate) fn reject_hook_collision(
        &self,
        ident: &Ident,
        hooks: &ComponentHookPaths,
    ) -> Result<(), TokenStream> {
        let owned = match self {
            RelationshipRole::Source(_) => hooks.on_insert.is_some() || hooks.on_replace.is_some(),
            RelationshipRole::Target(_) => hooks.on_replace.is_some(),
        };
        if owned {
            return Err(syn::Error::new(
                ident.span(),
                "a relationship owns its lifecycle hook slot(s): a \
                 #[derive(Relationship)] type owns on_insert + on_replace, a \
                 #[derive(RelationshipTarget)] type owns on_replace. Remove the \
                 conflicting #[component(on_insert=...)] / #[component(on_replace=...)] \
                 — the generic relationship hook is installed automatically.",
            )
            .to_compile_error()
            .into());
        }
        Ok(())
    }
}

/// Emits the explicit `Cloneability::Ignore` clone classification for a relationship
/// TARGET (B12): the reverse index is never byte-copied; a deep clone rebuilds it
/// from the sources' `Relationship` via the Link commands. Overrides the autoref
/// classification (a `Vec<Entity>` field would otherwise resolve `CloneViaFn`).
pub(crate) fn clone_ignore_codegen() -> TokenStream2 {
    quote! {
        const CLONE_BEHAVIOR:
            ::boyko_ecs::ecs::core::component::component_registry::Cloneability =
            ::boyko_ecs::ecs::core::component::component_registry::Cloneability::Ignore;

        #[inline]
        fn clone_behavior()
            -> ::boyko_ecs::ecs::core::component::component_registry::Cloneability {
            ::boyko_ecs::ecs::core::component::component_registry::Cloneability::Ignore
        }

        #[inline]
        fn clone_fn()
            -> ::std::option::Option<
                ::boyko_ecs::ecs::core::component::component_registry::CloneFn
            > {
            ::std::option::Option::None
        }
    }
}

/// Parses the relationship role from the item's attributes (Relations v1). Returns
/// `Ok(None)` when neither `#[relationship(...)]` nor `#[relationship_target(...)]` is
/// present (the plain-component path). The two are mutually exclusive — a type is one
/// side of a relation, never both.
pub(crate) fn parse_relationship_role(input: &DeriveInput) -> Result<Option<RelationshipRole>, TokenStream> {
    let has_rel = input.attrs.iter().any(|a| a.path().is_ident("relationship"));
    let has_target = input
        .attrs
        .iter()
        .any(|a| a.path().is_ident("relationship_target"));

    if has_rel && has_target {
        return Err(syn::Error::new(
            input.ident.span(),
            "a type cannot be both #[relationship(...)] (the source) and \
             #[relationship_target(...)] (the reverse index); a relation has two \
             distinct component types",
        )
        .to_compile_error()
        .into());
    }

    if has_rel {
        return Ok(Some(RelationshipRole::Source(parse_relationship_source(
            input,
        )?)));
    }
    if has_target {
        return Ok(Some(RelationshipRole::Target(Box::new(
            parse_relationship_target(input)?,
        ))));
    }
    Ok(None)
}

/// Parses `#[relationship(target = <Type> [, allow_self_referential])]` and selects the
/// foreign-key `Entity` field (Relations v1, Decision 4). `target` is required.
fn parse_relationship_source(input: &DeriveInput) -> Result<RelationshipSourceSpec, TokenStream> {
    let err = |span: Span, msg: &str| -> TokenStream {
        syn::Error::new(span, msg).to_compile_error().into()
    };

    let mut target: Option<Path> = None;
    let mut allow_self_referential = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("relationship") {
            continue;
        }
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("target") {
                if target.is_some() {
                    return Err(meta.error("duplicate #[relationship(...)] key; target may be set at most once"));
                }
                let value = meta.value()?; // consumes the `=`
                target = Some(value.parse::<Path>()?);
                return Ok(());
            }
            if meta.path.is_ident("allow_self_referential") {
                allow_self_referential = true;
                return Ok(());
            }
            Err(meta.error(
                "unknown #[relationship(...)] key; valid keys: \
                 target = <Type>, allow_self_referential",
            ))
        });
        if let Err(e) = result {
            return Err(e.to_compile_error().into());
        }
    }

    let target = target.ok_or_else(|| {
        err(
            input.ident.span(),
            "#[relationship(...)] requires `target = <Type>`: the RelationshipTarget \
             component on the target entity (e.g. #[relationship(target = Children)])",
        )
    })?;

    // Foreign-key field selection (mirror Bevy's `relationship_field()`).
    let field = select_relationship_field(input)?;

    Ok(RelationshipSourceSpec {
        target,
        field,
        allow_self_referential,
    })
}

/// Selects the single foreign-key field of a relationship source (Relations v1):
/// a tuple/named struct with ONE field → that field; a multi-field named struct →
/// the field annotated `#[relationship]` (error if zero or more than one); a unit
/// struct → error. The field type is NOT validated here — `impl Relationship`'s
/// `target(&self) -> Entity { self.#field }` makes a non-`Entity` field a loud type
/// error at the user's struct.
fn select_relationship_field(input: &DeriveInput) -> Result<FieldAccess, TokenStream> {
    let err = |span: Span, msg: &str| -> TokenStream {
        syn::Error::new(span, msg).to_compile_error().into()
    };
    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        Data::Enum(_) | Data::Union(_) => {
            return Err(err(
                input.ident.span(),
                "#[relationship] requires a struct (a single Entity foreign key); \
                 enums and unions cannot be a relationship source",
            ));
        }
    };

    let has_field_marker =
        |f: &syn::Field| f.attrs.iter().any(|a| a.path().is_ident("relationship"));

    match fields {
        Fields::Unit => Err(err(
            input.ident.span(),
            "#[relationship] requires exactly one Entity foreign-key field; a unit \
             struct has none (e.g. `struct Likes(Entity);`)",
        )),
        Fields::Unnamed(unnamed) => match unnamed.unnamed.len() {
            1 => Ok(FieldAccess::Index(0)),
            _ => {
                // Multi-field tuple struct: select the `#[relationship]`-annotated
                // field (exactly one).
                let annotated: Vec<usize> = unnamed
                    .unnamed
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| has_field_marker(f))
                    .map(|(i, _)| i)
                    .collect();
                match annotated.as_slice() {
                    [i] => Ok(FieldAccess::Index(*i)),
                    [] => Err(err(
                        input.ident.span(),
                        "#[relationship] on a multi-field struct requires exactly one \
                         field annotated `#[relationship]` (the Entity foreign key)",
                    )),
                    _ => Err(err(
                        input.ident.span(),
                        "#[relationship] requires exactly ONE foreign-key field; more \
                         than one field is annotated `#[relationship]`",
                    )),
                }
            }
        },
        Fields::Named(named) => match named.named.len() {
            1 => {
                let id = named.named[0]
                    .ident
                    .clone()
                    .expect("named field has an ident");
                Ok(FieldAccess::Named(id))
            }
            _ => {
                let annotated: Vec<&syn::Field> = named
                    .named
                    .iter()
                    .filter(|f| has_field_marker(f))
                    .collect();
                match annotated.as_slice() {
                    [f] => {
                        let id = f.ident.clone().expect("named field has an ident");
                        Ok(FieldAccess::Named(id))
                    }
                    [] => Err(err(
                        input.ident.span(),
                        "#[relationship] on a multi-field struct requires exactly one \
                         field annotated `#[relationship]` (the Entity foreign key)",
                    )),
                    _ => Err(err(
                        input.ident.span(),
                        "#[relationship] requires exactly ONE foreign-key field; more \
                         than one field is annotated `#[relationship]`",
                    )),
                }
            }
        },
    }
}

/// Parses `#[relationship_target(source = <Type> [, linked_despawn] [, retain_empty])]`
/// and selects the single collection field (Relations v1, Decision 4). `source` is
/// required; the field MUST be private (the reverse-index privacy fence); v1 requires
/// `retain_empty` (W1 — `RETAIN_EMPTY = false` is deferred to v1.1).
fn parse_relationship_target(input: &DeriveInput) -> Result<RelationshipTargetSpec, TokenStream> {
    let err = |span: Span, msg: &str| -> TokenStream {
        syn::Error::new(span, msg).to_compile_error().into()
    };

    let mut source: Option<Path> = None;
    let mut linked_despawn = false;
    let mut retain_empty = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("relationship_target") {
            continue;
        }
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("source") {
                if source.is_some() {
                    return Err(meta.error("duplicate #[relationship_target(...)] key; source may be set at most once"));
                }
                let value = meta.value()?; // consumes the `=`
                source = Some(value.parse::<Path>()?);
                return Ok(());
            }
            if meta.path.is_ident("linked_despawn") {
                linked_despawn = true;
                return Ok(());
            }
            if meta.path.is_ident("retain_empty") {
                retain_empty = true;
                return Ok(());
            }
            Err(meta.error(
                "unknown #[relationship_target(...)] key; valid keys: \
                 source = <Type>, linked_despawn, retain_empty",
            ))
        });
        if let Err(e) = result {
            return Err(e.to_compile_error().into());
        }
    }

    let source = source.ok_or_else(|| {
        err(
            input.ident.span(),
            "#[relationship_target(...)] requires `source = <Type>`: the Relationship \
             component on the source entity (e.g. #[relationship_target(source = ChildOf, \
             linked_despawn, retain_empty)])",
        )
    })?;

    // v1 (W1): `RETAIN_EMPTY = true` is MANDATORY. A target without `retain_empty`
    // would imply remove-on-empty, a NEW re-entrant edge (it fires the target's own
    // `on_replace` on emptying) deferred to v1.1. Reject the absence loudly so a user
    // does not silently get the unsupported policy.
    if !retain_empty {
        return Err(err(
            input.ident.span(),
            "#[relationship_target(...)] requires the `retain_empty` flag in v1 \
             (RETAIN_EMPTY = true is mandatory; remove-on-empty is deferred to v1.1)",
        ));
    }

    // Collection field selection: exactly one field, which must be private.
    let (field, field_ty) = select_relationship_target_field(input)?;

    Ok(RelationshipTargetSpec {
        source,
        field_ty,
        field,
        linked_despawn,
        retain_empty,
    })
}

/// Selects the single collection field of a relationship target (Relations v1) and
/// enforces the privacy fence (the reverse index must be unwritable by user code).
/// A tuple-struct field is private by default (no `pub`); a named-struct field must
/// have inherited (private) visibility — a `pub` / `pub(...)` field is a compile error.
fn select_relationship_target_field(
    input: &DeriveInput,
) -> Result<(FieldAccess, Type), TokenStream> {
    let err = |span: Span, msg: &str| -> TokenStream {
        syn::Error::new(span, msg).to_compile_error().into()
    };
    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        Data::Enum(_) | Data::Union(_) => {
            return Err(err(
                input.ident.span(),
                "#[relationship_target] requires a struct with one collection field; \
                 enums and unions cannot be a relationship target",
            ));
        }
    };

    let private = |vis: &Visibility| matches!(vis, Visibility::Inherited);

    match fields {
        Fields::Unit => Err(err(
            input.ident.span(),
            "#[relationship_target] requires exactly one collection field; a unit \
             struct has none (e.g. `struct LikedBy(Vec<Entity>);`)",
        )),
        Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
            let f = &unnamed.unnamed[0];
            if !private(&f.vis) {
                return Err(err(
                    input.ident.span(),
                    "#[relationship_target]'s collection field must be PRIVATE (the \
                     reverse index must not be writable by user code); remove the `pub`",
                ));
            }
            Ok((FieldAccess::Index(0), f.ty.clone()))
        }
        Fields::Named(named) if named.named.len() == 1 => {
            let f = &named.named[0];
            if !private(&f.vis) {
                return Err(err(
                    input.ident.span(),
                    "#[relationship_target]'s collection field must be PRIVATE (the \
                     reverse index must not be writable by user code); remove the `pub`",
                ));
            }
            let id = f.ident.clone().expect("named field has an ident");
            Ok((FieldAccess::Named(id), f.ty.clone()))
        }
        _ => Err(err(
            input.ident.span(),
            "#[relationship_target] requires exactly one collection field (e.g. \
             `struct LikedBy(Vec<Entity>);`)",
        )),
    }
}

/// Implementation of `#[derive(Relationship)]` (see the public entry in `lib.rs`).
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let spec = match parse_relationship_source(&input) {
        Ok(s) => s,
        Err(ts) => return ts,
    };

    let name = &input.ident;
    let target = &spec.target;
    let field_sel = spec.field.offset_of_selector();
    let allow_self = spec.allow_self_referential;

    // `from_target` constructs `Self` from the target Entity. A single-field tuple
    // struct uses `Self(target)`; a single-field named struct uses
    // `Self { #field: target }`. A multi-field struct sets only the FK field and
    // fills the rest via `..Default::default()` (so the source may carry extra data),
    // which requires `Self: Default` — the spec's documented multi-field requirement.
    let from_target_body = match &spec.field {
        FieldAccess::Index(0) if is_single_field_struct(&input) => {
            quote! { Self(target) }
        }
        FieldAccess::Index(i) => {
            // Multi-field tuple struct: set field `i`, default the rest.
            let idx = syn::Index::from(*i);
            quote! {
                let mut __this = <Self as ::std::default::Default>::default();
                __this.#idx = target;
                __this
            }
        }
        FieldAccess::Named(id) if is_single_field_struct(&input) => {
            quote! { Self { #id: target } }
        }
        FieldAccess::Named(id) => {
            quote! {
                Self {
                    #id: target,
                    ..<Self as ::std::default::Default>::default()
                }
            }
        }
    };

    let allow_self_const = if allow_self {
        quote! {
            const ALLOW_SELF_REFERENTIAL: bool = true;
        }
    } else {
        // Keep the trait default (`false`) — emit nothing.
        TokenStream2::new()
    };

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::relationship::Relationship for #name {
            type Target = #target;

            #[inline]
            fn target(&self) -> ::boyko_ecs::ecs::core::entity::entity::Entity {
                self.#field_sel
            }

            #[inline]
            fn from_target(
                target: ::boyko_ecs::ecs::core::entity::entity::Entity,
            ) -> Self {
                #from_target_body
            }

            #allow_self_const

            // The `on_insert` / `on_replace` HookFn forwarders keep the trait
            // defaults: they call `generic_hooks::relationship_on_insert::<Self>` /
            // `..on_replace::<Self>`, each monomorphizing to one bare HookFn for this
            // relation type (no `dyn`). The paired `#[derive(Component)]` wires those
            // trait methods into `hooks.on_insert` / `hooks.on_replace`.
            //
            // SAFETY (forwarded from the trait default): the `HookFn` contract — the
            // body is invoked only inside the single-threaded apply window with a view
            // that withholds every structural + `&mut`-into-storage method, and holds
            // no `world`-derived `&` across the `commands()` mint (F2). The generic
            // body upholds this verbatim.
        }
    };

    expanded.into()
}

/// Implementation of `#[derive(RelationshipTarget)]` (see the public entry in `lib.rs`).
pub(crate) fn expand_target(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let spec = match parse_relationship_target(&input) {
        Ok(s) => s,
        Err(ts) => return ts,
    };

    let name = &input.ident;
    let source = &spec.source;
    let field_ty = &spec.field_ty;
    let field_sel = spec.field.offset_of_selector();
    let linked_despawn = spec.linked_despawn;
    let retain_empty = spec.retain_empty;

    // `from_collection_risky` rebuilds the target from a collection. A tuple struct
    // uses `Self(c)`; a named struct uses `Self { #field: c }`.
    let from_collection_body = match &spec.field {
        FieldAccess::Index(_) => quote! { Self(collection) },
        FieldAccess::Named(id) => quote! { Self { #id: collection } },
    };

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::relationship::RelationshipTarget for #name {
            type Source = #source;
            type Collection = #field_ty;

            const LINKED_DESPAWN: bool = #linked_despawn;
            const RETAIN_EMPTY: bool = #retain_empty;

            #[inline]
            fn collection(&self) -> &Self::Collection {
                &self.#field_sel
            }

            #[inline]
            fn collection_mut_risky(&mut self) -> &mut Self::Collection {
                &mut self.#field_sel
            }

            #[inline]
            fn from_collection_risky(collection: Self::Collection) -> Self {
                #from_collection_body
            }

            // `on_replace` keeps the trait default: it calls
            // `generic_hooks::relationship_target_on_replace::<Self>` (the cascade /
            // unlink-only body, branched on `LINKED_DESPAWN`). The paired
            // `#[derive(Component)]` wires this trait method into `hooks.on_replace`
            // ONLY (never on_add/on_insert — B7 spurious-first-cascade guard).
            //
            // SAFETY (forwarded from the trait default): the `HookFn` contract — see
            // `Relationship`. The cascade body copies sources to a stack buffer (inline
            // path) or re-derives `&T` per turn (wide path) so no `world`-derived `&`
            // spans the `commands()` mint.
        }
    };

    expanded.into()
}

/// `true` iff the derive input is a struct with exactly one field (tuple or named).
/// Used to choose between the positional `Self(x)` constructor (single field) and the
/// `..Default::default()`-filled constructor (multi-field) in the `Relationship`
/// derive's `from_target`.
fn is_single_field_struct(input: &DeriveInput) -> bool {
    match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Unnamed(u) => u.unnamed.len() == 1,
            Fields::Named(n) => n.named.len() == 1,
            Fields::Unit => false,
        },
        _ => false,
    }
}
