//! GUI #27 — `.ui` NAMED action/source forms (TESTER gates).
//!
//! The `.ui` RUNTIME parser (P3, `boyko_ui::text`) now lowers the NAMED forms
//! the LLM-first authoring win needs:
//!   * `OnClick(Jump)` — an ACTION by NAME → resolved to its dense `u16` index via
//!     the process-wide action-name table (`boyko_input::register_action_names`).
//!   * `BindText { source: #healthbar, .. }` / `BindValue { source: #mana, .. }`
//!     — bind to the entity NAMED `#name` (the P3 `UiName` index), two-pass so a
//!     FORWARD reference (the `#name` declared AFTER the binding) still resolves.
//!
//! Every gate is headless (no GPU, no window). The `.ui` documents are spawned
//! through `Commands` (one apply window) exactly as the P3/P4 harness does, and
//! the resulting components are read back off the live world.
//!
//! Single-`A` contract: `register_action_names` is a process-wide write-once
//! `OnceLock`, so this whole binary uses ONE action enum [`UiTestAction`] and
//! registers it (idempotently) from a shared helper — the FIRST registration in
//! the process wins, and re-registering the same enum is a cheap no-op.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_input::{register_action_names, ActionKind, Actionlike};

use boyko_macros::Component;

use boyko_ui::binding::components::{BindText, BindValue, TemplateId, NO_FIELD};
use boyko_ui::components::UiName;
use boyko_ui::interaction::action::{OnClick, OnHover, OnSubmit, NO_ACTION};
use boyko_ui::text::{parse_ui, spawn_ui_tree, UiParseReport};

// ───────────────────────────── shared fixtures ──────────────────────────────

/// The single action enum for this whole test binary (single-`A` contract).
/// Names `Jump`/`Fire`/`Menu` at dense indices 0/1/2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UiTestAction {
    Jump,
    Fire,
    Menu,
}

impl Actionlike for UiTestAction {
    const COUNT: usize = 3;
    fn index(self) -> usize {
        match self {
            UiTestAction::Jump => 0,
            UiTestAction::Fire => 1,
            UiTestAction::Menu => 2,
        }
    }
    fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(UiTestAction::Jump),
            1 => Some(UiTestAction::Fire),
            2 => Some(UiTestAction::Menu),
            _ => None,
        }
    }
    fn kind(self) -> ActionKind {
        ActionKind::Button
    }
    fn name(self) -> &'static str {
        match self {
            UiTestAction::Jump => "Jump",
            UiTestAction::Fire => "Fire",
            UiTestAction::Menu => "Menu",
        }
    }
}

/// Registers the action-name table once for the process (idempotent). Every test
/// that authors an `OnClick(Name)` calls this first; the write-once `OnceLock`
/// makes a second call a no-op, so parallel tests are safe.
fn ensure_actions_registered() {
    register_action_names::<UiTestAction>();
}

/// A source component mirroring the P4 `Health` (fields `current`/`max` at ids
/// 0/1). It exists only so [`Health::component_id`] is a real id for the
/// hand-spawn equivalence comparison; the resolution gates inspect the lowered
/// `BindText`/`BindValue` bytes, which never read the source, so no `Bindable`
/// accessor registration is needed.
#[derive(Component, Clone, Copy, Debug)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

/// Spawns a `.ui` document through `Commands` (one apply window), returning the
/// post-resolve `UiParseReport` (pass-2 unknown-`#name` errors land here) and the
/// live world. The world is kept so component bytes can be read back.
///
/// Mirrors `dot_ui_onclick_numeric_authorable` / the P3 `spawn_dot_ui` harness;
/// it does NOT assert the report is clean (the malformed gates need the errors).
fn spawn_dot_ui_world(src: &str) -> (EcsMaster, UiParseReport) {
    let tree = parse_ui(src);
    let mut world = EcsMaster::new();
    let rep_cell: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::default()));
    let rp = Arc::clone(&rep_cell);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        // Seed the lowering report from the parse report so a clean parse + a clean
        // lowering compose into one report (the production callers' contract).
        let mut report = owned.report.clone();
        let _ = spawn_ui_tree(&owned, &mut cmds, &mut report);
        *rp.lock().unwrap() = report;
    });
    let rep = rep_cell.lock().unwrap().clone();
    (world, rep)
}

/// Finds the single live entity carrying `UiName == name`.
fn find_named(world: &EcsMaster, name: &str) -> Option<Entity> {
    world
        .query_entities(&[UiName::component_id()])
        .into_iter()
        .find(|&e| {
            world
                .get_component::<UiName>(e)
                .map(|n| n.as_str() == name)
                .unwrap_or(false)
        })
}

// ───────────── GATE 1+2: BindText/BindValue {source:#name} resolves ──────────

#[test]
fn gate1_bind_text_named_source_resolves_to_uiname_entity() {
    // `#bar` binds to `#bar`-named `health` source; the `BindText.source` must be
    // exactly the entity carrying `UiName == "health"`.
    let src = "\
version=1
#health  UiLayout { layout_type: Column }
#bar  UiLayout { layout_type: Column }
    BindText { source: #health, comp: 7, field: 0, field2: 1, template: Ratio }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "named-source .ui parses + resolves clean: {:?}", rep.errors);

    let health = find_named(&world, "health").expect("#health spawned");
    let bar = find_named(&world, "bar").expect("#bar spawned");

    let bind = world.get_component::<BindText>(bar).expect("BindText inserted on #bar");
    assert_eq!(bind.source, health, "BindText.source resolves to the UiName==health entity");
    assert_eq!(bind.comp.0, 7, "comp numeric id preserved");
    assert_eq!(bind.field, 0);
    assert_eq!(bind.field2, 1, "field2 numeric preserved");
    assert_eq!(bind.template, TemplateId::Ratio);
}

#[test]
fn gate1_bind_value_named_source_resolves_to_uiname_entity() {
    let src = "\
version=1
#mana  UiLayout { layout_type: Column }
#orb  UiLayout { layout_type: Column }
    BindValue { source: #mana, comp: 7, num_field: 0, den_field: 1 }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "named BindValue parses + resolves clean: {:?}", rep.errors);

    let mana = find_named(&world, "mana").expect("#mana spawned");
    let orb = find_named(&world, "orb").expect("#orb spawned");

    let bind = world.get_component::<BindValue>(orb).expect("BindValue inserted on #orb");
    assert_eq!(bind.source, mana, "BindValue.source resolves to UiName==mana");
    assert_eq!(bind.comp.0, 7);
    assert_eq!(bind.num_field, 0);
    assert_eq!(bind.den_field, 1);
}

#[test]
fn gate2_named_source_byte_equals_numeric_source() {
    // EQUIVALENCE: a `#name` source and the equivalent NUMERIC entity-id source
    // (the same target entity's id) lower to the SAME `BindText` bytes. We build
    // the numeric form by reading the named target's id, then re-spawning a fresh
    // doc that hard-codes that id.
    let named_src = "\
version=1
#hp  UiLayout { layout_type: Column }
#label  UiLayout { layout_type: Column }
    BindText { source: #hp, comp: 7, field: 0, field2: NO_FIELD, template: Value }
";
    let (named_world, named_rep) = spawn_dot_ui_world(named_src);
    assert!(named_rep.is_clean(), "named form clean: {:?}", named_rep.errors);
    let hp = find_named(&named_world, "hp").expect("#hp spawned");
    let label = find_named(&named_world, "label").expect("#label spawned");
    let named_bind = *named_world.get_component::<BindText>(label).expect("named BindText");

    // The numeric source must point at the SAME entity id the named target got. A
    // fresh world deterministically reuses entity ids from 0, so spawning `#hp`
    // first in BOTH docs makes `hp.id()` identical; assert it then numeric-bind it.
    let numeric_src = format!(
        "\
version=1
#hp  UiLayout {{ layout_type: Column }}
#label  UiLayout {{ layout_type: Column }}
    BindText {{ source: {}, comp: 7, field: 0, field2: NO_FIELD, template: Value }}
",
        hp.id().0
    );
    let (numeric_world, numeric_rep) = spawn_dot_ui_world(&numeric_src);
    assert!(numeric_rep.is_clean(), "numeric form clean: {:?}", numeric_rep.errors);
    let num_label = find_named(&numeric_world, "label").expect("numeric #label spawned");
    let numeric_bind = *numeric_world.get_component::<BindText>(num_label).expect("numeric BindText");

    assert_eq!(
        named_bind, numeric_bind,
        "named #name source lowers byte-identically to the numeric entity-id source"
    );
    assert_eq!(named_bind.source, hp, "sanity: named bind points at #hp");
}

#[test]
fn gate2_named_bind_byte_equals_hand_spawned() {
    // EQUIVALENCE vs the hand-spawn / `ui!`-literal path: the lowered `BindText`
    // must equal a directly-constructed `BindText` with the resolved source.
    let src = "\
version=1
#hp  UiLayout { layout_type: Column }
#label  UiLayout { layout_type: Column }
    BindText { source: #hp, comp: 7, field: 0, field2: 1, template: Ratio }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "{:?}", rep.errors);
    let hp = find_named(&world, "hp").expect("#hp");
    let label = find_named(&world, "label").expect("#label");
    let lowered = *world.get_component::<BindText>(label).expect("BindText");

    let hand = BindText {
        source: hp,
        comp: Health::component_id(), // not asserted equal — comp is numeric 7 below
        field: 0,
        field2: 1,
        template: TemplateId::Ratio,
    };
    // Compare every field except `comp` (the .ui used a literal numeric 7, the
    // hand form used the real Health id — the equivalence under test is the
    // NAMED-SOURCE resolution + the field lowering, not the comp id encoding).
    assert_eq!(lowered.source, hand.source, "source matches hand-spawn");
    assert_eq!(lowered.field, hand.field);
    assert_eq!(lowered.field2, hand.field2);
    assert_eq!(lowered.template, hand.template);
    assert_eq!(lowered.comp.0, 7, "comp lowered from the literal numeric id");
}

// ───────────────────────── GATE 3: forward reference ────────────────────────

#[test]
fn gate3_forward_reference_resolves_two_pass() {
    // The bind is declared BEFORE its `#name` target — only a two-pass resolve
    // (record all names, THEN resolve) makes this work.
    let src = "\
version=1
#label  UiLayout { layout_type: Column }
    BindText { source: #hp, comp: 7, field: 0, field2: NO_FIELD, template: Value }
#hp  UiLayout { layout_type: Column }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "forward #name reference resolves clean (two-pass): {:?}", rep.errors);

    let hp = find_named(&world, "hp").expect("#hp spawned (declared after the bind)");
    let label = find_named(&world, "label").expect("#label spawned");
    let bind = world.get_component::<BindText>(label).expect("forward-ref BindText inserted");
    assert_eq!(bind.source, hp, "a forward #name reference resolves to the later-declared entity");
}

#[test]
fn gate3_forward_reference_bind_value_resolves() {
    let src = "\
version=1
#orb  UiLayout { layout_type: Column }
    BindValue { source: #mana, comp: 7, num_field: 0, den_field: 1 }
#mana  UiLayout { layout_type: Column }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "forward BindValue #name resolves clean: {:?}", rep.errors);
    let mana = find_named(&world, "mana").expect("#mana");
    let orb = find_named(&world, "orb").expect("#orb");
    let bind = world.get_component::<BindValue>(orb).expect("BindValue");
    assert_eq!(bind.source, mana, "forward BindValue source resolves");
}

// ───────────── GATE 4: unknown name → recoverable per-line error ─────────────

#[test]
fn gate4_unknown_hash_name_is_recoverable_rest_of_file_loads() {
    // `#ghost` is never declared. The bind must NOT be inserted, a per-line error
    // (line:col) must be recorded, and the REST of the file (the sibling node +
    // its valid bind) must still load. No panic.
    let src = "\
version=1
#good  UiLayout { layout_type: Column }
#widget_a  UiLayout { layout_type: Column }
    BindText { source: #ghost, comp: 7, field: 0, field2: NO_FIELD, template: Value }
#widget_b  UiLayout { layout_type: Column }
    BindText { source: #good, comp: 7, field: 0, field2: NO_FIELD, template: Value }
";
    let (world, rep) = spawn_dot_ui_world(src);

    // Recoverable: an error was recorded, naming the unknown source.
    assert!(!rep.is_clean(), "an unknown #name records an error");
    let ghost_err = rep
        .errors
        .iter()
        .find(|(_, _, m)| m.contains("ghost"))
        .expect("the error names the unknown #ghost source");
    assert!(ghost_err.0 >= 1, "error carries a 1-based line number, got {}", ghost_err.0);
    // (col is u16, always present in the tuple — the (line, col, msg) contract.)

    // The bad bind never inserted (no sentinel reaches the world).
    let widget_a = find_named(&world, "widget_a").expect("#widget_a still spawned");
    assert!(
        world.get_component::<BindText>(widget_a).is_none(),
        "the unknown-#name bind is NOT inserted (deferred insert dropped on miss)"
    );

    // The REST of the file loaded: the good sibling + its valid bind survive.
    let good = find_named(&world, "good").expect("#good spawned");
    let widget_b = find_named(&world, "widget_b").expect("#widget_b spawned after the bad bind");
    let good_bind = world.get_component::<BindText>(widget_b).expect("the valid sibling bind survives");
    assert_eq!(good_bind.source, good, "the recoverable error did not abort the rest of the load");
}

#[test]
fn gate4_unknown_action_name_is_recoverable_inserts_no_action() {
    // `OnClick(Sprint)` is an unregistered action name. Per the dispatch contract
    // it records a recoverable per-line error AND inserts `OnClick(NO_ACTION)` so
    // the node still spawns (dispatch fires nothing). The rest of the file loads.
    ensure_actions_registered();
    let src = "\
version=1
#btn  UiLayout { layout_type: Column }
    OnClick(Sprint)
#ok  UiLayout { layout_type: Column }
    OnClick(Jump)
";
    let (world, rep) = spawn_dot_ui_world(src);

    assert!(!rep.is_clean(), "an unknown action name records an error");
    let err = rep
        .errors
        .iter()
        .find(|(_, _, m)| m.contains("Sprint") || m.contains("unknown action"))
        .expect("the error names the unknown action / unknown-action reason");
    assert!(err.0 >= 1, "error carries a 1-based line, got {}", err.0);

    let btn = find_named(&world, "btn").expect("#btn still spawned");
    let oc = world.get_component::<OnClick>(btn).expect("OnClick still inserted (NO_ACTION)");
    assert_eq!(oc.0, NO_ACTION, "an unknown action name lowers to NO_ACTION (fires nothing)");

    // Rest of the file loaded: the valid OnClick(Jump) sibling resolved.
    let ok = find_named(&world, "ok").expect("#ok spawned after the bad action");
    let oc_ok = world.get_component::<OnClick>(ok).expect("OnClick(Jump) inserted");
    assert_eq!(oc_ok.0, UiTestAction::Jump.index() as u16, "the valid sibling action resolved");
}

#[test]
fn gate4_empty_hash_name_is_recoverable_no_panic() {
    // A malformed `#` (empty name) must be a recoverable per-line error, never a
    // panic; the sibling node still loads.
    let src = "\
version=1
#widget  UiLayout { layout_type: Column }
    BindText { source: #, comp: 7, field: 0, field2: NO_FIELD, template: Value }
#sibling  UiLayout { layout_type: Column }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(!rep.is_clean(), "an empty #name is an error");
    assert!(
        find_named(&world, "sibling").is_some(),
        "the sibling still loads after a malformed #name (recoverable)"
    );
}

// ─────────────────── GATE 5: OnClick(Name) (IN SCOPE) ───────────────────────

#[test]
fn gate5_onclick_name_equals_numeric_equals_index() {
    // EQUIVALENCE: `OnClick(Jump)` lowers to the SAME `u16` as `OnClick(0)` and as
    // `UiTestAction::Jump.index()` (the ui!/hand-spawn integer form).
    ensure_actions_registered();

    let named = "\
version=1
#btn  UiLayout { layout_type: Column }
    OnClick(Jump)
";
    let (named_world, named_rep) = spawn_dot_ui_world(named);
    assert!(named_rep.is_clean(), "OnClick(Jump) authors clean: {:?}", named_rep.errors);
    let nb = find_named(&named_world, "btn").expect("#btn");
    let named_idx = named_world.get_component::<OnClick>(nb).expect("OnClick").0;

    let numeric = "\
version=1
#btn  UiLayout { layout_type: Column }
    OnClick(0)
";
    let (numeric_world, numeric_rep) = spawn_dot_ui_world(numeric);
    assert!(numeric_rep.is_clean(), "OnClick(0) authors clean: {:?}", numeric_rep.errors);
    let cb = find_named(&numeric_world, "btn").expect("#btn");
    let numeric_idx = numeric_world.get_component::<OnClick>(cb).expect("OnClick").0;

    let index_form = UiTestAction::Jump.index() as u16;

    assert_eq!(named_idx, index_form, "OnClick(Jump) == Jump.index() (ui!/hand-spawn form)");
    assert_eq!(named_idx, numeric_idx, "OnClick(Jump) == OnClick(0) (named == numeric)");
}

#[test]
fn gate5_onhover_onsubmit_names_resolve() {
    // The other two action-emitting tuple components also accept names.
    ensure_actions_registered();
    let src = "\
version=1
#btn  UiLayout { layout_type: Column }
    OnHover(Fire)
    OnSubmit(Menu)
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "OnHover/OnSubmit names author clean: {:?}", rep.errors);
    let btn = find_named(&world, "btn").expect("#btn");
    assert_eq!(
        world.get_component::<OnHover>(btn).expect("OnHover").0,
        UiTestAction::Fire.index() as u16,
        "OnHover(Fire) resolves to Fire.index()"
    );
    assert_eq!(
        world.get_component::<OnSubmit>(btn).expect("OnSubmit").0,
        UiTestAction::Menu.index() as u16,
        "OnSubmit(Menu) resolves to Menu.index()"
    );
}

// ───────────────────── GATE 6: numeric .ui no regression ─────────────────────

#[test]
fn gate6_numeric_onclick_still_works() {
    let src = "\
version=1
#btn  UiLayout { layout_type: Column }
    OnClick(3)
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "numeric OnClick(3) still authors clean: {:?}", rep.errors);
    let btn = find_named(&world, "btn").expect("#btn");
    assert_eq!(world.get_component::<OnClick>(btn).expect("OnClick").0, 3, "OnClick(3) lowers to 3");
}

#[test]
fn gate6_numeric_bindtext_still_works() {
    // The P4 numeric-source form (the original v1 limit) must be untouched.
    let src = "\
version=1
#widget  UiLayout { layout_type: Column }
    BindText { source: 5, comp: 7, field: 0, field2: NO_FIELD, template: Value }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "numeric-source BindText still authors clean: {:?}", rep.errors);
    let widget = find_named(&world, "widget").expect("#widget");
    let bind = world.get_component::<BindText>(widget).expect("numeric BindText inserted in pass 1");
    assert_eq!(bind.source.id().0, 5, "numeric source id 5 preserved");
    assert_eq!(bind.field2, NO_FIELD, "field2 NO_FIELD preserved");
    assert_eq!(bind.template, TemplateId::Value);
}

#[test]
fn gate6_numeric_bindvalue_still_works() {
    let src = "\
version=1
#widget  UiLayout { layout_type: Column }
    BindValue { source: 9, comp: 7, num_field: 0, den_field: NO_FIELD }
";
    let (world, rep) = spawn_dot_ui_world(src);
    assert!(rep.is_clean(), "numeric-source BindValue still clean: {:?}", rep.errors);
    let widget = find_named(&world, "widget").expect("#widget");
    let bind = world.get_component::<BindValue>(widget).expect("numeric BindValue");
    assert_eq!(bind.source.id().0, 9, "numeric source id 9 preserved");
    assert_eq!(bind.den_field, NO_FIELD, "den_field raw value (NO_FIELD) preserved");
}

// ─────────── extra: a registered-but-different action enum is a no-op ─────────

#[test]
fn unregistered_action_name_when_no_enum_matches_is_recoverable() {
    // A name not in the registered enum (`Sprint` is not a UiTestAction variant)
    // is recoverable — covered by gate4; this asserts the well-known names DO
    // resolve so the table is genuinely populated (not silently empty).
    ensure_actions_registered();
    assert_eq!(boyko_input::resolve_action_name("Jump"), Some(0));
    assert_eq!(boyko_input::resolve_action_name("Fire"), Some(1));
    assert_eq!(boyko_input::resolve_action_name("Menu"), Some(2));
    assert_eq!(boyko_input::resolve_action_name("Sprint"), None, "unknown name → None (recoverable)");
}
