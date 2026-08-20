//! The Aether AST — one node per construct, kept deliberately shallow.
//!
//! Bodies, types and paths are **verbatim `syn` fragments with the user's spans** (§2): the AST
//! carries structure, never re-lexed text, so downstream rustc errors land on the user's tokens.

use proc_macro2::{Span, TokenStream};
use syn::{Expr, Ident, Path, Type};

/// One parsed `aether!` block: a flat list of constructs sharing one parse context.
pub struct AetherBlock {
    /// The constructs, in source order (emission preserves it — deterministic output is what
    /// makes the unit-test snapshots exact).
    pub constructs: Vec<Construct>,
}

/// Every construct rung A0 knows. Later rungs add variants — one new variant, one new parser
/// row, one new expander module each (§6.1's extensibility contract).
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
