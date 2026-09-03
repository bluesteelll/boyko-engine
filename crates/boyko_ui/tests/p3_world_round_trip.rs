//! GATE 2b — WORLD ROUND-TRIP: `world → serialize → parse → world`, compared as
//! two WORLDS, component by component.
//!
//! # Why this exists beside `p3_round_trip.rs`, and why neither may be deleted
//!
//! `p3_round_trip.rs` asserts that `serialize → parse → serialize` is
//! byte-identical. That is the right gate for the CANONICAL-FORM property (the
//! writer is a fixed point) and the wrong gate for the LOSS property, because it
//! is STRUCTURALLY unable to fail on a dropped component: a component the writer
//! never writes is absent from both texts, so it never enters the comparison.
//! Measured on this tree: the writer emitted 8 of the 20 non-exempt members of
//! its own parser's vocabulary — every `UiText`, `UiImage`, `UiGrid`, `UiAnchor`,
//! `Button`, `Bar`, `BarFill`, `OnClick`, `OnHover`, `OnSubmit`, `BindText`,
//! `BindValue` was discarded on save — and that gate was green over all twelve.
//!
//! A round trip that starts from TEXT cannot see what the writer never wrote. So
//! this gate starts from a WORLD: spawn a document carrying every member of the
//! vocabulary with NON-DEFAULT values, serialize the live subtree, spawn the
//! serialized text into a SECOND world, and compare the two worlds by reading
//! each component back out of ECS storage. A dropped component is then a missing
//! component in world B — a red test rather than an invisible one.
//!
//! # Two hostile arrangements this gate makes on purpose
//!
//! 1. **Every authored field value differs from its `Default`.** A `.ui`
//!    per-field parse failure falls back to `T::default()`, so a corpus that
//!    authors the default value cannot tell a successful parse from a failed one.
//!    That is not hypothetical: `p6a_equivalence`'s image case authors
//!    `uv_min: [0, 0]` / `uv_max: [1, 1]`, which are exactly `UiImage::default()`.
//! 2. **World B's entity ids are OFFSET** from world A's by filler entities
//!    spawned first. `BindText` / `BindValue` carry an `Entity`; with identical
//!    spawn orders both worlds hand out identical ids, and a writer emitting a
//!    raw numeric `source` would round-trip by coincidence. With the offset, only
//!    the `#name` source form — re-resolved against world B's own name index —
//!    survives.
//!
//! The vocabulary comes from `boyko_ui::text::vocab`, so the corpus, the
//! comparison and the writer all answer to ONE list; the census tests at the
//! bottom go red when any of them drifts from it.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling spawned `Entity` handles and a `UiParseReport` out of the `Send + Sync`
// one-shot system closure. Not engine code — the whole file is compiled out of
// every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_ecs::ecs::core::system::Commands;

use boyko_ui::binding::{BindText, BindValue};
use boyko_ui::components::{
    Bar, BarFill, Button, ComputedClip, ContentSize, StackIndex, UiAbsolute, UiAlign, UiAnchor,
    UiGrid, UiImage, UiLayout, UiName, UiRoot, UiSpacing,
};
use boyko_ui::interaction::{OnClick, OnHover, OnSubmit};
use boyko_ui::reload::tree_view::UiTreeView;
use boyko_ui::text::vocab::{UiTextComponent, WriterPolicy};
use boyko_ui::text::{parse_ui, serialize_ui, spawn_ui_tree, UiParseReport, UiText};

// ─────────────────────────── the coverage corpus ──────────────────────────────

/// A document carrying EVERY member of the `.ui` vocabulary, each field set to a
/// value distinct from its `Default` (module docs, arrangement 1).
///
/// `#src` exists so the two bind components have a `#name` source to resolve. It
/// sits at `rel == STEP`, where the parser classifies by the leading sigil: a `#`
/// lead is a child head, a bare identifier is an attached component.
const EVERY_COMPONENT_DOC: &str = "\
version=1
#root  UiLayout { layout_type: Row, position_type: Absolute, width: Px(321), height: Pct(55.5), min_width: Px(3), min_height: Px(4), max_width: Px(999), max_height: Stretch(2.5) }
    UiSpacing { padding_left: Px(1), padding_right: Px(2), padding_top: Px(3), padding_bottom: Px(4), border_left: Px(5), border_right: Px(6), border_top: Px(7), border_bottom: Px(8), row_gap: Px(9), column_gap: Px(10) }
    UiAlign { main: SpaceBetween, cross: End }
    UiAbsolute { left: Px(11), right: Px(12), top: Px(13), bottom: Px(14) }
    ContentSize { width: 15.5, height: 16.25 }
    StackIndex(17)
    ComputedRect { x: 41, y: 42, w: 43, h: 44 }
    ComputedClip { x: 18, y: 19, w: 20, h: 21.5 }
    UiRoot
    UiText { color: 4278255615, size_px: 22.5, font: 3, align: Right }
    UiImage { texture: 9, uv_min: [0.125, 0.25], uv_max: [0.75, 0.875], tint: 16711935 }
    UiGrid { columns: 4, rows: 5 }
    UiAnchor { edge: BottomRight, offset_x: 23.5, offset_y: 24.5, use_safe_area: true }
    Button
    Bar
    BarFill
    OnClick(31)
    OnHover(32)
    OnSubmit(33)
    BindText { source: #src, comp: 7, field: 2, field2: 3, template: Ratio }
    BindValue { source: #src, comp: 8, num_field: 4, den_field: 5 }
    #src  UiLayout { layout_type: Column, width: Px(34), height: Px(35) }
";

// ─────────────────────────── world / text helpers ─────────────────────────────

/// Spawns `src` into `world`, returning the document roots and the merged parse +
/// LOWERING report.
///
/// Keeping the lowering report is load-bearing: the type-directed per-field parse
/// runs at lowering time, so a bad field value is recorded THERE. A harness that
/// checks only `parse_ui`'s syntactic report is green over every per-field
/// failure, and a failed field silently keeps `T::default()`.
fn spawn_doc(world: &mut EcsMaster, src: &str) -> (Vec<Entity>, UiParseReport) {
    let tree = parse_ui(src);
    let roots: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let report: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::default()));
    let root_probe = Arc::clone(&roots);
    let report_probe = Arc::clone(&report);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut rep = owned.report.clone();
        let spawned = spawn_ui_tree(&owned, &mut cmds, &mut rep);
        let mut v = root_probe.lock().expect("probe");
        for r in spawned.iter() {
            v.push(r);
        }
        *report_probe.lock().expect("probe") = rep;
    });
    let out = roots.lock().expect("probe").clone();
    let rep = report.lock().expect("probe").clone();
    (out, rep)
}

/// Snapshots the subtree at `roots` and serializes it to canonical `.ui` text.
fn serialize_doc(world: &EcsMaster, roots: &[Entity]) -> String {
    let view = UiTreeView::build(world, roots);
    let mut out = String::new();
    serialize_ui(&view, &mut out);
    out
}

/// A world whose entity ids are offset from a freshly created one, so a raw
/// numeric `Entity` cannot round-trip by coincidence (module docs, arrangement 2).
fn offset_world(fillers: usize) -> EcsMaster {
    let mut world = EcsMaster::new();
    world.run_system(move |mut cmds: Commands| {
        for _ in 0..fillers {
            cmds.spawn(ComputedClip::default());
        }
    });
    world
}

/// A node's `UiName` as an owned string, or `None`.
fn name_of(world: &EcsMaster, e: Entity) -> Option<String> {
    world.get_component::<UiName>(e).map(|n| n.as_str().to_string())
}

/// A node's children in `Children` slice order.
fn children_of(world: &EcsMaster, e: Entity) -> Vec<Entity> {
    world
        .get_component::<Children>(e)
        .map(|c| c.as_slice().to_vec())
        .unwrap_or_default()
}

// ───────────────────────── the component comparison ───────────────────────────

/// Declares, in ONE place, which components the world comparison reads and how.
///
/// `data` are compared by their `Debug` projection (the P3 method — most layout
/// components are POD without `PartialEq`, and a derived `Debug` is a TOTAL
/// projection of every field, so the comparison cannot forget a field the way a
/// hand-written one can). `zst` are markers, compared by presence. `bind` carry a
/// live `Entity`, which is per-world: their other fields are compared directly and
/// the source is compared by the NAME of the node it points at, which is what
/// "the same binding" means across two worlds.
///
/// The invocation also emits `COMPARED_NAMES`, which
/// `comparison_covers_every_emitted_vocabulary_member` pins against the
/// vocabulary — so this list cannot quietly cover less than the writer emits.
macro_rules! declare_comparison {
    (
        data: [ $($d:ident),+ $(,)? ],
        zst: [ $($z:ident),+ $(,)? ],
        bind: [ $( $b:ident { src: $src:ident, plain: [ $($f:ident),+ $(,)? ] } ),+ $(,)? ] $(,)?
    ) => {
        /// Every component name the world comparison actually reads.
        const COMPARED_NAMES: &[&str] = &[
            $( stringify!($d), )+
            $( stringify!($z), )+
            $( stringify!($b), )+
        ];

        /// Compares one matched node pair, pushing one line per mismatch.
        fn compare_node(
            a: &EcsMaster,
            ea: Entity,
            b: &EcsMaster,
            eb: Entity,
            path: &str,
            out: &mut Vec<String>,
        ) {
            $(
                {
                    let va = a.get_component::<$d>(ea).map(|v| format!("{v:?}"));
                    let vb = b.get_component::<$d>(eb).map(|v| format!("{v:?}"));
                    if va != vb {
                        out.push(format!(
                            "{path}: {} differs\n        world A: {}\n        world B: {}",
                            stringify!($d),
                            va.as_deref().unwrap_or("<absent>"),
                            vb.as_deref().unwrap_or("<absent>"),
                        ));
                    }
                }
            )+
            $(
                {
                    let pa = a.has_component(ea, <$z>::component_id());
                    let pb = b.has_component(eb, <$z>::component_id());
                    if pa != pb {
                        out.push(format!(
                            "{path}: {} presence differs (world A: {pa}, world B: {pb})",
                            stringify!($z),
                        ));
                    }
                }
            )+
            $(
                {
                    let va = a.get_component::<$b>(ea).copied();
                    let vb = b.get_component::<$b>(eb).copied();
                    match (va, vb) {
                        (None, None) => {}
                        (Some(x), Some(y)) => {
                            $(
                                if format!("{:?}", x.$f) != format!("{:?}", y.$f) {
                                    out.push(format!(
                                        "{path}: {}.{} differs (world A: {:?}, world B: {:?})",
                                        stringify!($b), stringify!($f), x.$f, y.$f,
                                    ));
                                }
                            )+
                            let na = name_of(a, x.$src);
                            let nb = name_of(b, y.$src);
                            if na != nb {
                                out.push(format!(
                                    "{path}: {}.{} points at a different node (world A: {}, world B: {})",
                                    stringify!($b),
                                    stringify!($src),
                                    na.as_deref().unwrap_or("<unnamed or dangling>"),
                                    nb.as_deref().unwrap_or("<unnamed or dangling>"),
                                ));
                            }
                        }
                        (x, y) => {
                            out.push(format!(
                                "{path}: {} presence differs (world A: {}, world B: {})",
                                stringify!($b),
                                x.is_some(),
                                y.is_some(),
                            ));
                        }
                    }
                }
            )+
        }
    };
}

declare_comparison! {
    data: [
        UiLayout, UiSpacing, UiAlign, UiAbsolute, ContentSize, StackIndex, ComputedClip, UiText,
        UiImage, UiGrid, UiAnchor, OnClick, OnHover, OnSubmit,
    ],
    zst: [UiRoot, Button, Bar, BarFill],
    bind: [
        BindText { src: source, plain: [comp, field, field2, template] },
        BindValue { src: source, plain: [comp, num_field, den_field] },
    ],
}

/// Walks both subtrees in parallel and compares every matched node pair.
fn compare_subtrees(
    a: &EcsMaster,
    ea: Entity,
    b: &EcsMaster,
    eb: Entity,
    path: &str,
    out: &mut Vec<String>,
) {
    let na = name_of(a, ea);
    let nb = name_of(b, eb);
    if na != nb {
        out.push(format!("{path}: UiName differs (world A: {na:?}, world B: {nb:?})"));
    }
    compare_node(a, ea, b, eb, path, out);

    let ca = children_of(a, ea);
    let cb = children_of(b, eb);
    if ca.len() != cb.len() {
        out.push(format!(
            "{path}: child count differs (world A: {}, world B: {})",
            ca.len(),
            cb.len()
        ));
        return;
    }
    for (i, (&kid_a, &kid_b)) in ca.iter().zip(cb.iter()).enumerate() {
        let child_path = match name_of(a, kid_a) {
            Some(n) => format!("{path}/#{n}"),
            None => format!("{path}/[{i}]"),
        };
        compare_subtrees(a, kid_a, b, kid_b, &child_path, out);
    }
}

// ─────────────────── document text → declared component names ─────────────────

/// The component names a `.ui` document declares, in source order.
///
/// A head line is `#name  Component { … }`; an attached line is `Component …`.
/// Reading the leading identifier is exact where a substring search is not:
/// `text.contains("Bar")` is true for a document that declares only `BarFill`.
fn declared_components(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        if rest.is_empty() || rest.starts_with("//") || rest.starts_with("version=") {
            continue;
        }
        if let Some(after_sigil) = rest.strip_prefix('#') {
            // A named head: skip the name and its separating whitespace.
            match after_sigil.find(char::is_whitespace) {
                Some(sep) => rest = after_sigil[sep..].trim_start(),
                None => continue, // a bare `#name` node with no component
            }
        }
        let ident: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !ident.is_empty() {
            out.push(ident);
        }
    }
    out
}

// ──────────────────────────────── the gate ────────────────────────────────────

#[test]
fn world_round_trip_preserves_every_emitted_component() {
    let mut world_a = EcsMaster::new();
    let (roots_a, report_a) = spawn_doc(&mut world_a, EVERY_COMPONENT_DOC);
    assert!(
        report_a.is_clean(),
        "the coverage corpus must parse AND lower clean; errors: {:?}",
        report_a.errors
    );
    assert_eq!(roots_a.len(), 1, "the corpus declares exactly one root");

    let text = serialize_doc(&world_a, &roots_a);

    // World B is id-offset so a numeric `Entity` cannot round-trip by luck.
    let mut world_b = offset_world(3);
    let (roots_b, report_b) = spawn_doc(&mut world_b, &text);
    assert!(
        report_b.is_clean(),
        "serialized text must parse AND lower clean; errors: {:?}\n--- text ---\n{text}",
        report_b.errors
    );
    assert_eq!(
        roots_b.len(),
        roots_a.len(),
        "root count survives the round trip\n--- text ---\n{text}"
    );

    let mut mismatches = Vec::new();
    for (i, (&ra, &rb)) in roots_a.iter().zip(roots_b.iter()).enumerate() {
        let path = match name_of(&world_a, ra) {
            Some(n) => format!("#{n}"),
            None => format!("[root {i}]"),
        };
        compare_subtrees(&world_a, ra, &world_b, rb, &path, &mut mismatches);
    }

    assert!(
        mismatches.is_empty(),
        "{} component(s) did not survive world → text → world:\n    {}\n--- serialized text ---\n{text}",
        mismatches.len(),
        mismatches.join("\n    ")
    );
}

// ─────────────────────────────── the censuses ─────────────────────────────────

#[test]
fn corpus_declares_every_vocabulary_member() {
    let declared = declared_components(EVERY_COMPONENT_DOC);
    let missing: Vec<&str> = UiTextComponent::ALL
        .iter()
        .map(|c| c.name())
        .filter(|n| !declared.iter().any(|d| d == n))
        .collect();
    assert!(
        missing.is_empty(),
        "the coverage corpus must author every vocabulary member (a gate over a corpus that \
         omits a component is a gate that cannot fail on it); missing: {missing:?}"
    );
}

#[test]
fn writer_emits_every_non_exempt_vocabulary_member() {
    let mut world = EcsMaster::new();
    let (roots, report) = spawn_doc(&mut world, EVERY_COMPONENT_DOC);
    assert!(report.is_clean(), "corpus lowers clean; errors: {:?}", report.errors);
    let text = serialize_doc(&world, &roots);
    let declared = declared_components(&text);

    let mut dropped = Vec::new();
    let mut leaked = Vec::new();
    for c in UiTextComponent::ALL.iter().copied() {
        let present = declared.iter().any(|d| d == c.name());
        match c.writer_policy() {
            WriterPolicy::Emit if !present => dropped.push(c.name()),
            WriterPolicy::Omit(_) if present => leaked.push(c.name()),
            _ => {}
        }
    }
    assert!(
        dropped.is_empty() && leaked.is_empty(),
        "writer coverage census failed\n    dropped (declared `Emit`, absent from the output): \
         {dropped:?}\n    leaked (declared `Omit`, present in the output): {leaked:?}\n\
         --- serialized text ---\n{text}"
    );
}

#[test]
fn parser_accepts_exactly_the_declared_vocabulary() {
    for c in UiTextComponent::ALL.iter().copied() {
        assert_eq!(
            UiTextComponent::from_name(c.name()),
            Some(c),
            "every vocabulary member resolves from its own text name"
        );
    }
    // Negative control — the census must be ABLE to fail. `UiBackground` is a real
    // `boyko_ui` component the `.ui` format does NOT accept (it is authorable only
    // from `ui!` / Rust), so it must resolve to nothing and be reported as an
    // unknown component by the lowering path.
    assert_eq!(
        UiTextComponent::from_name("UiBackground"),
        None,
        "a component outside the vocabulary must not resolve"
    );
    let mut world = EcsMaster::new();
    let (_roots, report) = spawn_doc(
        &mut world,
        "version=1\n#n  UiLayout { width: Px(1), height: Px(1) }\n    UiBackground { color: 1 }\n",
    );
    assert!(
        report.errors.iter().any(|(_, _, why)| why.contains("unknown component")),
        "a non-vocabulary component is a recoverable `unknown component` error; got: {:?}",
        report.errors
    );
}

#[test]
fn writer_exemptions_are_declared_and_minimal() {
    let exempt: Vec<&str> = UiTextComponent::ALL
        .iter()
        .copied()
        .filter(|c| !c.writer_policy().is_emitted())
        .map(|c| c.name())
        .collect();
    assert_eq!(
        exempt,
        vec!["ComputedRect"],
        "the writer's exemption list is a DECISION, not an accident: a member added to it must \
         carry its reason in `WriterPolicy::Omit` and be argued in this test"
    );
}

#[test]
fn comparison_covers_every_emitted_vocabulary_member() {
    let mut emitted: Vec<&str> = UiTextComponent::emitted().map(|c| c.name()).collect();
    let mut compared: Vec<&str> = COMPARED_NAMES.to_vec();
    emitted.sort_unstable();
    compared.sort_unstable();
    assert_eq!(
        compared, emitted,
        "the world comparison must read exactly the components the writer emits — a member the \
         writer emits but the comparison never reads is a loss this gate would not see"
    );
}
