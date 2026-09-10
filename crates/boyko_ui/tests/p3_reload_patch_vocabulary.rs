//! GATE 5b — RELOAD PATCH COVERAGE: what the FILE says about a SURVIVING node is
//! what the WORLD holds, for every member of the `.ui` vocabulary.
//!
//! # Why this exists beside `p3_hot_reload.rs`
//!
//! `p3_hot_reload.rs` proves the reconcile's TOPOLOGY (a survivor keeps its
//! entity, transient state rides an archetype migration, a move does not
//! cascade-despawn). It changes `UiLayout` and nothing else, so it is
//! structurally unable to fail on a component the patcher never patches.
//!
//! Measured on this tree: `patch_node` patched 10 of the 21 vocabulary members
//! (eight by value, plus the two data-bind columns by unconditional re-insert),
//! and of the eleven it left out only `ComputedRect` was left out on purpose. The
//! other ten — `UiText`, `UiImage`, `UiGrid`, `UiAnchor`, `Button`, `Bar`,
//! `BarFill`, `OnClick`, `OnHover`, `OnSubmit` — were silently ignored on a
//! surviving node, so editing a button's action or a label's colour in a live
//! `.ui` file did nothing, with no error anywhere. That is the same defect class
//! as the writer's dropped twelve, in a third direction: one vocabulary, spelled
//! out by hand in four places.
//!
//! # Three hostile arrangements this gate makes on purpose
//!
//! 1. **Every authored value differs from its `Default`, and the two documents
//!    differ from EACH OTHER field by field.** A `.ui` per-field parse failure
//!    falls back to `T::default()`, so a corpus authoring the default cannot tell
//!    a successful parse from a failed one — `p6a_equivalence`'s image case
//!    authors exactly `UiImage::default()` and was green over the UV defect for
//!    that reason. Because both documents are non-default AND mutually different,
//!    a patch that does nothing reds on the before/after comparison and a patch
//!    that writes garbage reds on the reference comparison. (The one field this
//!    cannot cover is `BindText.template`: the type has two variants, so one side
//!    must be the default. The report assertion below is what covers it.)
//! 2. **The patched survivor is compared against a FRESH SPAWN of the same text
//!    in a SECOND, id-OFFSET world.** "The value changed" is weaker than "the
//!    value is what the file says"; the reference spawn supplies the second half
//!    without hand-transcribing 60 expected field values into the test. The
//!    offset means a bind `source` — a per-world `Entity` — cannot match by
//!    coincidence, so the comparison-by-NAME is the one doing the work.
//! 3. **The LOWER-time report is asserted, not just the syntactic one.** Per-field
//!    errors are raised while lowering/patching, into a report that used to be
//!    dropped by the reload system's closure; a harness that checks only
//!    `parse_ui`'s report is green over every field-level failure in its corpus.
//!    This gate reads `UiHotReload::last_report`, which the watch system now
//!    records after phase 1, so a field that failed to re-parse during the patch
//!    is visible here rather than showing up as a value that "did not change".
//!
//! The corpus and the comparison are driven from `boyko_ui::text::vocab`, so this
//! gate answers to the same ONE list as the parser and the writer.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling spawned `Entity` handles and a `UiParseReport` out of the `Send + Sync`
// one-shot system closure. Not engine code — the whole file is compiled out of
// every shipping build.
#![allow(clippy::disallowed_types)]

mod p3_common;

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_ui::binding::{BindText, BindValue};
use boyko_ui::components::{
    Bar, BarFill, Button, ComputedClip, ComputedRect, ContentSize, StackIndex, UiAbsolute, UiAlign,
    UiAnchor, UiGrid, UiImage, UiLayout, UiName, UiRoot, UiSpacing,
};
use boyko_ui::interaction::{OnClick, OnHover, OnSubmit};
use boyko_ui::reload::UiHotReload;
use boyko_ui::text::vocab::{ReloadPolicy, UiTextComponent};
use boyko_ui::text::{parse_ui, spawn_ui_tree, UiParseReport, UiText};

use p3_common::ReloadWorld;

// ─────────────────────────── the two documents ────────────────────────────────

/// The initial document: every vocabulary member on ONE named node, every field
/// non-default (arrangement 1).
///
/// `#src` exists so the two bind components have a `#name` source to resolve
/// against; it sits at `rel == STEP`, where a `#` lead is a child head and a bare
/// identifier is an attached component.
const DOC_V1: &str = "\
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

/// The reloaded document: the SAME node keys, EVERY authored value different from
/// [`DOC_V1`] and still non-default (arrangement 1). `use_safe_area` stays `true`
/// in both because `false` is its default; the fields that move are `edge` and the
/// two offsets.
const DOC_V2: &str = "\
version=1
#root  UiLayout { layout_type: Overlay, position_type: Absolute, width: Px(654), height: Pct(77.25), min_width: Px(6), min_height: Px(8), max_width: Px(888), max_height: Stretch(4.5) }
    UiSpacing { padding_left: Px(21), padding_right: Px(22), padding_top: Px(23), padding_bottom: Px(24), border_left: Px(25), border_right: Px(26), border_top: Px(27), border_bottom: Px(28), row_gap: Px(29), column_gap: Px(30) }
    UiAlign { main: SpaceAround, cross: Center }
    UiAbsolute { left: Px(31), right: Px(32), top: Px(33), bottom: Px(34) }
    ContentSize { width: 51.5, height: 52.25 }
    StackIndex(71)
    ComputedRect { x: 141, y: 142, w: 143, h: 144 }
    ComputedClip { x: 81, y: 82, w: 83, h: 84.5 }
    UiRoot
    UiText { color: 4278190335, size_px: 33.25, font: 5, align: Center }
    UiImage { texture: 11, uv_min: [0.375, 0.5], uv_max: [0.625, 0.9375], tint: 65535 }
    UiGrid { columns: 6, rows: 7 }
    UiAnchor { edge: TopCenter, offset_x: 43.5, offset_y: 44.5, use_safe_area: true }
    Button
    Bar
    BarFill
    OnClick(61)
    OnHover(62)
    OnSubmit(63)
    BindText { source: #src, comp: 9, field: 6, field2: 7, template: Value }
    BindValue { source: #src, comp: 10, num_field: 8, den_field: 9 }
    #src  UiLayout { layout_type: Column, width: Px(34), height: Px(35) }
";

/// The same document with every attached component DELETED — the author's "I
/// removed that line" edit. The head `UiLayout` and the `#src` child stay because
/// a node without a `UiLayout` is not a node.
const DOC_BARE: &str = "\
version=1
#root  UiLayout { layout_type: Overlay, position_type: Absolute, width: Px(654), height: Pct(77.25), min_width: Px(6), min_height: Px(8), max_width: Px(888), max_height: Stretch(4.5) }
    #src  UiLayout { layout_type: Column, width: Px(34), height: Px(35) }
";

/// The members a deletion from the file does NOT remove, in `PROBED_NAMES` order.
///
/// `UiLayout` because a node requires one (the head line); the two binds because
/// the reconcile's bind columns are preserve-by-omission — a divergence from the
/// uniform "the file is the source of truth" policy, deliberate and documented at
/// `crates/boyko_ui/src/reload/reconcile.rs`'s bind patchers, and NOT changed in
/// the pass that closed the patch vocabulary (changing live bind semantics is its
/// own decision, with its own gate).
const KEEP_ON_OMIT: &[&str] = &["UiLayout", "BindText", "BindValue"];

// ─────────────────────────── world / probe helpers ────────────────────────────

/// A component's `Debug` projection, or `<absent>`.
///
/// `Debug` is this campaign's comparison method for the POD UI components: most
/// do not derive `PartialEq`, and a derived `Debug` is a TOTAL projection of every
/// field, so the comparison cannot forget a field the way a hand-written one can.
fn fmt_opt<T: core::fmt::Debug>(v: Option<&T>) -> String {
    match v {
        Some(v) => format!("{v:?}"),
        None => "<absent>".to_string(),
    }
}

/// A node's `UiName` as an owned string, or `None`.
fn name_of(world: &EcsMaster, e: Entity) -> Option<String> {
    world.get_component::<UiName>(e).map(|n| n.as_str().to_string())
}

/// Spawns `doc` into a FRESH, id-OFFSET world and returns it with its single root
/// (arrangement 2).
///
/// The lowering report is kept and asserted: the type-directed per-field parse
/// runs at lowering time, so a bad field value is recorded THERE and a harness
/// that checks only `parse_ui`'s syntactic report is green over all of them.
fn spawn_reference(doc: &str) -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    // Filler entities so world ids are offset from the reload world's: a bind
    // `source` that matched by raw id would then be a coincidence, and the
    // comparison-by-name is what proves the binding.
    world.run_system(|mut cmds: Commands| {
        for _ in 0..3 {
            cmds.spawn(ComputedClip::default());
        }
    });

    let tree = parse_ui(doc);
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

    let rep = report.lock().expect("probe").clone();
    assert!(
        rep.is_clean(),
        "the reference document must parse AND lower clean; errors: {:?}",
        rep.errors
    );
    let out = roots.lock().expect("probe").clone();
    assert_eq!(out.len(), 1, "the corpus declares exactly one root");
    (world, out[0])
}

/// The reload watch's most recent LOWER-time report (arrangement 3).
fn reload_report_errors(rw: &ReloadWorld) -> Vec<(usize, u16, String)> {
    rw.world().resource::<UiHotReload>().last_report.errors.clone()
}

/// Asserts the last reload (or the startup load) recorded no per-field error.
#[track_caller]
fn assert_reload_clean(rw: &ReloadWorld, what: &str) {
    let errors = reload_report_errors(rw);
    assert!(
        errors.is_empty(),
        "{what}: the reconcile's LOWER-time report must be clean — a per-field re-parse failure \
         silently keeps `T::default()` and would read as 'the value did not change'; errors: \
         {errors:?}"
    );
}

// ───────────────────────────── the probe table ────────────────────────────────

/// Declares, in ONE place, which vocabulary members this gate reads back and how.
///
/// `data` are compared by their `Debug` projection; `zst` are markers, compared by
/// presence; `bind` carry a live `Entity`, which is per-world — their other fields
/// are compared directly and the source by the NAME of the node it points at,
/// which is what "the same binding" means across two worlds.
///
/// The invocation emits `PROBED_NAMES`, which the census at the bottom pins
/// against the vocabulary roster, so this list cannot quietly cover less than the
/// reconcile patches.
macro_rules! declare_patch_probe {
    (
        data: [ $($d:ident),+ $(,)? ],
        zst: [ $($z:ident),+ $(,)? ],
        bind: [ $( $b:ident { src: $src:ident, plain: [ $($f:ident),+ $(,)? ] } ),+ $(,)? ] $(,)?
    ) => {
        /// Every vocabulary member this gate reads back out of the world, in the
        /// canonical order.
        const PROBED_NAMES: &[&str] = &[
            $( stringify!($d), )+
            $( stringify!($z), )+
            $( stringify!($b), )+
        ];

        /// The probed members PRESENT on `e`, in `PROBED_NAMES` order.
        fn present_names(world: &EcsMaster, e: Entity) -> Vec<&'static str> {
            let mut out: Vec<&'static str> = Vec::new();
            $( if world.has_component(e, <$d>::component_id()) { out.push(stringify!($d)); } )+
            $( if world.has_component(e, <$z>::component_id()) { out.push(stringify!($z)); } )+
            $( if world.has_component(e, <$b>::component_id()) { out.push(stringify!($b)); } )+
            out
        }

        /// `(what, Debug)` for every probed VALUE the two documents author
        /// differently — the entries the before/after comparison walks. The ZST
        /// markers carry no value; their gate is presence (the removal test).
        fn changing_values(world: &EcsMaster, e: Entity) -> Vec<(String, String)> {
            // The `data` half is one array literal (its length is fixed by the
            // list above); the `bind` half appends its per-field entries.
            let mut out: Vec<(String, String)> = Vec::from([
                $( (stringify!($d).to_string(), fmt_opt(world.get_component::<$d>(e))), )+
            ]);
            $(
                {
                    let v = world.get_component::<$b>(e).copied();
                    $(
                        out.push((
                            format!("{}.{}", stringify!($b), stringify!($f)),
                            match v {
                                Some(x) => format!("{:?}", x.$f),
                                None => "<absent>".to_string(),
                            },
                        ));
                    )+
                }
            )+
            out
        }

        /// Compares the patched survivor against a fresh spawn of the same text,
        /// pushing one line per mismatch.
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
                    let va = fmt_opt(a.get_component::<$d>(ea));
                    let vb = fmt_opt(b.get_component::<$d>(eb));
                    if va != vb {
                        out.push(format!(
                            "{path}: {} differs\n        patched survivor: {va}\n        fresh spawn:      {vb}",
                            stringify!($d),
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
                            "{path}: {} presence differs (patched survivor: {pa}, fresh spawn: {pb})",
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
                                        "{path}: {}.{} differs (patched survivor: {:?}, fresh spawn: {:?})",
                                        stringify!($b), stringify!($f), x.$f, y.$f,
                                    ));
                                }
                            )+
                            let na = name_of(a, x.$src);
                            let nb = name_of(b, y.$src);
                            if na != nb {
                                out.push(format!(
                                    "{path}: {}.{} points at a different node (patched survivor: {}, fresh spawn: {})",
                                    stringify!($b),
                                    stringify!($src),
                                    na.as_deref().unwrap_or("<unnamed or dangling>"),
                                    nb.as_deref().unwrap_or("<unnamed or dangling>"),
                                ));
                            }
                        }
                        (x, y) => {
                            out.push(format!(
                                "{path}: {} presence differs (patched survivor: {}, fresh spawn: {})",
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

declare_patch_probe! {
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

// ──────────────────────────────── the gates ───────────────────────────────────

#[test]
fn reload_patches_every_authored_component_on_a_surviving_node() {
    let mut rw = ReloadWorld::new("patch_vocab", DOC_V1);
    assert_reload_clean(&rw, "startup load");

    let root = rw.find_named("root").expect("the corpus root spawned");
    let present = present_names(rw.world(), root);
    assert_eq!(
        present, PROBED_NAMES,
        "the corpus must author every probed member onto the survivor — a gate over a corpus that \
         omits a component is a gate that cannot fail on it"
    );
    let before = changing_values(rw.world(), root);
    let rect_before = fmt_opt(rw.world().get_component::<ComputedRect>(root));

    rw.reload(DOC_V2);

    let after_root = rw.find_named("root").expect("the named root survives the reload");
    assert_eq!(
        root, after_root,
        "the named root must be PATCHED IN PLACE (same entity) — a respawn would make every \
         assertion below pass while proving nothing about the patcher"
    );
    assert_reload_clean(&rw, "reload to DOC_V2");

    // (a) every authored value MOVED. A patch arm that does nothing reds here.
    let after = changing_values(rw.world(), after_root);
    assert_eq!(before.len(), after.len(), "the probe reads the same entries before and after");
    let mut ignored: Vec<String> = Vec::new();
    for ((what, b), (what2, a)) in before.iter().zip(after.iter()) {
        assert_eq!(what, what2, "the probe walks the same entries in the same order");
        if b == a {
            ignored.push(format!("{what}: still {b} (the file says otherwise)"));
        }
    }
    assert!(
        ignored.is_empty(),
        "{} of {} authored values were NOT patched on the surviving node:\n    {}",
        ignored.len(),
        after.len(),
        ignored.join("\n    ")
    );

    // (b) and each landed value is what the FILE says, not merely something new.
    let (reference, ref_root) = spawn_reference(DOC_V2);
    let mut mismatches = Vec::new();
    compare_node(rw.world(), after_root, &reference, ref_root, "#root", &mut mismatches);
    assert!(
        mismatches.is_empty(),
        "{} component(s) on the patched survivor disagree with a fresh spawn of the same text:\n    {}",
        mismatches.len(),
        mismatches.join("\n    ")
    );

    // (c) `ComputedRect` is the declared exemption: layout OUTPUT, seeded at spawn
    //     and overwritten by the next layout pass, so the reconcile must NOT write
    //     the document's stale rect back onto a survivor.
    let rect_after = fmt_opt(rw.world().get_component::<ComputedRect>(after_root));
    assert_eq!(
        rect_before, rect_after,
        "ComputedRect must NOT be patched (layout output, not authored state)"
    );
    assert_ne!(
        rect_after,
        fmt_opt(reference.get_component::<ComputedRect>(ref_root)),
        "the exemption is only meaningful if the two documents author DIFFERENT rects"
    );
}

#[test]
fn reload_removes_what_the_file_drops_and_restores_it_when_it_returns() {
    let mut rw = ReloadWorld::new("patch_vocab_drop", DOC_V2);
    assert_reload_clean(&rw, "startup load");
    let root = rw.find_named("root").expect("the corpus root spawned");
    assert_eq!(present_names(rw.world(), root), PROBED_NAMES, "every member starts present");

    // ── the file drops every attached component ──
    rw.reload(DOC_BARE);
    let bare_root = rw.find_named("root").expect("the named root survives the deletion");
    assert_eq!(root, bare_root, "a deletion patches the survivor, it does not respawn it");
    assert_reload_clean(&rw, "reload to DOC_BARE");
    assert_eq!(
        present_names(rw.world(), bare_root),
        KEEP_ON_OMIT,
        "a component DELETED from the file is removed from the node (the document is the source \
         of truth for what it can author); the only survivors are the declared keep-on-omit \
         members"
    );
    // `ComputedRect` is exempt from the patch set entirely, so a deletion cannot
    // remove it either — it is the spawn-time seed layout owns.
    assert!(
        rw.world().has_component(bare_root, ComputedRect::component_id()),
        "ComputedRect is not in the patch set, so a file deletion does not remove it"
    );
    // The binds kept their VALUES, not merely their presence.
    let bind_text = rw.world().get_component::<BindText>(bare_root).copied();
    assert_eq!(
        bind_text.map(|b| (b.comp.0, b.field, b.field2)),
        Some((9, 6, 7)),
        "the preserved BindText keeps the values it was last patched with"
    );

    // ── and the file brings them back ──
    rw.reload(DOC_V2);
    let back_root = rw.find_named("root").expect("the named root survives the restore");
    assert_eq!(root, back_root, "the restore patches the same survivor");
    assert_reload_clean(&rw, "reload back to DOC_V2");
    assert_eq!(
        present_names(rw.world(), back_root),
        PROBED_NAMES,
        "a component RE-ADDED to the file is inserted back onto the surviving node"
    );

    let (reference, ref_root) = spawn_reference(DOC_V2);
    let mut mismatches = Vec::new();
    compare_node(rw.world(), back_root, &reference, ref_root, "#root", &mut mismatches);
    assert!(
        mismatches.is_empty(),
        "{} restored component(s) disagree with a fresh spawn of the same text:\n    {}",
        mismatches.len(),
        mismatches.join("\n    ")
    );
}

// ─────────────── document text → declared component names ─────────────────────

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

/// The roster names whose [`ReloadPolicy`] matches `pred`, in canonical order.
fn policy_names(pred: impl Fn(ReloadPolicy) -> bool) -> Vec<&'static str> {
    UiTextComponent::ALL
        .iter()
        .copied()
        .filter(|c| pred(c.reload_policy()))
        .map(|c| c.name())
        .collect()
}

// ─────────────────────────────── the censuses ─────────────────────────────────

#[test]
fn probe_covers_every_patched_vocabulary_member() {
    let mut patched: Vec<&str> = UiTextComponent::patched().map(|c| c.name()).collect();
    let mut probed: Vec<&str> = PROBED_NAMES.to_vec();
    patched.sort_unstable();
    probed.sort_unstable();
    assert_eq!(
        probed, patched,
        "this gate must read back exactly the members the reconcile declares it patches — a \
         member the reconcile patches but the gate never reads is a defect this gate would not \
         see, which is how the previous patch list stayed nine members wide"
    );
}

#[test]
fn reload_policy_departures_are_declared_and_argued() {
    assert_eq!(
        policy_names(|p| matches!(p, ReloadPolicy::PatchKeepOnOmit(_))),
        KEEP_ON_OMIT,
        "the members a file deletion does NOT remove are a DECISION, not an accident: each must \
         carry its reason in `ReloadPolicy::PatchKeepOnOmit` and be argued in this test.\n    \
         `UiLayout` — a node requires one (the head line).\n    `BindText` / `BindValue` — \
         preserve-by-omission inherited from GUI #27; removing a bind on reload changes LIVE \
         binding semantics and needs its own decision, so the pass that closed the patch \
         vocabulary pinned the behaviour instead of changing it."
    );
    assert_eq!(
        policy_names(|p| matches!(p, ReloadPolicy::Skip(_))),
        vec!["ComputedRect"],
        "the only member the reconcile never touches is layout OUTPUT: a spawn-time seed the \
         next `ui_layout_apply` overwrites, so patching a survivor from the document would pin a \
         stale rect (Decision 14 / §6)"
    );
    // Every departure carries a non-empty reason — the payload is the point.
    for c in UiTextComponent::ALL.iter().copied() {
        let why = match c.reload_policy() {
            ReloadPolicy::PatchAndRemove => continue,
            ReloadPolicy::PatchKeepOnOmit(why) | ReloadPolicy::Skip(why) => why,
        };
        assert!(!why.trim().is_empty(), "{}'s departure must state why", c.name());
    }
}

#[test]
fn the_corpus_authors_every_vocabulary_member() {
    for (label, doc) in [("DOC_V1", DOC_V1), ("DOC_V2", DOC_V2)] {
        let declared = declared_components(doc);
        let missing: Vec<&str> = UiTextComponent::ALL
            .iter()
            .map(|c| c.name())
            .filter(|n| !declared.iter().any(|d| d == n))
            .collect();
        assert!(
            missing.is_empty(),
            "{label} must author every vocabulary member (a gate over a corpus that omits a \
             component is a gate that cannot fail on it); missing: {missing:?}"
        );
    }
}

#[test]
fn the_bare_document_drops_every_removable_member() {
    let declared = declared_components(DOC_BARE);
    let still_authored: Vec<&str> = policy_names(ReloadPolicy::removes_on_omit)
        .into_iter()
        .filter(|n| declared.iter().any(|d| d == n))
        .collect();
    assert!(
        still_authored.is_empty(),
        "the deletion corpus must drop every member the policy says a deletion removes, or the \
         removal gate is vacuous for the ones it kept: {still_authored:?}"
    );
}
