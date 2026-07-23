//! Procedural macros for the `boyko-engine` ECS.
//!
//! Each `#[proc_macro*]` entry point below is a thin delegator whose full
//! documentation lives on the entry itself; the implementation is in the
//! sibling module of the same name (`component`, `relationship`, `resource`,
//! `event`, `bundle`, `system_set`, `actionlike`, `ui`, `bindable`). Genuinely
//! shared helpers live in `common`.
//!
//! `boyko-macros` has NO dependency on `boyko-ecs` / `boyko-ui` / `boyko-input`:
//! every `boyko_ecs::…` (etc.) path a derive produces is emitted as a TOKEN
//! inside `quote!` and resolved in the downstream consumer crate.

mod actionlike;
mod bindable;
mod bundle;
mod common;
mod component;
mod event;
mod relationship;
mod resource;
mod system_set;
mod ui;

use proc_macro::TokenStream;


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
/// macros. `boyko-macros` has NO dependency on `boyko-ecs`: every `boyko_ecs::…`
/// path the derives produce is emitted as a TOKEN inside `quote!` and resolved in
/// the downstream crate (which brings its own `boyko-ecs`), so the proc-macro
/// crate needs no such dependency of its own. Real usage lives in `boyko-ecs`
/// integration tests.
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
/// Derive hooks and the runtime `register_component_hooks` builder are
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
#[proc_macro_derive(
    Component,
    attributes(component, require, entities, relationship, relationship_target)
)]
pub fn component_macro(input: TokenStream) -> TokenStream {
    component::expand(input)
}

/// Derive macro for the source-of-truth side of a relation — `Relationship`.
///
/// ADDITIVE over `#[derive(Component)]` (both are required): this derive emits ONLY
/// the `impl Relationship` trait block; the paired `#[derive(Component)]` reads the
/// same `#[relationship(target = …)]` attribute and folds the generic hook wiring +
/// the auto entity-remap clone/serialize metadata + the layout fingerprint into its
/// `register_hooks` / `component_id()` (Decision 4). The component is the foreign key
/// on the SOURCE entity pointing at one target.
///
/// # Attribute
///
/// `#[relationship(target = <Type>)]` — `target` (the `RelationshipTarget` component)
/// is required. Optional bare `allow_self_referential` permits a self-link (default:
/// a self-link is reactively removed by the generic `on_insert` guard).
///
/// # Foreign-key field
///
/// A single-field tuple / named struct uses that field; a multi-field struct uses the
/// field annotated `#[relationship]` (exactly one). The field type must be `Entity`
/// (enforced by the generated `target()` body). A unit struct is a compile error.
///
/// ```ignore
/// #[derive(Component, Relationship)]
/// #[relationship(target = LikedBy)]
/// struct Likes(pub Entity);
/// ```
///
/// The example is `ignore`'d for the same reason as `#[derive(Component)]`: a
/// proc-macro crate cannot consume its own macros, and `boyko-macros` cannot depend on
/// `boyko-ecs` for tests. Real usage lives in `boyko-ecs` integration tests.
#[proc_macro_derive(Relationship, attributes(relationship))]
pub fn relationship_macro(input: TokenStream) -> TokenStream {
    relationship::expand(input)
}

/// Derive macro for the reverse-index side of a relation — `RelationshipTarget`.
///
/// ADDITIVE over `#[derive(Component)]` (both are required): this derive emits ONLY
/// the `impl RelationshipTarget` trait block; the paired `#[derive(Component)]` reads
/// the same `#[relationship_target(...)]` attribute and folds the cascade hook wiring
/// (ONLY `on_replace`, never `on_add`/`on_insert` — B7) + the `Cloneability::Ignore`
/// classification (the reverse index is rebuilt via Link commands, never byte-copied —
/// B12) into its `register_hooks` / `component_id()` (Decision 4). User code must NEVER
/// write the reverse index; the macro enforces the single collection field is PRIVATE.
///
/// # Attribute
///
/// `#[relationship_target(source = <Type> [, linked_despawn] [, retain_empty])]` —
/// `source` (the `Relationship` component) is required. Bare `linked_despawn` sets
/// `LINKED_DESPAWN = true` (despawning the target recursively despawns its sources);
/// bare `retain_empty` sets `RETAIN_EMPTY = true`. **v1 requires `retain_empty`** —
/// `RETAIN_EMPTY = false` (remove-on-empty) is deferred to v1.1 (W1).
///
/// ```ignore
/// #[derive(Component, RelationshipTarget)]
/// #[relationship_target(source = Likes, linked_despawn, retain_empty)]
/// struct LikedBy(Vec<Entity>);
/// ```
///
/// The example is `ignore`'d for the same reason as `#[derive(Component)]`.
#[proc_macro_derive(RelationshipTarget, attributes(relationship_target))]
pub fn relationship_target_macro(input: TokenStream) -> TokenStream {
    relationship::expand_target(input)
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
    resource::expand(input)
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
    event::expand(_args, input)
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
/// * More than `MAX_BUNDLE_ARITY` (16) fields — the runtime apply paths use
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
    bundle::expand(input)
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
    system_set::expand(input)
}

/// Derive macro for the [`Actionlike`] trait — `boyko_input` I2.
///
/// Generates `impl Actionlike for #Name` from a **fieldless enum**, emitting:
///
/// * `const COUNT` — the variant count, with a `const` assert that
///   `COUNT <= 256` (the `BitSet256` action cap, plan V8).
/// * `index(self)` — the dense `0..COUNT` declaration-order index.
/// * `from_index(i)` — the inverse (`None` for `i >= COUNT`).
/// * `kind(self)` — the per-variant `ActionKind`, selected by the optional
///   `#[actionlike(Button|Axis1D|Axis2D)]` field attribute (default `Button`).
/// * `name(self)` — the variant's identifier as a `&'static str`.
///
/// The `Actionlike` trait and `ActionKind` enum live in `boyko_input`; the
/// derive references them by absolute path (`::boyko_input::…`), so
/// `boyko_macros` needs no dependency on `boyko_input` (the path resolves at the
/// consumer's expansion site — the same pattern `#[derive(Component)]` uses for
/// `::boyko_ecs::…`).
///
/// # Supported inputs
///
/// * `enum PlayerAction { Jump, #[actionlike(Axis2D)] Move, Fire }` — a
///   non-generic, fieldless enum. Each variant is one action.
///
/// # Rejected inputs
///
/// * `struct`/`union` — `compile_error!` (actions are a closed enum set).
/// * Generic enum — `compile_error!` (a generic action set defeats the
///   fixed `[…; COUNT]` array sizing).
/// * Data-carrying variant — `compile_error!` (only fieldless variants have a
///   stable dense index).
/// * Empty enum — `compile_error!` (`COUNT == 0` has no usable action space).
/// * Unknown `#[actionlike(...)]` value — `compile_error!`.
///
/// # Example
///
/// ```ignore
/// use boyko_macros::Actionlike;
///
/// #[derive(Actionlike, Clone, Copy, PartialEq, Eq)]
/// enum PlayerAction {
///     Jump,                         // default kind: Button
///     #[actionlike(Axis2D)] Move,
///     Fire,
///     #[actionlike(Axis1D)] Throttle,
/// }
/// ```
///
/// The example is `ignore`'d because proc-macro crates cannot consume their own
/// macros and `boyko-macros` cannot depend on `boyko-input` (cycle). Real usage
/// lives in `boyko-input` integration tests.
#[proc_macro_derive(Actionlike, attributes(actionlike))]
pub fn actionlike_macro(input: TokenStream) -> TokenStream {
    actionlike::expand(input)
}

/// Function-like macro: author a UI entity tree as a literal nested block (GUI
/// Phase P2).
///
/// The macro expands to a block expression that runs against a `Commands`
/// binding in scope (default name `cmds`; override with `commands: <ident>;` as
/// the first clause). It evaluates to the root `Entity` — or, for several
/// top-level nodes, a tuple of root `Entity`s.
///
/// # Grammar
///
/// ```text
/// ui!         := preamble? node ( ',' node )* ','?
/// preamble    := 'commands' ':' IDENT ';'
/// node        := name? '{' body '}'
/// name        := '#' IDENT                       // declares a let-binding + UiName
/// body        := items? children?
/// items       := component_item ( ',' component_item )* ','?
/// component_item := EXPR                          // a real Rust component literal
/// children    := 'children' ':' '[' ( node ( ',' node )* ','? )? ']'
/// ```
///
/// Each node lowers to `cmds.spawn(<base>)` plus chained `.insert(<literal>)` for
/// every remaining component, then one standalone `cmds.entity(parent).add_child(child)`
/// per link. A node whose component set contains BOTH `UiLayout` and `ComputedRect`
/// spawns the canonical `UiNodeBundle` (hitting the Phase-8.5 static archetype
/// cache); otherwise it spawns the `UiLayout` literal and injects
/// `ComputedRect::default()`. A node without any `UiLayout` literal is a compile
/// error.
///
/// `#name` declares a `let name = <spawned Entity>;` binding (the user's ident)
/// plus a `UiName::new("name")` component, so the handle is usable *after* the
/// invocation. Value-position `#name` references *inside* a component field
/// expression are **not** supported in P2: a bare `#ident` is not valid Rust
/// expression syntax, so it cannot appear inside a component literal (each
/// component item is parsed as a `syn::Expr`). Cross-node entity wiring is done
/// by reading the post-invocation `let` bindings instead.
///
/// `children` and `commands` are reserved context keywords at their positions; a
/// component type literally named `children`/`commands` must be path-qualified.
///
/// # Example
///
/// ```ignore
/// // In a downstream crate that depends on boyko-ui (and has a `cmds: Commands`).
/// use boyko_ui::prelude::*;
///
/// let root = ui! {
///     UiLayout { layout_type: LayoutType::Column, ..Default::default() },
///     UiRoot,
///     children: [
///         #header { UiLayout { height: Unit::Px(48.0), ..Default::default() } },
///         { UiLayout { height: Unit::Px(48.0), ..Default::default() } }
///     ]
/// };
/// ```
///
/// The example is `ignore`'d: a proc-macro crate cannot consume its own macros,
/// and `boyko-macros` cannot depend on `boyko-ui` (a cycle). Real usage lives in
/// `boyko-ui` integration tests.
#[proc_macro]
pub fn ui(input: TokenStream) -> TokenStream {
    ui::expand(input)
}

/// Derive macro for `boyko_ui::binding::Bindable` (GUI P4 Decision 7).
///
/// Generates the per-component, **reflection-free** data-binding accessor: a
/// `u8`-indexed `fmt_field` / `value_field` over the struct's fields, a
/// parse-time `field_id(name) -> Option<u8>` name→index resolver, and a
/// `register_bind_accessor()` that installs the type-erased `BindAccessor`
/// (fn-pointer pair) into the registry's `BIND_ACCESSORS` table.
///
/// Only **named structs** are supported (binding is by field name). Each field
/// is assigned its declaration ordinal as its `u8` id; `field_id` maps the field
/// name string to that id at parse time (cold). `fmt_field` formats the field
/// via `core::write!`; `value_field` returns the field cast to `f32` (numeric
/// fields). Both are `match field { .. _ => /* no-op / 0.0 */ }`, so an
/// out-of-range id is a silent no-op (the release path; debug-asserted by the
/// caller).
///
/// No `Box<dyn Fn>`, no `serde`, no `Any` / `TypeId` — the field identity is a
/// compile-time `u8` resolved once at parse/spawn (Principle 1).
///
/// # Example
///
/// ```ignore
/// #[derive(Component, Bindable)]
/// #[repr(C)]
/// struct Health { current: f32, max: f32 }
/// // field_id("current") == Some(0); field_id("max") == Some(1).
/// ```
#[proc_macro_derive(Bindable, attributes(bind))]
pub fn bindable_macro(input: TokenStream) -> TokenStream {
    bindable::expand(input)
}
