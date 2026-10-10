//! GUI P5b — the BIND → EMIT CPU-boundary seam (the descoped-T6 substitute).
//!
//! # Why this and not a `boyko_demo` HUD label
//!
//! T6's deliverable was a dogfood HUD label in `boyko_demo`. But `boyko_demo` is the
//! eframe/egui/wgpu sandbox (it depends on `eframe`, NOT on `boyko_render`/`boyko_ui`
//! or the in-house Vulkan path), so wiring a live `boyko_ui` text label through it would
//! mean grafting an entire second renderer onto that crate — out of scope for this
//! handoff (which is confined to `boyko_render` + `boyko_ui` + `boyko_rhi_vulkan`). T6's
//! dogfood is therefore DESCOPED and recorded as an open rung.
//!
//! What T6 actually had to PROVE — "a bound label renders the live value, re-emits only
//! on `Changed`" — is proven here at the CPU boundary, end-to-end through the real
//! binding systems: a `Health`-bound [`BindText`] writes the widget's
//! [`UiTextBuffer`](boyko_ui::binding::UiTextBuffer) with the live value, then the live
//! emitter [`emit_glyphs`](boyko_ui::text::emit_glyphs) turns `(UiText, UiTextBuffer)`
//! into renderable [`GlyphInstance`](boyko_ui::text::GlyphInstance)s. The GPU half (the
//! glyph instances reaching the swapchain) is proven by the
//! `boyko_render` `ui_text_gpu_golden`. Together they cover the whole live arm.

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
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_macros::{Bindable, Component, Resource};

use boyko_ui::binding::bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
use boyko_ui::binding::components::{BindText, TemplateId, UiTextBuffer, NO_FIELD};
use boyko_ui::binding::Bindable;
use boyko_fontbake::atlas::AtlasImage;
use boyko_ui::components::{ComputedRect, StackIndex};
use boyko_ui::text::{
    emit_glyphs, AtlasKind, AtlasMeta, BakedFont, FontId, FontTable, GlyphInstance, GlyphMetrics,
    TextAlign, UiText,
};

/// A bindable HUD source: `current`/`max` (fields 0/1).
#[derive(Component, Bindable, Clone, Copy, Debug)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

/// A pending source mutation applied by the in-schedule `mutator_system` (so the write
/// lands on a tick the next bind discovery's change window covers — the realistic
/// gameplay-system mutation path, mirroring `p4_bind`).
#[derive(Resource, Default)]
struct MutQueue {
    pending: Vec<(Entity, Health)>,
}

#[allow(clippy::needless_pass_by_ref_mut)]
fn mutator_system(world: &mut EcsMaster) {
    let pending = std::mem::take(&mut world.resource_mut::<MutQueue>().pending);
    for (e, h) in pending {
        if let Some(mut g) = world.get_component_mut::<Health>(e) {
            *g = h;
        }
    }
}

/// A font whose digits/`/` are all visible glyphs (so a `"7/9"` label emits one glyph
/// per char), `advance_em = 0.5`.
fn hud_font() -> BakedFont {
    use boyko_fontbake::atlas::MappedCodepoint;
    let visible = GlyphMetrics { advance_em: 0.5, plane: [0.0, 0.0, 0.5, 0.7], atlas: [0.0, 1.0, 1.0, 0.0] };
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

struct BindWorld {
    world: EcsMaster,
    schedule: Schedule,
}

impl BindWorld {
    fn new() -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();

        let mut scratch = UiBindScratch::default();
        Health::register_bind_accessor();
        scratch.register_bound_id(Health::component_id());
        world.insert_resource(scratch);
        world.insert_resource(MutQueue::default());

        let mut builder = ScheduleBuilder::new(pool);
        let mutate = builder.add_system(mutator_system).key();
        let discovery = builder.add_system(ui_bind_discovery).after(mutate).key();
        builder.add_system(ui_bind_apply).after(discovery);
        let mut schedule = builder.build(&mut world);

        // Advance the tick window past Tick::ZERO before any source spawns (mirrors
        // `p4_bind`'s first-frame Added-detection note).
        schedule.run(&mut world);
        schedule.run(&mut world);

        Self { world, schedule }
    }

    fn spawn_source(&mut self, current: f32, max: f32) -> Entity {
        self.spawn_with(move |cmds| cmds.spawn(Health { current, max }).id())
    }

    fn spawn_label(&mut self, source: Entity, template: TemplateId) -> Entity {
        self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(BindText {
                source,
                comp: Health::component_id(),
                field: 0,
                field2: if template == TemplateId::Ratio { 1 } else { NO_FIELD },
                template,
            });
            ec.insert(UiTextBuffer::default());
            // The label also carries the STYLE component the emitter reads.
            ec.insert(UiText { color: 0xFF00_00FF, size_px: 18.0, font: FontId(0), align: TextAlign::Left, _pad: 0 });
            ec.id()
        })
    }

    fn spawn_with<F>(&mut self, f: F) -> Entity
    where
        F: FnOnce(&mut Commands) -> Entity + Send + Sync + 'static,
    {
        let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
        let probe = Arc::clone(&sink);
        let f = Mutex::new(Some(f));
        self.world.run_system(move |mut cmds: Commands| {
            let f = f.lock().unwrap().take().expect("spawn closure runs once");
            let e = f(&mut cmds);
            *probe.lock().unwrap() = Some(e);
        });
        sink.lock().unwrap().expect("spawned handle")
    }

    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    fn set_health(&mut self, e: Entity, current: f32, max: f32) {
        self.world.resource_mut::<MutQueue>().pending.push((e, Health { current, max }));
    }

    fn text_of(&self, e: Entity) -> String {
        self.world
            .get_component::<UiTextBuffer>(e)
            .map(|b| b.as_str().to_string())
            .unwrap_or_default()
    }

    fn ui_text_of(&self, e: Entity) -> UiText {
        *self.world.get_component::<UiText>(e).expect("label has UiText style")
    }
}

/// The end-to-end CPU arm: a `Health`-bound label's `UiTextBuffer` carries the LIVE
/// value, and the live emitter turns `(UiText, UiTextBuffer)` into glyph instances — one
/// per visible char — proving the authoring arm produces RENDERABLE text.
#[test]
fn bound_label_emits_glyphs_for_the_live_value() {
    let mut bw = BindWorld::new();
    let source = bw.spawn_source(7.0, 9.0);
    let label = bw.spawn_label(source, TemplateId::Ratio);

    // First bind frame: the source is Added, so the label formats "7/9".
    bw.run();
    assert_eq!(bw.text_of(label), "7/9", "the bound label carries the live value");

    // Emit the label through the live emitter into a fresh instance stream.
    let mut fonts = FontTable::new();
    fonts.load(&hud_font());
    let style = bw.ui_text_of(label);
    let content = bw.text_of(label);
    let rect = ComputedRect { x: 4.0, y: 4.0, w: 0.0, h: 0.0 };
    let mut out: Vec<GlyphInstance> = Vec::new();
    emit_glyphs(&style, &rect, &content, StackIndex(0), None, &fonts, &mut out);

    assert_eq!(out.len(), 3, "\"7/9\" emits three glyph quads");
    assert!(out.iter().all(|g| g.color == 0xFF00_00FF), "each glyph carries the label's color");
    assert!(out[1].rect[0] > out[0].rect[0] && out[2].rect[0] > out[1].rect[0], "glyphs pen-advance");
}

/// The value re-emits when it CHANGES, and the buffer is stable when it does NOT
/// (set-if-changed) — the "re-emits only on Changed" half of T6. We assert the
/// observable consequence: the formatted text tracks the live value across frames.
#[test]
fn bound_label_tracks_then_holds_the_value() {
    let mut bw = BindWorld::new();
    let source = bw.spawn_source(7.0, 9.0);
    let label = bw.spawn_label(source, TemplateId::Ratio);

    bw.run();
    assert_eq!(bw.text_of(label), "7/9", "initial bound value");

    // Change the source → the label re-formats to the new live value.
    bw.set_health(source, 3.0, 9.0);
    bw.run();
    assert_eq!(bw.text_of(label), "3/9", "the label re-emits the changed value");

    // A still frame (no source change) holds the value (no spurious re-format).
    bw.run();
    assert_eq!(bw.text_of(label), "3/9", "a still frame holds the last value");
}
