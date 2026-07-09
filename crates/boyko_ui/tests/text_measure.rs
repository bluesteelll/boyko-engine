//! GUI P5b — the measure→layout CPU-boundary seam (Decision T5-B, host-driven).
//!
//! P5b's text measure is HOST-DRIVEN: `boyko_ui` ships `ui_text_measure_system` and the
//! host registers it `.before(ui_layout_discovery)` (the same way it orders the layout
//! pair) — there is no in-crate App schedule binding the order (matching P5a Decision
//! T5-B). The reviewer asked that the seam be PROVEN at the CPU boundary regardless, so
//! these tests assert the two halves a correct ordering relies on:
//!
//! 1. `measure_one`'s reported extent equals the SHAPED run (same `shape_into` core the
//!    emitter uses) — so the size fed into `ContentSize` is the size the emitter lays
//!    down (no measure/emit drift).
//! 2. An `Auto`-sized leaf node fed that `ContentSize` HUGS it through the layout's leaf
//!    intrinsic-size fallback — so once the host writes `ContentSize` before discovery,
//!    the relayout sizes the node to the text.
//!
//! The first two halves do not need a scheduler: the measure is a pure function and the
//! layout's `ContentSize` read is exercised directly via the shared `Ui` harness.
//!
//! [`text_change_relayouts_the_auto_node`] closes the seam END-TO-END on a real schedule
//! (`ui_text_measure_system -> ui_layout_discovery -> ui_layout_apply`): it proves that a
//! text-content change (`Changed<UiTextBuffer>`) actually flows through the measure into a
//! `Changed<ContentSize>` tick and re-sizes the Auto node the SAME frame. This is the
//! regression guard for the `&mut ContentSize` → `Mut<ContentSize>` fix — an `&mut`
//! query item never bumps the changed tick, so the relayout gate (`Changed<ContentSize>`)
//! would never fire and the node would re-measure invisibly.

mod common;

use std::sync::{Arc, Mutex};

use common::{approx, NodeSpec, Ui};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_fontbake::atlas::AtlasImage;
use boyko_ui::binding::UiTextBuffer;
use boyko_ui::components::{ComputedRect, ContentSize, UiLayout, UiRoot};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiViewport};
use boyko_ui::text::{
    measure_one, shape_into, ui_text_measure_system, AtlasKind, AtlasMeta, BakedFont, FontId,
    FontTable, GlyphMetrics, TextAlign, UiText,
};
use boyko_ui::units::{LayoutType, Unit};

/// A font with a known, simple metric so the shaped extent is hand-computable. Every
/// printable ASCII codepoint maps to one visible glyph of `advance_em = 0.5` and a
/// `plane` of `[0, 0, 0.5, 0.7]` (non-degenerate ⇒ emitted), line metrics
/// `ascender 0.8 / descender -0.2 / gap 0.0` ⇒ `line_height = 1.0 * size_px`.
fn known_font() -> BakedFont {
    use boyko_fontbake::atlas::MappedCodepoint;
    let visible = GlyphMetrics {
        advance_em: 0.5,
        plane: [0.0, 0.0, 0.5, 0.7],
        atlas: [0.0, 1.0, 1.0, 0.0],
    };
    let mut glyphs = vec![GlyphMetrics { advance_em: 0.0, plane: [0.0; 4], atlas: [0.0; 4] }];
    let mut cmap = Vec::new();
    for (slot, cp) in (1u16..).zip(0x20u32..0x7F) {
        glyphs.push(visible);
        cmap.push(MappedCodepoint { codepoint: cp, slot });
    }
    BakedFont {
        meta: AtlasMeta {
            distance_range_texels: 6.0,
            pixels_per_em: 48.0,
            atlas_w: 1,
            atlas_h: 1,
            ascender_em: 0.8,
            descender_em: -0.2,
            line_gap_em: 0.0,
            kind: AtlasKind::Mtsdf,
        },
        glyphs,
        cmap,
        kern: Vec::new(),
        atlas: AtlasImage { width: 1, height: 1, pixels: vec![0u8; 4] },
    }
}

fn font_table() -> (FontTable, FontId) {
    let mut t = FontTable::new();
    let id = t.load(&known_font());
    (t, id)
}

/// `measure_one`'s extent equals the shaped run extent (no measure/emit drift): the
/// measure shapes through the SAME `shape_into` and reports its returned extent, so a
/// re-shape that counts glyphs and tracks the run width must agree.
#[test]
fn measure_one_matches_the_shaped_run() {
    let (fonts, id) = font_table();
    let text = UiText { color: 0xFFFF_FFFF, size_px: 20.0, font: id, align: TextAlign::Left, _pad: 0 };
    let body = "Hello";
    let rect = ComputedRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; // unwrapped (single line)

    // Re-shape the run independently, summing what the emitter would lay down.
    let entry = fonts.entry(id).expect("loaded font");
    let mut emitted = 0usize;
    let extent = shape_into(body, entry, text.size_px, 0.0, TextAlign::Left, |_g| {
        emitted += 1;
    });

    let measured = measure_one(&text, body, &rect, &fonts);
    approx(measured.width, extent.width, "measure width == shaped run width");
    approx(measured.height, extent.height, "measure height == shaped run height");
    assert_eq!(emitted, 5, "all five glyphs shaped (no whitespace ⇒ single line)");

    // Hand-check: 5 glyphs × advance 0.5 em × 20 px = 50 px wide; one line × 1.0 em ×
    // 20 px = 20 px tall (no kerning in this font).
    approx(measured.width, 50.0, "5 * 0.5em * 20px");
    approx(measured.height, 20.0, "one line, line_height 1.0em * 20px");
}

/// An empty / whitespace-only content measures to nothing (an empty label hugs to zero),
/// so the relayout does not reserve space for an absent value.
#[test]
fn measure_one_empty_content_is_zero() {
    let (fonts, id) = font_table();
    let text = UiText { color: 0xFFFF_FFFF, size_px: 16.0, font: id, align: TextAlign::Left, _pad: 0 };
    let rect = ComputedRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    let measured = measure_one(&text, "", &rect, &fonts);
    approx(measured.width, 0.0, "empty content width");
    approx(measured.height, 0.0, "empty content height");
}

/// An `Auto`×`Auto` leaf node fed the measured `ContentSize` HUGS it through the
/// layout's leaf intrinsic-size fallback (`relative_count == 0` ⇒ use `ContentSize`).
/// This is the downstream half of the measure→layout seam: once the host writes
/// `ContentSize` before discovery, the relayout sizes the node to the text. A Column
/// leaf takes `(width=cross=cw, height=main=ch)`.
#[test]
fn auto_leaf_hugs_the_measured_content_size() {
    let (fonts, id) = font_table();
    let text = UiText { color: 0xFFFF_FFFF, size_px: 20.0, font: id, align: TextAlign::Left, _pad: 0 };
    let body = "Hello"; // 50 × 20 px measured above
    let content_rect = ComputedRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    let measured = measure_one(&text, body, &content_rect, &fonts);

    // A fixed root with one Auto×Auto Column leaf carrying the measured ContentSize.
    let mut ui = Ui::default_world();
    let root = ui.spawn_root(UiLayout {
        layout_type: LayoutType::Column,
        width: Unit::Px(400.0),
        height: Unit::Px(300.0),
        ..UiLayout::default()
    });
    let leaf = ui.spawn(
        NodeSpec::new(UiLayout {
            layout_type: LayoutType::Column,
            width: Unit::Auto,
            height: Unit::Auto,
            ..UiLayout::default()
        })
        .with_content(ContentSize { width: measured.width, height: measured.height }),
        Some(root),
    );
    ui.run();

    let r = ui.rect(leaf);
    approx(r.w, measured.width, "Auto leaf width hugs ContentSize.width");
    approx(r.h, measured.height, "Auto leaf height hugs ContentSize.height");
}

/// Builds a [`UiTextBuffer`] from `s` via its `core::fmt::Write` sink (the only writer).
fn text_buffer(s: &str) -> UiTextBuffer {
    use core::fmt::Write;
    let mut b = UiTextBuffer::default();
    write!(b, "{s}").expect("UiTextBuffer write");
    b
}

/// END-TO-END on a real schedule: a text-content change re-sizes the Auto node the same
/// frame. This is the regression guard for the `Mut<ContentSize>` fix — with an
/// `&mut ContentSize` query item the measure writes `ContentSize` WITHOUT bumping its
/// changed tick, so `ui_layout_discovery`'s `Changed<ContentSize>` relayout trigger never
/// fires and the Auto-sized node re-measures INVISIBLY (its rect stays stale).
///
/// The world runs `[editor -> ui_text_measure_system -> ui_layout_discovery ->
/// ui_layout_apply]` (measure→layout is the exact order the host is contracted to
/// register; the editor stands in for a bind/edit system). A fixed root holds one
/// fixed-width / Auto-HEIGHT text leaf (the height axis is wrap-independent). Frame 1
/// measures the 1-line content and hugs it; then the editor writes a 3-line string
/// (bumping `Changed<UiTextBuffer>`), and frame 2 must (a) re-measure to a taller
/// `ContentSize`, (b) bump `Changed<ContentSize>` via the `Mut` deref, and (c) relayout
/// the leaf TALLER — all in one schedule run.
#[test]
fn text_change_relayouts_the_auto_node() {
    let (fonts, id) = font_table();

    // A world with the three resources the measure + layout systems read.
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world.insert_resource(fonts);
    world.insert_resource(UiViewport { width: 1000.0, height: 800.0, scale_factor: 1.0, generation: 0 });
    world.insert_resource(LayoutScratch::with_seeds());

    // The Auto axis under test is HEIGHT (line count), which is wrap-INDEPENDENT: the
    // leaf has a FIXED, generous width, so the wrap width `measure_one` reads from the
    // rect never shrinks the run, and each hard-broken line is a single glyph that fits.
    // (Testing WIDTH would feed the just-shrunk Auto width back in as the wrap width and
    // chunk an overlong run — a re-measure artifact, not the seam under test.) A 1-line
    // string → 1 * line_height; a 3-line string → 3 * line_height (line_height = 20 px).
    let short = "A"; // 1 line → 20 px tall
    let long = "A\nB\nC"; // 3 hard-broken lines → 60 px tall
    let style = UiText { color: 0xFFFF_FFFF, size_px: 20.0, font: id, align: TextAlign::Left, _pad: 0 };
    // The wrap-width `measure_one` reads from a fixed-width leaf's rect.
    let leaf_rect = ComputedRect { x: 0.0, y: 0.0, w: 200.0, h: 0.0 };

    // A shared trigger drives the text edit from WITHIN the schedule (the realistic path:
    // a bind/edit system writes `UiTextBuffer` in-frame via a `Mut` guard, and the measure
    // — ordered after it — observes `Changed<UiTextBuffer>` the SAME frame). When the slot
    // is `Some`, the editor system writes it into the sole text node and clears the slot.
    let pending_text: Arc<Mutex<Option<UiTextBuffer>>> = Arc::new(Mutex::new(None));
    let editor_slot = Arc::clone(&pending_text);
    let editor = move |mut q: Query<Mut<UiTextBuffer>, With<UiText>>| {
        let Some(next) = editor_slot.lock().expect("pending").take() else {
            return;
        };
        for mut buf in q.iter_mut() {
            *buf = next; // `Mut` deref bumps Changed<UiTextBuffer> at this frame's this_run.
        }
    };

    // The host-contracted order: edit BEFORE measure BEFORE discovery BEFORE apply.
    let mut builder = ScheduleBuilder::new(pool);
    let edit = builder.add_system(editor).key();
    let measure = builder.add_system(ui_text_measure_system).after(edit).key();
    let discovery = builder.add_system(ui_layout_discovery).after(measure).key();
    builder.add_system(ui_layout_apply).after(discovery);
    let mut schedule: Schedule = builder.build(&mut world);

    // Spawn a fixed root with one Auto×Auto text leaf carrying UiText + the short buffer.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let root = cmds
            .spawn(UiLayout {
                layout_type: LayoutType::Column,
                width: Unit::Px(400.0),
                height: Unit::Px(300.0),
                ..UiLayout::default()
            })
            .insert(ComputedRect::default())
            .insert(UiRoot)
            .id();
        let mut leaf = cmds.spawn(UiLayout {
            layout_type: LayoutType::Column,
            width: Unit::Px(200.0),
            height: Unit::Auto,
            ..UiLayout::default()
        });
        leaf.insert(ComputedRect::default())
            .insert(ContentSize::default())
            .insert(style)
            .insert(text_buffer(short))
            .set_parent(root);
        *probe.lock().expect("probe") = Some(leaf.id());
    });
    let leaf = sink.lock().expect("probe").expect("leaf handle");
    assert!(world.has_entity(leaf), "leaf is live after apply");

    // Frame 1: measure the short content and hug it (on the HEIGHT / main axis).
    schedule.run(&mut world);
    let h_short = world.get_component::<ComputedRect>(leaf).expect("rect").h;
    let expect_short = measure_one(&style, short, &leaf_rect, world.resource::<FontTable>());
    approx(h_short, expect_short.height, "frame 1: Auto leaf hugs the short measured height");
    assert!(h_short > 0.0, "frame 1: leaf measured a non-zero height");

    // Arm the edit: frame 2's editor system will write the long buffer through a
    // `Mut<UiTextBuffer>` guard, bumping `Changed<UiTextBuffer>` at frame 2's this_run.
    *pending_text.lock().expect("pending") = Some(text_buffer(long));

    // Frame 2: the measure must re-run (Changed<UiTextBuffer>), write a TALLER ContentSize
    // AND bump Changed<ContentSize> (the Mut deref), so discovery relayouts the leaf.
    schedule.run(&mut world);
    let content = *world.get_component::<ContentSize>(leaf).expect("content");
    let expect_long = measure_one(&style, long, &leaf_rect, world.resource::<FontTable>());
    approx(content.height, expect_long.height, "frame 2: ContentSize re-measured to the NEW height");

    let h_long = world.get_component::<ComputedRect>(leaf).expect("rect").h;
    approx(h_long, expect_long.height, "frame 2: Auto leaf hugs the NEW measured height");
    assert!(
        h_long > h_short + 1.0,
        "frame 2: the text change relayouts the Auto node TALLER (short {h_short} -> long {h_long}); \
         a stale rect here means the measure wrote ContentSize without bumping Changed<ContentSize> \
         (the &mut ContentSize footgun the Mut<ContentSize> fix closes)"
    );
}
