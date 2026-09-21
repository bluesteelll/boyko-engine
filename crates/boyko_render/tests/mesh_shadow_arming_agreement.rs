//! **The header's shadow bits, the host's pass arming and the punctual light slots all derive from
//! one fact** — the device-free half of the mesh-shadow producer fix (shadow gates SG1/SG2).
//!
//! One headless world composes the four pieces the windowed host composes — `CsmPlugin`,
//! `ShadowAtlasPlugin`, `LightingPlugin`, and the app-wired `sync_csm_light_gate` /
//! `sync_punctual_light_gate` — with CSM and the shadow atlas ENABLED, a sun, a
//! `CastsPunctualShadow` spot and three `CastsPunctualShadow` points (19 layers against a 16-layer
//! budget, so one point loses its slot), one point and one spot WITHOUT the marker, and a
//! perspective view. It then sweeps the boot carrier
//! over every `{Deferred, Forward, ForwardPlus, VisibilityBuffer} × {Mesh, Sdf, Both}` request
//! (through `resolve_render_path`, the real boot entry) and the caster gather over `{0, 2}`
//! batches, running 3 frames per configuration — enough for the unordered sync gates to settle
//! (the resolve lands by frame 1, the gate by frame 2, the header fold by frame 3).
//!
//! Per configuration it asserts:
//!
//! * **SG2 agreement** — `LightingConfig::csm_shadows == ResolvedCsm::depth_pass_armed(casters)`
//!   and `punctual_shadows == ResolvedShadowAtlas::depth_pass_armed(casters)`: the header gates
//!   and the host's arming call are one predicate, and the staged header word 7 carries exactly
//!   those two bits.
//! * **SG1 under a mesh-less leg set** — both fits `DISABLED`, the slot handoff `EMPTY`, header
//!   bits 2/3 clear, and NO staged light row carrying a real atlas slot, whatever the config and
//!   the casters say.
//! * **Anti-vacuity** — every mesh-carrying `(path, legs)` pair arms BOTH bits at 2 casters, so the
//!   sweep cannot pass by never arming anything.
//! * **R1 (gate G2): every point/spot row is slotted iff its slot field is real** —
//!   `(kind & CASTS_SHADOW_BIT != 0) == (light_atlas_slot(kind) != SLOT_NONE)` on each staged row.
//!   The shader samples by the field alone (header bit 3, then `light_atlas_slot(kind) !=
//!   SLOT_NONE`), so an un-slotted row whose field decoded `0` would sample layer 0 — another
//!   light's map. The rows it covers come through the production resolve → assignment →
//!   `collect_lights` path: the un-flagged point and spot on every configuration, the budget loser
//!   whenever the atlas is live (it must then slot exactly 3 rows), and every flagged row under a
//!   mesh-less leg set (the EMPTY handoff). At least one row must carry `SLOT_NONE`, so the check
//!   cannot hold over an empty set.
//!
//! The SG1 "no slotted row" check counts `CASTS_SHADOW_BIT`, which is set if and only if a real
//! slot was packed; R1 is what ties that bit to the field the shaders actually read.
//!
//! # Why one world, and why its own binary
//!
//! `LightingPlugin::build` registers the light components' eviction hooks, which is legal at most
//! once per PROCESS and only before any light component was archetyped anywhere in it
//! (`tests/le_support/common.rs`). So the sweep rewrites the boot carrier inside one world rather
//! than booting 24 (a production host never changes the carrier after boot; this harness does it
//! only between settled configurations), and this file holds exactly ONE test.

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::system::Commands;
use boyko_math::{Affine3A, Vec3};
use boyko_rhi::enums::IndexType;
use boyko_scene::{GlobalTransform, Projection, Transform, ViewUniform};

use boyko_render::instance_model::InstanceModelCol;
use boyko_render::light::{LightingConfig, PointLight, SpotLight};
use boyko_render::light_system::LightTableStaging;
use boyko_render::mesh_draw::PerInstanceMaterial;
use boyko_render::shadow_atlas::PunctualSlotAssignment;
use boyko_render::{
    CASTS_SHADOW_BIT, CastsPunctualShadow, CsmConfig, CsmCasterScratch, CsmPlugin, DirectionalLight,
    DirectionalLightObject, GeometryLegs, LIGHT_KIND_POINT, LIGHT_KIND_SPOT, LightingPlugin,
    PointLightObject, RenderPath, RenderPathConfig, RenderPathConsumers, RenderPathDeviceCaps,
    ResolvedCsm, ResolvedShadowAtlas, SLOT_NONE, ShadowAtlasPlugin, ShadowConfig, SpotLightObject,
    light_atlas_slot, resolve_render_path, sync_csm_light_gate, sync_punctual_light_gate,
};

/// Header word 7's CSM sample bit (`CSM_MODE_BIT`).
const CSM_BIT: u32 = 1 << 2;
/// Header word 7's punctual sample bit (`PUNCTUAL_MODE_BIT`).
const PUNCTUAL_BIT: u32 = 1 << 3;
/// `LightHeaderGpu` bytes before the first light row.
const HEADER_BYTES: usize = 64;
/// One `GpuLight` row.
const ROW_BYTES: usize = 48;
/// `CastsPunctualShadow` points in the scene: three cubes (18 layers) plus the flagged spot's one
/// exceed the 16-layer budget, so the production resolve leaves exactly one point a slot LOSER.
const FLAGGED_POINTS: usize = 3;
/// Every point/spot row the table must carry: the flagged spot and points, plus one un-flagged
/// point and one un-flagged spot.
const PUNCTUAL_ROWS: usize = 1 + FLAGGED_POINTS + 2;
/// Rows holding a real slot whenever the atlas is live: the spot's layer and two cubes
/// (`1 + 2 * 6 = 13 <= 16`); the third cube does not fit.
const SLOTTED_WHEN_LIVE: usize = 1 + FLAGGED_POINTS - 1;

/// A caster scratch holding exactly `n <= 2` one-instance batches, built through the production
/// gather core rather than by hand.
fn casters(n: usize) -> CsmCasterScratch {
    let identity = InstanceModelCol {
        rows: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
    };
    let rows = [identity; 2];
    let mut scratch = CsmCasterScratch::default();
    scratch.0.gather_mixed_into(
        n,
        |_| Some((6, IndexType::Uint16)),
        || {
            rows[..n]
                .iter()
                .enumerate()
                .map(|(i, col)| (i as u32, col, None, PerInstanceMaterial::default(), false))
        },
    );
    assert_eq!(scratch.batch_count(), n, "the caster fixture must hold {n} batch(es)");
    scratch
}

/// What the staged light table says about its point/spot rows.
struct Staged {
    /// Header word 7.
    word7: u32,
    /// Rows carrying `CASTS_SHADOW_BIT` (a real atlas slot was packed).
    slotted: usize,
    /// Point/spot rows (kind tag `POINT` or `SPOT`; directional and sky rows are not counted).
    punctual: usize,
    /// Point/spot rows whose slot field decodes `SLOT_NONE`.
    slot_none: usize,
    /// `(row, kind word)` of every point/spot row whose casts bit disagrees with its slot field —
    /// the shader samples by the field alone, so a clear bit over a real-looking field (0) is a
    /// row that samples someone else's layer.
    disagreeing: Vec<(usize, u32)>,
}

/// Reads the staged light table.
fn staged(app: &App) -> Staged {
    let bytes = app.world().resource::<LightTableStaging>().bytes();
    let word = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().expect("a word"));
    let rows = bytes.len().saturating_sub(HEADER_BYTES) / ROW_BYTES;
    let mut s = Staged { word7: word(28), slotted: 0, punctual: 0, slot_none: 0, disagreeing: Vec::new() };
    for r in 0..rows {
        let k = word(HEADER_BYTES + r * ROW_BYTES + 12);
        let casts = k & CASTS_SHADOW_BIT != 0;
        s.slotted += usize::from(casts);
        let kind = k & 0xFFFF;
        if kind != LIGHT_KIND_POINT && kind != LIGHT_KIND_SPOT {
            continue;
        }
        s.punctual += 1;
        let field_none = light_atlas_slot(k) == SLOT_NONE;
        s.slot_none += usize::from(field_none);
        if casts == field_none {
            s.disagreeing.push((r, k));
        }
    }
    s
}

#[test]
fn header_bits_pass_arming_and_slots_agree_on_every_leg_set() {
    let mut app = App::new();
    // LightingPlugin FIRST: its hook registration must precede every light archetype.
    app.insert_resource(LightTableStaging::default());
    app.insert_resource(LightingConfig::default());
    app.add_plugins(LightingPlugin);
    app.add_plugin(CsmPlugin);
    app.add_plugin(ShadowAtlasPlugin);
    app.insert_resource(CsmCasterScratch::default());
    app.add_systems_cfg(|b| {
        b.add_system(sync_csm_light_gate);
        b.add_system(sync_punctual_light_gate);
    });
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    let eye = Vec3::new(0.0, 2.0, 6.0);
    let world_xf = Affine3A::look_at_rh(eye, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    let proj = Projection::Perspective {
        fov_y: core::f32::consts::FRAC_PI_3,
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: 100.0,
    };
    app.insert_resource(ViewUniform::from_camera(world_xf, proj));

    // No transform propagation runs here, so every light's world pose is the identity:
    // `light_reconcile` places each at the origin shining down world -Z. That is a valid pose for
    // every fit this test reads (a sun fit and a spot/point slot need only a non-degenerate
    // direction and a positive priority from the view above).
    app.world_mut().run_system(|mut cmds: Commands| {
        cmds.spawn(DirectionalLightObject {
            transform: Transform::IDENTITY,
            global: GlobalTransform::IDENTITY,
            light: DirectionalLight::new([-0.45, 0.82, 0.36], [1.0; 3], 3.0),
        });
        cmds.spawn(SpotLightObject {
            transform: Transform::IDENTITY,
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new([0.0; 3], [0.0, 0.0, -1.0], [1.0; 3], 200.0, 8.0, 15.0, 30.0),
        })
        .insert(CastsPunctualShadow);
        // Whatever order the (equal) priorities rank in, a spot and three cubes need 19 layers, so
        // the bump allocation refuses exactly one point and places the other two and the spot.
        for _ in 0..FLAGGED_POINTS {
            cmds.spawn(PointLightObject {
                transform: Transform::IDENTITY,
                global: GlobalTransform::IDENTITY,
                light: PointLight::new([0.0; 3], [1.0; 3], 200.0, 8.0),
            })
            .insert(CastsPunctualShadow);
        }
        // R1: one point and one spot WITHOUT `CastsPunctualShadow` — rows the resolve never
        // assigns, on every configuration, armed or not.
        cmds.spawn(PointLightObject {
            transform: Transform::IDENTITY,
            global: GlobalTransform::IDENTITY,
            light: PointLight::new([0.0; 3], [1.0; 3], 200.0, 8.0),
        });
        cmds.spawn(SpotLightObject {
            transform: Transform::IDENTITY,
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new([0.0; 3], [0.0, 0.0, -1.0], [1.0; 3], 200.0, 8.0, 15.0, 30.0),
        });
    });

    let consumers =
        RenderPathConsumers { csm_on: true, punctual_shadows_on: true, ..RenderPathConsumers::default() };
    let mut findings: Vec<String> = Vec::new();
    let mut configs = 0u32;
    for path in
        [RenderPath::Deferred, RenderPath::Forward, RenderPath::ForwardPlus, RenderPath::VisibilityBuffer]
    {
        let mut both_bits_armed = 0u32;
        for legs in [GeometryLegs::Mesh, GeometryLegs::Sdf, GeometryLegs::Both] {
            let (carrier, _) =
                resolve_render_path(&RenderPathConfig { path, legs }, consumers, RenderPathDeviceCaps::new(true));
            assert_eq!((carrier.path, carrier.legs), (path, legs), "the request must resolve as asked");
            for n in [0usize, 2] {
                configs += 1;
                app.insert_resource(carrier);
                app.insert_resource(casters(n));
                app.run_n(3);

                let world = app.world();
                let cfg = *world.resource::<LightingConfig>();
                let csm = *world.resource::<ResolvedCsm>();
                let atlas = *world.resource::<ResolvedShadowAtlas>();
                let slots = *world.resource::<PunctualSlotAssignment>();
                let scratch = world.resource::<CsmCasterScratch>();
                let csm_armed = csm.depth_pass_armed(scratch);
                let punctual_armed = atlas.depth_pass_armed(scratch);
                let rows = staged(&app);
                let (word7, slotted) = (rows.word7, rows.slotted);
                let at = format!("{path:?} × {legs:?}, {n} caster batch(es)");

                // R1 (gate G2): a point/spot row is slotted iff its slot field is real, through the
                // production resolve → assignment → `collect_lights` path.
                if rows.punctual != PUNCTUAL_ROWS {
                    findings.push(format!(
                        "{at}: the staged table carries {} point/spot rows, the scene spawns {PUNCTUAL_ROWS}",
                        rows.punctual
                    ));
                }
                for &(r, k) in &rows.disagreeing {
                    findings.push(format!(
                        "{at}: R1 row {r} kind word {k:#010x} — casts bit {} but slot field {} \
                         (every shader site samples by the field alone)",
                        u32::from(k & CASTS_SHADOW_BIT != 0),
                        light_atlas_slot(k)
                    ));
                }
                if rows.slot_none == 0 {
                    findings.push(format!(
                        "{at}: R1 anti-vacuity — no point/spot row carries a SLOT_NONE field, yet two \
                         lights are un-flagged; the agreement check above would hold vacuously"
                    ));
                }
                if atlas != ResolvedShadowAtlas::DISABLED && slotted != SLOTTED_WHEN_LIVE {
                    findings.push(format!(
                        "{at}: R1 budget loser — the live atlas ({} layers) slotted {slotted} rows, \
                         want {SLOTTED_WHEN_LIVE}: one flagged point must lose its slot to the budget",
                        atlas.active_layers
                    ));
                }

                if cfg.csm_shadows != csm_armed || cfg.punctual_shadows != punctual_armed {
                    findings.push(format!(
                        "{at}: header gates (csm {}, punctual {}) disagree with the host's arming \
                         call (csm {csm_armed}, punctual {punctual_armed})",
                        cfg.csm_shadows, cfg.punctual_shadows
                    ));
                }
                let want_word = (u32::from(csm_armed) * CSM_BIT) | (u32::from(punctual_armed) * PUNCTUAL_BIT);
                if word7 & (CSM_BIT | PUNCTUAL_BIT) != want_word {
                    findings.push(format!(
                        "{at}: staged header word 7 {word7:#010x} does not carry the armed bits {want_word:#06x}"
                    ));
                }
                if !carrier.mesh_shadow_producers()
                    && (csm != ResolvedCsm::DISABLED
                        || atlas != ResolvedShadowAtlas::DISABLED
                        || slots != PunctualSlotAssignment::EMPTY
                        || slotted != 0)
                {
                    findings.push(format!(
                        "{at}: a leg set without mesh-shadow producers published a live plan — \
                         csm mode {} active {}, atlas mode {} layers {}, slot handoff empty {}, \
                         slotted staged rows {slotted}",
                        csm.csm_mode_word,
                        csm.active_count,
                        atlas.mode_word,
                        atlas.active_layers,
                        slots == PunctualSlotAssignment::EMPTY
                    ));
                }
                if csm_armed && punctual_armed {
                    both_bits_armed += 1;
                    if slotted == 0 {
                        findings.push(format!("{at}: both passes armed but no staged row carries a slot"));
                    }
                }
            }
        }
        if both_bits_armed == 0 {
            findings.push(format!(
                "{path:?}: no mesh-carrying configuration armed both shadow bits — the sweep would \
                 hold vacuously"
            ));
        }
    }
    assert_eq!(configs, 24, "the sweep covers 4 paths × 3 leg sets × 2 caster counts");
    assert!(findings.is_empty(), "{} finding(s):\n{}", findings.len(), findings.join("\n"));
}
