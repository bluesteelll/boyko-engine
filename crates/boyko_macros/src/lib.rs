use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Expr, Fields, Ident, ItemStruct, Path, Type, parse_macro_input,
};

/// Derive macro for implementing the Component trait.
///
/// Generates all required methods for the Component trait and adds inherent
/// constants for layout queries (`SIZE`, `ALIGN`, `layout()`).
///
/// Component IDs are assigned lazily at runtime via a per-type `OnceLock` and
/// the global registry — see
/// `boyko_ecs::ecs::core::component::component_registry` for the assignment
/// algorithm and startup warm-up contract.
///
/// # Example
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// #[derive(Component)]
/// struct Position {
///     x: f32,
///     y: f32,
///     z: f32,
/// }
/// ```
///
/// The example is `ignore`'d because proc-macro crates cannot consume their own
/// macros, and `boyko-macros` cannot depend on `boyko-ecs` (that would create a
/// cycle). Real usage lives in `boyko-ecs` integration tests.
///
/// # Lifecycle hooks (Phase 14a)
///
/// An optional `#[component(...)]` helper attribute binds lifecycle-hook
/// functions to the component type. Each key takes a path to an
/// `unsafe fn(DeferredEcsMaster<'_>, HookContext)`:
///
/// ```ignore
/// #[derive(Component)]
/// #[component(on_add = my_on_add, on_remove = my_on_remove)]
/// struct Health(u32);
///
/// unsafe fn my_on_add(world: DeferredEcsMaster<'_>, ctx: HookContext) { /* ... */ }
/// unsafe fn my_on_remove(world: DeferredEcsMaster<'_>, ctx: HookContext) { /* ... */ }
/// ```
///
/// Valid keys: `on_add`, `on_insert`, `on_replace`, `on_remove`. Any other key
/// (including `on_despawn`, which is deferred to Phase 14b) is a compile error,
/// as is a duplicate key. When at least one key is present the derive emits
/// `const HAS_HOOKS: bool = true;` and a `register_hooks` impl; the
/// macro-generated `component_id()` then installs the hooks into the cold
/// `HOOKS` table on first call, atomically with ID assignment and therefore
/// before the component can appear in any archetype (the staleness-immunity
/// property, plan §6.1 / Q-A5).
///
/// Derive hooks and the runtime [`register_component_hooks`] builder are
/// **mutually exclusive** per type: a type carrying `#[component(...)]` keeps
/// its slot installed by `component_id()`, so calling the runtime builder for
/// it panics. A plain `#[derive(Component)]` (no keys ⇒ `HAS_HOOKS = false`)
/// installs nothing — its slot stays unset until the runtime builder commits.
///
/// # Single-component `Bundle` emission (Phase 22, D7)
///
/// By default the derive ALSO emits `impl Bundle for Self` (a one-component
/// bundle), so `commands.spawn(PlayerTag)` / `.insert(Velocity { .. })` work
/// for every derived component without a wrapper struct. Because
/// `Bundle: Send + Sync + Unpin + 'static`, the derive tightens the effective
/// requirements on the derived type — a named const-assert
/// (`_boyko_component_as_bundle_requires_send_sync_unpin`) leads the
/// diagnostics with a readable, comment-bearing error when the bounds fail.
///
/// Opt out with the `no_bundle` flag key:
///
/// ```ignore
/// #[derive(Component)]
/// #[component(no_bundle)]
/// struct Exotic(std::rc::Rc<u32>); // !Send — storable, but not spawnable
/// ```
///
/// `no_bundle` suppresses BOTH the const-assert and the `Bundle` /
/// `BundleSealed` impls. The type remains a full `Component` (usable through
/// the type-erased direct API); it simply cannot be passed where a `Bundle`
/// is expected (wrap it in a `#[derive(Bundle)]` struct instead). The flag is
/// also the escape hatch when a type must derive BOTH `Component` and
/// `Bundle` — without it the two derives now collide on the `Bundle` impl.
#[proc_macro_derive(Component, attributes(component, require))]
pub fn component_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // Phase 14a: parse the optional `#[component(...)]` hook attribute
    // (extended in Phase 22 with the bare `no_bundle` flag key, and in EnableTag
    // Wave 5 with the `storage = "bitset"` NameValue key).
    let hooks = match parse_component_hooks(&input.attrs) {
        Ok(h) => h,
        Err(ts) => return ts,
    };

    // Required components (Feature 1): parse the optional `#[require(...)]`
    // attribute(s). Each key is a component type with an optional ctor:
    // `B` (uses `B::default()`), `C = expr` (a capture-free expression), or
    // `D(args)` (a call expression `D(args)`). Duplicate same-id keys are a
    // compile error.
    let requires = match parse_requires(&input.attrs) {
        Ok(r) => r,
        Err(ts) => return ts,
    };

    // EnableTag D5: a bitset enable tag has NO `ComponentPool`, so its data has
    // nowhere to live — a fielded `storage = "bitset"` struct is nonsensical.
    // Reject it loudly (fail-loud, plan Step 10 (3)). The macro cannot see the
    // runtime size, so it enforces the syntactic rule "a bitset tag must be a
    // fieldless struct" (unit struct, or an empty named/tuple struct), which is
    // the common ZST tag shape the plan targets (`struct Stunned;`). Enums and
    // unions are rejected for the same reason.
    if hooks.storage_bitset
        && let Err(ts) = reject_non_zst_bitset_tag(&input)
    {
        return ts;
    }

    // EnableTag D5 (Step 10 hardening A): a bitset enable tag has NO
    // `ComponentPool`, so the structural lifecycle hooks (on_add / on_insert /
    // on_replace / on_remove) can NEVER fire for it — enable/disable is a
    // per-row bit RMW, not a structural component op. Silently accepting the
    // combination would install dead hooks (a compile-but-lie footgun), so
    // reject it loudly at macro time. (A future enable-bit observer, if any,
    // would be a SEPARATE key, not these structural hooks.)
    if hooks.storage_bitset && hooks.any() {
        return syn::Error::new(
            input.ident.span(),
            "#[component(storage = \"bitset\")] cannot combine with lifecycle hooks \
             (on_add/on_insert/on_replace/on_remove): an enable-bit tag has no \
             ComponentPool, so these hooks never fire. Remove the hook(s), or drop \
             `storage = \"bitset\"` to use a normal component.",
        )
        .to_compile_error()
        .into();
    }

    // Feature 3 (D3 gate): scan the struct fields for an `Entity` / `ChildOf` type
    // BEFORE `input.ident` is moved out below. A `Copy`-with-`Entity` component is
    // forced to `CloneViaFn` so the deep-clone remap can run.
    let has_entity_field = struct_has_entity_field(&input);

    let name = input.ident;

    // Emit `const HAS_HOOKS = true;` + a `register_hooks` impl only when at
    // least one hook key is present; otherwise the trait defaults
    // (`HAS_HOOKS = false`, empty `register_hooks`) apply.
    let hook_items = hooks.codegen();

    // EnableTag D5: emit `const STORAGE_IS_BITSET = true;` (overriding the trait
    // default) and the install call for the minted id, only for a bitset tag.
    let storage_items = hooks.storage_codegen();
    let storage_install = hooks.storage_install_codegen();

    // Required components (Feature 1): emit `const HAS_REQUIRES = true;`, the
    // `register_required` impl, and the free `__require_ctor_*` fns when at least
    // one `#[require(...)]` key is present; the gated `install_required::<Self>`
    // install call in `component_id()`. A require-free derive emits NOTHING here
    // (the trait defaults apply) — the 0%-gate.
    let require_ctor_fns = requires.ctor_fns_codegen(&name);
    let require_items = requires.codegen(&name);
    let require_install = requires.install_codegen();

    // Entity cloning (Feature 3): emit the `CLONE_BEHAVIOR` const + `clone_fn()`
    // override classifying the type, and the UNGATED `install_clone_fn::<Self>(raw)`
    // call in `component_id()`. The classification (`TriviallyCopyable` /
    // `CloneViaFn` / `Ignore`) is decided by:
    //   * `#[component(no_clone)]`  → `Ignore` (the trait defaults — emit nothing);
    //   * `#[component(clone = f)]` → `CloneViaFn` with the user free fn `f`;
    //   * else → autoref specialization at the type level: a `Copy`-no-`Entity`
    //     type is `TriviallyCopyable` (batch memcpy, `clone_fn = None`), a `Clone`
    //     type (incl. `Copy`-with-`Entity`) is `CloneViaFn` with
    //     `clone_via_clone::<Self>`, a non-`Clone` type is `Ignore`.
    // The `Entity`-field scan is syntactic (forces `CloneViaFn` so the deep-clone
    // remap can run; computed above before `input.ident` was moved). The install is
    // ALWAYS emitted (ungated) — one cold `OnceLock::set` per type, the 0%-gate
    // (registration-time only).
    //
    // W1 (b): `storage = "bitset"` SUPPRESSES the clone override AND the install — a
    // bitset enable tag has NO `ComponentPool`, so it must classify `Ignore` (the
    // trait default `CLONE_BEHAVIOR` / `clone_fn`) and never enter a clone's column
    // set. The clone materialization additionally skips any bitset id (W1 (a)); the
    // suppression here keeps the metadata table consistent (`Ignore` / `None`) and
    // mirrors the single-component `Bundle` suppression below.
    let (clone_items, clone_install) = if hooks.storage_bitset {
        (TokenStream2::new(), TokenStream2::new())
    } else {
        (
            clone_codegen(&name, &hooks, has_entity_field),
            quote! {
                boyko_ecs::ecs::core::component::component_registry::install_clone_fn::<Self>(raw);
            },
        )
    };

    // Phase 22 D7: single-component Bundle emission (suppressed by
    // `#[component(no_bundle)]`). EnableTag D6: `storage = "bitset"` ALSO
    // suppresses it — a bitset tag has no `ComponentPool` and must not be
    // spawnable as a one-component bundle (`storage = "bitset"` implies
    // `no_bundle`).
    let bundle_items = if hooks.no_bundle || hooks.storage_bitset {
        TokenStream2::new()
    } else {
        component_self_bundle_codegen(&name)
    };

    let expanded = quote! {
        #bundle_items

        #require_ctor_fns

        impl #name {
            /// Size of this component in bytes.
            pub const SIZE: usize = std::mem::size_of::<Self>();

            /// Alignment requirement of this component.
            pub const ALIGN: usize = std::mem::align_of::<Self>();

            /// Returns the memory layout for this component type.
            pub const fn layout() -> std::alloc::Layout {
                // SAFETY: size and alignment come from `size_of` / `align_of` for
                // `Self`, which are always valid: alignment is a power of two and
                // size fits within `isize::MAX` (otherwise `Self` could not exist).
                unsafe { std::alloc::Layout::from_size_align_unchecked(Self::SIZE, Self::ALIGN) }
            }
        }

        impl boyko_ecs::ecs::core::component::component::Component for #name {
            #[inline]
            fn component_id() -> boyko_ecs::ecs::identifiers::primitives::ComponentId {
                static ID: ::std::sync::OnceLock<boyko_ecs::ecs::identifiers::primitives::ComponentId>
                    = ::std::sync::OnceLock::new();
                *ID.get_or_init(|| {
                    let raw = boyko_ecs::ecs::core::component::component_registry::register_new::<Self>();
                    // Phase 14a (plan §6.1 step 4): install this type's derive
                    // hooks into `HOOKS[raw]` atomically with ID assignment,
                    // before the component can appear in any archetype. Gated on
                    // the const `Self::HAS_HOOKS` (derive XOR runtime-builder
                    // contract): for a plain `#[derive(Component)]` the const is
                    // `false`, so this call const-folds away and the slot stays
                    // UNSET — which means "no hooks" everywhere downstream and
                    // leaves the runtime builder free to commit via `set`.
                    if Self::HAS_HOOKS {
                        boyko_ecs::ecs::core::component::component_registry::install_hooks::<Self>(raw);
                    }
                    #storage_install
                    #require_install
                    #clone_install
                    boyko_ecs::ecs::identifiers::primitives::ComponentId(raw)
                })
            }

            #storage_items

            #hook_items

            #require_items

            #clone_items

            // NOTE: `std::any::type_name::<Self>()` is not yet stable as a const fn.
            // Calling it from a regular fn body is fine; the compiler folds it to a
            // string literal at codegen, so there is no measurable overhead.
            #[inline]
            fn debug_type_name() -> &'static str {
                std::any::type_name::<Self>()
            }

            #[inline]
            fn mem_size() -> usize {
                Self::SIZE
            }

            #[inline]
            fn alignment() -> usize {
                Self::ALIGN
            }
        }
    };

    expanded.into()
}

/// Parsed `#[component(...)]` lifecycle-hook paths (Phase 14a). Each field holds
/// the user-supplied path to an `unsafe fn(DeferredEcsMaster<'_>, HookContext)`,
/// or `None` when the key was omitted.
///
/// Phase 22: also carries the bare `no_bundle` flag key, which suppresses the
/// derive's single-component `Bundle` emission (D7 opt-out).
///
/// EnableTag (Wave 5 Step 10 / D5): also carries the `storage = "bitset"`
/// NameValue key (a `LitStr`, NOT a bare flag — W1-r6). When set, the derive
/// emits `const STORAGE_IS_BITSET = true`, routes the minted id's registration
/// through `install_storage_kind::<Self>` (classifying it `StorageKind::Bitset`),
/// and suppresses the single-component `Bundle` emission (a bitset tag has no
/// `ComponentPool` and must not be spawnable — D6 implies `no_bundle`).
#[derive(Default)]
struct ComponentHookPaths {
    on_add: Option<Path>,
    on_insert: Option<Path>,
    on_replace: Option<Path>,
    on_remove: Option<Path>,
    no_bundle: bool,
    /// `true` iff `storage = "bitset"` was supplied (EnableTag D5).
    storage_bitset: bool,
    /// Feature 3: `true` iff the bare `no_clone` flag was supplied — opts the
    /// component out of cloning (`Cloneability::Ignore`, `clone_fn = None`), even if
    /// the type is `Clone` / `Copy`.
    no_clone: bool,
    /// Feature 3: `Some(path)` iff `clone = <free fn>` was supplied — a capture-free
    /// `unsafe fn(*const u8, *mut u8)` (the `CloneFn` shape) the user installs as the
    /// custom clone. Mutually exclusive with `no_clone`.
    clone_with: Option<Path>,
}

impl ComponentHookPaths {
    /// `true` iff at least one hook key was supplied.
    fn any(&self) -> bool {
        self.on_add.is_some()
            || self.on_insert.is_some()
            || self.on_replace.is_some()
            || self.on_remove.is_some()
    }

    /// Emits the `const HAS_HOOKS = true;` + `register_hooks` impl when any key
    /// is present, or an empty token stream (trait defaults apply) otherwise.
    ///
    /// Each provided path is assigned into the corresponding `ComponentHooks`
    /// field. The `Option<HookFn>` field coerces the user `unsafe fn` path to
    /// the `HookFn` pointer type, so no explicit cast is emitted.
    fn codegen(&self) -> TokenStream2 {
        if !self.any() {
            return TokenStream2::new();
        }

        let mut assigns: Vec<TokenStream2> = Vec::new();
        if let Some(p) = &self.on_add {
            assigns.push(quote! { hooks.on_add = ::std::option::Option::Some(#p); });
        }
        if let Some(p) = &self.on_insert {
            assigns.push(quote! { hooks.on_insert = ::std::option::Option::Some(#p); });
        }
        if let Some(p) = &self.on_replace {
            assigns.push(quote! { hooks.on_replace = ::std::option::Option::Some(#p); });
        }
        if let Some(p) = &self.on_remove {
            assigns.push(quote! { hooks.on_remove = ::std::option::Option::Some(#p); });
        }

        quote! {
            const HAS_HOOKS: bool = true;

            #[inline]
            fn register_hooks(
                hooks: &mut boyko_ecs::ecs::core::component::hooks::ComponentHooks,
            ) {
                #(#assigns)*
            }
        }
    }

    /// EnableTag D5: emits `const STORAGE_IS_BITSET = true;` (overriding the
    /// `Component` trait default of `false`) when `storage = "bitset"` was
    /// supplied, or an empty token stream (trait default applies) otherwise.
    ///
    /// This const is what makes `Added<T>` / `Changed<T>` on the tag a compile
    /// error (the D4 per-monomorphization const-asserts read it) and what
    /// `install_storage_kind::<Self>` const-gates on.
    fn storage_codegen(&self) -> TokenStream2 {
        if !self.storage_bitset {
            return TokenStream2::new();
        }
        quote! {
            const STORAGE_IS_BITSET: bool = true;
        }
    }

    /// EnableTag D5: emits the registration-time call that classifies the minted
    /// id as `StorageKind::Bitset`, or an empty token stream otherwise.
    ///
    /// Emitted into the derive's `component_id()` `OnceLock` init closure,
    /// AFTER `register_new` mints `raw` and (when present) hooks install — the
    /// same atomic-with-id-assignment, before-any-archetype ordering that
    /// `install_hooks` relies on. Routed through the `pub` wrapper
    /// `install_storage_kind::<Self>` because the underlying `set_storage_kind`
    /// is `pub(crate)` and unreachable from a downstream crate's derive output.
    fn storage_install_codegen(&self) -> TokenStream2 {
        if !self.storage_bitset {
            return TokenStream2::new();
        }
        quote! {
            boyko_ecs::ecs::core::component::component_registry::install_storage_kind::<Self>(raw);
        }
    }
}

/// EnableTag D5 (Step 10 (3)): rejects a `#[component(storage = "bitset")]` on
/// anything other than a fieldless struct.
///
/// A bitset enable tag has no `ComponentPool`, so component data has nowhere to
/// live — a tag carrying fields is nonsensical. The proc-macro cannot compute
/// the runtime size of `T`, so it enforces the conservative syntactic rule "a
/// bitset tag must be a fieldless struct" (a unit struct `struct Stunned;`, or
/// an empty `struct Stunned {}` / `struct Stunned()`), which is exactly the ZST
/// tag shape the plan targets. Enums and unions are rejected as well — they have
/// no fieldless single-shape meaning for a tag. The fail-loud diagnostic names
/// the requirement so the user fixes it at the declaration.
///
/// # Generic-tag limitation
///
/// A bitset tag must be a **true fieldless struct**. Generic type or lifetime
/// parameters are permitted ONLY as long as the struct still carries no fields:
/// `struct Tag<'a>;` and `struct Tag<T>;` are accepted, but
/// `struct Tag<T>(PhantomData<T>);` is rejected — the `PhantomData<T>` field
/// (though a runtime ZST) is a field, so it falls into the reject arm. This is
/// an intentional conservative limitation: a proc-macro sees only the surface
/// syntax and cannot prove the runtime ZST-ness of an arbitrary field type, so
/// it refuses anything with a field rather than accept a tag whose data has no
/// `ComponentPool` to live in. Wrap the phantom-carrying type in a plain
/// `#[derive(Component)]` if it truly needs to carry a parameter; a bitset tag
/// that must be generic should hold its parameter in the struct head, not a
/// field.
fn reject_non_zst_bitset_tag(input: &DeriveInput) -> Result<(), TokenStream> {
    let err = |span: Span, msg: &str| -> TokenStream {
        syn::Error::new(span, msg).to_compile_error().into()
    };

    match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Unit => Ok(()),
            Fields::Named(named) if named.named.is_empty() => Ok(()),
            Fields::Unnamed(unnamed) if unnamed.unnamed.is_empty() => Ok(()),
            _ => Err(err(
                input.ident.span(),
                "#[component(storage = \"bitset\")] requires a fieldless struct \
                 (e.g. `struct Stunned;`): a bitset enable tag has no ComponentPool, \
                 so any field data would have nowhere to live",
            )),
        },
        Data::Enum(_) | Data::Union(_) => Err(err(
            input.ident.span(),
            "#[component(storage = \"bitset\")] requires a fieldless struct \
             (e.g. `struct Stunned;`); enums and unions cannot be enable tags",
        )),
    }
}

/// Parses the optional `#[component(on_add = path, ...)]` attribute (Phase 14a,
/// plan §6.1). Mirrors the `#[event]` macro's `parse_nested_meta` idiom.
///
/// Accepts the four lifecycle-hook keys (`on_add` / `on_insert` / `on_replace` /
/// `on_remove`), each `= <path>`, the bare `no_bundle` flag key (Phase 22
/// D7 — suppresses the single-component `Bundle` emission), and the
/// `storage = "bitset"` NameValue key (EnableTag D5 — a `LitStr` value, Wave 5
/// Step 10). Rejects:
/// - `on_despawn` (removed from 14a — deferred to 14b),
/// - any other unknown key,
/// - a duplicate key,
/// - a key missing its `= <path>` value (surfaced by `meta.value()` / `parse`),
/// - an unknown `storage` string (only `"bitset"` is supported),
/// - more than one `#[component(...)]` attribute on the same item.
fn parse_component_hooks(attrs: &[syn::Attribute]) -> Result<ComponentHookPaths, TokenStream> {
    let mut paths = ComponentHookPaths::default();
    let mut seen_attr = false;

    for attr in attrs {
        if !attr.path().is_ident("component") {
            continue;
        }
        if seen_attr {
            return Err(syn::Error::new_spanned(
                attr,
                "duplicate #[component(...)] attribute; combine all hooks into one",
            )
            .to_compile_error()
            .into());
        }
        seen_attr = true;

        let result = attr.parse_nested_meta(|meta| {
            // `on_despawn` was removed from Phase 14a — emit a clear error rather
            // than letting it fall into the generic "unknown key" branch.
            if meta.path.is_ident("on_despawn") {
                return Err(meta.error(
                    "on_despawn is not supported in this version (deferred to Phase 14b); \
                     valid keys: on_add, on_insert, on_replace, on_remove, no_bundle, \
                     no_clone, clone = <fn>, storage = \"bitset\"",
                ));
            }

            // Phase 22 D7: bare flag key — no `= <value>` follows.
            if meta.path.is_ident("no_bundle") {
                if paths.no_bundle {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; no_bundle may be set at most once",
                    ));
                }
                paths.no_bundle = true;
                return Ok(());
            }

            // Feature 3: bare flag key `no_clone` — opt out of cloning.
            if meta.path.is_ident("no_clone") {
                if paths.no_clone {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; no_clone may be set at most once",
                    ));
                }
                if paths.clone_with.is_some() {
                    return Err(meta.error(
                        "#[component(no_clone)] conflicts with #[component(clone = ...)]: \
                         a component is EITHER non-cloneable OR has a custom clone fn",
                    ));
                }
                paths.no_clone = true;
                return Ok(());
            }

            // Feature 3: NameValue key `clone = <free fn path>` — a custom
            // capture-free `unsafe fn(*const u8, *mut u8)` clone (the `CloneFn` shape).
            if meta.path.is_ident("clone") {
                if paths.clone_with.is_some() {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; clone may be set at most once",
                    ));
                }
                if paths.no_clone {
                    return Err(meta.error(
                        "#[component(clone = ...)] conflicts with #[component(no_clone)]: \
                         a component is EITHER non-cloneable OR has a custom clone fn",
                    ));
                }
                let value = meta.value()?; // consumes the `=`
                paths.clone_with = Some(value.parse::<Path>()?);
                return Ok(());
            }

            // EnableTag D5 (Wave 5 Step 10): `storage = "bitset"` — a NameValue
            // key whose value is a STRING LITERAL (W1-r6: parsed as a `LitStr`,
            // NOT a bare-key flag and NOT a path). Any other string is rejected
            // with a message naming the allowed value.
            if meta.path.is_ident("storage") {
                if paths.storage_bitset {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; storage may be set at most once",
                    ));
                }
                let value = meta.value()?; // consumes the `=`
                let lit: syn::LitStr = value.parse()?;
                match lit.value().as_str() {
                    "bitset" => paths.storage_bitset = true,
                    other => {
                        return Err(syn::Error::new_spanned(
                            &lit,
                            format!(
                                "unknown component storage {other:?}; \
                                 the only supported value is \"bitset\""
                            ),
                        ));
                    }
                }
                return Ok(());
            }

            let slot = if meta.path.is_ident("on_add") {
                &mut paths.on_add
            } else if meta.path.is_ident("on_insert") {
                &mut paths.on_insert
            } else if meta.path.is_ident("on_replace") {
                &mut paths.on_replace
            } else if meta.path.is_ident("on_remove") {
                &mut paths.on_remove
            } else {
                return Err(meta.error(
                    "unknown #[component(...)] key; \
                     valid keys: on_add, on_insert, on_replace, on_remove, no_bundle, \
                     no_clone, clone = <fn>, storage = \"bitset\"",
                ));
            };

            if slot.is_some() {
                return Err(meta.error(
                    "duplicate #[component(...)] key; each hook may be set at most once",
                ));
            }
            let value = meta.value()?; // consumes the `=`
            *slot = Some(value.parse::<Path>()?);
            Ok(())
        });

        if let Err(e) = result {
            return Err(e.to_compile_error().into());
        }
    }

    Ok(paths)
}

/// Phase 22 D7: emits the single-component `Bundle` impl block for
/// `#[derive(Component)]` — the whole `self` is the one component.
///
/// Emitted items, in this order:
///
/// 1. A **named const-assert** (`_boyko_component_as_bundle_requires_send_sync_unpin`)
///    placed before the impls so its readable, comment-bearing E0277 leads the
///    diagnostics when the type is `!Send` / `!Sync` / `!Unpin`. It does NOT
///    suppress the impl-level supertrait E0277 (supertrait obligations on a
///    concrete impl cannot be silenced) — both diagnostics appear; the named
///    symbol is the anchor.
/// 2. `impl BundleSealed for T {}` + `impl Bundle for T` mirroring the
///    `#[derive(Bundle)]` expansion for a one-component bundle: per-type
///    concrete `static INFO: OnceLock<BundleStaticInfo>` (sidesteps the
///    Phase-12.5 generic-fn-static collapse trap), a 1-element id slice
///    (trivially canonical — B1), `cached_archetype_id` delegating to the
///    per-world cache helper (SBC4), and the `ManuallyDrop`-upfront (B4) +
///    pointer-based byte-erasure (C5) `for_each_component_bytes`.
///
/// The in-crate `impl_self_bundle!` macro
/// (`boyko_ecs::ecs::core::bundle::self_bundle`) is the hand-written mirror of
/// this emission — keep the two in lock-step.
fn component_self_bundle_codegen(name: &Ident) -> TokenStream2 {
    quote! {
        const _: () = {
            // Single-component bundle emission requires Send + Sync + Unpin.
            // Opt out with #[component(no_bundle)] for intentionally exotic types.
            const fn _boyko_component_as_bundle_requires_send_sync_unpin<
                T: Send + Sync + Unpin,
            >() {}
            _boyko_component_as_bundle_requires_send_sync_unpin::<#name>();
        };

        impl ::boyko_ecs::ecs::core::bundle::bundle::sealed::BundleSealed for #name {}

        impl ::boyko_ecs::ecs::core::bundle::bundle::Bundle for #name {
            fn static_info() -> &'static ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo {
                // O3 coalesced static (Decision SBC-D5). One OnceLock holds
                // BundleTypeId + the 1-element component-ids slice. Cached
                // path: single Acquire load.
                static INFO: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo
                > = ::std::sync::OnceLock::new();

                INFO.get_or_init(|| {
                    // B1 canonical order: a 1-element slice is trivially
                    // sorted. Leak bounded by SBC8 (once per type per process).
                    let leaked: &'static [
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId;
                        1
                    ] = ::std::boxed::Box::leak(::std::boxed::Box::new([
                        <#name as ::boyko_ecs::ecs::core::component::component::Component>::component_id()
                    ]));

                    ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo {
                        type_id: ::boyko_ecs::ecs::core::bundle::bundle_type_registry::register_new(),
                        component_ids: leaked.as_slice(),
                    }
                })
            }

            fn cached_archetype_id(
                world: &mut ::boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster,
            ) -> ::boyko_ecs::ecs::identifiers::primitives::ArchetypeId {
                // Per-world cache helper (SBC4) — hot path is one Acquire load.
                world.bundle_archetype_id_for::<Self>()
            }

            fn for_each_component_bytes<F>(self, mut f: F)
            where
                F: ::std::ops::FnMut(
                    ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                    &[u8],
                ),
            {
                // B4: ManuallyDrop the whole value UPFRONT, before the callback
                // runs — a callback panic suppresses Drop (leak, never
                // double-drop with archetype-side ownership).
                let this = ::std::mem::ManuallyDrop::new(self);
                let id = <#name as ::boyko_ecs::ecs::core::component::component::Component>::component_id();
                let ptr = &raw const *this as *const u8;
                let len = ::std::mem::size_of::<#name>();
                // SAFETY (C5 byte-erasure, single-component arm):
                //   (i)   `ptr` derives from `&raw const *ManuallyDrop<Self>` over a
                //         live stack local — valid for `len = size_of::<Self>()` bytes
                //         for the duration of this call.
                //   (ii)  `len` is exactly `size_of::<Self>()` — no over-read; for a
                //         ZST this is a valid zero-length slice over a non-null,
                //         u8-aligned pointer.
                //   (iii) The materialized slice is shared/immutable and the only
                //         live borrow of `this`; on callback success ownership of the
                //         bytes transfers to the archetype, on panic the ManuallyDrop
                //         suppresses Drop.
                let bytes: &[u8] = unsafe { ::std::slice::from_raw_parts(ptr, len) };
                f(id, bytes);
            }
        }
    }
}

/// One parsed `#[require(...)]` entry (Feature 1). The required component `ty`
/// is constructed by one of three forms:
///
/// * `B`        → `B::default()` (requires `B: Default`);
/// * `C = expr` → the capture-free expression `expr` (the no-`Default` escape
///   hatch — must evaluate to a `C`);
/// * `D(args)`  → the call expression `D(args)` (sugar for `= D(args)`).
struct RequireEntry {
    /// The required component type (also the constructed value's type).
    ty: Path,
    /// The constructor expression. `None` means "use `<ty>::default()`".
    ctor: Option<Expr>,
}

/// All parsed `#[require(...)]` declarations on one component (Feature 1).
#[derive(Default)]
struct RequiresSpec {
    entries: Vec<RequireEntry>,
}

impl RequiresSpec {
    /// `true` iff at least one `#[require(...)]` key is present.
    fn any(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Emits the free `unsafe fn __require_ctor_N(dst: *mut u8)` constructor
    /// functions (D2) — one per `#[require]` entry, in declaration order. Each
    /// writes one fully-initialized value of the required type into `dst` via a
    /// capture-free expression; it never touches the world (F2-immune). Empty
    /// when no `#[require]` key is present (the 0%-gate).
    fn ctor_fns_codegen(&self, owner: &Ident) -> TokenStream2 {
        if !self.any() {
            return TokenStream2::new();
        }
        let fns: Vec<TokenStream2> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let fn_ident = format_ident!("__require_ctor_{}_{}", owner, i);
                let ty = &e.ty;
                let value: TokenStream2 = match &e.ctor {
                    Some(expr) => quote! { #expr },
                    None => quote! { <#ty as ::std::default::Default>::default() },
                };
                quote! {
                    /// Capture-free required-component constructor (Feature 1, D2).
                    ///
                    /// # Safety
                    /// `dst` must point at properly-aligned, writable, uninitialized
                    /// memory of at least `size_of` of the required type. Upheld by
                    /// the constructor pass (`SpawnAtCommand` / `migrate_entity_insert`),
                    /// which writes into a reserved, logically-uninit pool slot.
                    #[doc(hidden)]
                    // BUG-REQ-SNAKE-1: the fn name embeds the owner type's
                    // CamelCase identifier (`__require_ctor_<Owner>_<i>`), which
                    // trips `non_snake_case` under `clippy -D warnings` on every
                    // `#[require]` user. Standard derive hygiene: allow it here.
                    #[allow(non_snake_case)]
                    unsafe fn #fn_ident(dst: *mut u8) {
                        // SAFETY: the caller (the engine's constructor pass) guarantees
                        // `dst` is an aligned, uninit slot of the required type's layout
                        // and is exclusively owned for this call. `write` does not drop
                        // the (uninit) destination.
                        unsafe {
                            ::std::ptr::write(dst.cast::<#ty>(), { #value });
                        }
                    }
                }
            })
            .collect();
        quote! { #(#fns)* }
    }

    /// Emits `const HAS_REQUIRES = true;` + the `register_required` impl when any
    /// `#[require]` key is present, or an empty token stream otherwise. The
    /// `register_required` body pushes one `(component_id, ctor)` pair per entry
    /// into the builder, in declaration order (the W1 first-DFS precedence).
    fn codegen(&self, owner: &Ident) -> TokenStream2 {
        if !self.any() {
            return TokenStream2::new();
        }
        let pushes: Vec<TokenStream2> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let fn_ident = format_ident!("__require_ctor_{}_{}", owner, i);
                let ty = &e.ty;
                quote! {
                    // BUG-REQ-CYCLE-1: pass `component_id` as an UNCALLED fn item
                    // (no parentheses) so registering this edge does NOT resolve
                    // the required type's id inside the requiring type's own
                    // `component_id()` `OnceLock` init. The resolver is invoked
                    // lazily in `build_required_plan` at archetype-expansion time;
                    // a `#[require]` cycle then re-enters there (on the BUILDING
                    // stack) and panics instead of deadlocking.
                    builder.require(
                        <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id
                            as ::boyko_ecs::ecs::core::component::component_registry::RequiredIdFn,
                        #fn_ident as ::boyko_ecs::ecs::core::component::component_registry::RequiredCtor,
                    );
                }
            })
            .collect();
        quote! {
            const HAS_REQUIRES: bool = true;

            #[inline]
            fn register_required(
                builder: &mut ::boyko_ecs::ecs::core::component::component::RequiredBuilder,
            ) {
                #(#pushes)*
            }
        }
    }

    /// Emits the registration-time `install_required::<Self>(raw)` call into the
    /// derive's `component_id()` `OnceLock` init closure (after `register_new` +
    /// hooks + storage), or an empty token stream when no `#[require]` key is
    /// present (the 0%-gate — const-folds away exactly like `install_hooks`).
    fn install_codegen(&self) -> TokenStream2 {
        if !self.any() {
            return TokenStream2::new();
        }
        quote! {
            if Self::HAS_REQUIRES {
                boyko_ecs::ecs::core::component::component_registry::install_required::<Self>(raw);
            }
        }
    }
}

/// Parses every `#[require(...)]` attribute on the item (Feature 1). Each
/// attribute is a comma-separated list of entries; each entry is one of:
///
/// * `B`        — bare path, constructed via `B::default()`;
/// * `C = expr` — a NameValue: the capture-free expression `expr`;
/// * `D(args)`  — a call expression (sugar for `= D(args)`).
///
/// Multiple `#[require(...)]` attributes accumulate. Rejects:
/// * a duplicate same-id required component (two entries naming the SAME type
///   path) — a compile error (strictly better than Bevy's runtime panic);
/// * an empty `#[require()]` (no entries).
fn parse_requires(attrs: &[syn::Attribute]) -> Result<RequiresSpec, TokenStream> {
    let mut spec = RequiresSpec::default();

    for attr in attrs {
        if !attr.path().is_ident("require") {
            continue;
        }

        // Parse the comma-separated list of entry expressions. Each entry is an
        // `Expr`: a bare path (`B`), an assignment (`C = expr`), or a call
        // (`D(args)`). `parse_terminated` over `Expr` accepts all three.
        let parsed = attr.parse_args_with(
            syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated,
        );
        let list = match parsed {
            Ok(l) => l,
            Err(e) => return Err(e.to_compile_error().into()),
        };

        if list.is_empty() {
            return Err(syn::Error::new_spanned(
                attr,
                "empty #[require(...)]: list at least one required component, e.g. \
                 #[require(Velocity, Mass = Mass(1.0))]",
            )
            .to_compile_error()
            .into());
        }

        for expr in list {
            let entry = parse_require_entry(expr)?;
            // Reject a SYNTACTICALLY-IDENTICAL duplicate required component
            // (compile-time): `path_eq` compares the type path's token text, so it
            // catches `#[require(B, B)]` / `#[require(B)] #[require(B)]` where the
            // two paths are spelled the same. Differently-spelled paths to the SAME
            // type (`B` vs `crate::B` vs `self::B`) are NOT caught here — the macro
            // cannot resolve a path to a `ComponentId` — but they are harmless: the
            // registry's W1 dedup in `build_required_plan` collapses them to a single
            // entry by `ComponentId` downstream (last/direct ctor wins). This check
            // is therefore a best-effort early diagnostic, strictly better than
            // Bevy's runtime panic, never a soundness gate.
            if spec
                .entries
                .iter()
                .any(|prev| path_eq(&prev.ty, &entry.ty))
            {
                let ty = &entry.ty;
                return Err(syn::Error::new_spanned(
                    ty,
                    "duplicate #[require(...)] for the same component; each required \
                     component may be listed at most once",
                )
                .to_compile_error()
                .into());
            }
            spec.entries.push(entry);
        }
    }

    Ok(spec)
}

/// Lowers one `#[require(...)]` list element [`Expr`] into a [`RequireEntry`].
fn parse_require_entry(expr: Expr) -> Result<RequireEntry, TokenStream> {
    let err = |e: Expr, msg: &str| -> TokenStream {
        syn::Error::new_spanned(e, msg).to_compile_error().into()
    };
    match expr {
        // `B` — bare path ⇒ `B::default()`.
        Expr::Path(p) => Ok(RequireEntry {
            ty: p.path,
            ctor: None,
        }),
        // `C = expr` — explicit capture-free ctor expression.
        Expr::Assign(assign) => {
            let ty = match *assign.left {
                Expr::Path(p) => p.path,
                other => {
                    return Err(err(
                        other,
                        "#[require(C = expr)]: the left side must be a component type path",
                    ));
                }
            };
            Ok(RequireEntry {
                ty,
                ctor: Some(*assign.right),
            })
        }
        // `D(args)` — call expression ⇒ ctor is the whole call, type is the
        // callee path (sugar for `D = D(args)`).
        Expr::Call(ref call) => {
            let ty = match call.func.as_ref() {
                Expr::Path(p) => p.path.clone(),
                _ => {
                    return Err(err(
                        expr,
                        "#[require(D(...))]: the callee must be a component type path",
                    ));
                }
            };
            Ok(RequireEntry {
                ty,
                ctor: Some(expr),
            })
        }
        other => Err(err(
            other,
            "unsupported #[require(...)] entry; use `B`, `C = expr`, or `D(args)`",
        )),
    }
}

/// Conservative structural equality for two type paths (Feature 1 dup check):
/// compares the token-stream text of each path. Two entries with the same path
/// text resolve to the same `ComponentId`, so this catches the
/// `#[require(B, B)]` / `#[require(B)] #[require(B)]` duplicate at compile time.
fn path_eq(a: &Path, b: &Path) -> bool {
    quote!(#a).to_string() == quote!(#b).to_string()
}

/// Feature 3 (D3 gate): conservative SYNTACTIC scan for an `Entity` / `ChildOf`
/// field anywhere in the struct's field types. A `Copy` component carrying an
/// `Entity` reference is bitwise-copyable but a blind memcpy would NOT remap the
/// entity under deep clone, so the derive must classify it `CloneViaFn` (NOT
/// `TriviallyCopyable`) — this flag suppresses the `Copy` autoref arm. Conservative:
/// a false positive only costs a fn-call (never correctness); a path spelled
/// differently than `Entity` / `ChildOf` is NOT caught (documented v1 boundary —
/// the general per-field `#[entities]` remap is out of v1, D5).
fn struct_has_entity_field(input: &DeriveInput) -> bool {
    fn ty_mentions_entity(ty: &Type) -> bool {
        // Match the token text — catches `Entity`, `ChildOf`, `Option<Entity>`,
        // `[Entity; N]`, `Vec<Entity>`, `path::Entity`, etc. (substring on the
        // last path segment text). Cheap and conservative.
        let text = quote!(#ty).to_string();
        text.contains("Entity") || text.contains("ChildOf")
    }
    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        // Enums/unions: scan every field of every variant conservatively.
        Data::Enum(e) => {
            return e
                .variants
                .iter()
                .flat_map(|v| v.fields.iter())
                .any(|f| ty_mentions_entity(&f.ty));
        }
        Data::Union(u) => {
            return u.fields.named.iter().any(|f| ty_mentions_entity(&f.ty));
        }
    };
    match fields {
        Fields::Named(named) => named.named.iter().any(|f| ty_mentions_entity(&f.ty)),
        Fields::Unnamed(unnamed) => unnamed.unnamed.iter().any(|f| ty_mentions_entity(&f.ty)),
        Fields::Unit => false,
    }
}

/// Feature 3: emits the `CLONE_BEHAVIOR` const + `clone_behavior()` /
/// `clone_fn()` method overrides classifying the derived component for cloning.
///
/// * `#[component(no_clone)]` → `Ignore` / `None` (emit the trait defaults — return
///   an empty token stream so the const stays `Ignore` and `clone_fn` stays `None`).
/// * `#[component(clone = f)]` → `CloneViaFn` with the user free fn `f` (cast to the
///   `CloneFn` shape).
/// * else → AUTOREF specialization (`CloneProbe`): `TriviallyCopyable` for a
///   `Copy`-no-`Entity` type, `CloneViaFn` for a `Clone` (incl. `Copy`-with-`Entity`)
///   type, `Ignore` for a non-`Clone` type. `clone_behavior()` carries the runtime
///   result (a const cannot run autoref); the `CLONE_BEHAVIOR` const is left at the
///   trait default (`Ignore`) on this path — only `install_clone_fn` reads the
///   method, and the secondary `if const { ... }` const checks treat the derive
///   default as "no static guarantee", which is correct.
fn clone_codegen(name: &Ident, hooks: &ComponentHookPaths, has_entity_field: bool) -> TokenStream2 {
    // `no_clone`: keep the trait defaults (Ignore / None) — emit nothing.
    if hooks.no_clone {
        return TokenStream2::new();
    }

    // `clone = f`: a custom capture-free clone fn → CloneViaFn with `f`.
    if let Some(f) = &hooks.clone_with {
        return quote! {
            const CLONE_BEHAVIOR:
                ::boyko_ecs::ecs::core::component::component_registry::Cloneability =
                ::boyko_ecs::ecs::core::component::component_registry::Cloneability::CloneViaFn;

            #[inline]
            fn clone_behavior()
                -> ::boyko_ecs::ecs::core::component::component_registry::Cloneability {
                ::boyko_ecs::ecs::core::component::component_registry::Cloneability::CloneViaFn
            }

            #[inline]
            fn clone_fn()
                -> ::std::option::Option<
                    ::boyko_ecs::ecs::core::component::component_registry::CloneFn
                > {
                ::std::option::Option::Some(
                    #f as ::boyko_ecs::ecs::core::component::component_registry::CloneFn
                )
            }
        };
    }

    // Default path: autoref specialization. `TRIVIAL` is the syntactic "no Entity
    // field" flag — `false` suppresses the `Copy` arm so a `Copy`-with-`Entity` type
    // falls to `CloneViaFn` (D3). The probe is invoked through THREE refs
    // (`(&&&probe).method()`): with three leading refs the resolver reaches all three
    // `&self` arms and picks the highest-priority APPLICABLE one — the most-ref'd
    // `Self` wins (`&&CloneProbe` = Copy → `&CloneProbe` = Clone → `CloneProbe` =
    // Ignore), i.e. Copy → Clone → neither. The ref count MUST stay `&&&` to agree
    // with the arm receiver depths in `component_registry` (a `&&` call misclassifies
    // every `Copy` type as `CloneViaFn` — the C1 bug).
    let trivial = if has_entity_field {
        quote! { false }
    } else {
        quote! { true }
    };
    quote! {
        #[inline]
        fn clone_behavior()
            -> ::boyko_ecs::ecs::core::component::component_registry::Cloneability {
            // Bring the autoref-arm traits into scope so method resolution can pick
            // the `&` (Clone) / `&&` (Copy) arms over the by-value (Ignore) inherent.
            use ::boyko_ecs::ecs::core::component::component_registry::{
                CloneIgnoreArm as _, CloneViaFnArm as _, TriviallyCopyableArm as _,
            };
            let probe = ::boyko_ecs::ecs::core::component::component_registry::CloneProbe::<
                #name, #trivial,
            >::new();
            (&&&probe).clone_behavior()
        }

        #[inline]
        fn clone_fn()
            -> ::std::option::Option<
                ::boyko_ecs::ecs::core::component::component_registry::CloneFn
            > {
            use ::boyko_ecs::ecs::core::component::component_registry::{
                CloneIgnoreArm as _, CloneViaFnArm as _, TriviallyCopyableArm as _,
            };
            let probe = ::boyko_ecs::ecs::core::component::component_registry::CloneProbe::<
                #name, #trivial,
            >::new();
            (&&&probe).clone_fn_ptr()
        }
    }
}

/// Derive macro for implementing the Resource trait.
///
/// Generates the lazy `resource_id()` accessor backed by a per-type
/// `OnceLock`; `debug_type_name`, `type_id`, `mem_size`, and `alignment`
/// are inherited from the `Resource` trait's default methods.
///
/// Resource IDs are assigned lazily at runtime via the global resource
/// registry — see `boyko_ecs::ecs::core::resources::resource_registry`.
/// The registry guarantees the same Rust type cannot be registered both
/// as a `Component` and as a `Resource` (audit M6).
///
/// # Example
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// #[derive(Resource)]
/// struct GameTick(u32);
/// ```
///
/// The example is `ignore`'d because proc-macro crates cannot consume their
/// own macros, and `boyko-macros` cannot depend on `boyko-ecs` for tests
/// (that would create a cycle). Real usage lives in `boyko-ecs` integration
/// tests.
#[proc_macro_derive(Resource)]
pub fn resource_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::resources::resource::Resource for #name {
            #[inline]
            fn resource_id() -> ::boyko_ecs::ecs::identifiers::primitives::ResourceId {
                static ID: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::identifiers::primitives::ResourceId
                > = ::std::sync::OnceLock::new();
                // Route through the `resources` module's `pub use` of
                // `register_new` so the macro path stays inside the public
                // surface — the `resource_registry` module itself is
                // `pub(crate)` per Q6 RESOLUTION.
                *ID.get_or_init(|| ::boyko_ecs::ecs::identifiers::primitives::ResourceId(
                    ::boyko_ecs::ecs::core::resources::register_new::<Self>()
                ))
            }
        }
    };

    expanded.into()
}

/// Attribute macro for defining an event type.
///
/// Rewrites the user struct into a two-field native layout:
/// `{ participants: <Name>Participants, parameters: <Name>Parameters }`.
/// This eliminates the unsound pointer cast that `#[derive(Event)]` generated;
/// all accessors become safe typed-field reads.
///
/// Fields must be annotated with either `#[participant(components = "TypeA, TypeB")]`
/// or `#[parameter]`. Every field must carry exactly one of these markers.
///
/// Constraints:
/// - Generic structs are not supported (Q-001 scope).
/// - Only named-field structs are supported.
///
/// Event IDs are assigned lazily at runtime via a per-type `OnceLock` and the
/// global registry — mirror of the Component ID model.
///
/// # Limitations
///
/// User-supplied `#[derive(...)]` attributes above `#[event]` apply only to
/// the rewritten outer struct. The generated `<Name>Participants` and
/// `<Name>Parameters` substructs always derive `Clone, Copy` and nothing else.
/// Trait bounds that recurse through fields (such as `Debug`, `PartialEq`,
/// `Hash`) therefore fail to compile if used as a derive on the outer struct.
/// If you need such traits on an event type, implement them by hand on both
/// substructs (or on the outer struct alone if it does not recurse).
///
/// `#[allow(...)]`, `#[doc = "..."]`, and other non-derive outer attributes
/// are forwarded to the outer struct only — same as `#[derive]`.
///
/// # Example
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// #[event]
/// struct DamageEvent {
///     #[participant(components = "Position, Health")]
///     victim: Entity,
///     #[parameter]
///     amount: f32,
/// }
/// ```
///
/// The example is `ignore`'d for the same reason as `#[derive(Component)]`:
/// proc-macro crates cannot pull in their own consumers. End-to-end tests live
/// in `boyko-ecs/tests/event_attribute.rs`.
#[proc_macro_attribute]
pub fn event(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input2: proc_macro2::TokenStream = input.clone().into();
    let item_struct = match syn::parse::<ItemStruct>(input) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    if let Err(ts) = validate_event_struct(&item_struct) {
        return ts;
    }

    match generate_event_impl(item_struct, input2) {
        Ok(ts) => ts,
        Err(ts) => ts,
    }
}

/// Validates top-level constraints: no generics, named fields only.
fn validate_event_struct(s: &ItemStruct) -> Result<(), TokenStream> {
    if !s.generics.params.is_empty() {
        return Err(
            syn::Error::new(
                s.ident.span(),
                "#[event] does not support generic structs (Q-001 scope)",
            )
            .to_compile_error()
            .into(),
        );
    }
    match &s.fields {
        Fields::Named(_) => Ok(()),
        _ => Err(
            syn::Error::new(
                s.ident.span(),
                "#[event] requires a struct with named fields",
            )
            .to_compile_error()
            .into(),
        ),
    }
}

/// Holds everything extracted from a single annotated field.
struct ParticipantField<'a> {
    ident: &'a Ident,
    ty: &'a syn::Type,
    /// Visibility as written by the user (e.g. `pub`, `pub(crate)`, or inherited).
    vis: syn::Visibility,
    /// Comma-separated component type names from `components = "..."`.
    component_names: Vec<String>,
    /// Remaining non-marker attributes (doc comments, `#[allow(...)]`, etc.).
    other_attrs: Vec<&'a syn::Attribute>,
}

struct ParameterField<'a> {
    ident: &'a Ident,
    ty: &'a syn::Type,
    /// Visibility as written by the user (e.g. `pub`, `pub(crate)`, or inherited).
    vis: syn::Visibility,
    other_attrs: Vec<&'a syn::Attribute>,
}

/// Parses field annotations and generates the full token stream.
fn generate_event_impl(
    s: ItemStruct,
    _input2: proc_macro2::TokenStream,
) -> Result<TokenStream, TokenStream> {
    let name = &s.ident;
    let vis = &s.vis;
    let outer_attrs = &s.attrs;

    let named = match &s.fields {
        Fields::Named(f) => f,
        _ => unreachable!("validated above"),
    };

    let mut participant_fields: Vec<ParticipantField<'_>> = Vec::new();
    let mut parameter_fields: Vec<ParameterField<'_>> = Vec::new();

    for field in &named.named {
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let field_vis = field.vis.clone();

        let mut is_participant = false;
        let mut is_parameter = false;
        let mut component_names: Vec<String> = Vec::new();
        let mut other_attrs: Vec<&syn::Attribute> = Vec::new();

        for attr in &field.attrs {
            if attr.path().is_ident("participant") {
                if is_participant {
                    return Err(
                        syn::Error::new(
                            attr.path().get_ident().map_or(Span::call_site(), |i| i.span()),
                            "field has duplicate #[participant] markers",
                        )
                        .to_compile_error()
                        .into(),
                    );
                }
                is_participant = true;

                // N3: parse `components = "TypeA, TypeB"` via syn::parse_nested_meta
                // so that typos and unknown keys surface as a proper compile_error!.
                let mut parse_error: Option<TokenStream> = None;
                let result = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("components") {
                        let value = meta.value()?; // consumes the `=`
                        let lit: syn::LitStr = value.parse()?;
                        for comp in lit.value().split(',') {
                            let comp = comp.trim();
                            if !comp.is_empty() {
                                component_names.push(comp.to_string());
                            }
                        }
                        Ok(())
                    } else {
                        Err(meta.error(
                            "unknown #[participant(...)] argument; \
                             expected `components = \"TypeA, TypeB\"`",
                        ))
                    }
                });
                if let Err(e) = result {
                    parse_error = Some(e.to_compile_error().into());
                }
                if let Some(ts) = parse_error {
                    return Err(ts);
                }
            } else if attr.path().is_ident("parameter") {
                is_parameter = true;
            } else {
                other_attrs.push(attr);
            }
        }

        match (is_participant, is_parameter) {
            (true, true) => {
                return Err(
                    syn::Error::new(
                        field_ident.span(),
                        format!(
                            "field `{}` has both #[participant] and #[parameter] markers; \
                             use exactly one",
                            field_ident
                        ),
                    )
                    .to_compile_error()
                    .into(),
                );
            }
            (false, false) => {
                return Err(
                    syn::Error::new(
                        field_ident.span(),
                        format!(
                            "field `{}` has no #[participant] or #[parameter] marker; \
                             every field must have exactly one",
                            field_ident
                        ),
                    )
                    .to_compile_error()
                    .into(),
                );
            }
            (true, false) => {
                participant_fields.push(ParticipantField {
                    ident: field_ident,
                    ty: field_ty,
                    vis: field_vis,
                    component_names,
                    other_attrs,
                });
            }
            (false, true) => {
                parameter_fields.push(ParameterField {
                    ident: field_ident,
                    ty: field_ty,
                    vis: field_vis,
                    other_attrs,
                });
            }
        }
    }

    let participants_name = format_ident!("{}Participants", name);
    let parameters_name = format_ident!("{}Parameters", name);

    // Build the Participants substruct field tokens (with non-marker attrs preserved).
    // N1: emit the user's original visibility instead of unconditionally widening to `pub`.
    let participants_struct_fields: Vec<proc_macro2::TokenStream> = participant_fields
        .iter()
        .map(|f| {
            let attrs = &f.other_attrs;
            let ident = f.ident;
            let ty = f.ty;
            let vis = &f.vis;
            quote! { #(#attrs)* #vis #ident: #ty }
        })
        .collect();

    // Build the Parameters substruct field tokens.
    // N1: same visibility preservation.
    let parameters_struct_fields: Vec<proc_macro2::TokenStream> = parameter_fields
        .iter()
        .map(|f| {
            let attrs = &f.other_attrs;
            let ident = f.ident;
            let ty = f.ty;
            let vis = &f.vis;
            quote! { #(#attrs)* #vis #ident: #ty }
        })
        .collect();

    // Build `Event::new` body: construct participants substruct, then parameters substruct,
    // then the outer struct.
    let p_field_idents: Vec<&Ident> = participant_fields.iter().map(|f| f.ident).collect();
    let q_field_idents: Vec<&Ident> = parameter_fields.iter().map(|f| f.ident).collect();

    // Build participant_info() implementation.
    let participant_count = participant_fields.len();
    let participants_impl = build_participants_impl(
        &participants_name,
        &participant_fields,
        participant_count,
    );

    let event_name_str = name.to_string();

    let expanded = quote! {
        // Outer event struct: two native fields, no unsafe casts anywhere.
        #(#outer_attrs)*
        #[repr(C)]
        #vis struct #name {
            pub participants: #participants_name,
            pub parameters: #parameters_name,
        }

        // Participants substruct.
        #[repr(C)]
        #[derive(Clone, Copy)]
        #vis struct #participants_name {
            #(#participants_struct_fields),*
        }

        #participants_impl

        // Parameters substruct.
        #[repr(C)]
        #[derive(Clone, Copy)]
        #vis struct #parameters_name {
            #(#parameters_struct_fields),*
        }

        impl ::boyko_ecs::ecs::core::events::parameters::parameters::Parameters
            for #parameters_name {}

        // Inherent const.
        impl #name {
            pub const EVENT_NAME: &'static str = #event_name_str;
        }

        // Event trait impl.
        impl ::boyko_ecs::ecs::core::events::event::Event for #name {
            type Participants = #participants_name;
            type Parameters = #parameters_name;

            #[inline]
            fn event_id() -> ::boyko_ecs::ecs::core::events::event::EventId {
                static ID: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::core::events::event::EventId
                > = ::std::sync::OnceLock::new();
                *ID.get_or_init(||
                    ::boyko_ecs::ecs::core::events::event_registry::register_event_new::<Self>()
                )
            }

            #[inline]
            fn event_name() -> &'static str {
                Self::EVENT_NAME
            }

            #[inline]
            fn new(
                participants: Self::Participants,
                parameters: Self::Parameters,
            ) -> Self {
                Self { participants, parameters }
            }

            #[inline]
            fn participants(&self) -> &Self::Participants {
                &self.participants
            }

            #[inline]
            fn participants_mut(&mut self) -> &mut Self::Participants {
                &mut self.participants
            }

            #[inline]
            fn parameters(&self) -> &Self::Parameters {
                &self.parameters
            }

            #[inline]
            fn parameters_mut(&mut self) -> &mut Self::Parameters {
                &mut self.parameters
            }
        }

        // Silence unused-field warnings for the field-name identifiers collected
        // during parsing. The idents are validated and appear in the substruct
        // fields above; this block is a no-op at runtime.
        const _: () = {
            #[allow(dead_code)]
            fn _assert_field_names_compile() {
                // These references exist only to silence unused-ident lints that can
                // surface in edge cases from the generated substruct constructors.
                let _ = |p: #participants_name, q: #parameters_name| {
                    let _ = (#(p.#p_field_idents,)*);
                    let _ = (#(q.#q_field_idents,)*);
                };
            }
        };
    };

    Ok(expanded.into())
}

/// Builds the `Participants` trait impl for the participants substruct.
fn build_participants_impl(
    participants_name: &Ident,
    fields: &[ParticipantField<'_>],
    participant_count: usize,
) -> proc_macro2::TokenStream {
    if participant_count == 0 {
        // Empty: no Box::leak needed — return the empty slice literal directly.
        return quote! {
            impl ::boyko_ecs::ecs::core::events::participants::participants::Participants
                for #participants_name
            {
                fn participant_count() -> usize { 0 }
                fn participant_info()
                    -> &'static [::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo]
                {
                    &[]
                }
            }
        };
    }

    // Build the vec of ParticipantInfo initializers, each with an inner Box::leak
    // for the required_components slice.
    let participant_infos: Vec<proc_macro2::TokenStream> = fields
        .iter()
        .map(|f| {
            let name_str = f.ident.to_string();
            let comp_idents: Vec<Ident> = f
                .component_names
                .iter()
                .map(|c| Ident::new(c, Span::call_site()))
                .collect();

            let required_components = if comp_idents.is_empty() {
                // No components listed — cheaper to reference an empty static slice.
                quote! {
                    ::std::boxed::Box::leak(
                        ::std::vec![].into_boxed_slice()
                    )
                }
            } else {
                quote! {
                    ::std::boxed::Box::leak(::std::vec![
                        #(
                            <#comp_idents as ::boyko_ecs::ecs::core::component::component::Component>::component_id()
                        ),*
                    ].into_boxed_slice())
                }
            };

            quote! {
                ::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo {
                    name: #name_str,
                    required_components: #required_components,
                }
            }
        })
        .collect();

    quote! {
        impl ::boyko_ecs::ecs::core::events::participants::participants::Participants
            for #participants_name
        {
            fn participant_count() -> usize { #participant_count }

            fn participant_info()
                -> &'static [::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo]
            {
                static INFO: ::std::sync::OnceLock<
                    &'static [::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo]
                > = ::std::sync::OnceLock::new();
                *INFO.get_or_init(|| {
                    ::std::boxed::Box::leak(::std::vec![
                        #(#participant_infos),*
                    ].into_boxed_slice())
                })
            }
        }
    }
}

/// Derive macro for the sealed [`Bundle`] trait — Phase 8.5 Step 4.
///
/// Generates a non-generic `impl Bundle for #Name` whose hot path is dominated
/// by a single `OnceLock::get` Acquire load on a per-impl
/// `static INFO: OnceLock<BundleStaticInfo>` (the O3 coalesced static — see
/// plan §4.4 / §6.1 / Decision SBC-D5).
///
/// # Supported inputs
///
/// * `struct Foo { a: A, b: B }` — named-field struct.
/// * `struct Foo(A, B)` — tuple struct.
///
/// # Rejected inputs
///
/// * Unit struct (`struct Foo;`) — `compile_error!` pointing at
///   `Commands::spawn_empty()` for zero-component spawns (Phase 22 D5/D7).
/// * Generic struct (`struct Foo<T> { ... }`) — `compile_error!("Bundle derive does not support generics (Phase 8.5 scope)")`.
/// * Enum / union — `compile_error!("Bundle can only be derived for structs")`.
/// * More than [`MAX_BUNDLE_ARITY`] (16) fields — the runtime apply paths use
///   fixed-size stack collectors sized to this ceiling (Phase 22: 8 → 16).
///
/// # Generated impl summary (named-struct example)
///
/// ```ignore
/// #[derive(Bundle)]
/// struct PlayerBundle { pos: Position, vel: Velocity }
/// ```
///
/// expands (sketch) to:
///
/// ```ignore
/// impl ::boyko_ecs::...::sealed::BundleSealed for PlayerBundle {}
/// impl ::boyko_ecs::...::Bundle for PlayerBundle
/// where Position: Component, Velocity: Component {
///     fn static_info() -> &'static BundleStaticInfo { /* OnceLock::get_or_init */ }
///     fn cached_archetype_id(world) -> ArchetypeId { world.bundle_archetype_id_for::<Self>() }
///     fn for_each_component_bytes<F>(self, mut f: F) where F: FnMut(...) {
///         // ManuallyDrop UPFRONT (B4); then build [(id, *const u8, len); N];
///         // sort by ComponentId.0 (B1); iterate, reconstruct &[u8] inside loop (C5).
///     }
/// }
/// ```
///
/// See plan §6.3 (mandatory `for_each_component_bytes` codegen template — C5
/// pointer-based pattern + four-clause SAFETY block) for the exact byte
/// pattern emitted.
///
/// # Example
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// #[derive(Bundle)]
/// struct ProjectileBundle {
///     pos: Position,
///     vel: Velocity,
/// }
/// ```
///
/// The example is `ignore`'d because proc-macro crates cannot consume their own
/// macros, and `boyko-macros` cannot depend on `boyko-ecs` for tests (that
/// would create a cycle). Real usage lives in `boyko-ecs` integration tests.
#[proc_macro_derive(Bundle)]
pub fn bundle_macro(input: TokenStream) -> TokenStream {
    /// Maximum component count for a derived `Bundle` (Phase 22: 8 → 16).
    ///
    /// Kept in lock-step with the `MAX_BUNDLE_ARITY` stack-collector ceilings
    /// in `boyko_ecs` (`spawn_at_command.rs` / `insert_command.rs` /
    /// `migration_helpers.rs`): rejecting wider bundles at macro time makes
    /// the runtime debug_asserts unreachable for derived bundles.
    const MAX_BUNDLE_ARITY: usize = 16;

    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_span = name.span();

    // SBC1 / Phase 8.5 scope: reject generics outright. The per-impl
    // `static INFO: OnceLock<BundleStaticInfo>` works only when the impl is
    // non-generic — otherwise monomorphization would create one static per
    // (B, T1, ..., Tn) tuple, defeating the cache and breaking SBC2.
    if !input.generics.params.is_empty() {
        return syn::Error::new(
            name_span,
            "Bundle derive does not support generics (Phase 8.5 scope)",
        )
        .to_compile_error()
        .into();
    }

    let data = match &input.data {
        Data::Struct(s) => s,
        Data::Enum(_) | Data::Union(_) => {
            return syn::Error::new(
                name_span,
                "Bundle can only be derived for structs",
            )
            .to_compile_error()
            .into();
        }
    };

    let fields: Vec<BundleField> = match &data.fields {
        Fields::Named(named) => named
            .named
            .iter()
            .enumerate()
            .map(|(idx, f)| BundleField {
                local_ident: format_ident!("__bundle_field_{}", idx),
                accessor: {
                    let ident = f.ident.clone().expect("named field");
                    quote! { self.#ident }
                },
                ty: f.ty.clone(),
            })
            .collect(),
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(idx, f)| {
                let idx_lit = syn::Index::from(idx);
                BundleField {
                    local_ident: format_ident!("__bundle_field_{}", idx),
                    accessor: quote! { self.#idx_lit },
                    ty: f.ty.clone(),
                }
            })
            .collect(),
        Fields::Unit => {
            return syn::Error::new(
                name_span,
                "Bundle requires at least one field; \
                 to spawn an entity with zero components use Commands::spawn_empty()",
            )
            .to_compile_error()
            .into();
        }
    };

    if fields.is_empty() {
        // Defensive: tuple struct `Foo()` and named struct `Foo {}` both
        // arrive here with zero fields. Treat identically to unit struct.
        return syn::Error::new(
            name_span,
            "Bundle requires at least one field; \
             to spawn an entity with zero components use Commands::spawn_empty()",
        )
        .to_compile_error()
        .into();
    }

    // Phase 22: hard arity ceiling, mirrored by the runtime stack collectors.
    if fields.len() > MAX_BUNDLE_ARITY {
        return syn::Error::new(
            name_span,
            format!(
                "Bundle supports at most {MAX_BUNDLE_ARITY} components (MAX_BUNDLE_ARITY); \
                 split the bundle and insert the remainder with EntityCommands::insert"
            ),
        )
        .to_compile_error()
        .into();
    }

    let n_fields = fields.len();

    // Per-field token fragments, indexed in declaration order.
    let field_types: Vec<&Type> = fields.iter().map(|f| &f.ty).collect();
    let field_locals: Vec<&Ident> = fields.iter().map(|f| &f.local_ident).collect();
    let field_accessors: Vec<&TokenStream2> = fields.iter().map(|f| &f.accessor).collect();

    // §6.1 build_info: each field's `T::component_id()`.
    let component_id_exprs: Vec<TokenStream2> = field_types
        .iter()
        .map(|ty| {
            quote! {
                <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id()
            }
        })
        .collect();

    // §6.3 sort-array entries: (ComponentId, *const u8, usize) triples derived
    // from the ManuallyDrop locals. C5: pointer + length (not &[u8]) sidesteps
    // E0521 (MaybeUninit/array lifetime invariance) — we materialize the slice
    // inside the dispatch loop via slice::from_raw_parts.
    let sort_entries: Vec<TokenStream2> = fields
        .iter()
        .map(|f| {
            let ty = &f.ty;
            let local = &f.local_ident;
            quote! {
                (
                    <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id(),
                    &raw const *#local as *const u8,
                    ::std::mem::size_of::<#ty>(),
                )
            }
        })
        .collect();

    // Phase 22.1 D-E: per-field push fragments for the `data`-only walk. Each
    // push is wrapped in `if size_of::<FieldTy>() != 0 { ... }`. Because the
    // size is a monomorphisation-time constant, the branch folds entirely:
    // a ZST field's entry never enters the array, so the subsequent sort and
    // dispatch loop run over data columns only — the ZST byte-copy is elided
    // BEFORE the runtime sort (unlike a post-sort `bytes.is_empty()` guard,
    // which would launder into a per-column-per-row runtime branch).
    let data_push_stmts: Vec<TokenStream2> = fields
        .iter()
        .map(|f| {
            let ty = &f.ty;
            let local = &f.local_ident;
            quote! {
                if ::std::mem::size_of::<#ty>() != 0 {
                    // SAFETY (C5 / §6.3, identical to `for_each_component_bytes`):
                    //   `__data_len < #n_fields` because at most `#n_fields`
                    //   entries are ever pushed (one per field, and only when
                    //   non-ZST). `&raw const *#local` is a valid `*const u8`
                    //   for `size_of::<#ty>()` bytes for this function's scope.
                    unsafe {
                        *__data_sorted.get_unchecked_mut(__data_len) = (
                            <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id(),
                            &raw const *#local as *const u8,
                            ::std::mem::size_of::<#ty>(),
                        );
                    }
                    __data_len += 1;
                }
            }
        })
        .collect();

    // `where T: Component` for every field — gives a sharper diagnostic than
    // letting the `component_id()` reference fail down in the impl body. Per
    // step spec acceptance §9 Step 4 bullet "Bound check".
    let component_bounds: Vec<TokenStream2> = field_types
        .iter()
        .map(|ty| {
            quote! {
                #ty: ::boyko_ecs::ecs::core::component::component::Component
            }
        })
        .collect();

    // Decision 4 (D4b): the typed-write threshold. Mirrors
    // `boyko_ecs::ecs::core::bundle::MAX_TYPED_WRITE_ARITY` (the macro cannot
    // import boyko_ecs; the cross-check `const _: () = assert!(... == 16)` in
    // bundle.rs pins this literal in lock-step).
    const MAX_TYPED_WRITE_ARITY: usize = 16;
    let has_typed_write = n_fields <= MAX_TYPED_WRITE_ARITY;

    // Decision 4 (W3): per-field perm-build statements for `write_row_perm`.
    // For each DECLARATION field `k`:
    //   - ZST field (`size_of::<Tk>() == 0`) → `out_perm[k] = PERM_SKIP`
    //     (const-folds: the branch picks the SKIP arm at monomorphisation);
    //   - otherwise find `Tk::component_id()`'s position in the canonical
    //     `data_component_ids` slice (linear scan; arity is ≤ 16) and store it
    //     as the canonical data-column slot. Correct-by-construction IDENTITY:
    //     the slot is keyed on the field's ComponentId, never on size.
    let perm_build_stmts: Vec<TokenStream2> = fields
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let ty = &f.ty;
            quote! {
                if ::std::mem::size_of::<#ty>() == 0 {
                    out_perm[#k] =
                        ::boyko_ecs::ecs::core::bundle::BundleColumnPtrs::PERM_SKIP;
                } else {
                    let __cid = <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id();
                    let mut __slot = usize::MAX;
                    let mut __i = 0usize;
                    while __i < data_component_ids.len() {
                        if data_component_ids[__i] == __cid {
                            __slot = __i;
                            break;
                        }
                        __i += 1;
                    }
                    debug_assert!(
                        __slot != usize::MAX,
                        "write_row_perm: field {} ComponentId not found in canonical data_component_ids", #k
                    );
                    debug_assert!(
                        __slot < (::boyko_ecs::ecs::core::bundle::BundleColumnPtrs::PERM_SKIP as usize),
                        "write_row_perm: data-column slot exceeds PERM_SKIP sentinel"
                    );
                    out_perm[#k] = __slot as u8;
                }
            }
        })
        .collect();

    // Decision 4: per-field typed-store statements for `write_row_typed`,
    // const-unrolled in DECLARATION order. Each field `k`:
    //   - skips at compile time if `size_of::<Tk>() == 0` (ZST const-folds out
    //     — the whole `if` block disappears, matching the byte path's ZST
    //     elision and the per-batch `data_pool_ids` filter);
    //   - otherwise relocates field `k` via `ManuallyDrop::take` (suppressing
    //     the source Drop — B4/O1) into its destination column with a
    //     FIXED-WIDTH `ptr::write::<Tk>` store at `base.add(row * STRIDE_k)`.
    // The destination column slot is `dst.perm(k)` (declaration field → data
    // column slot, built once per batch by the caller). W3 IDENTITY assert +
    // Q1 row-bound assert are debug-only.
    let typed_write_stmts: Vec<TokenStream2> = fields
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let ty = &f.ty;
            let local = &f.local_ident;
            quote! {
                if ::std::mem::size_of::<#ty>() != 0 {
                    // Declaration field `k` → its canonical data-column slot.
                    let __slot = dst.perm(#k) as usize;
                    debug_assert!(
                        __slot != (::boyko_ecs::ecs::core::bundle::BundleColumnPtrs::PERM_SKIP as usize),
                        "write_row_typed: non-ZST field {} mapped to PERM_SKIP", #k
                    );
                    // W3 IDENTITY: the resolved column's ComponentId must equal
                    // this field's ComponentId — two same-size, different-type
                    // components must NEVER silently swap columns.
                    #[cfg(debug_assertions)]
                    debug_assert_eq!(
                        dst.column_comp_id(__slot),
                        <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id(),
                        "write_row_typed: column identity mismatch for field {}", #k
                    );
                    // Stride parity: the resolved column's stride must equal the
                    // field's size (the registry layout reflects `size_of::<Tk>()`).
                    #[cfg(debug_assertions)]
                    debug_assert_eq!(
                        dst.column_stride(__slot),
                        ::std::mem::size_of::<#ty>(),
                        "write_row_typed: column stride mismatch for field {}", #k
                    );
                    // Q1: restore the `idx < committed_rows` guard the byte path
                    // has at write_at_unchecked_initialized (the typed path
                    // bypasses it).
                    #[cfg(debug_assertions)]
                    debug_assert!(
                        row < dst.column_committed_rows(__slot),
                        "write_row_typed: row {} >= committed_rows for field {}", row, #k
                    );
                    // SAFETY (D4 §"Unsafe delta"): the caller's `write_row_typed`
                    //   contract (1)-(4) guarantees:
                    //   - `dst.column_base(__slot)` carries write-capable,
                    //     unaliased provenance for this column resolved once per
                    //     batch under a single &mut that has ended (W2);
                    //   - `row < committed_rows` (debug-asserted above; the
                    //     caller pre-grew via reserve_capacity);
                    //   - `base.add(row * size_of::<Tk>())` is in-bounds of the
                    //     column data sub-region and `Tk`-aligned (column-base
                    //     alignment contract);
                    //   - the slot is uninit (commit happens post-loop, B4), so
                    //     no Drop runs on the destination;
                    //   - `ManuallyDrop::take` performs a bitwise relocation that
                    //     suppresses the source Drop and cannot panic (no user
                    //     code), so this body is panic-free — the only batch
                    //     panic source is `iter.next()` between rows.
                    unsafe {
                        let __dst_ptr = dst
                            .column_base(__slot)
                            .add(row * ::std::mem::size_of::<#ty>())
                            as *mut #ty;
                        ::std::ptr::write(
                            __dst_ptr,
                            ::std::mem::ManuallyDrop::take(&mut #local),
                        );
                    }
                }
            }
        })
        .collect();

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::bundle::bundle::sealed::BundleSealed for #name {}

        impl ::boyko_ecs::ecs::core::bundle::bundle::Bundle for #name
        where
            #(#component_bounds),*
        {
            fn static_info() -> &'static ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo {
                // O3 coalesced static (Decision SBC-D5). One OnceLock holds
                // BundleTypeId + canonical-sorted component_ids slice. Cached
                // path: single Acquire load.
                static INFO: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo
                > = ::std::sync::OnceLock::new();

                INFO.get_or_init(|| {
                    // B1 canonical order: collect declaration-order IDs into a
                    // fixed-size stack array, sort ascending by ComponentId.0,
                    // then leak the boxed array to obtain a `&'static` slice.
                    // Leak is bounded by SBC8 (one slice per Bundle type per
                    // process — at most MAX_BUNDLE_TYPES × N_max × 8 B).
                    let mut arr: [
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId;
                        #n_fields
                    ] = [#(#component_id_exprs),*];
                    arr.sort_unstable_by_key(|id| id.0);
                    let leaked: &'static [
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId;
                        #n_fields
                    ] = ::std::boxed::Box::leak(::std::boxed::Box::new(arr));

                    ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo {
                        // BundleTypeId minted exactly once per Bundle type per
                        // process — OnceLock::get_or_init enforces single
                        // winner across threads (§7.3).
                        type_id: ::boyko_ecs::ecs::core::bundle::bundle_type_registry::register_new(),
                        component_ids: leaked.as_slice(),
                    }
                })
            }

            fn cached_archetype_id(
                world: &mut ::boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster,
            ) -> ::boyko_ecs::ecs::identifiers::primitives::ArchetypeId {
                // Delegate to the per-world cache helper. The helper performs
                // the hot-path Acquire load on `bundle_archetype_cache[id.0]`
                // and falls back to a cold ArchetypeMaster registration on
                // the first call per (Bundle, world) pair (§6.2).
                world.bundle_archetype_id_for::<Self>()
            }

            fn for_each_component_bytes<F>(self, mut f: F)
            where
                F: ::std::ops::FnMut(
                    ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                    &[u8],
                ),
            {
                // §6.3 MANDATORY codegen template — C5 pointer-based pattern.
                //
                // Step 1: ManuallyDrop-wrap EVERY destructured field UPFRONT,
                // before any callback can run. This is the B4 panic-safety
                // contract: on callback panic mid-iteration, the remaining
                // fields' `Drop` impls are suppressed unconditionally (they
                // leak — never double-drop alongside archetype-side ownership).
                #(
                    let #field_locals = ::std::mem::ManuallyDrop::new(#field_accessors);
                )*

                // Step 2: build the sort array as (ComponentId, *const u8,
                // usize). The *const u8 + len triple sidesteps E0521 — the
                // borrow checker treats `&[u8]` as lifetime-invariant inside
                // array/MaybeUninit contexts, but raw pointers are fine. The
                // slice is reconstructed inside the dispatch loop.
                let mut sorted: [
                    (
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                        *const u8,
                        usize,
                    );
                    #n_fields
                ] = [#(#sort_entries),*];

                // Step 3: B1 canonical sort. unstable acceptable because
                // ComponentId values are unique per Bundle (a Bundle that
                // declares the same Component twice fails at archetype
                // registration, not here).
                sorted.sort_unstable_by_key(|(id, _, _)| id.0);

                // Step 4: dispatch in canonical order, materializing the
                // shared byte slice on each iteration.
                for &(id, ptr, len) in &sorted {
                    // SAFETY (C5 / §6.3):
                    //   (i)   `ptr` was derived from `&raw const *ManuallyDrop<T>`,
                    //         where T is a live stack local in this function — ptr
                    //         is valid for `len = size_of::<T>()` bytes for the
                    //         duration of this loop.
                    //   (ii)  `len` is exactly `size_of::<T>()` matching the
                    //         component type — no over-read.
                    //   (iii) The slice we materialize is shared (immutable) and
                    //         non-overlapping with any other live borrow: each
                    //         ManuallyDrop local is borrowed exactly once in this
                    //         scope (via the iter slot above).
                    //   (iv)  ManuallyDrop suppresses Drop on the local
                    //         unconditionally at end-of-scope (does not "leak"
                    //         semantically — never invokes Drop). For components
                    //         that the callback successfully consumed (memcpy'd
                    //         into ECS storage via create_entity), ownership has
                    //         transferred to the archetype, and that storage now
                    //         owns the eventual Drop on entity despawn. For
                    //         components that the callback did not reach because
                    //         `f` panicked on an earlier iteration, their bytes
                    //         remain in the stack ManuallyDrop locals and leak
                    //         unconditionally — Drop is suppressed regardless of
                    //         panic state. This is the documented B4 panic-safety
                    //         guarantee: panic → leak, never double-drop.
                    let bytes: &[u8] = unsafe { ::std::slice::from_raw_parts(ptr, len) };
                    f(id, bytes);
                }
            }

            fn for_each_data_component_bytes<F>(self, mut f: F)
            where
                F: ::std::ops::FnMut(
                    ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                    &[u8],
                ),
            {
                // Phase 22.1 D-E: identical to `for_each_component_bytes`
                // EXCEPT zero-size (ZST tag) fields are filtered out at
                // monomorphisation (the `size_of::<FieldTy>() != 0` guards
                // below const-fold). The callback is invoked once per
                // NON-ZST component in canonical `ComponentId.0` order.
                //
                // B4 panic-safety is unchanged: EVERY field (ZST or not) is
                // ManuallyDrop-wrapped upfront, so a callback panic leaks the
                // remaining fields' bytes rather than double-dropping. ZST
                // fields carry no bytes — their `ManuallyDrop` is a no-op —
                // but the wrap is uniform to keep the contract obvious.
                #(
                    let #field_locals = ::std::mem::ManuallyDrop::new(#field_accessors);
                )*

                // Worst-case-sized stack array (all fields non-ZST). The
                // const-folded push guards keep `__data_len` at exactly the
                // non-ZST field count; only `__data_sorted[..__data_len]` is
                // ever read. A single placeholder initialiser keeps the array
                // a plain `[T; N]` (no MaybeUninit churn) — every written slot
                // is overwritten before use.
                let mut __data_sorted: [
                    (
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                        *const u8,
                        usize,
                    );
                    #n_fields
                ] = [
                    (
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId(0),
                        ::std::ptr::null(),
                        0usize,
                    );
                    #n_fields
                ];
                let mut __data_len: usize = 0;
                #(#data_push_stmts)*

                // B1 canonical sort over the populated prefix only.
                __data_sorted[..__data_len].sort_unstable_by_key(|(id, _, _)| id.0);

                for &(id, ptr, len) in &__data_sorted[..__data_len] {
                    debug_assert!(len != 0, "ZST entry leaked into the data walk");
                    // SAFETY (C5 / §6.3): `ptr` was derived from
                    //   `&raw const *ManuallyDrop<T>` for a NON-ZST live stack
                    //   local; it is valid for `len = size_of::<T>()` bytes for
                    //   the duration of this loop. The slice is shared,
                    //   non-overlapping (each local borrowed once), and Drop is
                    //   suppressed by ManuallyDrop (B4: panic → leak, never
                    //   double-drop). Identical invariants to
                    //   `for_each_component_bytes`.
                    let bytes: &[u8] = unsafe { ::std::slice::from_raw_parts(ptr, len) };
                    f(id, bytes);
                }
            }

            // Decision 4 (D4b): every derived bundle within the arity ceiling
            // takes the typed fixed-width write path. The retained byte path
            // (`for_each_data_component_bytes`) is the fallback for any future
            // lowering of the threshold.
            const HAS_TYPED_WRITE: bool = #has_typed_write;

            fn write_row_perm(
                data_component_ids: &[::boyko_ecs::ecs::identifiers::primitives::ComponentId],
                out_perm: &mut [u8],
            ) {
                debug_assert!(
                    out_perm.len() >= #n_fields,
                    "write_row_perm: out_perm too short for declaration arity"
                );
                #(#perm_build_stmts)*
            }

            unsafe fn write_row_typed(
                self,
                dst: &::boyko_ecs::ecs::core::bundle::BundleColumnPtrs,
                row: usize,
            ) {
                // B4 / O1 drop-suppression discipline (identical to
                // `for_each_component_bytes`): ManuallyDrop-wrap EVERY field
                // UPFRONT, before any relocation runs. `write_row_typed` is
                // panic-free internally (ptr::write / ManuallyDrop::take cannot
                // panic), so on this path the wrap simply makes the move-out
                // explicit — each field is relocated exactly once into its
                // column slot and never dropped at end-of-scope.
                #(
                    let mut #field_locals =
                        ::std::mem::ManuallyDrop::new(#field_accessors);
                )*

                // Const-unrolled, DECLARATION-order, fixed-width stores. ZST
                // fields const-fold out (their `if size_of != 0` block
                // disappears at monomorphisation).
                #(#typed_write_stmts)*
            }
        }
    };

    expanded.into()
}

/// Internal helper: a single destructured Bundle field.
///
/// `accessor` carries the original `self.<ident>` or `self.<index>` token
/// stream (so the same struct shape is faithfully reproduced inside the
/// derive output). `local_ident` is the synthetic `__bundle_field_N` ident
/// used as the ManuallyDrop binding — uniform across named and tuple
/// structs to keep the generated code identical in shape.
struct BundleField {
    local_ident: Ident,
    accessor: TokenStream2,
    ty: Type,
}

/// Derive macro for the [`SystemSet`] marker trait — Phase 9 Wave 7 Step 21,
/// extended in Phase 15 with enum support.
///
/// Generates `impl SystemSet for #Name { … }`. Set identity is the pair
/// `(TypeId::of::<Self>(), set_discriminant(self))`:
///
/// * For a **unit struct** the derive emits an empty impl — `set_discriminant`
///   defaults to `0`, so each distinct type is exactly one set.
/// * For an **enum** the derive overrides `set_discriminant` with a `match`
///   returning the variant index, and `set_name` returning `"Type::Variant"`,
///   so each fieldless variant becomes a distinct set with a distinct name.
///
/// # Supported inputs
///
/// * `struct PhysicsSet;` — unit struct (the recommended single-label shape).
/// * `enum CombatSet { Target, Damage, Cleanup }` — fieldless enum; each
///   variant is its own set.
///
/// # Rejected inputs
///
/// * Generic struct/enum (`struct Foo<T>;`) — sets are keyed by
///   `TypeId::of::<S>()`; a generic set would mint a fresh id per
///   monomorphisation. Reported via `compile_error!`.
/// * Union — `compile_error!`. A marker has no use for a union.
/// * Named-field or tuple struct with at least one field — `compile_error!`.
///   A per-instance set would imply a per-value identity the `(TypeId,
///   discriminant)` key cannot represent.
/// * Enum with a data-carrying variant — `compile_error!`. Only fieldless
///   variants have a stable type-level identity.
///
/// # Example
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// use boyko_macros::SystemSet;
///
/// #[derive(SystemSet)]
/// struct PhysicsSet;
///
/// #[derive(SystemSet)]
/// enum CombatSet { Target, Damage, Cleanup }
/// ```
///
/// The example is `ignore`'d because proc-macro crates cannot consume their
/// own macros, and `boyko-macros` cannot depend on `boyko-ecs` for tests
/// (that would create a cycle). Real usage lives in
/// `boyko-ecs/tests/derive_system_set_smoke.rs`.
///
/// [`SystemSet`]: ../boyko_ecs/ecs/core/schedule/system_set/trait.SystemSet.html
/// [`TypeId`]: std::any::TypeId
#[proc_macro_derive(SystemSet)]
pub fn system_set_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_span = name.span();

    // Generics: rejected for both structs and enums. Sets are keyed by
    // `(TypeId, discriminant)`; a generic set type would mint a fresh id per
    // monomorphisation, which is virtually never what the user wants.
    if !input.generics.params.is_empty() {
        return syn::Error::new(
            name_span,
            "SystemSet derive does not support generics (Phase 9 scope)",
        )
        .to_compile_error()
        .into();
    }

    // Body: the `set_discriminant` / `set_name` overrides (if any). Unit
    // structs emit nothing (trait defaults apply); enums emit both.
    let body = match &input.data {
        Data::Struct(s) => {
            // Only unit structs (no fields). A SystemSet is a pure marker;
            // per-instance state contradicts the identity model.
            if !matches!(&s.fields, Fields::Unit) {
                return syn::Error::new(
                    name_span,
                    "SystemSet derive requires a unit struct (no fields)",
                )
                .to_compile_error()
                .into();
            }
            // Unit struct → no override; trait defaults (disc 0, type name).
            TokenStream2::new()
        }
        Data::Enum(e) => match system_set_enum_body(&name, e) {
            Ok(tokens) => tokens,
            Err(err) => return err.to_compile_error().into(),
        },
        Data::Union(_) => {
            return syn::Error::new(
                name_span,
                "SystemSet can only be derived for unit structs or fieldless enums",
            )
            .to_compile_error()
            .into();
        }
    };

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::schedule::SystemSet for #name {
            #body
        }
    };

    expanded.into()
}

/// Generates the `set_discriminant` + `set_name` method bodies for an enum
/// `SystemSet`. Each fieldless variant maps to its index. A data-carrying
/// variant is a hard error (no stable type-level identity).
fn system_set_enum_body(
    name: &Ident,
    data: &syn::DataEnum,
) -> syn::Result<TokenStream2> {
    let mut disc_arms: Vec<TokenStream2> = Vec::with_capacity(data.variants.len());
    let mut name_arms: Vec<TokenStream2> = Vec::with_capacity(data.variants.len());

    for (index, variant) in data.variants.iter().enumerate() {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new(
                variant.ident.span(),
                "SystemSet enum variants must be unit variants (no fields)",
            ));
        }
        let variant_ident = &variant.ident;
        let disc = index as u32;
        let qualified = format!("{name}::{variant_ident}");
        disc_arms.push(quote! { #name::#variant_ident => #disc });
        name_arms.push(quote! { #name::#variant_ident => #qualified });
    }

    Ok(quote! {
        #[inline]
        fn set_discriminant(&self) -> u32 {
            match self {
                #(#disc_arms),*
            }
        }

        #[inline]
        fn set_name(&self) -> &'static str {
            match self {
                #(#name_arms),*
            }
        }
    })
}
