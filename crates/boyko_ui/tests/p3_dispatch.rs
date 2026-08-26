//! GATE 7 — DISPATCH exhaustiveness: every vocabulary component is dispatchable
//! and "text name == type name".
//!
//! The closed `.ui` vocabulary (Decision 3) is the 10 builtin `boyko_ui`
//! components: 9 are dispatched by the `parse_and_insert` closed match keyed on
//! the TEXT name (which by invariant equals the Rust type name), and `UiName`
//! comes from the `#name` sigil (never dispatched as a component). This test:
//!
//!   1. authors ONE `.ui` node naming every dispatchable component by its TYPE
//!      name, lowers it, and asserts EACH component landed on the entity (so the
//!      closed match has a live arm for every one — exhaustiveness);
//!   2. asserts the dispatch keys on the type name: a component spelled with a
//!      WRONG name (not equal to any type name) is rejected as unknown, and a
//!      component spelled with the EXACT type name is accepted;
//!   3. asserts `#name` lowers to `UiName` (the 10th, sigil-sourced) and is NOT
//!      accepted as a `UiName { .. }` component line.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod p3_common;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use std::sync::{Arc, Mutex};

use boyko_ui::components::{
    ComputedClip, ComputedRect, ContentSize, StackIndex, UiAbsolute, UiAlign, UiLayout, UiName,
    UiNineSlice, UiRoot, UiSpacing, UiSpriteAnim, UiSpriteSheet,
};
use boyko_ui::text::{parse_ui, spawn_ui_tree, UiParseReport};

/// Lowers `src`, returning the single spawned root entity + the lowering report.
fn lower_one(src: &str) -> (EcsMaster, Option<Entity>, UiParseReport) {
    let tree = parse_ui(src);
    let mut world = EcsMaster::new();
    let ent_cell: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let rep_cell: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::default()));
    let ep = Arc::clone(&ent_cell);
    let rp = Arc::clone(&rep_cell);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut report = owned.report.clone();
        let roots = spawn_ui_tree(&owned, &mut cmds, &mut report);
        let mut v = ep.lock().unwrap();
        for r in roots.iter() {
            v.push(r);
        }
        *rp.lock().unwrap() = report;
    });
    let ent = ent_cell.lock().unwrap().first().copied();
    let rep = rep_cell.lock().unwrap().clone();
    (world, ent, rep)
}

/// The dispatchable component type names this census walks.
///
/// ⚠️ **It is a SUBSET, and saying so is the point.** `parse_and_insert`'s closed
/// match has 22 arms as of UI-ADVANCED S6; this list carries the P1 layout set
/// plus S6's three. The GUI P6a widget arms (`Button`, `Bar`, `BarFill`,
/// `UiImage`, `UiGrid`, `UiAnchor`, `OnClick`/`OnHover`/`OnSubmit`) are covered by
/// `p6a_equivalence` instead and were never added here, so a census that claimed
/// completeness would be claiming more than it walks.
///
/// ⚠️ **And 12 + 9 is 21, not 22.** The twenty-second arm is **`UiText`**, which is
/// in neither list: it has a dispatch arm, it is not in `DISPATCHABLE`, and
/// `grep -c UiText p6a_equivalence.rs` returns **0**. Caught at the S6
/// verification — the correction written to stop this comment claiming more than
/// it walks was itself claiming coverage for a name that has none. `UiText` is
/// walked by nothing; that is a gap, recorded here rather than papered over. Widening it to all 22 is a
/// separate rung's work (`docs/UI-PLAN-SPRITES.md` S-D20); what S6 owes is that
/// the components IT adds are in it.
const DISPATCHABLE: [&str; 12] = [
    "UiLayout",
    "UiSpacing",
    "UiAlign",
    "UiAbsolute",
    "ContentSize",
    "ComputedRect",
    "ComputedClip",
    "StackIndex",
    "UiRoot",
    // UI-ADVANCED S6 — the sprite vocabulary.
    "UiNineSlice",
    "UiSpriteSheet",
    "UiSpriteAnim",
];

#[test]
fn dispatch_all_ten_vocabulary_components_land_on_entity() {
    // One node naming every dispatchable component by its exact TYPE name, plus a
    // `#name` for the 10th (UiName via the sigil).
    let src = "\
version=1
#full  UiLayout { layout_type: Column, width: Px(50) }
    ComputedRect { x: 0, y: 0, w: 0, h: 0 }
    UiSpacing { padding_left: Px(3) }
    UiAlign { main: Center }
    UiAbsolute { left: Px(5) }
    ContentSize { width: 12, height: 7 }
    ComputedClip { x: 1, y: 2, w: 3, h: 4 }
    StackIndex(10)
    UiRoot
";
    let (world, ent, report) = lower_one(src);
    assert!(report.is_clean(), "the full vocabulary lowers clean: {:?}", report.errors);
    let e = ent.expect("the #full node spawned");

    // Every dispatchable component must be PRESENT (its match arm fired).
    macro_rules! present {
        ($t:ty) => {
            assert!(
                world.has_component(e, <$t>::component_id()),
                "{} must be dispatchable and present on the entity",
                stringify!($t)
            );
        };
    }
    present!(UiLayout);
    present!(ComputedRect);
    present!(UiSpacing);
    present!(UiAlign);
    present!(UiAbsolute);
    present!(ContentSize);
    present!(ComputedClip);
    present!(StackIndex);
    present!(UiRoot);
    // The 10th: UiName via the #name sigil.
    present!(UiName);
    assert_eq!(
        world.get_component::<UiName>(e).map(|n| n.as_str().to_string()),
        Some("full".to_string()),
        "#name lowers to UiName == the sigil name"
    );
}

#[test]
fn dispatch_text_name_equals_type_name_each_component() {
    // For each dispatchable component, a node whose ONLY attached component is
    // that one (spelled by its type name) must lower clean and land that
    // component. This pins "text name == type name" component-by-component.
    for ty in DISPATCHABLE {
        // Build a minimal valid body per component shape.
        let line = match ty {
            "UiLayout" => "    UiLayout { layout_type: Column }".to_string(),
            "StackIndex" => "    StackIndex(0)".to_string(),
            "UiRoot" => "    UiRoot".to_string(),
            "ComputedRect" | "ComputedClip" => format!("    {ty} {{ x: 0, y: 0, w: 0, h: 0 }}"),
            "ContentSize" => "    ContentSize { width: 0, height: 0 }".to_string(),
            "UiSpacing" => "    UiSpacing { padding_left: Px(0) }".to_string(),
            "UiAlign" => "    UiAlign { main: Start }".to_string(),
            "UiAbsolute" => "    UiAbsolute { left: Px(0) }".to_string(),
            // UI-ADVANCED S6. `mode: Stretch` on the nine-slice is the token-shape
            // collision Decision 4 exists for: the SAME spelling is
            // `AlignCross::Stretch` in a `UiAlign` field and `Unit::Stretch(f)` in
            // a `Unit` field, and it resolves here only because the leaf parser is
            // chosen by the DESTINATION field's type.
            "UiNineSlice" => "    UiNineSlice { mode: Stretch }".to_string(),
            "UiSpriteSheet" => "    UiSpriteSheet { sheet: 0 }".to_string(),
            "UiSpriteAnim" => "    UiSpriteAnim { mode: PingPong }".to_string(),
            other => panic!("unhandled component {other}"),
        };
        // A host node carrying UiLayout (required) + the component under test.
        let src = format!("version=1\n#host  UiLayout {{ layout_type: Column }}\n{line}\n");
        let (world, ent, report) = lower_one(&src);
        assert!(
            report.is_clean(),
            "component `{ty}` spelled by its type name lowers clean: {:?}",
            report.errors
        );
        let e = ent.unwrap_or_else(|| panic!("host node spawned for `{ty}`"));

        // The component under test must be present.
        let present = match ty {
            "UiLayout" => world.has_component(e, UiLayout::component_id()),
            "ComputedRect" => world.has_component(e, ComputedRect::component_id()),
            "UiSpacing" => world.has_component(e, UiSpacing::component_id()),
            "UiAlign" => world.has_component(e, UiAlign::component_id()),
            "UiAbsolute" => world.has_component(e, UiAbsolute::component_id()),
            "ContentSize" => world.has_component(e, ContentSize::component_id()),
            "ComputedClip" => world.has_component(e, ComputedClip::component_id()),
            "StackIndex" => world.has_component(e, StackIndex::component_id()),
            "UiRoot" => world.has_component(e, UiRoot::component_id()),
            "UiNineSlice" => world.has_component(e, UiNineSlice::component_id()),
            "UiSpriteSheet" => world.has_component(e, UiSpriteSheet::component_id()),
            "UiSpriteAnim" => world.has_component(e, UiSpriteAnim::component_id()),
            _ => unreachable!(),
        };
        assert!(present, "component `{ty}` landed on the entity via its type-name key");
    }
}

#[test]
fn dispatch_wrong_name_is_rejected_as_unknown() {
    // A name that is NOT a vocabulary type name is an unknown-component error.
    let src = "\
version=1
#host  UiLayout { layout_type: Column }
    UiLayoutt { width: Px(1) }
    NotAComponent { x: 1 }
";
    let (_world, _ent, report) = lower_one(src);
    assert!(!report.is_clean(), "unknown component names are rejected");
    assert_eq!(
        report.errors.iter().filter(|(_, _, r)| r.contains("unknown component")).count(),
        2,
        "both misspelled components are rejected as unknown: {:?}",
        report.errors
    );
}

#[test]
fn dispatch_uiname_not_a_component_line() {
    // `UiName` is sigil-sourced, NOT a dispatchable component. A `UiName { .. }`
    // line must be rejected as unknown (it is not in the closed match).
    let src = "\
version=1
#host  UiLayout { layout_type: Column }
    UiName { bytes: 0 }
";
    let (world, ent, report) = lower_one(src);
    assert!(!report.is_clean(), "UiName is not a dispatchable component");
    assert!(
        report.errors.iter().any(|(_, _, r)| r.contains("unknown component")),
        "a UiName component line is unknown: {:?}",
        report.errors
    );
    // The host's UiName comes ONLY from the #name sigil.
    let e = ent.expect("host spawned");
    assert_eq!(
        world.get_component::<UiName>(e).map(|n| n.as_str().to_string()),
        Some("host".to_string()),
        "UiName is the sigil name, unaffected by the rejected component line"
    );
}

#[test]
fn dispatch_struct_vs_tuple_kind_enforced() {
    // StackIndex must use the tuple form; `StackIndex { 0: 1 }` / `StackIndex { }`
    // is a kind mismatch. UiLayout must use the struct form; `UiLayout(1)` is a
    // kind mismatch.
    let src_tuple_wrong = "\
version=1
#a  UiLayout { layout_type: Column }
    StackIndex { value: 1 }
";
    let (_w, _e, r) = lower_one(src_tuple_wrong);
    assert!(!r.is_clean(), "StackIndex in struct form is rejected");
    assert!(
        r.errors.iter().any(|(_, _, m)| m.contains("StackIndex") || m.contains("tuple")),
        "kind mismatch reported for StackIndex: {:?}",
        r.errors
    );

    let src_struct_wrong = "\
version=1
#b  UiLayout(1)
";
    let (_w2, _e2, r2) = lower_one(src_struct_wrong);
    assert!(!r2.is_clean(), "UiLayout in tuple form is rejected");
}
