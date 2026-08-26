//! `#[derive(Component)]` implementation.
//!
//! Emits the `Component` trait impl plus inherent layout constants, and folds in
//! the optional lifecycle-hook, required-component, entity-clone/remap,
//! serialization, single-component `Bundle`, and relationship wiring. The bulk of
//! the relationship-side parsing/types live in [`crate::relationship`]; the shared
//! [`crate::common::FieldAccess`] selector is reused here.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, Fields, Ident, Path, Type, parse_macro_input};

use crate::common::FieldAccess;
use crate::relationship::{RelationshipRole, clone_ignore_codegen, parse_relationship_role};

/// Implementation of `#[derive(Component)]` (see the public entry in `lib.rs`).
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // Phase 14a: parse the optional `#[component(...)]` hook attribute
    // (extended in Phase 22 with the bare `no_bundle` flag key, and in EnableTag
    // Wave 5 with the `storage = "bitset"` NameValue key).
    let hooks = match parse_component_hooks(&input.attrs) {
        Ok(h) => h,
        Err(ts) => return ts,
    };

    // Relations v1 (Decision 4): parse the optional `#[relationship(...)]` /
    // `#[relationship_target(...)]` attribute. A `#[derive(Component)]` carrying one
    // of these is the SOURCE / TARGET side of a relation: the `Component` derive
    // folds the relationship hook wiring + (source) the entity-remap clone/serialize
    // metadata into its OWN `register_hooks` / `component_id()` (composed, not a
    // separate impl block — the `impl Relationship` / `impl RelationshipTarget` block
    // itself is emitted by the paired `#[derive(Relationship)]` /
    // `#[derive(RelationshipTarget)]`). The two are mutually exclusive (a type is one
    // side of the relation, never both).
    let relationship = match parse_relationship_role(&input) {
        Ok(r) => r,
        Err(ts) => return ts,
    };

    // Collision rule: the relationship OWNS the hook slots it wires
    // (`on_insert`/`on_replace` for the source, `on_replace` for the target). A
    // user `#[component(on_insert=…)]` / `#[component(on_replace=…)]` alongside a
    // relationship attribute would silently lose to (or double-install with) the
    // generic hook — reject it loudly (R5 `relationship_hook_collision`).
    if let Some(role) = &relationship
        && let Err(ts) = role.reject_hook_collision(&input.ident, &hooks)
    {
        return ts;
    }

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
    // forced to `CloneViaFn` so the deep-clone remap can run. Serialization S0
    // (C3) reuses the same scan to suppress the POB arm for an Entity-bearing type.
    let has_entity_field = struct_has_entity_field(&input);

    // Serialization S0 (C4): accept the `#[entities]` field attribute; reject a
    // malformed `#[entities(..)]` shape.
    if let Err(ts) = validate_entities_attrs(&input) {
        return ts;
    }

    // Serialization S2.5 (C4): emit the `map_entities_fn()` override + the
    // monomorphized remap free fn for a component with `#[entities]`-annotated
    // Entity field(s). A component with no annotated field emits nothing (the trait
    // default `map_entities_fn() == None` applies — a plain Entity field keeps its
    // raw saved id, the explicit-opt-in decision). Computed BEFORE `input.ident`
    // moves; suppressed for a bitset enable tag (no `ComponentPool` → never
    // serialized, mirrors the serialize / clone suppression).
    //
    // Relations v1 (C2/B10/B11): a relationship SOURCE auto-emits the load-remap for
    // its foreign-key `Entity` field WITHOUT requiring `#[entities]` (a relation's
    // foreign key is by definition an `Entity` that must be remapped on clone/load,
    // else deep-clone / save-load silently break — see the hand-mirror
    // `child_of_load_map_entities`). The accessor list is the FK field; the same
    // `entities_map_codegen` machinery generates the load-remap fn + `map_entities_fn`
    // override, which `install_serialize_fn` reads into the `SerializeInfo`.
    let (entities_items, entities_module_items) = if hooks.storage_bitset {
        (TokenStream2::new(), TokenStream2::new())
    } else if let Some(RelationshipRole::Source(src)) = &relationship {
        entities_map_codegen_for_accessors(&input.ident, &[src.field_access()])
    } else {
        entities_map_codegen(&input, &input.ident)
    };

    // Serialization S0 (plan §3.7, §5 C1–C3): compute the serialize classification
    // overrides (the `SerializeProbe` invocation + `SerPod` proof + fingerprint +
    // stable_name + format_version) BEFORE `input.ident` moves. Suppressed for a
    // bitset enable tag (no `ComponentPool`, so it must classify `Ignore` and never
    // enter a serialized column set — mirrors the clone suppression below).
    //
    // Relations v1 (B12): a relationship TARGET (the reverse index) is NEVER
    // serialized as data — it is rebuilt from the sources' `Relationship` on load,
    // exactly as it is rebuilt on clone. Keep it at the `Serializability::Ignore`
    // trait default (emit no serialize overrides), mirroring the in-crate `Children`
    // hand-mirror, which carries no serialize metadata. (The SOURCE keeps the default
    // path: its `Entity` foreign key → `SerializeViaFn` + the auto load-remap, B11.)
    let (serialize_items, serialize_module_items) =
        if hooks.storage_bitset || matches!(&relationship, Some(RelationshipRole::Target(_))) {
            // A bitset tag / relationship target stays at the trait defaults
            // (`Ignore` / version 0 / fingerprint 0 / type-name key) — emit nothing.
            (TokenStream2::new(), TokenStream2::new())
        } else {
            match serialize_codegen(&input, &input.ident, &hooks, has_entity_field) {
                Ok(items) => items,
                Err(ts) => return ts,
            }
        };

    // Reflection CORE C7: `#[component(reflect)]` emits a free `static TypeInfo` plus the
    // `impl boyko_reflect::Reflect` pointing at it, and D20's `ReflectDefault` witness.
    // The whole emission is `#[cfg(feature = "reflect")]` evaluated in the EXPANDING
    // crate (D2), so an un-annotated derive and a feature-off consumer both emit nothing,
    // and this crate keeps no edge to `boyko_reflect` (D17). CORE C8 adds the install
    // call in `component_id()` below, which is what makes the descriptor reachable.
    //
    // CORE C9 / D37 — D29's `!hooks.storage_bitset` term is REPLACED here, not joined.
    //
    // C8 landed a *silent* suppression: `hooks.reflect && !hooks.storage_bitset`, so a
    // `#[component(reflect, storage = "bitset")]` tag compiled and published nothing. C9
    // makes the combination a spanned `compile_error!`, which leaves that term unreachable
    // in its suppressing branch and its only witness (`reflect_fixture`'s
    // `c8_bitset_suppression.rs`) unable to compile — a dead datum whose gate has just been
    // deleted, and a RED (*"drop the `storage_bitset` term"*) with no subject left to
    // observe it. So the term goes with the gate it served. Nothing is lost: feature off,
    // the whole emission is `cfg`-stripped and nothing installs; feature on, the refusal
    // stops the compile. ECS D5's *"two mechanisms at two boundaries"* is the compile-time
    // refusal plus the release `assert!` inside `install_type_info` — not three.
    //
    // The bitset condition is now an ARGUMENT rather than a suppression: `codegen` needs
    // the `reflect` key's span to put the caret on it (D37), and a `bool` cannot carry one.
    //
    // Computed BEFORE `input.ident` moves below, like every other codegen that needs to
    // walk the fields.
    let (reflect_items, reflect_default_witness, reflect_refused) = if hooks.reflect {
        let no_default = match crate::reflect::parse_reflect_no_default(&input.attrs) {
            Ok(v) => v,
            Err(ts) => return ts,
        };
        let bitset_reflect_key = if hooks.storage_bitset { hooks.reflect_span } else { None };
        crate::reflect::codegen(&input, &input.ident, no_default, bitset_reflect_key)
    } else {
        (TokenStream2::new(), TokenStream2::new(), false)
    };
    // A REFUSED item emits refusals and no descriptor, so the install slot below must go
    // with it: `<Self as Reflect>::TYPE_INFO` on a type with no `impl Reflect` is an
    // E0277 that would land in every refused fixture's blessed `.stderr` beside the real
    // message, freezing rustc's rendering of a second, derived error. One refusal, one
    // error.
    let reflect_enabled = hooks.reflect && !reflect_refused;

    let name = input.ident;

    // Emit `const HAS_HOOKS = true;` + a `register_hooks` impl only when at
    // least one hook key is present; otherwise the trait defaults
    // (`HAS_HOOKS = false`, empty `register_hooks`) apply.
    //
    // Relations v1 (Decision 4): a relationship side OWNS the hook slots — its
    // `register_hooks` wires the GENERIC monomorphized hooks
    // (`<Self as Relationship>::on_insert` etc.), overriding any user
    // `#[component(on_*)]` (which the collision check already rejected). A SOURCE
    // wires `on_insert` (link) + `on_replace` (unlink); a TARGET wires ONLY
    // `on_replace` (the cascade — never `on_add`/`on_insert`, B7).
    //
    // The C2 install tripwire rides the const gate: `hook_items` emits
    // `const HAS_HOOKS: bool = true`, and the `if Self::HAS_HOOKS { install_hooks }`
    // line in the generated `component_id()` reads THAT const — so the cold `HOOKS`
    // slot is installed for a relationship even though the in-macro `hooks` struct
    // carries no user hook path.
    let hook_items = match &relationship {
        Some(role) => role.hook_items_codegen(),
        None => hooks.codegen(),
    };

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
    //
    // Relations v1 (B12): a relationship TARGET (the reverse index, e.g. `Children`
    // / `LikedBy`) is ALWAYS `Cloneability::Ignore` — it is never byte-copied; a deep
    // clone rebuilds it from the sources' `Relationship` via the Link commands.
    // Override the autoref classification (the `Vec<Entity>` field would otherwise
    // push it to `CloneViaFn`) to the explicit `Ignore`. A SOURCE keeps the default
    // path (its `Entity` foreign key forces `CloneViaFn` so the remap runs, B10).
    let (clone_items, clone_install) = if hooks.storage_bitset {
        (TokenStream2::new(), TokenStream2::new())
    } else {
        let items = match &relationship {
            Some(RelationshipRole::Target(_)) => clone_ignore_codegen(),
            _ => clone_codegen(&name, &hooks, has_entity_field),
        };
        (
            items,
            quote! {
                boyko_ecs::ecs::core::component::component_registry::install_clone_fn::<Self>(raw);
            },
        )
    };

    // BUG-RELATIONS-CLONE-1: the relationship CLONE-direction installs (the missing
    // half of the Option-A generalization). A SOURCE installs the generic clone-remap
    // + relink (`get_map_entities_fn(R)` / `get_relationship_relink_fn(R)` now return
    // `Some`), so the deep clone remaps its foreign key + rebuilds the clone-side
    // reverse index. A TARGET sets the relationship-target flag so the generic
    // clone-deny (`select_clone_ids` via `is_relationship_target`) denies it instead of
    // tripping the non-cloneable `debug_assert!`. A bitset tag suppresses both (it has
    // no `ComponentPool` and is never cloned). One cold `OnceLock::set` per type — the
    // 0%-gate, registration-time only.
    let relationship_install = if hooks.storage_bitset {
        TokenStream2::new()
    } else {
        match &relationship {
            Some(RelationshipRole::Source(_)) => quote! {
                boyko_ecs::ecs::core::component::component_registry::install_relationship_clone_remap::<Self>(raw);
            },
            Some(RelationshipRole::Target(_)) => quote! {
                boyko_ecs::ecs::core::component::component_registry::set_relationship_target(raw);
            },
            None => TokenStream2::new(),
        }
    };

    // BUG-RELATIONS-CLONE-1 (secondary): a relationship SOURCE carrying the `Entity`
    // foreign key MUST be `Clone` — otherwise the autoref clone classification falls to
    // the by-value `Ignore` arm, the source is never cloned, and its FK silently fails
    // to remap on deep clone (the corruption this fix exists to prevent). The generic
    // remap fn is installed unconditionally above, so a non-`Clone` source would be the
    // worst case: a remap fn registered for a component that is never cloned. Fail
    // LOUDLY at compile time with a clear message instead of the silent `Ignore`
    // demotion. (A TARGET is always `Cloneability::Ignore` by design — no bound.)
    let relationship_clone_assert = match &relationship {
        Some(RelationshipRole::Source(_)) => quote! {
            const _: () = {
                const fn __assert_relationship_source_is_clone<T: ::core::clone::Clone>() {}
                // A clear compile error if `#name` is not `Clone`: a relationship source
                // must `#[derive(Clone)]` (or `Clone, Copy`) so its foreign key is
                // remapped on deep clone (BUG-RELATIONS-CLONE-1).
                __assert_relationship_source_is_clone::<#name>();
            };
        },
        _ => TokenStream2::new(),
    };

    // Phase 4 Seam 1 (D1): the UNGATED residency install. One cold read of the
    // `C::RESIDENCY` const per type per process (behind the `component_id()`
    // `OnceLock`); `install_residency_class` self-gates on the default `Cpu`
    // const, so a plain `#[derive(Component)]` short-circuits to a no-op (the
    // 0%-gate). Always emitted — like `install_clone_fn`, not gated on a derive
    // flag — so a hand-set `const RESIDENCY` (no attribute) is still installed.
    let residency_install = quote! {
        boyko_ecs::ecs::core::component::component_registry::install_residency_class::<Self>(raw);
    };

    // Serialization S0 (plan §3.7): the UNGATED install calls in `component_id()`
    // (like `install_clone_fn`) — one cold `OnceLock::set` (the `SERIALIZE` table)
    // + one `Mutex` insert (the C1 `STABLE_NAME_INDEX`) per type per process, the
    // 0%-gate. Suppressed for a bitset enable tag (no `ComponentPool` → never
    // serialized): the `serialize_items` above already emit nothing for it, so the
    // table would record the `Ignore` trait default — keep the metadata table
    // consistent by also skipping the install + name-index registration.
    let serialize_install = if hooks.storage_bitset {
        TokenStream2::new()
    } else {
        quote! {
            boyko_ecs::ecs::core::component::component_registry::install_serialize_fn::<Self>(raw);
            boyko_ecs::ecs::core::component::component_registry::register_stable_name::<Self>(raw);
        }
    };

    // Reflection CORE C8 — the SEVENTH install slot, and the campaign's central claim in
    // one line. `component_id()` is the funnel every other per-type datum is published
    // through; the descriptor C7 baked is inert until it joins them, because nothing else
    // in the expansion references it (MEASURED at C7: zero `__REFLECT_TYPE_INFO` symbols
    // in every link configuration — an uncalled static is dropped before the linker sees
    // it).
    //
    // The paths are ABSOLUTE, and that is a decision rather than a copy of the neighbours.
    // The six slots above use the non-absolute `boyko_ecs::…` form and matching them would
    // be defensible — but this is the one path whose ABSENCE IN A SHIP BUILD is what the
    // whole campaign claims, and a bare first segment resolves through the consumer's own
    // scope before the extern prelude, so a consumer `mod boyko_reflect` or
    // `use x as boyko_reflect` would shadow it. Every other path the reflect emission puts
    // into a consumer crate is already absolute (`reflect.rs:190`, `:249`, `:378`, `:463`);
    // this closes the last one.
    //
    // `#[cfg(feature = "reflect")]` on the STATEMENT, evaluated in the crate the derive
    // expanded into (D2) — the same gate the descriptor itself carries. Feature-off, the
    // statement is not merely dead, it does not exist: `boyko_reflect` is not in the
    // consumer's resolved graph at all, so an un-`cfg`'d form would be `E0433` rather than
    // a silent leak (which is exactly what C8's first RED observes).
    //
    // No `IS_REFLECT` const (D7): "is `T` reflectable?" has one carrier, and it is
    // `type_info_of(id).is_some()`. `boyko_macros` gains NO dependency on `boyko_reflect`
    // (D17) — this is a token stream, not a call site in this crate.
    let reflect_install = if reflect_enabled {
        quote! {
            #[cfg(feature = "reflect")]
            ::boyko_reflect::install_type_info(
                raw,
                <Self as ::boyko_reflect::Reflect>::TYPE_INFO,
            );
        }
    } else {
        TokenStream2::new()
    };

    // Phase 22 D7: single-component Bundle emission (suppressed by
    // `#[component(no_bundle)]`). EnableTag D6: `storage = "bitset"` ALSO
    // suppresses it — a bitset tag has no `ComponentPool` and must not be
    // spawnable as a one-component bundle (`storage = "bitset"` implies
    // `no_bundle`). Dense plan D0: `storage = "dense"` likewise suppresses it —
    // at D0 a dense component has NO global `DenseStore` yet (that lands in D1)
    // and no per-archetype pool, so a naive single-component spawn would have
    // nowhere to write the data. Suppressing the standalone `Bundle` makes
    // "spawn a dense component" a clean compile-time absence rather than a silent
    // data drop; D1/D2 wire the real store + structural-op routing.
    let bundle_items = if hooks.no_bundle || hooks.storage_bitset || hooks.storage_dense {
        TokenStream2::new()
    } else {
        component_self_bundle_codegen(&name)
    };

    let expanded = quote! {
        #bundle_items

        #reflect_items

        #reflect_default_witness

        #require_ctor_fns

        #relationship_clone_assert

        #serialize_module_items

        #entities_module_items

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
                    #relationship_install
                    #residency_install
                    #serialize_install
                    #reflect_install
                    boyko_ecs::ecs::identifiers::primitives::ComponentId(raw)
                })
            }

            #storage_items

            #hook_items

            #require_items

            #clone_items

            #serialize_items

            #entities_items

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
pub(crate) struct ComponentHookPaths {
    on_add: Option<Path>,
    pub(crate) on_insert: Option<Path>,
    pub(crate) on_replace: Option<Path>,
    on_remove: Option<Path>,
    no_bundle: bool,
    /// `true` iff `storage = "bitset"` was supplied (EnableTag D5).
    storage_bitset: bool,
    /// `true` iff `storage = "dense"` was supplied (Dense plan D0). Mutually
    /// exclusive with `storage_bitset` (the `storage` key may be set at most
    /// once, enforced in `parse_component_hooks`).
    storage_dense: bool,
    /// Feature 3: `true` iff the bare `no_clone` flag was supplied — opts the
    /// component out of cloning (`Cloneability::Ignore`, `clone_fn = None`), even if
    /// the type is `Clone` / `Copy`.
    no_clone: bool,
    /// Feature 3: `Some(path)` iff `clone = <free fn>` was supplied — a capture-free
    /// `unsafe fn(*const u8, *mut u8)` (the `CloneFn` shape) the user installs as the
    /// custom clone. Mutually exclusive with `no_clone`.
    clone_with: Option<Path>,
    /// Serialization S0: `true` iff the bare `no_serialize` flag was supplied — opts
    /// the component out of serialization (`Serializability::Ignore`), even if the
    /// type is otherwise POB / `Clone`.
    no_serialize: bool,
    /// Serialization S0 (C1): `Some(name)` iff `stable_name = "..."` was supplied —
    /// overrides the default fully-qualified type name as the stable on-disk key.
    stable_name: Option<String>,
    /// Serialization S0 (§3.5): `Some(v)` iff `format_version = N` was supplied —
    /// the human-facing layout/semantic version. Default `0` when omitted.
    format_version: Option<u16>,
    /// Reflection CORE C7: `true` iff the bare `reflect` flag was supplied — opts the
    /// component into the EDITOR-ONLY reflection layer. The emission it turns on is
    /// itself `#[cfg(feature = "reflect")]`, evaluated in the crate the derive expanded
    /// into (CORE D2), so the key is inert in a consumer that has not enabled the
    /// feature and this crate never gains an edge to `boyko_reflect` (CORE D17).
    reflect: bool,
    /// Reflection CORE C9 / D37: the span of the `reflect` key itself, kept so the
    /// `storage = "bitset"` refusal can put its caret **on that token**.
    ///
    /// A `bool` cannot carry a caret, and the caret is the deliverable here: three
    /// census-gated documents specified three different ones for this single refusal, and
    /// the blessed `.stderr` freezes whichever is emitted. `storage = "bitset"` is
    /// legitimate on its own; `reflect` is the token that is wrong.
    reflect_span: Option<proc_macro2::Span>,
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

    /// EnableTag D5 / Dense plan D0: emits `const STORAGE_IS_BITSET = true;` for
    /// `storage = "bitset"` or `const STORAGE_IS_DENSE = true;` for
    /// `storage = "dense"` (overriding the `Component` trait default of `false`),
    /// or an empty token stream (trait defaults apply) otherwise.
    ///
    /// `STORAGE_IS_BITSET` is what makes `Added<T>` / `Changed<T>` on a bitset
    /// tag a compile error (the D4 per-monomorphization const-asserts read it).
    /// Both consts are what the matching `install_*_storage_kind::<Self>`
    /// const-gates on. The two keys are mutually exclusive (`storage may be set at
    /// most once`), so at most one branch fires.
    fn storage_codegen(&self) -> TokenStream2 {
        if self.storage_bitset {
            quote! {
                const STORAGE_IS_BITSET: bool = true;
            }
        } else if self.storage_dense {
            quote! {
                const STORAGE_IS_DENSE: bool = true;
            }
        } else {
            TokenStream2::new()
        }
    }

    /// EnableTag D5 / Dense plan D0: emits the registration-time call that
    /// classifies the minted id as `StorageKind::Bitset` (`storage = "bitset"`)
    /// or `StorageKind::Dense` (`storage = "dense"`), or an empty token stream
    /// otherwise.
    ///
    /// Emitted into the derive's `component_id()` `OnceLock` init closure,
    /// AFTER `register_new` mints `raw` and (when present) hooks install — the
    /// same atomic-with-id-assignment, before-any-archetype ordering that
    /// `install_hooks` relies on. Routed through the `pub` wrappers
    /// `install_storage_kind::<Self>` / `install_dense_storage_kind::<Self>`
    /// because the underlying `set_storage_kind` is `pub(crate)` and unreachable
    /// from a downstream crate's derive output.
    fn storage_install_codegen(&self) -> TokenStream2 {
        if self.storage_bitset {
            quote! {
                boyko_ecs::ecs::core::component::component_registry::install_storage_kind::<Self>(raw);
            }
        } else if self.storage_dense {
            quote! {
                boyko_ecs::ecs::core::component::component_registry::install_dense_storage_kind::<Self>(raw);
            }
        } else {
            TokenStream2::new()
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
                     no_clone, clone = <fn>, storage = \"bitset\", no_serialize, \
                     stable_name = \"..\", format_version = N, reflect",
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

            // Reflection CORE C7: bare flag key `reflect` — opt this component into the
            // editor-only reflection layer. `no_bundle` (just above) is the precedent
            // for the shape: no `= <value>` follows, and a repeat is an error rather
            // than a silent second `true`.
            if meta.path.is_ident("reflect") {
                if paths.reflect {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; reflect may be set at most once",
                    ));
                }
                paths.reflect = true;
                // CORE C9 / D37 -- the caret for the `storage = "bitset"` refusal. Taken
                // from the key's own path so it survives whatever else the attribute
                // carries and however it is formatted.
                paths.reflect_span = Some(meta.path.span());
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

            // Serialization S0: bare flag key `no_serialize` — opt out of
            // serialization (`Serializability::Ignore`).
            if meta.path.is_ident("no_serialize") {
                if paths.no_serialize {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; no_serialize may be set at most once",
                    ));
                }
                paths.no_serialize = true;
                return Ok(());
            }

            // Serialization S0 (C1): NameValue key `stable_name = "..."` — the
            // stable on-disk type key (a STRING LITERAL). Overrides the default
            // fully-qualified type name.
            if meta.path.is_ident("stable_name") {
                if paths.stable_name.is_some() {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; stable_name may be set at most once",
                    ));
                }
                let value = meta.value()?; // consumes the `=`
                let lit: syn::LitStr = value.parse()?;
                paths.stable_name = Some(lit.value());
                return Ok(());
            }

            // Serialization S0 (§3.5): NameValue key `format_version = N` — the
            // human-facing layout/semantic version (a `u16` INTEGER LITERAL).
            if meta.path.is_ident("format_version") {
                if paths.format_version.is_some() {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; format_version may be set at most once",
                    ));
                }
                let value = meta.value()?; // consumes the `=`
                let lit: syn::LitInt = value.parse()?;
                paths.format_version = Some(lit.base10_parse::<u16>()?);
                return Ok(());
            }

            // EnableTag D5 (Wave 5 Step 10) / Dense plan D0: `storage = "bitset"`
            // or `storage = "dense"` — a NameValue key whose value is a STRING
            // LITERAL (W1-r6: parsed as a `LitStr`, NOT a bare-key flag and NOT a
            // path). Any other string is rejected with a message naming the
            // allowed values.
            if meta.path.is_ident("storage") {
                if paths.storage_bitset || paths.storage_dense {
                    return Err(meta.error(
                        "duplicate #[component(...)] key; storage may be set at most once",
                    ));
                }
                let value = meta.value()?; // consumes the `=`
                let lit: syn::LitStr = value.parse()?;
                match lit.value().as_str() {
                    "bitset" => paths.storage_bitset = true,
                    "dense" => paths.storage_dense = true,
                    other => {
                        return Err(syn::Error::new_spanned(
                            &lit,
                            format!(
                                "unknown component storage {other:?}; \
                                 the only supported values are \"bitset\" and \"dense\""
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
                     no_clone, clone = <fn>, storage = \"bitset\", no_serialize, \
                     stable_name = \"..\", format_version = N, reflect",
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

// ── Serialization S0 derive support (plan §3.6 C2, §3.5 C1, §5 C3) ──────────────

/// The `#[repr(...)]` kind relevant to serialization (plan §3.6 / C2). Only the
/// layout-stable reprs (`C`, `transparent`) make a type blittable; `Rust` (the
/// default) and `packed` do not (the compiler may reorder fields). Folded into the
/// layout fingerprint so a `#[repr]` change is detected, and gates POB eligibility.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReprKind {
    /// The default Rust repr — field order is unspecified, NOT blittable.
    RustDefault,
    /// `#[repr(C)]` — a fixed, predictable layout. Blittable.
    C,
    /// `#[repr(transparent)]` — single-field newtype, same layout as the field.
    /// Blittable.
    Transparent,
    /// Any other repr (`packed`, `align(N)`, a primitive enum repr, …). Treated as
    /// non-blittable for POB (conservative): the macro folds the kind into the
    /// fingerprint but never classifies it `PlainOldBytes`.
    Other,
}

impl ReprKind {
    /// A stable `u64` tag for the layout fingerprint (plan §3.6). Distinct values
    /// so a `#[repr(C)]` → `#[repr(transparent)]` (or → default) change perturbs
    /// the fingerprint.
    fn fingerprint_tag(self) -> u64 {
        match self {
            ReprKind::RustDefault => 0,
            ReprKind::C => 1,
            ReprKind::Transparent => 2,
            ReprKind::Other => 3,
        }
    }

    /// `true` iff a POB classification is layout-permitted by the repr (plan §3.6
    /// C2: `#[repr(C)]`/`#[repr(transparent)]` required for blittable).
    fn pob_layout_ok(self) -> bool {
        matches!(self, ReprKind::C | ReprKind::Transparent)
    }
}

/// Parses the `#[repr(...)]` attribute(s) into a [`ReprKind`] (plan §3.6 / C2).
/// Recognizes `C` and `transparent`; everything else (including `packed`,
/// `align(N)`, primitive enum reprs, or no `#[repr]` at all) maps to a
/// non-blittable kind. `C` wins if both `C` and an alignment/other modifier are
/// present (`#[repr(C, align(8))]` is still C-layout for blit purposes).
fn parse_repr(attrs: &[syn::Attribute]) -> ReprKind {
    let mut kind = ReprKind::RustDefault;
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        // Each `#[repr(...)]` is a comma-separated list of idents/calls.
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("C") {
                kind = ReprKind::C;
            } else if meta.path.is_ident("transparent") {
                // Do not let `transparent` downgrade an already-seen `C`.
                if kind != ReprKind::C {
                    kind = ReprKind::Transparent;
                }
            } else if kind == ReprKind::RustDefault {
                // `packed` / `align` / primitive enum reprs / unknown → Other,
                // but never overwrite a recognized C/transparent.
                kind = ReprKind::Other;
            }
            // `parse_nested_meta` wants every nested item consumed; `align(N)`
            // carries a parenthesized value — swallow it so parsing succeeds.
            if meta.input.peek(syn::token::Paren) {
                let _ = meta.parse_nested_meta(|_| Ok(()));
            }
            Ok(())
        });
    }
    kind
}

/// Collects every field type of a struct (named or tuple), in declaration order
/// (plan §3.6 / C2 — the fingerprint and the `SerPod` field gate both need them).
/// Returns `None` for a unit struct (no fields — a ZST tag) and for enums/unions
/// (no single field shape; never POB). The `bool` is `true` for a unit struct
/// (distinguishes "zero fields, blittable ZST" from "not a plain struct").
fn struct_field_types(input: &DeriveInput) -> Option<(Vec<Type>, Vec<FieldAccess>)> {
    match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => {
                let tys = named.named.iter().map(|f| f.ty.clone()).collect();
                let access = named
                    .named
                    .iter()
                    .map(|f| FieldAccess::Named(f.ident.clone().expect("named field has ident")))
                    .collect();
                Some((tys, access))
            }
            Fields::Unnamed(unnamed) => {
                let tys = unnamed.unnamed.iter().map(|f| f.ty.clone()).collect();
                let access = (0..unnamed.unnamed.len()).map(FieldAccess::Index).collect();
                Some((tys, access))
            }
            // Unit struct: zero fields. A blittable ZST tag (no field-validity
            // concern). Returned as empty vectors.
            Fields::Unit => Some((Vec::new(), Vec::new())),
        },
        // Enums and unions are never POB (no single-field-shape, validity-invariant
        // discriminant / overlapping fields). Returning None routes them to the
        // ViaFn / Ignore arms and suppresses the fingerprint's offset_of terms.
        Data::Enum(_) | Data::Union(_) => None,
    }
}

/// Emits the `const LAYOUT_FINGERPRINT: u64` override (plan §3.6 / C2):
/// a `const fn`-folded hash of `(size_of, align_of, repr_tag, [offset_of!(C, f)
/// for each field f], per-field size_of, field_count)`. NO field-type-NAME
/// dependency (C2 — a proc-macro cannot see a field's stable type name; only
/// `offset_of!`/`size_of`, which are derivable). Catches a same-size field reorder
/// IFF it changes an offset; a pure same-size swap of two identical-type fields is
/// layout-invisible (the bytes are interchangeable) and therefore safe to blit —
/// documented in the plan.
///
/// Reuses the registry's `fnv1a_64` over a little-endian byte buffer of the layout
/// scalars so the hash is identical to what the loader would recompute.
fn layout_fingerprint_codegen(
    name: &Ident,
    repr: ReprKind,
    field_access: &[FieldAccess],
    field_tys: &[Type],
) -> TokenStream2 {
    let repr_tag = repr.fingerprint_tag();
    let field_count = field_access.len() as u64;

    // Per-field `(offset_of!(Name, f), size_of::<FieldTy>())` u64 pairs, pushed
    // into the byte buffer in declaration order.
    let field_terms: Vec<TokenStream2> = field_access
        .iter()
        .zip(field_tys.iter())
        .map(|(acc, ty)| {
            let sel = acc.offset_of_selector();
            quote! {
                __boyko_fp_push(
                    &mut __buf,
                    &mut __len,
                    ::core::mem::offset_of!(#name, #sel) as u64,
                );
                __boyko_fp_push(
                    &mut __buf,
                    &mut __len,
                    ::core::mem::size_of::<#ty>() as u64,
                );
            }
        })
        .collect();

    quote! {
        const LAYOUT_FINGERPRINT: u64 = {
            // A fixed-capacity scratch buffer + a const-fn FNV-1a fold: no alloc,
            // fully const-evaluable. 8 scalars of header + 2 per field; 64 fields
            // is far above any realistic component arity.
            const __CAP: usize = 8 * (2 + 2 * 64);
            let mut __buf = [0u8; __CAP];
            let mut __len: usize = 0;

            const fn __boyko_fp_push(buf: &mut [u8; __CAP], len: &mut usize, value: u64) {
                let bytes = value.to_le_bytes();
                let mut i = 0;
                while i < 8 {
                    buf[*len] = bytes[i];
                    *len += 1;
                    i += 1;
                }
            }

            __boyko_fp_push(&mut __buf, &mut __len, ::core::mem::size_of::<#name>() as u64);
            __boyko_fp_push(&mut __buf, &mut __len, ::core::mem::align_of::<#name>() as u64);
            __boyko_fp_push(&mut __buf, &mut __len, #repr_tag);
            __boyko_fp_push(&mut __buf, &mut __len, #field_count);
            #(#field_terms)*

            // `__len <= __CAP` by construction (4 header pushes + 2 per field; the
            // field count is bounded by the 64-field capacity above — a struct with
            // > 64 fields overflows `__buf` and the const eval fails loudly at
            // compile time, never at runtime). The `[..__len]` const-slice hands the
            // exact written prefix to the same FNV-1a the loader recomputes.
            let __slice: &[u8] = __buf.split_at(__len).0;
            ::boyko_ecs::ecs::core::component::component_registry::fnv1a_64(__slice)
        };
    }
}

/// Emits the serialization classification overrides (plan §3.7, §5 C3): the
/// `SERIALIZABILITY` const + `serializability_runtime()` method (via the autoref
/// `SerializeProbe`, STRICTER than clone), the conditional `unsafe impl SerPod`
/// (the all-bits-valid aggregate proof for the POB arm), the `FORMAT_VERSION`
/// const, the `stable_name()` override, and the `LAYOUT_FINGERPRINT` const.
///
/// * `#[component(no_serialize)]` → `Ignore` (emit only the metadata consts;
///   `serializability_runtime` stays the `Ignore` default — return early).
/// * `#[repr(C/transparent)]` + no `Entity` field → `POB_ELIGIBLE = true` + a
///   `where`-gated `unsafe impl SerPod for Self` so the POB arm fires ONLY if every
///   field is `SerPod` (all-bits-valid). A `bool`/`char`/enum/niche field fails the
///   `where` clause, the `impl SerPod` does not apply, and the autoref probe falls
///   to `SerializeViaFn` (C3).
/// * else → `POB_ELIGIBLE = false`: the probe resolves `SerializeViaFn` (Clone) or
///   `Ignore` (non-Clone).
///
/// Returns `(trait_items, module_items)`: the trait-body items (consts + methods,
/// spliced into `impl Component for Self`) and the MODULE-SCOPE items (the
/// `unsafe impl SerPod for Self`, which is a top-level item and CANNOT live inside
/// an `impl` block — it is emitted alongside the single-component `Bundle` impl).
fn serialize_codegen(
    input: &DeriveInput,
    name: &Ident,
    hooks: &ComponentHookPaths,
    has_entity_field: bool,
) -> Result<(TokenStream2, TokenStream2), TokenStream> {
    let repr = parse_repr(&input.attrs);

    // Field shape: None for enums/unions (never POB). For POB eligibility the type
    // must be a plain struct AND repr-C/transparent AND have no Entity field.
    let fields = struct_field_types(input);

    // `format_version` const + `stable_name()` override are emitted regardless of
    // the classification (a ViaFn / Ignore component still carries a stable key +
    // version once it opts in via a future encode path; and the fingerprint guards
    // even a decode-path mismatch — C2).
    let format_version = hooks.format_version.unwrap_or(0);
    let format_version_item = quote! {
        const FORMAT_VERSION: u16 = #format_version;
    };
    let stable_name_item = match &hooks.stable_name {
        Some(s) => quote! {
            #[inline]
            fn stable_name() -> &'static str { #s }
        },
        None => TokenStream2::new(),
    };

    // The layout fingerprint. For an enum/union (no field shape) the fingerprint
    // folds only size/align/repr/field_count==0 — still a valid guard, never POB.
    let (field_access, field_tys): (Vec<FieldAccess>, Vec<Type>) = match &fields {
        Some((tys, access)) => {
            // Unzip preserving order.
            let a: Vec<FieldAccess> = access
                .iter()
                .map(|f| match f {
                    FieldAccess::Named(id) => FieldAccess::Named(id.clone()),
                    FieldAccess::Index(i) => FieldAccess::Index(*i),
                })
                .collect();
            (a, tys.clone())
        }
        None => (Vec::new(), Vec::new()),
    };
    let fingerprint_item = layout_fingerprint_codegen(name, repr, &field_access, &field_tys);

    // `no_serialize`: classify Ignore. Emit the metadata consts (so the file key /
    // version / fingerprint are still recorded) but leave `serializability_runtime`
    // at the `Ignore` trait default.
    if hooks.no_serialize {
        let trait_items = quote! {
            const SERIALIZABILITY:
                ::boyko_ecs::ecs::core::component::component_registry::Serializability =
                ::boyko_ecs::ecs::core::component::component_registry::Serializability::Ignore;
            #format_version_item
            #fingerprint_item
            #stable_name_item
        };
        // `no_serialize` never emits a `SerPod` impl (the type must not be POB).
        return Ok((trait_items, TokenStream2::new()));
    }

    // POB eligibility (C2 + C3): a plain struct, repr-C/transparent, no Entity
    // field. The all-bits-valid FIELD proof is NOT done here — it is the
    // `Fields: SerPodTuple` bound on the autoref POB arm (a generic bound the
    // resolver can leave un-matched, demoting to ViaFn — never a hard error). So a
    // `#[repr(Rust)]` struct (POB_ELIGIBLE == false) is a SILENT demotion to ViaFn,
    // and a repr-C struct with a bool/char/enum/niche field also silently demotes
    // (its field tuple is not `SerPodTuple`). This realizes the plan's "not
    // provably POB → SerializeViaFn" (§5 C3) without a hard error, which matches
    // the decode path always being sound.
    let is_plain_struct = fields.is_some();
    let pob_eligible = is_plain_struct && repr.pob_layout_ok() && !has_entity_field;

    let pob_flag = if pob_eligible {
        quote! { true }
    } else {
        quote! { false }
    };

    // The field tuple `(F0, F1, …)` the probe's `Fields` parameter carries — the
    // `SerPodTuple` field-validity proof operates on it. A unit struct / non-plain
    // struct uses `()` (vacuously `SerPodTuple` when POB_ELIGIBLE, but a non-plain
    // type carries POB_ELIGIBLE == false so the POB arm never fires anyway).
    let field_tuple = quote! { ( #(#field_tys,)* ) };

    // Phase S1.5 (plan §3.1 / §3.7): the per-element encode glue. A plain struct
    // with AT LEAST ONE field emits a bound-free `WireBridge` (struct ↔ field tuple)
    // + `serialize_fn` / `deserialize_fn` overrides that select the generic
    // `serialize_via_wire` / `deserialize_via_wire` glue through the `WireFnProbe`
    // autoref arm. The arm installs `Some(glue)` ONLY when every field is `Wire` AND
    // the type is not POB-eligible; otherwise it DEFERS to `None` (graceful demotion
    // — a non-`Wire` field does NOT hard-error, matching the `SerPodTuple` house
    // style).
    //
    // A ZERO-field struct (unit / empty tuple struct) emits NEITHER: it is a ZST tag
    // with no field bytes, so it stays at the trait `None` defaults (a POB ZST tag is
    // blitted as zero bytes; a non-POB ZST tag encodes zero bytes via the zero-length
    // ViaFn path). Emitting a `WireBridge` over `()` would also trip the `unused_unit`
    // clippy lint at the call site. An enum/union (no field shape) likewise emits
    // neither — a `SerializeViaFn`-classified enum stays zero-length (a documented
    // S1.5 gap; enum encoding is a later macro phase).
    let (wire_fn_items, wire_bridge_item) = if is_plain_struct && !field_access.is_empty() {
        let bridge = wire_bridge_codegen(name, &field_access, &field_tys);
        let fns = wire_fn_overrides_codegen(name);
        (fns, bridge)
    } else {
        (TokenStream2::new(), TokenStream2::new())
    };

    let trait_items = quote! {
        #[inline]
        fn serializability_runtime()
            -> ::boyko_ecs::ecs::core::component::component_registry::Serializability {
            // Bring the autoref-arm traits into scope so method resolution can pick
            // the `&&` (POB) / `&` (ViaFn) arms over the by-value (Ignore) inherent.
            use ::boyko_ecs::ecs::core::component::component_registry::{
                SerIgnoreArm as _, SerViaFnArm as _, SerPobArm as _,
            };
            // The probe is invoked through THREE refs (`(&&&probe).method()`): the
            // resolver reaches all three `&self` arms and picks the highest-priority
            // APPLICABLE one — most-ref'd `Self` wins (`&&` = POB → `&` = ViaFn →
            // by-value = Ignore). The ref count MUST stay `&&&` (a `&&` call would
            // misclassify). `POB_ELIGIBLE` suppresses the POB arm for a non-repr-C /
            // Entity-bearing type; the `Fields: SerPodTuple` bound on the POB arm
            // additionally suppresses it when a field is not all-bits-valid (C3) —
            // a deferral (demote to ViaFn), never a hard error.
            let probe = ::boyko_ecs::ecs::core::component::component_registry::SerializeProbe::<
                #name, #field_tuple, #pob_flag,
            >::new();
            (&&&probe).serializability()
        }

        #wire_fn_items

        #format_version_item
        #fingerprint_item
        #stable_name_item
    };

    // Module-scope item: the `WireBridge` impl (a top-level `impl`, cannot live
    // inside the `impl Component` block). The field-validity proof still lives in the
    // glue's `C::Owned: WireTuple` bound (checked by the `WireFnProbe` arm), NOT a
    // per-struct bound — so the bridge compiles even for a struct with a non-`Wire`
    // field, and that field merely demotes the component to `serialize_fn = None`.
    Ok((trait_items, wire_bridge_item))
}

/// Phase S1.5 (plan §3.7): emits the bound-free `WireBridge` impl mapping a plain
/// struct to its field tuple. `Owned = (F0, F1, …)` with `from_owned(t) =
/// Name { f0: t.0, … }` / `Name(t.0, …)` (declaration order, the read constructor);
/// `Refs<'a> = (&'a F0, …)` with `as_refs(&self) = (&self.f0, …)` (the write source —
/// BORROWS, never a field move-out, so a `Drop` component is fine — `E0509`-free).
/// Carries NO `Wire` bound — the requirement lives on the generic glue's
/// `WireRefTuple` / `WireTuple` bounds, so this compiles for ANY plain struct (a
/// non-`Wire` field merely makes the encode-fn arm defer to `None`). A unit struct
/// maps to `Owned = ()` / `Refs<'a> = ()`.
fn wire_bridge_codegen(
    name: &Ident,
    field_access: &[FieldAccess],
    field_tys: &[Type],
) -> TokenStream2 {
    // `as_refs`: borrow each field into the ref tuple in declaration order. NO move,
    // NO clone — works for a `Drop` component (a field move-out would be `E0509`).
    let ref_terms: Vec<TokenStream2> = field_access
        .iter()
        .map(|acc| {
            let sel = acc.offset_of_selector();
            quote! { &self.#sel }
        })
        .collect();

    // `from_owned`: rebuild the struct from the decoded owned tuple. Named structs
    // use `Name { f0: owned.0, … }`; tuple structs use `Name(owned.0, …)`. The caller
    // only invokes this for a struct with at least one field (a zero-field ZST tag
    // emits no bridge), so `field_access` is never empty here.
    let is_tuple_struct = matches!(field_access.first(), Some(FieldAccess::Index(_)));
    let from_body = if is_tuple_struct {
        let owned_idx = (0..field_access.len()).map(syn::Index::from);
        quote! { #name( #( owned.#owned_idx ),* ) }
    } else {
        let field_idents: Vec<TokenStream2> = field_access
            .iter()
            .map(|acc| acc.offset_of_selector())
            .collect();
        let owned_idx = (0..field_access.len()).map(syn::Index::from);
        quote! { #name { #( #field_idents: owned.#owned_idx ),* } }
    };

    quote! {
        impl ::boyko_ecs::ecs::core::component::component_registry::WireBridge for #name {
            type Owned = ( #(#field_tys,)* );
            type Refs<'__boyko_wire> = ( #( &'__boyko_wire #field_tys, )* )
            where
                Self: '__boyko_wire;

            #[inline]
            fn as_refs(&self) -> Self::Refs<'_> {
                ( #(#ref_terms,)* )
            }

            #[inline]
            fn from_owned(owned: Self::Owned) -> Self {
                #from_body
            }
        }
    }
}

/// Phase S1.5 (plan §3.7): emits the `serialize_fn()` / `deserialize_fn()` trait
/// overrides selecting the per-element glue through the `WireFnProbe` autoref arm.
/// Invoked through TWO refs (`(&&probe).method()`): the `&`-`Self` "some" arm
/// (gated `C: WireBridge`, `for<'a> C::Refs<'a>: WireRefTuple`, `C::Owned: WireTuple`)
/// wins when every field is `Wire`; otherwise the by-value "none" arm wins (`None`).
/// The ref count MUST stay `&&` to agree with the arm receiver depths (`&Self` some /
/// by-value none).
///
/// The probe does NOT key on POB-eligibility — a `#[repr(C)]`-but-not-all-bits-valid
/// struct (a `String` field) is POB-eligible syntactically yet `SerializeViaFn` at
/// runtime, so it must still install the encoder. `install_serialize_fn` gates the
/// resulting `Some` on the runtime `Serializability`, dropping a genuinely
/// `PlainOldBytes` component back to `None` (the blit path).
fn wire_fn_overrides_codegen(name: &Ident) -> TokenStream2 {
    quote! {
        #[inline]
        fn serialize_fn()
            -> ::std::option::Option<
                ::boyko_ecs::ecs::core::component::component_registry::SerializeFn
            > {
            use ::boyko_ecs::ecs::core::component::component_registry::{
                WireFnNoneArm as _, WireFnSomeArm as _,
            };
            let probe = ::boyko_ecs::ecs::core::component::component_registry::WireFnProbe::<
                #name,
            >::new();
            (&&probe).serialize_fn_ptr()
        }

        #[inline]
        fn deserialize_fn()
            -> ::std::option::Option<
                ::boyko_ecs::ecs::core::component::component_registry::DeserializeFn
            > {
            use ::boyko_ecs::ecs::core::component::component_registry::{
                WireFnNoneArm as _, WireFnSomeArm as _,
            };
            let probe = ::boyko_ecs::ecs::core::component::component_registry::WireFnProbe::<
                #name,
            >::new();
            (&&probe).deserialize_fn_ptr()
        }
    }
}

/// Serialization S0 (C4 acceptance-only): rejects an unknown `#[entities]` field
/// attribute usage shape and otherwise accepts it. v1 does NOT auto-emit a remap
/// from `#[entities]` (that is S2+, built on the hand-written `ChildOf` path); S0
/// only PARSES the attribute so a user can annotate `#[entities] target: Entity`
/// without a compile error, and so a future phase can wire the remap. An
/// `#[entities]` on a non-field position is impossible (it is a field attribute);
/// a malformed `#[entities(...)]` with arguments is rejected (none are defined).
fn validate_entities_attrs(input: &DeriveInput) -> Result<(), TokenStream> {
    let check = |attrs: &[syn::Attribute]| -> Result<(), TokenStream> {
        for attr in attrs {
            if !attr.path().is_ident("entities") {
                continue;
            }
            // S0 accepts only the bare `#[entities]` form (a path-style attribute).
            // Any argument list is reserved for a future phase and rejected now so a
            // typo is loud rather than silently ignored.
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[entities] takes no arguments in this version; write a bare \
                     `#[entities]` on the Entity-bearing field (its remap is wired \
                     in a later serialization phase)",
                )
                .to_compile_error()
                .into());
            }
        }
        Ok(())
    };

    match &input.data {
        Data::Struct(s) => {
            for f in s.fields.iter() {
                check(&f.attrs)?;
            }
        }
        Data::Enum(e) => {
            for v in &e.variants {
                for f in v.fields.iter() {
                    check(&f.attrs)?;
                }
            }
        }
        Data::Union(u) => {
            for f in &u.fields.named {
                check(&f.attrs)?;
            }
        }
    }
    Ok(())
}

/// Serialization S2.5 (C4): collects the field accessors of every `#[entities]`-
/// annotated field of a plain struct (named or tuple), in declaration order.
///
/// Returns `None` for an enum / union (S2.5 supports `#[entities]` only on a plain
/// struct field — an annotated enum-variant field is parse-accepted by
/// [`validate_entities_attrs`] but emits no remap, a documented v1 gap mirroring
/// the `WireBridge` "plain struct only" boundary) and for a struct with no
/// annotated field. A non-annotated `Entity` field is NOT collected — the C4
/// explicit-opt-in decision (a plain `Entity` field without `#[entities]` keeps its
/// raw saved id on load).
fn entities_field_accessors(input: &DeriveInput) -> Option<Vec<FieldAccess>> {
    let has_entities_attr = |f: &syn::Field| f.attrs.iter().any(|a| a.path().is_ident("entities"));
    let Data::Struct(s) = &input.data else {
        return None;
    };
    let accessors: Vec<FieldAccess> = match &s.fields {
        Fields::Named(named) => named
            .named
            .iter()
            .filter(|f| has_entities_attr(f))
            .map(|f| FieldAccess::Named(f.ident.clone().expect("named field has ident")))
            .collect(),
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .filter(|(_, f)| has_entities_attr(f))
            .map(|(i, _)| FieldAccess::Index(i))
            .collect(),
        Fields::Unit => Vec::new(),
    };
    if accessors.is_empty() {
        None
    } else {
        Some(accessors)
    }
}

/// Serialization S2.5 (C4): emits the `map_entities_fn()` trait override for a
/// component with at least one `#[entities]`-annotated field, plus the monomorphized
/// free fn it returns. The fn rewrites each annotated `Entity` field in place via the
/// load-direction saved→fresh [`LoadEntityMap`], returning
/// [`DecodeError::UnmappedEntity`] on a dangling saved id (loud — never a silent
/// stale reference). A component with NO annotated field emits nothing (the trait
/// default `map_entities_fn() == None` applies — the field keeps its raw saved id).
///
/// Each annotated field is taken to be a plain `Entity` (the v1 shape the test suite
/// and the hand-written `ChildOf` cover); a wrapped form (`Option<Entity>` /
/// `Vec<Entity>`) is out of v1 scope (a documented boundary — the generated body
/// calls `Entity::id()` directly, so a non-`Entity` annotated field is a loud type
/// error at the user's struct, not a silent skip).
///
/// Returns `(trait_items, module_items)`: the `map_entities_fn()` override (spliced
/// into `impl Component`) and the module-scope free fn (it cannot live inside the
/// `impl` block as a free fn).
fn entities_map_codegen(input: &DeriveInput, name: &Ident) -> (TokenStream2, TokenStream2) {
    let Some(accessors) = entities_field_accessors(input) else {
        return (TokenStream2::new(), TokenStream2::new());
    };
    entities_map_codegen_for_accessors(name, &accessors)
}

/// Serialization S2.5 (C4) — the accessor-driven core of [`entities_map_codegen`],
/// also reused by the Relations v1 source side (C2/B11). Emits the
/// `map_entities_fn()` override + the monomorphized load-remap free fn for the given
/// `accessors` (each a plain `Entity` field). The `#[entities]`-driven path and the
/// relationship-foreign-key path share ONE codegen so the load-remap is byte-identical
/// to the hand-mirror `child_of_load_map_entities`. `accessors` is never empty here
/// (both callers gate on a non-empty list).
fn entities_map_codegen_for_accessors(
    name: &Ident,
    accessors: &[FieldAccess],
) -> (TokenStream2, TokenStream2) {
    // Per-field in-place remap: look up the saved id, rewrite on a hit, error on a
    // miss. `&mut (*value).<sel>` is a `&mut Entity` (a non-`Entity` annotated field
    // is a loud type error at `Entity::id`).
    let field_remaps: Vec<TokenStream2> = accessors
        .iter()
        .map(|acc| {
            let sel = acc.offset_of_selector();
            quote! {
                {
                    let __field: &mut ::boyko_ecs::ecs::core::entity::entity::Entity =
                        &mut (*__value).#sel;
                    match __map.get(__field.id().0) {
                        ::std::option::Option::Some(__fresh) => { *__field = __fresh; }
                        ::std::option::Option::None => {
                            return ::std::result::Result::Err(
                                ::boyko_ecs::ecs::core::serialize::DecodeError::UnmappedEntity,
                            );
                        }
                    }
                }
            }
        })
        .collect();

    let fn_ident = format_ident!("__boyko_map_entities_{}", name);

    let module_items = quote! {
        /// Serialization S2.5 — derive-generated load-direction entity-remap for a
        /// `#[entities]`-bearing component. Rewrites each annotated `Entity` field
        /// through the saved→fresh `LoadEntityMap`; errors loudly on an unmapped
        /// saved id (C4). Panic-free by construction.
        ///
        /// # Safety
        /// `value` points at a live, initialized `Self`; `__map` is shared and not
        /// aliased mutably (the `LoadMapEntitiesFn` contract).
        #[doc(hidden)]
        #[allow(non_snake_case)]
        unsafe fn #fn_ident(
            value: *mut u8,
            __map: &::boyko_ecs::ecs::core::serialize::LoadEntityMap,
        ) -> ::std::result::Result<(), ::boyko_ecs::ecs::core::serialize::DecodeError> {
            // SAFETY: `value` is a live, initialized `#name` (the load remap pass
            //   derives it from the pool's live-row pointer). We form `&mut #name`
            //   to rewrite its annotated `Entity` field(s) in place; no other
            //   reference aliases it (single-threaded `&mut EcsMaster`).
            let __value: *mut #name = value.cast::<#name>();
            unsafe {
                #(#field_remaps)*
            }
            ::std::result::Result::Ok(())
        }
    };

    let trait_items = quote! {
        #[inline]
        fn map_entities_fn()
            -> ::std::option::Option<
                ::boyko_ecs::ecs::core::component::component_registry::LoadMapEntitiesFn
            > {
            ::std::option::Option::Some(
                #fn_ident
                    as ::boyko_ecs::ecs::core::component::component_registry::LoadMapEntitiesFn
            )
        }
    };

    (trait_items, module_items)
}
