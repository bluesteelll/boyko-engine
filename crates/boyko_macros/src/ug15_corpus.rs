//! UG-15 leg (1)'s expanded corpus, M-b (03 §6; cut §4.1): every `boyko_macros` entry point's
//! `proc_macro2` twin, fed a fixture corpus, with every output run through the SAME visitor leg (1)
//! runs over source (`boyko_symcensus::source`).
//!
//! **Why this exists.** `syn` keeps a macro body as opaque tokens, so leg (1)'s walk cannot see an
//! attribute a derive EMITS. M-a scans the `quote!` templates as tokens, which sees a literal
//! `#[used]` in a template but not one assembled from `format_ident!` outside every template
//! (control vi). M-b sees both, because it reads what the macro actually produced.
//!
//! **What makes it binding.**
//! - [`KEY_TABLES`] are the parsers' own key tables (03 §6: "the key list is the parsers' own
//!   `const` table"). `keys_are_the_parsers_own` makes them a bijection with the parser code:
//!   every identifier-shaped string literal a parser module compares against (its `is_ident`,
//!   `match` arms, `==`, `matches!`) is a table row or a [`NON_KEY_LITERALS`] row with a reason,
//!   and every row is still in the code. A new parser key without a row is RED (control K).
//! - `corpus_covers_every_key`: every row is claimed by a fixture whose input really contains
//!   it. A row without a fixture is RED (control F). Fixtures are written by hand, never generated
//!   from the tables, or control F could not go red.
//! - `corpus_covers_every_shape_arm` (critique W5): every `Data` / `Fields` / generics arm a
//!   parser module branches on has a fixture of that input shape for each entry point reaching the
//!   module, so code emitted only on a tuple-struct, enum or generic path is expanded too.
//! - `every_entry_point_is_a_one_line_wrapper_over_its_twin`: the exported set is exactly
//!   [`Entry::ALL`], each a single `<twin>(….into()).into()` call.
//! - Anti-vacuity per fixture: its output parses, the visitor visits it, and it carries its
//!   anchor (the trait impl, the refusal's `compile_error!`, the `ui!` block, the chart's items).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{TokenStream, TokenTree};

use boyko_symcensus::source::{self, AllowEntry, Via};

use self::Anchor::{CompileError, Impl, Item, UiBlock};
use self::Entry::{Actionlike, Bindable, Bundle, Component, Event, Relationship, RelationshipTarget, Resource, StateChart, SystemSet, Ui};

/// The eleven entry points of `lib.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    Component,
    Relationship,
    RelationshipTarget,
    Resource,
    Event,
    Bundle,
    SystemSet,
    Actionlike,
    Ui,
    Bindable,
    StateChart,
}

impl Entry {
    const ALL: [Self; 11] = [
        Self::Component,
        Self::Relationship,
        Self::RelationshipTarget,
        Self::Resource,
        Self::Event,
        Self::Bundle,
        Self::SystemSet,
        Self::Actionlike,
        Self::Ui,
        Self::Bindable,
        Self::StateChart,
    ];

    /// The exported fn in `lib.rs`.
    const fn lib_fn(self) -> &'static str {
        match self {
            Self::Component => "component_macro",
            Self::Relationship => "relationship_macro",
            Self::RelationshipTarget => "relationship_target_macro",
            Self::Resource => "resource_macro",
            Self::Event => "event",
            Self::Bundle => "bundle_macro",
            Self::SystemSet => "system_set_macro",
            Self::Actionlike => "actionlike_macro",
            Self::Ui => "ui",
            Self::Bindable => "bindable_macro",
            Self::StateChart => "state_chart",
        }
    }

    /// The derive name, for a `#[proc_macro_derive]` entry.
    const fn derive_name(self) -> Option<&'static str> {
        match self {
            Self::Component => Some("Component"),
            Self::Relationship => Some("Relationship"),
            Self::RelationshipTarget => Some("RelationshipTarget"),
            Self::Resource => Some("Resource"),
            Self::Bundle => Some("Bundle"),
            Self::SystemSet => Some("SystemSet"),
            Self::Actionlike => Some("Actionlike"),
            Self::Bindable => Some("Bindable"),
            Self::Event | Self::Ui | Self::StateChart => None,
        }
    }

    /// The helper attributes the derive declares (`attributes(...)` in `lib.rs`).
    const fn helpers(self) -> &'static [&'static str] {
        match self {
            Self::Component => &["component", "require", "entities", "relationship", "relationship_target", "reflect"],
            Self::Relationship => &["relationship"],
            Self::RelationshipTarget => &["relationship_target"],
            Self::Actionlike => &["actionlike"],
            Self::Bindable => &["bind"],
            _ => &[],
        }
    }

    /// The source modules (paths under `src/`) the entry's expansion reaches. Closed under
    /// `crate::` references (`entry_modules_are_closed`).
    const fn modules(self) -> &'static [&'static str] {
        match self {
            Self::Component | Self::Relationship | Self::RelationshipTarget => {
                &["component.rs", "relationship.rs", "reflect.rs", "common.rs"]
            }
            Self::Resource => &["resource.rs", "common.rs"],
            Self::Event => &["event.rs"],
            Self::Bundle => &["bundle.rs", "common.rs"],
            Self::SystemSet => &["system_set.rs", "common.rs"],
            Self::Actionlike => &["actionlike.rs", "common.rs"],
            Self::Ui => &["ui.rs"],
            Self::Bindable => &["bindable.rs", "common.rs"],
            Self::StateChart => &["state_chart/mod.rs", "state_chart/ast.rs", "state_chart/emit.rs", "state_chart/model.rs"],
        }
    }

    /// Runs the entry's `proc_macro2` twin.
    fn expand(self, args: TokenStream, input: TokenStream) -> TokenStream {
        match self {
            Self::Component => crate::component::component_macro_impl(input),
            Self::Relationship => crate::relationship::relationship_macro_impl(input),
            Self::RelationshipTarget => crate::relationship::relationship_target_macro_impl(input),
            Self::Resource => crate::resource::resource_macro_impl(input),
            Self::Event => crate::event::event_impl(args, input),
            Self::Bundle => crate::bundle::bundle_macro_impl(input),
            Self::SystemSet => crate::system_set::system_set_macro_impl(input),
            Self::Actionlike => crate::actionlike::actionlike_macro_impl(input),
            Self::Ui => crate::ui::ui_impl(input),
            Self::Bindable => crate::bindable::bindable_macro_impl(input),
            Self::StateChart => crate::state_chart::state_chart_impl(input),
        }
    }

    /// `true` for the entries whose input is an item (`DeriveInput`-shaped).
    const fn item_input(self) -> bool {
        !matches!(self, Self::Ui | Self::StateChart)
    }
}

/// One parser key table.
struct KeyTable {
    /// The table's name in receipts.
    name: &'static str,
    /// The parser modules whose code the rows live in.
    modules: &'static [&'static str],
    /// The keys (attribute names, keys, values, keywords, input type names the parser matches).
    rows: &'static [&'static str],
}

/// The parsers' key tables (cut §4.1). Each row is a string the parser code compares input
/// against; `keys_are_the_parsers_own` binds them to the code in both directions.
const KEY_TABLES: &[KeyTable] = &[
    KeyTable { name: "COMPONENT_ATTRS", modules: &["component.rs"], rows: &["component", "require", "entities", "repr"] },
    KeyTable {
        name: "COMPONENT_KEYS",
        modules: &["component.rs"],
        rows: &[
            "no_bundle",
            "reflect",
            "no_clone",
            "clone",
            "no_serialize",
            "stable_name",
            "format_version",
            "storage",
            "on_add",
            "on_insert",
            "on_replace",
            "on_remove",
            "on_despawn",
        ],
    },
    KeyTable { name: "COMPONENT_STORAGE_VALUES", modules: &["component.rs"], rows: &["bitset", "dense"] },
    KeyTable { name: "COMPONENT_REPR_KEYS", modules: &["component.rs"], rows: &["C", "transparent"] },
    KeyTable { name: "COMPONENT_ENTITY_FIELD_TYPES", modules: &["component.rs"], rows: &["Entity", "ChildOf"] },
    KeyTable { name: "RELATIONSHIP_ATTRS", modules: &["relationship.rs"], rows: &["relationship", "relationship_target"] },
    KeyTable { name: "RELATIONSHIP_KEYS", modules: &["relationship.rs"], rows: &["target", "allow_self_referential"] },
    KeyTable { name: "RELATIONSHIP_TARGET_KEYS", modules: &["relationship.rs"], rows: &["source", "linked_despawn", "retain_empty"] },
    KeyTable { name: "REFLECT_ATTRS", modules: &["reflect.rs"], rows: &["reflect", "repr"] },
    KeyTable { name: "REFLECT_TYPE_KEYS", modules: &["reflect.rs"], rows: &["no_default"] },
    KeyTable { name: "REFLECT_FIELD_KEYS", modules: &["reflect.rs"], rows: &["skip"] },
    KeyTable {
        name: "REFLECT_SCALAR_TYPES",
        modules: &["reflect.rs"],
        rows: &["bool", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64"],
    },
    KeyTable {
        name: "REFLECT_INT_REPRS",
        modules: &["reflect.rs"],
        rows: &["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"],
    },
    KeyTable { name: "EVENT_FIELD_ATTRS", modules: &["event.rs"], rows: &["participant", "parameter"] },
    KeyTable { name: "EVENT_PARTICIPANT_KEYS", modules: &["event.rs"], rows: &["components"] },
    KeyTable { name: "ACTIONLIKE_ATTRS", modules: &["actionlike.rs"], rows: &["actionlike"] },
    KeyTable { name: "ACTIONLIKE_VALUES", modules: &["actionlike.rs"], rows: &["Button", "Axis1D", "Axis2D"] },
    KeyTable { name: "UI_KEYWORDS", modules: &["ui.rs"], rows: &["commands", "children"] },
    KeyTable { name: "UI_KNOWN_TYPES", modules: &["ui.rs"], rows: &["UiLayout", "ComputedRect"] },
    KeyTable {
        name: "STATE_CHART_KEYWORDS",
        modules: &["state_chart/ast.rs", "state_chart/mod.rs"],
        rows: &["chart", "initial", "state", "enter", "exit", "on"],
    },
];

/// Identifier-shaped literals in parser code that are NOT keys: `(module, literal, reason)`.
const NON_KEY_LITERALS: &[(&str, &str, &str)] = &[
    ("ui.rs", "cmds", "the default `Commands` binding name the macro EMITS when no `commands:` preamble is given; never compared against input"),
    ("reflect.rs", "Bool", "a `ValueKind` name `scalar_kind` RETURNS and `prim_accessors` maps back; internal, never input"),
    ("reflect.rs", "U8", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "U16", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "U32", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "U64", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "I8", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "I16", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "I32", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "I64", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "F32", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "F64", "a `ValueKind` name, internal (see `Bool`)"),
    ("reflect.rs", "Struct", "a `TypeKind` variant name the codegen EMITS through `Ident::new`"),
    ("reflect.rs", "TupleStruct", "a `TypeKind` variant name the codegen EMITS through `Ident::new`"),
    ("reflect.rs", "Opaque", "a `ValueKind` variant name the codegen EMITS through `Ident::new`"),
    ("reflect.rs", "bitset_storage_rejected", "a `REFUSALS` rule name, keyed by `tests/reflect_refusal_census.rs`, never compared against input"),
    ("reflect.rs", "vec_field_rejected", "a `REFUSALS` rule name (see `bitset_storage_rejected`)"),
    ("reflect.rs", "fieldless_enum_without_repr_rejected", "a `REFUSALS` rule name (see `bitset_storage_rejected`)"),
    ("reflect.rs", "data_carrying_enum_rejected", "a `REFUSALS` rule name (see `bitset_storage_rejected`)"),
    ("reflect.rs", "union_rejected", "a `REFUSALS` rule name (see `bitset_storage_rejected`)"),
    ("reflect.rs", "missing_default_rejected", "a `REFUSALS` rule name (see `bitset_storage_rejected`)"),
];

/// What a fixture's output must carry.
#[derive(Clone, Copy, Debug)]
enum Anchor {
    /// A trait impl whose trait path ends in this name.
    Impl(&'static str),
    /// The refusal: a `compile_error!` invocation.
    CompileError,
    /// `ui!`: the block parses inside a fn and spawns (`spawn` is called).
    UiBlock,
    /// An item whose name contains this text.
    Item(&'static str),
}

/// One corpus input.
struct Fixture {
    name: &'static str,
    entry: Entry,
    /// The attribute arguments (`#[event]` only).
    args: &'static str,
    input: &'static str,
    anchor: Anchor,
    /// The table rows this fixture exercises; each must appear in `input` as a token.
    keys: &'static [&'static str],
}

const fn fx(name: &'static str, entry: Entry, input: &'static str, anchor: Anchor, keys: &'static [&'static str]) -> Fixture {
    Fixture { name, entry, args: "", input, anchor, keys }
}

/// The corpus. Every row of every [`KEY_TABLES`] entry is claimed here, and every shape arm of
/// every entry's modules has an input of that shape.
const FIXTURES: &[Fixture] = &[
    // ---- #[derive(Component)]: shapes
    fx("component_named", Component, "struct Position { x: f32, y: f32 }", Impl("Component"), &[]),
    fx("component_tuple", Component, "struct Health(u32);", Impl("Component"), &[]),
    fx("component_unit", Component, "struct Marker;", Impl("Component"), &[]),
    fx("component_enum", Component, "enum Mode { Idle, Walk(f32), Jump { height: f32 } }", Impl("Component"), &[]),
    fx("component_union", Component, "union Bits { word: u32, real: f32 }", Impl("Component"), &[]),
    // ---- #[derive(Component)]: keys
    fx(
        "component_hooks",
        Component,
        "#[component(on_add = hook_a, on_insert = hook_b, on_replace = hook_c, on_remove = hook_d, on_despawn = hook_e)] struct Hooked(u32);",
        Impl("Component"),
        &["component", "on_add", "on_insert", "on_replace", "on_remove", "on_despawn"],
    ),
    fx(
        "component_flags",
        Component,
        "#[component(no_bundle, no_clone, no_serialize, stable_name = \"game::Flagged\", format_version = 2)] struct Flagged { v: u32 }",
        Impl("Component"),
        &["component", "no_bundle", "no_clone", "no_serialize", "stable_name", "format_version"],
    ),
    fx("component_clone_fn", Component, "#[component(clone = my_clone)] struct Cloned(u32);", Impl("Component"), &["component", "clone"]),
    fx("component_storage_bitset", Component, "#[component(storage = \"bitset\")] struct Stunned;", Impl("Component"), &["component", "storage", "bitset"]),
    fx("component_storage_dense", Component, "#[component(storage = \"dense\")] struct Solver { w: f32 }", Impl("Component"), &["component", "storage", "dense"]),
    fx(
        "component_bitset_with_hook_refused",
        Component,
        "#[component(storage = \"bitset\", on_add = hook_a)] struct Bad;",
        CompileError,
        &["storage", "bitset", "on_add"],
    ),
    fx(
        "component_require",
        Component,
        "#[require(Velocity, Mass = Mass(1.0), Place(1, 2))] struct Body { m: f32 }",
        Impl("Component"),
        &["require"],
    ),
    fx(
        "component_entities",
        Component,
        "struct Link { #[entities] target: Entity, parent: ChildOf, w: f32 }",
        Impl("Component"),
        &["entities", "Entity", "ChildOf"],
    ),
    fx("component_repr_c", Component, "#[repr(C)] struct Pod { a: u32, b: u32 }", Impl("Component"), &["repr", "C"]),
    fx("component_repr_transparent", Component, "#[repr(transparent)] struct Wrap(u64);", Impl("Component"), &["repr", "transparent"]),
    fx(
        "component_reflect_scalars",
        Component,
        "#[component(reflect)] #[reflect(no_default)] struct Stats { a: bool, b: u8, c: u16, d: u32, e: u64, f: i8, g: i16, h: i32, i: i64, j: f32, k: f64, #[reflect(skip)] list: Vec<u8> }",
        Impl("Component"),
        &["reflect", "no_default", "skip", "bool", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64"],
    ),
    fx(
        "component_reflect_tuple",
        Component,
        "#[derive(Default)] #[component(reflect)] struct Pair(f32, f32);",
        Impl("Component"),
        &["reflect"],
    ),
    fx(
        "component_reflect_bitset_refused",
        Component,
        "#[component(reflect, storage = \"bitset\")] struct Tag;",
        Impl("Component"),
        &["reflect", "storage", "bitset"],
    ),
    fx("component_reflect_enum_u8", Component, "#[component(reflect)] #[repr(u8)] enum E8 { A, B }", Impl("Component"), &["reflect", "repr", "u8"]),
    fx("component_reflect_enum_u16", Component, "#[component(reflect)] #[repr(u16)] enum E16 { A, B }", Impl("Component"), &["u16"]),
    fx("component_reflect_enum_u32", Component, "#[component(reflect)] #[repr(u32)] enum E32 { A, B }", Impl("Component"), &["u32"]),
    fx("component_reflect_enum_u64", Component, "#[component(reflect)] #[repr(u64)] enum E64 { A, B }", Impl("Component"), &["u64"]),
    fx("component_reflect_enum_u128", Component, "#[component(reflect)] #[repr(u128)] enum E128 { A, B }", Impl("Component"), &["u128"]),
    fx("component_reflect_enum_usize", Component, "#[component(reflect)] #[repr(usize)] enum Eus { A, B }", Impl("Component"), &["usize"]),
    fx("component_reflect_enum_i8", Component, "#[component(reflect)] #[repr(i8)] enum Ei8 { A, B }", Impl("Component"), &["i8"]),
    fx("component_reflect_enum_i16", Component, "#[component(reflect)] #[repr(i16)] enum Ei16 { A, B }", Impl("Component"), &["i16"]),
    fx("component_reflect_enum_i32", Component, "#[component(reflect)] #[repr(i32)] enum Ei32 { A, B }", Impl("Component"), &["i32"]),
    fx("component_reflect_enum_i64", Component, "#[component(reflect)] #[repr(i64)] enum Ei64 { A, B }", Impl("Component"), &["i64"]),
    fx("component_reflect_enum_i128", Component, "#[component(reflect)] #[repr(i128)] enum Ei128 { A, B }", Impl("Component"), &["i128"]),
    fx("component_reflect_enum_isize", Component, "#[component(reflect)] #[repr(isize)] enum Eis { A, B }", Impl("Component"), &["isize"]),
    fx("component_reflect_enum_without_repr", Component, "#[component(reflect)] enum NoRepr { A, B }", Impl("Component"), &["reflect"]),
    fx("component_reflect_data_enum", Component, "#[component(reflect)] enum Payload { A(u32), B { x: f32 } }", Impl("Component"), &["reflect"]),
    fx("component_reflect_union", Component, "#[component(reflect)] union RU { a: u32, b: f32 }", Impl("Component"), &["reflect"]),
    fx(
        "component_relationship_source",
        Component,
        "#[relationship(target = Children)] struct ChildOf(Entity);",
        Impl("Component"),
        &["relationship", "target"],
    ),
    fx(
        "component_relationship_target",
        Component,
        "#[relationship_target(source = ChildOf, linked_despawn, retain_empty)] struct Children(Vec<Entity>);",
        Impl("Component"),
        &["relationship_target", "source", "linked_despawn", "retain_empty"],
    ),
    // ---- #[derive(Relationship)]
    fx("relationship_tuple", Relationship, "#[relationship(target = Children)] struct ChildOf(Entity);", Impl("Relationship"), &["relationship", "target"]),
    fx(
        "relationship_named_marked",
        Relationship,
        "#[relationship(target = LikedBy, allow_self_referential)] struct Likes { #[relationship] who: Entity, weight: f32 }",
        Impl("Relationship"),
        &["relationship", "target", "allow_self_referential"],
    ),
    fx("relationship_named_single", Relationship, "#[relationship(target = Owners)] struct OwnedBy { owner: Entity }", Impl("Relationship"), &["relationship", "target"]),
    fx("relationship_unit_refused", Relationship, "#[relationship(target = X)] struct Nothing;", CompileError, &["relationship"]),
    fx("relationship_enum_refused", Relationship, "#[relationship(target = X)] enum Either { A(Entity), B { e: Entity }, C }", CompileError, &["relationship"]),
    fx("relationship_union_refused", Relationship, "#[relationship(target = X)] union Raw { e: u64 }", CompileError, &["relationship"]),
    // ---- #[derive(RelationshipTarget)]
    fx(
        "relationship_target_tuple",
        RelationshipTarget,
        "#[relationship_target(source = ChildOf, linked_despawn, retain_empty)] struct Children(Vec<Entity>);",
        Impl("RelationshipTarget"),
        &["relationship_target", "source", "linked_despawn", "retain_empty"],
    ),
    fx(
        "relationship_target_named",
        RelationshipTarget,
        "#[relationship_target(source = Likes, retain_empty)] struct LikedBy { sources: Vec<Entity> }",
        Impl("RelationshipTarget"),
        &["relationship_target", "source", "retain_empty"],
    ),
    fx("relationship_target_unit_refused", RelationshipTarget, "#[relationship_target(source = X, retain_empty)] struct None;", CompileError, &["relationship_target"]),
    fx(
        "relationship_target_enum_refused",
        RelationshipTarget,
        "#[relationship_target(source = X, retain_empty)] enum Many { A(Vec<Entity>), B { v: Vec<Entity> }, C }",
        CompileError,
        &["relationship_target"],
    ),
    fx(
        "relationship_target_union_refused",
        RelationshipTarget,
        "#[relationship_target(source = X, retain_empty)] union Raw { e: u64 }",
        CompileError,
        &["relationship_target"],
    ),
    // ---- #[derive(Resource)]
    fx("resource", Resource, "struct GameTick(u32);", Impl("Resource"), &[]),
    // ---- #[event]
    Fixture {
        name: "event_participants_and_parameters",
        entry: Event,
        args: "",
        input: "struct Damage { #[participant(components = \"Position, Health\")] victim: Entity, #[parameter] amount: f32 }",
        anchor: Impl("Event"),
        keys: &["participant", "components", "parameter"],
    },
    Fixture { name: "event_generic_refused", entry: Event, args: "", input: "struct Gen<T> { #[parameter] v: T }", anchor: CompileError, keys: &["parameter"] },
    Fixture { name: "event_tuple_refused", entry: Event, args: "", input: "struct Tup(#[parameter] f32);", anchor: CompileError, keys: &["parameter"] },
    // ---- #[derive(Bundle)]
    fx("bundle_named", Bundle, "struct PlayerBundle { pos: Position, vel: Velocity }", Impl("Bundle"), &[]),
    fx("bundle_tuple", Bundle, "struct PairBundle(Position, Velocity);", Impl("Bundle"), &[]),
    fx("bundle_unit_refused", Bundle, "struct Empty;", CompileError, &[]),
    fx("bundle_generic_refused", Bundle, "struct G<T> { t: T }", CompileError, &[]),
    fx("bundle_enum_refused", Bundle, "enum E { A(Position), B { v: Velocity }, C }", CompileError, &[]),
    fx("bundle_union_refused", Bundle, "union U { a: u32 }", CompileError, &[]),
    // ---- #[derive(SystemSet)]
    fx("system_set_unit", SystemSet, "struct PhysicsSet;", Impl("SystemSet"), &[]),
    fx("system_set_enum", SystemSet, "enum CombatSet { Target, Damage, Cleanup }", Impl("SystemSet"), &[]),
    fx("system_set_generic_refused", SystemSet, "struct G<T>;", CompileError, &[]),
    fx("system_set_union_refused", SystemSet, "union U { a: u32 }", CompileError, &[]),
    fx("system_set_fielded_refused", SystemSet, "struct S { a: u32 }", CompileError, &[]),
    fx("system_set_data_enum_refused", SystemSet, "enum E { A(u32) }", CompileError, &[]),
    // ---- #[derive(Actionlike)]
    fx(
        "actionlike_enum",
        Actionlike,
        "enum PlayerAction { Jump, #[actionlike(Axis2D)] Move, #[actionlike(Axis1D)] Throttle, #[actionlike(Button)] Fire }",
        Impl("Actionlike"),
        &["actionlike", "Button", "Axis1D", "Axis2D"],
    ),
    fx("actionlike_struct_refused", Actionlike, "struct NotAnEnum;", CompileError, &[]),
    fx("actionlike_union_refused", Actionlike, "union U { a: u32 }", CompileError, &[]),
    fx("actionlike_generic_refused", Actionlike, "enum G<T> { A, B(T) }", CompileError, &[]),
    fx("actionlike_data_variant_refused", Actionlike, "enum D { A, B(u32) }", CompileError, &[]),
    fx("actionlike_empty_refused", Actionlike, "enum Nothing {}", CompileError, &[]),
    // ---- ui!
    fx(
        "ui_tree",
        Ui,
        "commands: c; #header { UiLayout { height: 48.0, ..Default::default() }, ComputedRect::default(), UiRoot, children: [ { UiLayout::default() }, #footer { UiLayout::default() } ] }",
        UiBlock,
        &["commands", "children", "UiLayout", "ComputedRect"],
    ),
    fx("ui_implicit_root", Ui, "UiLayout { width: 1.0, ..Default::default() }, UiRoot", UiBlock, &["UiLayout"]),
    // ---- #[derive(Bindable)]
    fx("bindable_named", Bindable, "struct Health { #[bind] current: f32, max: f32 }", Impl("Bindable"), &[]),
    fx("bindable_tuple_refused", Bindable, "struct T(f32);", CompileError, &[]),
    // ---- state_chart!
    fx(
        "state_chart_game_flow",
        StateChart,
        "chart GameFlow; initial Boot; state Boot { on AssetsReady => Playing; } state Playing { initial Running; enter(mut cmds: Commands) { cmds.spawn(HudRoot); } exit(mut cmds: Commands) { cmds.spawn(Fade); } state Running { on PausePressed => Playing.Paused; } state Paused { on PausePressed => Playing.Running; } on PlayerDied => GameOver; } state GameOver { on RestartPressed => Boot; }",
        Item("__state_chart_install_game_flow"),
        &["chart", "initial", "state", "enter", "exit", "on"],
    ),
];

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` under `src/` (repo-relative to `src/`, `/`-separated), minus `lib.rs` (the entry
/// points, no parser code) and this file.
fn parser_modules() -> Vec<(String, syn::File)> {
    let src = src_dir();
    let files = source::rs_files(&src).unwrap_or_else(|r| panic!("{r}"));
    let mut out = Vec::new();
    for f in files {
        let rel = source::rel(&src, &f);
        if rel == "lib.rs" || rel == "ug15_corpus.rs" {
            continue;
        }
        out.push((rel, source::parse_path(&f).unwrap_or_else(|r| panic!("{r}"))));
    }
    assert!(out.len() >= 14, "the parser-module walk read {} files", out.len());
    out
}

fn tokens(src: &str) -> TokenStream {
    TokenStream::from_str(src).unwrap_or_else(|e| panic!("fixture does not tokenize: {src}: {e}"))
}

/// Every identifier and string-literal value in `ts`, recursively.
fn words(ts: TokenStream, out: &mut BTreeSet<String>) {
    for t in ts {
        match t {
            TokenTree::Group(g) => words(g.stream(), out),
            TokenTree::Ident(i) => {
                out.insert(i.to_string());
            }
            TokenTree::Literal(l) => {
                if let Ok(s) = syn::parse_str::<syn::LitStr>(&l.to_string()) {
                    out.insert(s.value());
                }
            }
            TokenTree::Punct(_) => {}
        }
    }
}

/// The input-shape arms a fixture's item takes: its `Data` kind, its (variants') `Fields` kinds,
/// and `generics` when it has parameters.
fn fixture_shapes(f: &Fixture) -> BTreeSet<String> {
    let di: syn::DeriveInput = syn::parse2(tokens(f.input)).unwrap_or_else(|e| panic!("fixture {} is not an item: {e}", f.name));
    let mut s = BTreeSet::new();
    let kind = |fields: &syn::Fields| match fields {
        syn::Fields::Named(_) => "Fields::Named",
        syn::Fields::Unnamed(_) => "Fields::Unnamed",
        syn::Fields::Unit => "Fields::Unit",
    };
    match &di.data {
        syn::Data::Struct(d) => {
            s.insert("Data::Struct".to_owned());
            s.insert(kind(&d.fields).to_owned());
        }
        syn::Data::Enum(e) => {
            s.insert("Data::Enum".to_owned());
            for v in &e.variants {
                s.insert(kind(&v.fields).to_owned());
            }
        }
        syn::Data::Union(_) => {
            s.insert("Data::Union".to_owned());
            s.insert("Fields::Named".to_owned());
        }
    }
    if !di.generics.params.is_empty() {
        s.insert("generics".to_owned());
    }
    s
}

#[test]
fn every_entry_point_is_a_one_line_wrapper_over_its_twin() {
    let lib = source::parse_path(&src_dir().join("lib.rs")).unwrap_or_else(|r| panic!("{r}"));
    let mut exported: BTreeMap<String, syn::ItemFn> = BTreeMap::new();
    for item in &lib.items {
        if let syn::Item::Fn(f) = item
            && f.attrs.iter().any(|a| a.path().segments.last().is_some_and(|s| s.ident.to_string().starts_with("proc_macro")))
        {
            exported.insert(f.sig.ident.to_string(), f.clone());
        }
    }
    let want: BTreeSet<String> = Entry::ALL.iter().map(|e| e.lib_fn().to_owned()).collect();
    let got: BTreeSet<String> = exported.keys().cloned().collect();
    println!("M-b: {} exported entry point(s) in lib.rs, {} twin(s) in the corpus", got.len(), want.len());
    assert_eq!(got, want, "the exported entry points are not the corpus's entries: a new one needs a twin, fixtures and a row in Entry");
    for e in Entry::ALL {
        let f = &exported[e.lib_fn()];
        let body = &f.block.stmts;
        assert_eq!(body.len(), 1, "{} is not a one-line wrapper", e.lib_fn());
        let text = quote::ToTokens::to_token_stream(&body[0]).to_string().replace(' ', "");
        let twin = format!("{}_impl(", e.lib_fn());
        assert!(text.contains(&twin) && text.ends_with(".into()"), "{} does not wrap its twin `{twin}…`: {text}", e.lib_fn());
        // The derive's declared helpers are this corpus's table of them.
        if let Some(derive) = e.derive_name() {
            let attr = f.attrs.iter().find(|a| a.path().is_ident("proc_macro_derive")).unwrap_or_else(|| panic!("{derive} has no proc_macro_derive"));
            let mut w = BTreeSet::new();
            words(quote::ToTokens::to_token_stream(&attr.meta), &mut w);
            let declared: BTreeSet<String> = w.into_iter().filter(|x| !matches!(x.as_str(), "proc_macro_derive" | "attributes") && x != derive).collect();
            let tabled: BTreeSet<String> = e.helpers().iter().map(|s| (*s).to_owned()).collect();
            assert_eq!(declared, tabled, "{derive}'s declared helper attributes are not Entry::helpers");
        }
    }
}

#[test]
fn keys_are_the_parsers_own() {
    let modules = parser_modules();
    let mut literals: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (m, file) in &modules {
        literals.insert(m.clone(), source::key_literals(file).into_iter().map(|k| k.text).collect());
    }
    let mut problems = Vec::new();
    for (m, lits) in &literals {
        for l in lits {
            let tabled = KEY_TABLES.iter().any(|t| t.modules.contains(&m.as_str()) && t.rows.contains(&l.as_str()));
            let non_key = NON_KEY_LITERALS.iter().any(|(nm, nl, _)| nm == m && nl == l);
            if !tabled && !non_key {
                problems.push(format!("{m}: the parser compares against `{l}`, which is in no key table and not a NON_KEY_LITERALS row"));
            }
        }
    }
    for t in KEY_TABLES {
        for m in t.modules {
            assert!(literals.contains_key(*m), "{} names module {m}, which does not exist", t.name);
        }
        for r in t.rows {
            if !t.modules.iter().any(|m| literals.get(*m).is_some_and(|l| l.contains(*r))) {
                problems.push(format!("{}: the row `{r}` is in none of {:?} any more (a stale row)", t.name, t.modules));
            }
        }
    }
    for (m, l, why) in NON_KEY_LITERALS {
        assert!(!why.trim().is_empty(), "NON_KEY_LITERALS {m} {l} carries no reason");
        if !literals.get(*m).is_some_and(|s| s.contains(*l)) {
            problems.push(format!("NON_KEY_LITERALS: `{l}` is no longer in {m} (a stale row)"));
        }
    }
    let total: usize = literals.values().map(BTreeSet::len).sum();
    let rows: usize = KEY_TABLES.iter().map(|t| t.rows.len()).sum();
    println!("M-b key self-census: {} module(s), {total} distinct literal(s); {} table(s), {rows} row(s); {} non-key row(s)", modules.len(), KEY_TABLES.len(), NON_KEY_LITERALS.len());
    assert!(total > 50, "the key self-census read {total} literals; it read nothing");
    assert!(problems.is_empty(), "M-b RED:\n  {}", problems.join("\n  "));
}

#[test]
fn entry_modules_are_closed() {
    let modules = parser_modules();
    let file_of = |name: &str| -> Vec<String> {
        if name == "state_chart" {
            modules.iter().filter(|(m, _)| m.starts_with("state_chart/")).map(|(m, _)| m.clone()).collect()
        } else {
            vec![format!("{name}.rs")]
        }
    };
    for (m, _) in &modules {
        assert!(Entry::ALL.iter().any(|e| e.modules().contains(&m.as_str())), "module {m} is reached by no Entry::modules list");
    }
    for e in Entry::ALL {
        for m in e.modules() {
            let file = &modules.iter().find(|(x, _)| x == m).unwrap_or_else(|| panic!("{e:?} names missing module {m}")).1;
            for r in source::crate_refs(file) {
                for f in file_of(&r) {
                    assert!(e.modules().contains(&f.as_str()), "{e:?}: {m} reaches crate::{r} ({f}), which Entry::modules does not list");
                }
            }
        }
    }
}

#[test]
fn corpus_covers_every_key() {
    let mut claimed: BTreeSet<&str> = BTreeSet::new();
    let all_rows: BTreeSet<&str> = KEY_TABLES.iter().flat_map(|t| t.rows.iter().copied()).collect();
    for f in FIXTURES {
        let mut w = BTreeSet::new();
        words(tokens(f.input), &mut w);
        for k in f.keys {
            assert!(all_rows.contains(k), "fixture {} claims `{k}`, which is no table row", f.name);
            assert!(w.contains(*k), "fixture {} claims `{k}` but its input does not contain it", f.name);
            claimed.insert(k);
        }
    }
    let unclaimed: Vec<String> = KEY_TABLES
        .iter()
        .flat_map(|t| t.rows.iter().filter(|r| !claimed.contains(**r)).map(move |r| format!("{}::{r}", t.name)))
        .collect();
    println!("M-b key coverage: {} row(s), {} claimed by {} fixture(s)", all_rows.len(), claimed.len(), FIXTURES.len());
    assert!(unclaimed.is_empty(), "M-b RED: key table rows without a fixture: {unclaimed:?}");
}

#[test]
fn corpus_covers_every_shape_arm() {
    let modules = parser_modules();
    let mut problems = Vec::new();
    for e in Entry::ALL {
        let mut need = BTreeSet::new();
        for m in e.modules() {
            let file = &modules.iter().find(|(x, _)| x == m).unwrap_or_else(|| panic!("missing module {m}")).1;
            need.extend(source::shape_arms(file));
        }
        if need.is_empty() {
            continue;
        }
        assert!(e.item_input(), "{e:?} branches on input shapes but takes no item input");
        let mut have = BTreeSet::new();
        for f in FIXTURES.iter().filter(|f| f.entry == e) {
            have.extend(fixture_shapes(f));
        }
        let missing: Vec<&String> = need.difference(&have).collect();
        println!("M-b shapes {e:?}: modules branch on {need:?}; fixtures take {have:?}");
        if !missing.is_empty() {
            problems.push(format!("{e:?}: no fixture takes {missing:?}"));
        }
    }
    assert!(problems.is_empty(), "M-b RED (critique W5):\n  {}", problems.join("\n  "));
}

#[test]
fn every_fixture_expands_to_zero_counted_forms() {
    let allow_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../boyko_symcensus/ug15/allowlist-leg1.toml");
    let allow_text = std::fs::read_to_string(&allow_path).unwrap_or_else(|e| panic!("{}: {e}", allow_path.display()));
    let allowed: Vec<AllowEntry> = source::parse_allowlist(&allow_text)
        .unwrap_or_else(|r| panic!("{r}"))
        .into_iter()
        .filter(|a| a.file.starts_with("fixture:"))
        .map(|a| AllowEntry { reason: String::new(), ..a })
        .collect();
    let mut counted: Vec<AllowEntry> = Vec::new();
    let (mut items, mut bodies) = (0usize, 0usize);
    let mut per_entry: BTreeMap<Entry, usize> = BTreeMap::new();
    for f in FIXTURES {
        let out = f.entry.expand(tokens(f.args), tokens(f.input));
        let text = match f.entry {
            Entry::Ui => format!("fn __ug15() {{ let _ = {out}; }}"),
            _ => out.to_string(),
        };
        let file: syn::File = syn::parse_str(&text).unwrap_or_else(|e| panic!("fixture {}: the expansion does not parse as a file: {e}\n{text}", f.name));
        let label = format!("fixture:{}", f.name);
        let (found, st) = source::count_file(&label, &file, Via::Expanded);
        let o = source::outline(&file);
        let anchored = match f.anchor {
            Impl(t) => o.trait_impls.iter().any(|(tr, _)| tr == t),
            CompileError => o.macros.iter().any(|m| m == "compile_error"),
            UiBlock => o.macros.is_empty() && text.contains(". spawn"),
            Item(needle) => o.items.iter().any(|i| i.contains(needle)),
        };
        assert!(anchored, "fixture {}: the expansion lacks its anchor {:?}; outline {o:?}\n{text}", f.name, f.anchor);
        assert!(st.items > 0 || f.entry == Entry::Ui, "fixture {}: the visitor visited no item of the expansion", f.name);
        items += st.items;
        bodies += st.macro_bodies;
        *per_entry.entry(f.entry).or_insert(0) += 1;
        for x in found {
            println!("  counted: {x}");
            counted.push(AllowEntry { file: x.file, item: x.item, kind: x.kind.to_string(), reason: String::new() });
        }
    }
    for e in Entry::ALL {
        assert!(per_entry.get(&e).copied().unwrap_or(0) > 0, "entry {e:?} has no fixture");
    }
    counted.sort();
    println!(
        "M-b read: {} fixture(s) over {} entry point(s), {items} item(s), {bodies} macro bod(ies) in the expansions; counted {}, allowlisted {}",
        FIXTURES.len(),
        per_entry.len(),
        counted.len(),
        allowed.len()
    );
    assert_eq!(counted, allowed, "M-b RED: the expansions' counted forms are not the owner-signed allowlist's `fixture:` entries");
}
