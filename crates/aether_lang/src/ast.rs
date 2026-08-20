//! The Aether AST — one node per construct, kept deliberately shallow.
//!
//! Bodies, types and paths are **verbatim `syn` fragments with the user's spans** (§2): the AST
//! carries structure, never re-lexed text, so downstream rustc errors land on the user's tokens.

use proc_macro2::{Span, TokenStream};
use syn::{Expr, Ident, Path, Type};

/// One parsed `aether!` block: a flat list of constructs sharing one parse context.
pub struct AetherBlock {
    /// The `aether vN;` header's version (§6.3). Absent header = [`SyntaxVersion::CURRENT`].
    pub version: SyntaxVersion,
    /// The constructs that PARSED, in source order (emission preserves it — deterministic output
    /// is what makes the unit-test snapshots exact).
    pub constructs: Vec<Construct>,
    /// The constructs that did NOT parse, in source order — §7.3's recovery record. Empty for
    /// every well-formed block, so the common path pays nothing for it.
    pub broken: Vec<BrokenConstruct>,
}

/// The syntax version an `aether!` block is written against (§6.3).
///
/// ONE table, spelling and dispatch token together — the `MATERIAL_KEYS` discipline: what the
/// diagnostic PRINTS and what the parser ACCEPTS are the same rows, so neither can gain a version
/// the other lacks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyntaxVersion {
    /// `aether v1;` — the version this crate speaks.
    V1,
}

/// Every version this aether accepts, in order — the parse table and the "this aether speaks: …"
/// list, from one source.
const SYNTAX_VERSIONS: &[(&str, SyntaxVersion)] = &[("v1", SyntaxVersion::V1)];

impl SyntaxVersion {
    /// The version a header-less block is read as: §6.3's "absent = the crate's current default".
    pub const CURRENT: SyntaxVersion = SyntaxVersion::V1;

    /// Parse a version from its surface spelling, `None` for anything else.
    pub fn from_str(s: &str) -> Option<SyntaxVersion> {
        SYNTAX_VERSIONS.iter().find(|(k, _)| *k == s).map(|(_, v)| *v)
    }

    /// The accepted spellings, for the unknown-version diagnostic (§7.1: exhaustive, in table
    /// order) and its did-you-mean candidate set.
    pub fn spellings() -> Vec<&'static str> {
        SYNTAX_VERSIONS.iter().map(|(k, _)| *k).collect()
    }

    /// This version's surface spelling — the same table the parser dispatches on, so a message
    /// that offers `aether v1;` cannot advertise a header the parser would reject.
    pub fn spelling(self) -> &'static str {
        SYNTAX_VERSIONS
            .iter()
            .find(|(_, v)| *v == self)
            .map(|(k, _)| *k)
            .expect("invariant: every SyntaxVersion variant has a SYNTAX_VERSIONS row")
    }
}

/// One construct the parser could not read, kept so the expander can honor §7.3's recovery
/// contract: the error at its own span, a name-resolving stub, and the FULL expansion of every
/// sibling construct in the block.
///
/// A broken construct is not a hole in the block. It still PARTICIPATES in every whole-block rule
/// by NAME and KIND — it holds its name, and a broken `plugin` still means "this block has a
/// plugin". Rules that read it as absent produce faults that DERIVE from the fault already
/// reported: `clauses need a plugin` under a half-typed `plugin`, or a duplicate that goes
/// unnoticed because one of the two names was unreadable. §4 runs over the union
/// ([`crate::ctx`]); only rules whose failure could not exist without the break stay suppressed.
pub struct BrokenConstruct {
    /// The failure, at the offending token's own span.
    pub error: syn::Error,
    /// The construct's keyword when it was one of the registry's — the whole-block rules key on
    /// it. `None` for an unknown head (nothing about its kind is knowable).
    ///
    /// `&'static str`: the value is the [`crate::diag::CONSTRUCT_KEYWORDS`] row, not the user's
    /// spelling, so a diagnostic printing it cannot print a typo back at the reader as if it were
    /// a construct name.
    pub keyword: Option<&'static str>,
    /// How many constructs PARSED before this one — the block-order key. Source order across the
    /// two lists is what a duplicate diagnostic needs ("the first … is here" must point at the
    /// earlier declaration), and it is not recoverable from two separate vectors otherwise.
    pub after: usize,
    /// The best-effort stub — `None` when the construct's NAME never parsed (an unknown keyword,
    /// or a head that is not an ident at all), because a stub needs a name to declare.
    pub stub: Option<Stub>,
}

impl BrokenConstruct {
    /// The declared name, when one was parsed — the whole-block rules' key.
    pub fn name(&self) -> Option<&Ident> {
        self.stub.as_ref().map(Stub::name)
    }
}

/// The item shape a §7.3 recovery stub takes: the construct's own kind, so a downstream reference
/// to its name keeps resolving while the author is still typing.
pub enum Stub {
    /// A type-producing construct (`component`, `tag`, `bundle`, `event`, `machine`).
    Type(Ident),
    /// `plugin` — a type AND the `Plugin` impl its only use site needs.
    ///
    /// Split from [`Stub::Type`] by what "keeps resolving" MEANS for this kind: every reference to
    /// a plugin is `app.add_plugin(P)`, which needs the trait, not the name. A bare `pub struct P;`
    /// turns the unresolved-name error into an unsatisfied-trait-bound error at the same call site
    /// — a different error, not one fewer.
    Plugin(Ident),
    /// A fn-producing construct (`system`, `material`, `scene`).
    Fn(Ident),
}

impl Stub {
    /// The stub shape for a construct KEYWORD.
    ///
    /// The recovery path's mirror of [`Construct::emits_fn`], which needs a parsed construct that
    /// a FAILED parse never produced. The two are stated beside each other because a construct
    /// added to one and not the other would stub as the wrong item kind — a `material` stubbed as
    /// a struct resolves the name and then fails at every call site, which is exactly the
    /// cascade §7.3 exists to prevent. `aether_lang`'s unit tests pin them equal, keyword by
    /// keyword.
    ///
    /// `None` for a keyword outside the registry: an unknown construct has no known item kind, so
    /// there is nothing honest to declare.
    pub fn for_keyword(keyword: &str, name: Ident) -> Option<Stub> {
        match keyword {
            "system" | "material" | "scene" => Some(Stub::Fn(name)),
            "plugin" => Some(Stub::Plugin(name)),
            "component" | "tag" | "bundle" | "event" | "machine" => Some(Stub::Type(name)),
            _ => None,
        }
    }

    /// The stubbed construct's name.
    pub fn name(&self) -> &Ident {
        match self {
            Stub::Type(n) | Stub::Plugin(n) | Stub::Fn(n) => n,
        }
    }

    /// `true` iff this construct's name occupies a FN item — [`Construct::emits_fn`] for the
    /// recovery half, so §4's duplicate rule draws its line at the same place on both.
    pub fn emits_fn(&self) -> bool {
        matches!(self, Stub::Fn(_))
    }
}

/// Every construct in the v1 registry (§6.1) — complete as of rung A6. A construct is one
/// variant, one parser row and one expander section; §6.1's extensibility claim was checked
/// against each of the nine as it landed.
pub enum Construct {
    /// `component NAME { fields / requires / hooks / no_bundle }`
    Component(ComponentDef),
    /// `tag NAME;` / `tag NAME(bitset);`
    Tag(TagDef),
    /// `bundle NAME { field: Type, … }` (rung A1)
    Bundle(BundleDef),
    /// `event NAME { participant/parameter fields }` (rung A1)
    Event(EventDef),
    /// `system NAME(params) clauses { body }` (rung A2)
    System(SystemDef),
    /// `plugin NAME;` (rung A2) — the block's registration holder.
    Plugin(PluginDef),
    /// `machine NAME { initial X; state* }` (rung A3) — Harel-lite, flattened at expansion.
    Machine(MachineDef),
    /// `material NAME { base: (…), … }` (rung A5) — a PBR material builder fn.
    ///
    /// BOXED: `MaterialDef` holds seven optional `syn::Expr` slots, and `syn::Expr` (with the
    /// `full` feature) is a ~200-byte enum — inline, this one variant would have made EVERY
    /// `Construct` in the block's `Vec` 952 bytes. The `Box` costs one build-time allocation per
    /// declared material; the hot-path `Box` ban is a RUNTIME rule and this is transpiler state
    /// (the same build-time exemption §4 grants `AetherCtx`'s keyed collections).
    Material(Box<MaterialDef>),
    /// `scene NAME { let … ; node* }` (rung A6) — a spawn fn over the engine's own bundles.
    Scene(SceneDef),
}

impl Construct {
    /// The construct's surface keyword — the noun every cross-construct diagnostic prints.
    pub fn keyword(&self) -> &'static str {
        match self {
            Construct::Component(_) => "component",
            Construct::Tag(_) => "tag",
            Construct::Bundle(_) => "bundle",
            Construct::Event(_) => "event",
            Construct::System(_) => "system",
            Construct::Plugin(_) => "plugin",
            Construct::Machine(_) => "machine",
            Construct::Material(_) => "material",
            Construct::Scene(_) => "scene",
        }
    }

    /// The declared name, for the §4 duplicate-symbol rule.
    pub fn name(&self) -> &Ident {
        match self {
            Construct::Component(d) => &d.name,
            Construct::Tag(d) => &d.name,
            Construct::Bundle(d) => &d.name,
            Construct::Event(d) => &d.name,
            Construct::System(d) => &d.name,
            Construct::Plugin(d) => &d.name,
            Construct::Machine(d) => &d.name,
            Construct::Material(d) => &d.name,
            Construct::Scene(d) => &d.name,
        }
    }

    /// The Rust ITEM KIND this construct's name occupies (§4's duplicate rule keys on it): a
    /// `fn` for the name-is-a-value constructs, a type for the rest.
    ///
    /// Aether owns the duplicate diagnostic only for the `fn` half — see [`crate::ctx`] for the
    /// measurement that split the rule.
    pub fn emits_fn(&self) -> bool {
        matches!(self, Construct::System(_) | Construct::Material(_) | Construct::Scene(_))
    }

    /// The noun a duplicate-name diagnostic uses for the emitted fn ("each `material` expands to
    /// a *builder fn* of its own name").
    pub fn fn_noun(&self) -> &'static str {
        match self {
            Construct::Material(_) => "builder fn",
            Construct::Scene(_) => "spawn fn",
            _ => "fn",
        }
    }
}

/// The four Phase-14a hook keys the `component` construct forwards (§3.1). Mutually exclusive
/// with the runtime builder — the DERIVE enforces that; Aether just forwards the pairs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// `on_add = path`
    Add,
    /// `on_insert = path`
    Insert,
    /// `on_replace = path`
    Replace,
    /// `on_remove = path`
    Remove,
}

impl HookKind {
    /// The derive-attribute key this hook emits (`#[component(<key> = path)]`).
    pub fn key(self) -> &'static str {
        match self {
            HookKind::Add => "on_add",
            HookKind::Insert => "on_insert",
            HookKind::Replace => "on_replace",
            HookKind::Remove => "on_remove",
        }
    }

    /// Parse a hook key from its surface ident, `None` for anything else.
    pub fn from_str(s: &str) -> Option<HookKind> {
        match s {
            "on_add" => Some(HookKind::Add),
            "on_insert" => Some(HookKind::Insert),
            "on_replace" => Some(HookKind::Replace),
            "on_remove" => Some(HookKind::Remove),
            _ => None,
        }
    }
}

/// `component NAME { … }` (§3.1).
pub struct ComponentDef {
    /// The type name (uppercase by the §2 case convention — diagnosed at parse).
    pub name: Ident,
    /// `field: Type,` pairs, verbatim types.
    pub fields: Vec<(Ident, Type)>,
    /// `requires A, B::C,` — accumulated across multiple `requires` items.
    pub requires: Vec<Path>,
    /// `on_add = path,` etc. Duplicate KEYS are diagnosed at parse (the derive would too, but
    /// the parser owns the better span — the §3.1 pre-check rule).
    pub hooks: Vec<(HookKind, Path)>,
    /// `no_bundle,` — suppresses the Phase-22 single-component `Bundle` emission.
    pub no_bundle: bool,
}

/// `tag NAME;` / `tag NAME(bitset);` (§3.1) — a zero-data marker.
pub struct TagDef {
    /// The type name.
    pub name: Ident,
    /// `(bitset)` — the EnableTag backend (`storage = "bitset"`): O(1) toggle, no migration.
    pub bitset: bool,
}

/// `bundle NAME { … }` (§3.2) — pure surface uniformity: the derive owns every rule. Aether
/// pre-checks only the arity cap, because it owns the friendlier span (the 17th field's name).
pub struct BundleDef {
    /// The type name.
    pub name: Ident,
    /// `field: Type,` pairs, verbatim types.
    pub fields: Vec<(Ident, Type)>,
}

/// One `event` field (§3.4): a participant carries its component CONTEXT type-shaped
/// (`entity(A, B)`), a parameter is a plain typed field.
pub enum EvField {
    /// `name: entity(A, B),` → `#[participant(components = "A, B")] name: Entity`.
    Participant {
        /// The field name.
        name: Ident,
        /// The participant's component context — deliberately never defaulted (§3.4: the
        /// engine's participant contract wants it explicit).
        components: Vec<Path>,
    },
    /// `name: Type,` → `#[parameter] name: Type`.
    Parameter {
        /// The field name.
        name: Ident,
        /// The verbatim type.
        ty: Type,
    },
}

/// `event NAME { … }` (§3.4).
pub struct EventDef {
    /// The type name.
    pub name: Ident,
    /// The fields, in source order (the `#[event]` macro's two-band rewrite is ITS business).
    pub fields: Vec<EvField>,
}

/// The three `on` targets (§3.3). `None` on a [`SystemDef`] means Main — the engine's own
/// default schedule for `add_systems_cfg`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    /// `on startup` — runs once, pre-loop, via `add_startup_system`; rejects every other clause.
    Startup,
    /// `on update` — the Main schedule (`add_systems_cfg`).
    Update,
    /// `on fixed` — the fixed-timestep schedule (`add_systems_cfg_in(CoreSchedule::Fixed, …)`).
    Fixed,
}

/// `before X` / `after X` (§3.3) — the target is resolved at EXPAND time: a sibling aether
/// system name becomes a captured `SystemKey`, anything else is a `SystemSet` path.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    /// `before X`.
    Before,
    /// `after X`.
    After,
}

/// One `system` parameter's sugared type (§3.3's `param_ty` row). Everything the sugar does
/// not claim passes through [`SysParamTy::Verbatim`] — any real `SystemParam` works day one.
pub enum SysParamTy {
    /// `query<D>` / `query<D, filters>` → `Query<D, (F₁, …)>`. `D` is a verbatim type — the
    /// engine's `QueryData` trait is the authority; Aether never validates it.
    Query {
        /// The query data, verbatim (tuples, `&T`, `&mut T`, `Option<&T>`, …).
        data: Type,
        /// The sugar filters, in source order.
        filters: Vec<(FilterKind, Path)>,
    },
    /// `res<T>` → `Res<T>`.
    Res(Type),
    /// `mut res<T>` → `ResMut<T>` (and a `mut` binding, inferred).
    ResMut(Type),
    /// `local<T>` → `Local<T>`.
    Local(Type),
    /// `commands` → `Commands` (and a `mut` binding, inferred).
    Commands,
    /// `events<E>` → `EventReader<E>` (and a `mut` binding — `read()` takes `&mut self` in
    /// this engine, a recorded deviation from the plan's inference list, which omitted it).
    Events(Type),
    /// `emit<E>` → `EventWriter<E>` (and a `mut` binding, inferred).
    Emit(Type),
    /// The escape hatch: any real `SystemParam` type, verbatim.
    Verbatim(Type),
}

/// The six query-filter sugars (§3.3) — each maps to the kernel filter of the same name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    /// `with P` → `With<P>`.
    With,
    /// `without P` → `Without<P>`.
    Without,
    /// `added P` → `Added<P>`.
    Added,
    /// `changed P` → `Changed<P>`.
    Changed,
    /// `enabled P` → `Enabled<P>`.
    Enabled,
    /// `disabled P` → `Disabled<P>`.
    Disabled,
}

impl FilterKind {
    /// The kernel filter type name this sugar emits.
    pub fn type_name(self) -> &'static str {
        match self {
            FilterKind::With => "With",
            FilterKind::Without => "Without",
            FilterKind::Added => "Added",
            FilterKind::Changed => "Changed",
            FilterKind::Enabled => "Enabled",
            FilterKind::Disabled => "Disabled",
        }
    }

    /// Parse a filter sugar from its surface ident, `None` for anything else.
    pub fn from_str(s: &str) -> Option<FilterKind> {
        match s {
            "with" => Some(FilterKind::With),
            "without" => Some(FilterKind::Without),
            "added" => Some(FilterKind::Added),
            "changed" => Some(FilterKind::Changed),
            "enabled" => Some(FilterKind::Enabled),
            "disabled" => Some(FilterKind::Disabled),
            _ => None,
        }
    }
}

/// One `system` parameter: `mut? name: param_ty`.
pub struct SysParam {
    /// An explicit `mut` on the binding (inference may add one the user omitted).
    pub explicit_mut: bool,
    /// The binding name.
    pub name: Ident,
    /// The sugared type.
    pub ty: SysParamTy,
}

/// `system NAME(params) clauses BLOCK` (§3.3). The body is UNTOUCHED verbatim Rust — Aether
/// sugars the signature and the registration, never the code.
pub struct SystemDef {
    /// The fn name (snake_case by convention — diagnosed at parse).
    pub name: Ident,
    /// The parameters, in source order.
    pub params: Vec<SysParam>,
    /// The `on` clause; `None` defaults to Main at registration.
    pub schedule: Option<Schedule>,
    /// `in PATH` clauses (each emits `.in_set(PATH)`), with the clause keyword's span.
    pub in_sets: Vec<(Path, Span)>,
    /// `before`/`after` clauses in source order, with each clause keyword's span.
    pub orders: Vec<(OrderKind, Path, Span)>,
    /// `when EXPR` clauses (each emits `.run_if(EXPR)`), with the clause keyword's span.
    pub whens: Vec<(Expr, Span)>,
    /// The body tokens, verbatim, spans preserved.
    pub body: TokenStream,
}

impl SystemDef {
    /// `true` iff any scheduling clause is present — the §3.3 plugin-header requirement's
    /// trigger (a clause-free system is a plain fn and needs no plugin).
    pub fn has_clauses(&self) -> bool {
        self.schedule.is_some()
            || !self.in_sets.is_empty()
            || !self.orders.is_empty()
            || !self.whens.is_empty()
    }
}

/// `plugin NAME;` (§3.3) — at most one per block; holds every sibling system's registration.
pub struct PluginDef {
    /// The plugin type name (UpperCamelCase — it expands to a struct).
    pub name: Ident,
}

/// `machine NAME { initial X; state* }` (§3.5) — the hierarchy exists ONLY inside the
/// transpiler: leaves flatten to one enum, superstate handlers are copied into leaves,
/// LCA entry/exit sequences are inlined into the generated transition systems.
pub struct MachineDef {
    /// The machine enum name (UpperCamelCase — it expands to a type).
    pub name: Ident,
    /// The required top-level `initial` target (resolved through composite `initial`
    /// chains to a leaf at expansion).
    pub initial: Ident,
    /// Top-level states, in source order.
    pub states: Vec<StateDef>,
}

/// One `state` node (§3.5) — possibly composite (children non-empty).
pub struct StateDef {
    /// The state name (UpperCamelCase — leaves concatenate into enum variant names).
    pub name: Ident,
    /// `initial CHILD;` — required for a composite that is ever a transition target.
    pub initial: Option<Ident>,
    /// `enter (params)? { … }` — at most one per state (duplicate diagnosed at parse).
    pub enter: Option<HandlerDef>,
    /// `exit (params)? { … }` — at most one per state.
    pub exit: Option<HandlerDef>,
    /// `on EVENT … => target` transitions, in source order.
    pub transitions: Vec<TransitionDef>,
    /// Nested states, in source order. Empty ⇒ this is a LEAF.
    pub children: Vec<StateDef>,
}

/// An `enter`/`exit` action: optional system-grammar params + a verbatim body.
pub struct HandlerDef {
    /// Params, same grammar as `system` (§3.5: "same param grammar").
    pub params: Vec<SysParam>,
    /// The verbatim action body.
    pub body: TokenStream,
}

/// A `color` production (§3.6): `'(' EXPR ',' EXPR ',' EXPR (',' EXPR)? ')'`. The components are
/// verbatim exprs — a color channel may be any const/`f32` expression the user writes, not just a
/// literal.
pub struct ColorLit {
    /// 3 or 4 components for `base`, exactly 3 for `emissive`.
    ///
    /// Arity is validated at PARSE (where the parenthesized group's own span is in hand — §3.6
    /// puts the error on the tuple), so the expander is total and carries no span of its own: a
    /// field kept "for later" is a datum nothing reads.
    pub components: Vec<Expr>,
}

/// `material NAME { base: (…), metallic: EXPR, … }` (§3.6).
///
/// Materials are RUNTIME-MINTED assets (`Assets<Material>::add`), so the expansion target is a
/// builder fn over the engine's own constructors — a static table would be exactly the parallel
/// data system Principle 0 forbids.
pub struct MaterialDef {
    /// The builder-fn name (lowercase by the §2 case convention — diagnosed at parse).
    pub name: Ident,
    /// `base: (r, g, b)` / `(r, g, b, a)` — REQUIRED: §3.6's default table covers every other
    /// key and conspicuously omits this one, so Aether refuses rather than inventing a color.
    pub base: ColorLit,
    /// `metallic: EXPR` — defaults to `0.0`.
    pub metallic: Option<Expr>,
    /// `roughness: EXPR` — defaults to `0.5`.
    pub roughness: Option<Expr>,
    /// `reflectance: EXPR` — defaults to `0.5` (the standard 4% dielectric F0).
    pub reflectance: Option<Expr>,
    /// `emissive: (r, g, b)` — defaults to `[0.0; 3]`. Exactly 3 components: the engine's
    /// `Material::new` takes `emissive: [f32; 3]`.
    pub emissive: Option<ColorLit>,
    /// `flags: EXPR` — defaults to `0`.
    pub flags: Option<Expr>,
    /// `textures: EXPR` — the §3.6 escape: a `MaterialTextures` expression that switches the
    /// emission to `Material::with_textures`, so the `MATERIAL_FLAG_TEXTURED` derivation stays in
    /// the engine's one authority.
    pub textures: Option<Expr>,
}

/// `scene NAME { scene_item* }` (§3.7) — an entity tree that expands to ONE spawn fn.
///
/// Scenes are the `AetherCtx` showcase: `material: gold` resolves against a sibling `material`
/// construct, `mesh floor` against this scene's own `let` bindings, and the fn's parameter list is
/// DEMAND-DRIVEN — computed from what the body actually uses, so a scene with neither mesh lets
/// nor material props compresses to `(commands)` alone.
pub struct SceneDef {
    /// The spawn-fn name (lowercase by the §2 case convention — diagnosed at parse).
    pub name: Ident,
    /// `let NAME = plane(…) | cube(…) | mesh(…, …);` bindings, in source order.
    pub lets: Vec<MeshLet>,
    /// Top-level nodes, in source order (emission order is spawn order).
    pub nodes: Vec<SceneNode>,
}

/// `let NAME = mesh_src ;` (§3.7) — a mesh registered once per scene run and reused by every
/// `mesh NAME` node.
pub struct MeshLet {
    /// The binding name a `mesh NAME` node resolves against.
    pub name: Ident,
    /// The registration call.
    pub src: MeshSrc,
}

/// The §3.7 `mesh_src` production — each row is one `MeshAssetsExt` constructor.
pub enum MeshSrc {
    /// `plane(SIZE)` → `MeshAssetsExt::plane(…, size)`.
    Plane(Expr),
    /// `cube(SIZE)` → `MeshAssetsExt::cube(…, size)`.
    Cube(Expr),
    /// `mesh(VERTICES, INDICES)` → `MeshAssetsExt::register_mesh(…, &[Vertex], &[u32])`.
    Mesh(Expr, Expr),
}

/// One `node` (§3.7). A node spawns exactly one entity; `children` nest.
pub struct SceneNode {
    /// The sugar head (or [`NodeHead::Entity`], the §8 R8 universal fallback).
    pub head: NodeHead,
    /// The head keyword's own span — every per-head refusal points here.
    pub head_span: Span,
    /// `at EXPR`, when the head has a pose to place (see [`NodeHead::takes_at`]).
    pub at: Option<AtPose>,
    /// One slot per row of [`NodeHead::keys`], in TABLE order; `None` = absent ⇒ the row's
    /// default. Positional rather than keyed so the expander is total: a key the parser accepts
    /// cannot reach emission unhandled (the `MATERIAL_KEYS` discipline, one step further).
    pub keys: Vec<Option<NodeKeyValue>>,
    /// `material: IDENT` — a sibling `material` construct's name, resolved through `AetherCtx`.
    pub material: Option<Ident>,
    /// `casts_shadow` — the flag's own span (the refusal for a head with no shadow form).
    pub casts_shadow: Option<Span>,
    /// Bare component expressions (the `ui!` fallback) — each becomes one `.insert(EXPR)`.
    pub extras: Vec<Expr>,
    /// `children: [ node, … ]`, in source order.
    pub children: Vec<SceneNode>,
}

/// A node's `at` pose (§3.7's two sugars over a verbatim `Transform` expression).
pub enum AtPose {
    /// `at (x, y, z)` → `Transform::from_translation(Vec3::new(x, y, z))`.
    Translation(Vec<Expr>),
    /// `at EXPR` → the verbatim `Transform` expression, spans preserved.
    ///
    /// BOXED for the reason `Construct::Material` is: `syn::Expr` is a ~200-byte enum and this
    /// `Option` sits in every node of the tree.
    Verbatim(Box<Expr>),
}

/// The value shape a head key takes — the parser validates against it, so the expander can read
/// the slot without re-checking.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyShape {
    /// `(x, y, z)` — exactly three components (a direction, a position, or a linear color).
    Tuple3,
    /// A scalar expression (`lux: 3.2`, `range: 9.0`).
    Scalar,
}

/// One head-specific key (§3.7's `sun { dir:, color:, lux: }` family).
///
/// ONE table per head, spelling and shape together — the `MATERIAL_KEYS` rule: what a diagnostic
/// PRINTS and what the parser ACCEPTS are the same rows, so neither can gain a key the other lacks.
pub struct NodeKeySpec {
    /// The surface spelling.
    pub name: &'static str,
    /// The value shape.
    pub shape: KeyShape,
    /// `false` when the row has a default Aether can honestly synthesize. A key whose engine
    /// parameter has no neutral value is REQUIRED (the §3.6 `base:` precedent — Aether refuses
    /// rather than inventing a value no default can be right about).
    pub required: bool,
}

/// A head key's parsed value.
pub enum NodeKeyValue {
    /// `(x, y, z)` — three verbatim component exprs; arity is validated at parse against the
    /// parenthesized group's own span.
    Tuple(Vec<Expr>),
    /// A scalar/verbatim expression. BOXED for the `AtPose::Verbatim` reason — `spot` carries
    /// seven key slots and a `syn::Expr` is ~200 bytes.
    Scalar(Box<Expr>),
}

/// The §3.7 `node_head` production. Every head lowers to engine bundles that already exist; the
/// `entity` row is the universal fallback §8 R8 requires, so no engine feature is walled off.
pub enum NodeHead {
    /// `mesh IDENT` — a `let` binding of this scene.
    Mesh(Ident),
    /// `sun` → `DirectionalLightObject`.
    Sun,
    /// `spot` → `SpotLightObject`.
    Spot,
    /// `point` → `PointLightObject`.
    Point,
    /// `sky` → `SkyLight`.
    Sky,
    /// `camera` → `CameraRig`.
    Camera,
    /// `sdf EXPR` → `SdfPrimitive(EXPR)`.
    Sdf(Expr),
    /// `entity` → a bare spawn carrying only its `at` pose and its component exprs.
    Entity,
}

/// The `sun` key table — `DirectionalLight::new(direction, color, illuminance)`.
const SUN_KEYS: &[NodeKeySpec] = &[
    NodeKeySpec { name: "dir", shape: KeyShape::Tuple3, required: true },
    NodeKeySpec { name: "color", shape: KeyShape::Tuple3, required: false },
    NodeKeySpec { name: "lux", shape: KeyShape::Scalar, required: true },
];

/// The `sky` key table — `SkyLight::new(sky_color, ground_color)`. Neither hemisphere has a
/// neutral value (a black ground and a white ground light a scene differently), so both are
/// required.
const SKY_KEYS: &[NodeKeySpec] = &[
    NodeKeySpec { name: "sky", shape: KeyShape::Tuple3, required: true },
    NodeKeySpec { name: "ground", shape: KeyShape::Tuple3, required: true },
];

/// The `point` key table — `PointLight::new(position, color, power, range)`.
const POINT_KEYS: &[NodeKeySpec] = &[
    NodeKeySpec { name: "pos", shape: KeyShape::Tuple3, required: true },
    NodeKeySpec { name: "color", shape: KeyShape::Tuple3, required: false },
    NodeKeySpec { name: "power", shape: KeyShape::Scalar, required: true },
    NodeKeySpec { name: "range", shape: KeyShape::Scalar, required: true },
];

/// The `spot` key table — `SpotLight::new(position, direction, color, power, range, inner_deg,
/// outer_deg)`. `dir` is the SHINE axis; the pose is derived from `pos` + `dir` exactly as the
/// shipped scenes derive theirs (`look_at_rh` + `Quat::from_mat3`), because `light_reconcile`
/// overwrites the seeded direction from the transform's world `-Z`.
const SPOT_KEYS: &[NodeKeySpec] = &[
    NodeKeySpec { name: "pos", shape: KeyShape::Tuple3, required: true },
    NodeKeySpec { name: "dir", shape: KeyShape::Tuple3, required: true },
    NodeKeySpec { name: "color", shape: KeyShape::Tuple3, required: false },
    NodeKeySpec { name: "power", shape: KeyShape::Scalar, required: true },
    NodeKeySpec { name: "range", shape: KeyShape::Scalar, required: true },
    NodeKeySpec { name: "inner", shape: KeyShape::Scalar, required: true },
    NodeKeySpec { name: "outer", shape: KeyShape::Scalar, required: true },
];

/// The `camera` key table — the `Projection::Perspective` fields. `aspect` is REQUIRED: it is the
/// TARGET's width/height and no default can be right about it (the §3.6 `base:` rule); `fov`
/// (degrees), `near` and `far` carry the conventional defaults.
///
/// The head fills `CameraRig`'s `camera` field with `Camera::DEFAULT`, whose `order` is 0. A
/// SECOND camera therefore needs its own order, and the escape is the same one every head has: a
/// bare component expression is inserted AFTER the bundle, so
/// `camera at (…) { aspect: …, Camera { order: 1, ..Camera::DEFAULT } }` overwrites the field. The
/// orthographic projection is reached the same way, through `entity { CameraRig { … } }` (§8 R8's
/// universal fallback) — sugar is additive over it, never a wall around it.
const CAMERA_KEYS: &[NodeKeySpec] = &[
    NodeKeySpec { name: "fov", shape: KeyShape::Scalar, required: false },
    NodeKeySpec { name: "aspect", shape: KeyShape::Scalar, required: true },
    NodeKeySpec { name: "near", shape: KeyShape::Scalar, required: false },
    NodeKeySpec { name: "far", shape: KeyShape::Scalar, required: false },
];

/// The heads that carry no key table (`mesh` / `sdf` / `entity` — geometry and the fallback).
const NO_KEYS: &[NodeKeySpec] = &[];

/// What `casts_shadow` means on a head (§3.7: "mesh: `ShadowCaster`; spot/point:
/// `CastsPunctualShadow`").
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShadowForm {
    /// The CSM caster marker.
    Caster,
    /// The punctual-atlas opt-in.
    Punctual,
}

impl NodeHead {
    /// The head's surface keyword — the noun every per-head diagnostic prints.
    pub fn kw(&self) -> &'static str {
        match self {
            NodeHead::Mesh(_) => "mesh",
            NodeHead::Sun => "sun",
            NodeHead::Spot => "spot",
            NodeHead::Point => "point",
            NodeHead::Sky => "sky",
            NodeHead::Camera => "camera",
            NodeHead::Sdf(_) => "sdf",
            NodeHead::Entity => "entity",
        }
    }

    /// The head's key table (empty for the heads whose whole content is their `at` pose and their
    /// component exprs).
    pub fn keys(&self) -> &'static [NodeKeySpec] {
        match self {
            NodeHead::Sun => SUN_KEYS,
            NodeHead::Sky => SKY_KEYS,
            NodeHead::Point => POINT_KEYS,
            NodeHead::Spot => SPOT_KEYS,
            NodeHead::Camera => CAMERA_KEYS,
            NodeHead::Mesh(_) | NodeHead::Sdf(_) | NodeHead::Entity => NO_KEYS,
        }
    }

    /// `true` iff `at` has somewhere to go. The light heads derive their whole pose from their own
    /// keys (`dir` / `pos`), and an `sdf` edit carries WORLD-SPACE position inside the edit itself
    /// — on those, an `at` would be silently dropped, so it is refused instead.
    pub fn takes_at(&self) -> bool {
        matches!(self, NodeHead::Mesh(_) | NodeHead::Camera | NodeHead::Entity)
    }

    /// `true` iff `material: NAME` has a component to become (`MaterialHandle`).
    ///
    /// `Entity` is here DELIBERATELY, not as a side effect of how the check was written. §8 R8
    /// makes `entity` the head an author reaches for when no sugar head covers their case — the
    /// one that must never be POORER than the sugar heads in the props it accepts, or the escape
    /// hatch stops being one. An `entity` node that carries a `MeshHandle` component expression is
    /// a drawable prop assembled by hand, and refusing it the `material:` prop would force the
    /// author to spell `MaterialHandle(h.index() as u16)` themselves — the exact narrowing §3.7
    /// exists to hide. The same argument makes [`shadow_form`](Self::shadow_form) answer
    /// `Caster` for it.
    pub fn takes_material(&self) -> bool {
        matches!(self, NodeHead::Mesh(_) | NodeHead::Entity)
    }

    /// The `casts_shadow` form for this head, `None` when it has none (§3.7's `sky` diagnostic).
    ///
    /// `Entity` answers `Caster` for the reason [`takes_material`](Self::takes_material) records.
    pub fn shadow_form(&self) -> Option<ShadowForm> {
        match self {
            NodeHead::Mesh(_) | NodeHead::Entity => Some(ShadowForm::Caster),
            NodeHead::Spot | NodeHead::Point => Some(ShadowForm::Punctual),
            NodeHead::Sun | NodeHead::Sky | NodeHead::Camera | NodeHead::Sdf(_) => None,
        }
    }
}

/// The §3.7 node heads, in `node_head` order — the "expected one of" list and the did-you-mean
/// candidate set.
pub const NODE_HEADS: &[&str] =
    &["mesh", "sun", "spot", "point", "sky", "camera", "sdf", "entity"];

/// `on EVENT (params)? (if GUARD)? => state.path (BLOCK | ;)` (§3.5).
pub struct TransitionDef {
    /// The event type path, verbatim.
    pub event: Path,
    /// The `on` keyword's span — duplicate-handler errors point at the SECOND `on`.
    pub kw_span: Span,
    /// Source position among the machine's transitions, assigned as the parser walks the body.
    ///
    /// §5.1's arbitration for two transitions accepted on one frame is "last write wins, made
    /// deterministic by registering in DECLARATION order". Inheritance walks each leaf's chain
    /// innermost-first, which is not declaration order, so the expander re-sorts on this index.
    /// A span cannot serve: `proc_macro2::Span::start()` needs the `span-locations` feature,
    /// which this crate does not take.
    pub decl_index: usize,
    /// Params the guard and action may use, same grammar as `system`.
    pub params: Vec<SysParam>,
    /// `if EXPR` — verbatim; a failed guard SKIPS the event (does not consume the frame).
    pub guard: Option<Expr>,
    /// The root-anchored target path (`Playing.Paused`), unresolved segments.
    pub target: Vec<Ident>,
    /// The action block body (`None` for the `;` form).
    pub action: Option<TokenStream>,
}
