//! Rung A6 end-to-end: an `aether!` block's `scene` spawns REAL entities into a REAL `App` — the
//! plugin registers the spawn fn as a startup one-shot, the kernel applies the commands, and a
//! sibling aether `system` reads the world back through ordinary queries.
//!
//! # What only this test can catch
//!
//! The unit snapshots pin the TOKENS a scene emits. They cannot tell whether those tokens produce
//! an entity that the kernel actually holds, with the components actually attached:
//!
//! * **The material seam A5 deferred.** `material: arena_gold` has to travel through FOUR steps —
//!   the sibling `material` construct resolves, its builder fn is called, `Assets::add` mints a
//!   row, and the row's index narrows into a `MaterialHandle(u16)` on the spawned entity. Only a
//!   run can follow it end to end, and this test does it by RESOLVING each spawned handle back to
//!   its asset and comparing base colors: a handle that pointed at the wrong row would type-check
//!   perfectly and light the prop with the wrong material.
//! * **The hoist is a hoist.** Two props share `arena_chalk`. If the mint were emitted per NODE
//!   instead of once per scene fn, both props would still render — with two asset rows. The
//!   assertion that the two handles are EQUAL is what makes that observable.
//! * **`children:` really parents.** Aether emits `Commands::add_child`, whose whole effect is a
//!   `ChildOf` insertion; the reverse `Children` collection is maintained by that component's own
//!   hooks. Asserting on `Children` (not on `ChildOf`) proves the kernel's reactive half ran.
//!
//! # The mesh half is a REGISTRATION gate, deliberately
//!
//! `plane`/`cube`/`mesh` bindings need a live `VulkanContext` (`MeshAssetsExt` uploads buffers), so
//! the §3.7 `lab` scene cannot RUN in a headless test. What it must still prove is the §8 R4
//! anti-drift claim: that the emitted tokens name real items with real signatures. Building the
//! plugin does exactly that — `add_startup_system(lab)` will not compile unless every param of the
//! generated fn is a legal `SystemParam` and every call in its body resolves. The `App` is never
//! updated, so no device is touched.
//!
//! That gate is why the `vb_lab` module carries a SECOND scene (`annex`). The `aether-lang` token
//! pins have no engine dependency and therefore cannot notice an engine change at all; this file
//! is the only place a `SpotLight::new` parameter, a `Projection::Perspective` field or a
//! `register_mesh` slice type is checked against Aether's emission. An emission path absent from
//! this file is a path NO compiler ever sees, so the three scenes here cover the surface by
//! construction rather than by accident:
//!
//! | path | covered by |
//! |---|---|
//! | `mesh` / `sun` / `sky` / `sdf` heads, `plane` + `cube` sources, verbatim `at Transform { … }` | `vb_lab::lab` |
//! | `spot` / `point` / `camera` heads, the `mesh(&V, &I)` source, `CastsPunctualShadow`, the second-camera `Camera { order }` escape | `vb_lab::annex` |
//! | `entity` head (both shapes), `children:`, `ShadowCaster`, `material:` on a hand-assembled node | `arena` (the RUNNING scene) |

use aether::aether;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_math::{Quat, Vec3};
use boyko_render::{
    DirectionalLight, Material, PointLight, SdfEdit, SdfPrimitive, ShadowCaster, SkyLight, sdf_op,
};
use boyko_render::mesh::Vertex;
use boyko_scene::{Camera, MaterialHandle, Transform};

aether! {
    plugin Arena;

    material arena_gold { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
    material arena_chalk { base: (0.86, 0.86, 0.88), roughness: 0.85 }

    // A scene with no mesh binding: §3.7's demand-driven rule gives it `(commands, materials)`
    // only, which is exactly why it runs headless.
    scene arena {
        entity at (2.0, 0.5, -1.0) { material: arena_gold, casts_shadow };
        entity at (-2.0, 0.5, -1.0) { material: arena_chalk };
        entity at (4.0, 0.5, -1.0) { material: arena_chalk };

        entity at (0.0, 0.0, 0.0) {
            children: [
                entity at (0.0, 1.0, 0.0) { },
                entity at (0.0, 2.0, 0.0) { }
            ]
        };

        sun { dir: (-0.42, 0.80, 0.42), color: (1.0, 0.97, 0.92), lux: 3.2 }
        sky { sky: (0.28, 0.36, 0.50), ground: (0.15, 0.14, 0.13) }
        point { pos: (-1.8, 2.2, 2.4), color: (0.5, 0.7, 1.0), power: 240.0, range: 9.0 }

        sdf SdfEdit::sphere([3.2, 0.85, 1.8], 0.85, sdf_op::UNION, 0.0);
    }

    // The read-back. Aether scenes are plain spawn fns with no captures, so a sibling system + a
    // probe resource is the only honest channel — the same shape rung A2's behavior test uses.
    //
    // TWO probes, not one: the prop half and the light half together take nine `SystemParam`s, and
    // clippy's `too_many_arguments` fires on the generated fn with its span on the whole `aether!`
    // block. Splitting is the honest fix here (a probe is not one job), and it costs nothing —
    // both take `mut res<Probe>`, so the scheduler serializes them.
    system probe_props(
        props: query<(&MaterialHandle, &Transform)>,
        casters: query<&MaterialHandle, with ShadowCaster>,
        materials: res<Assets<Material>>,
        out: mut res<Probe>,
    ) on update {
        for (handle, transform) in &props {
            out.props += 1;
            let asset = *materials.get_by_index(handle.0 as u32).expect(
                "invariant: the scene minted this row before it wrote the handle",
            );
            out.seen.push((handle.0, asset.gpu.base_color, transform.translation));
        }
        out.casters = (&casters).into_iter().count() as u32;
    }

    system probe_lights(
        suns: query<&DirectionalLight>,
        skies: query<&SkyLight>,
        points: query<&PointLight>,
        edits: query<&SdfPrimitive>,
        broods: query<&Children>,
        out: mut res<Probe>,
    ) on update {
        out.suns = (&suns).into_iter().count() as u32;
        out.skies = (&skies).into_iter().count() as u32;
        out.points = (&points).into_iter().count() as u32;
        out.edits = (&edits).into_iter().count() as u32;
        for sun in &suns {
            out.sun_lux = sun.illuminance;
            out.sun_color = sun.color;
        }
        for sky in &skies {
            out.sky = sky.sky_color;
            out.ground = sky.ground_color;
        }
        for p in &points {
            out.point_range = p.range;
            out.point_power = p.power;
            out.point_pos = p.position;
        }
        for brood in &broods {
            out.brood_sizes.push(brood.as_slice().len() as u32);
        }
    }
}

/// The read-back channel for [`probe_props`] and [`probe_lights`].
#[derive(boyko_macros::Resource, Default)]
struct Probe {
    props: u32,
    casters: u32,
    suns: u32,
    skies: u32,
    points: u32,
    edits: u32,
    sun_lux: f32,
    sun_color: [f32; 3],
    sky: [f32; 3],
    ground: [f32; 3],
    point_range: f32,
    point_power: f32,
    point_pos: [f32; 3],
    /// `(material slot, the row's base color, the entity's translation)` per placed prop.
    seen: Vec<(u16, [f32; 4], Vec3)>,
    brood_sizes: Vec<u32>,
}

#[test]
fn an_aether_scene_spawns_real_entities_with_the_declared_components() {
    let mut app = App::new();
    // The world-global CPU material authority the host inserts at boot; no GPU device involved.
    app.insert_resource(Assets::<Material>::with_reserved(8));
    app.insert_resource(Probe::default());
    app.add_plugin(Arena);

    app.update();

    let probe = app.world().resource::<Probe>();

    // --- The three placed props, each carrying the handle its `material:` prop declared.
    assert_eq!(probe.props, 3, "three material-carrying `entity` nodes spawned three entities");
    assert_eq!(probe.casters, 1, "exactly the node that wrote `casts_shadow` got `ShadowCaster`");

    let gold = probe
        .seen
        .iter()
        .find(|(_, _, t)| t.x == 2.0)
        .expect("invariant: the `at (2.0, 0.5, -1.0)` prop spawned");
    assert_eq!(gold.1, [1.0, 0.72, 0.30, 1.0], "the gold prop resolves to `arena_gold`'s row");
    assert_eq!(gold.2, Vec3::new(2.0, 0.5, -1.0), "the `at (x, y, z)` sugar placed it");

    let chalk: Vec<_> = probe.seen.iter().filter(|(_, c, _)| *c == [0.86, 0.86, 0.88, 1.0]).collect();
    assert_eq!(chalk.len(), 2, "two props resolve to `arena_chalk`'s row");
    // The hoist's observable: one material named twice is ONE minted row, not two.
    assert_eq!(chalk[0].0, chalk[1].0, "the mint is hoisted once per scene fn, not once per node");
    assert_ne!(gold.0, chalk[0].0, "two distinct materials take two distinct asset rows");

    // --- The light heads lower to the engine's own constructors, with the authored values.
    assert_eq!(probe.suns, 1, "`sun` spawned one DirectionalLightObject");
    assert_eq!(probe.sun_lux, 3.2, "`lux:` reached `DirectionalLight::new`'s illuminance");
    assert_eq!(probe.sun_color, [1.0, 0.97, 0.92], "`color:` reached it in the right slot");
    assert_eq!(probe.skies, 1, "`sky` spawned one SkyLight");
    assert_eq!(probe.sky, [0.28, 0.36, 0.50], "`sky:` is the upper hemisphere");
    assert_eq!(probe.ground, [0.15, 0.14, 0.13], "`ground:` is the lower one — not swapped");
    assert_eq!(probe.points, 1, "`point` spawned one PointLightObject");
    // `power` and `range` are adjacent `f32` parameters, so a transposition type-checks; two
    // different numbers are what make the swap visible (the A5 `roughness`/`reflectance` lesson).
    assert_eq!(probe.point_power, 240.0, "`power:`, as written");
    assert_eq!(probe.point_range, 9.0, "`range:`, as written (NOT power's value)");
    assert_eq!(probe.point_pos, [-1.8, 2.2, 2.4], "`pos:` seeded the light's own position lane");

    assert_eq!(probe.edits, 1, "`sdf EXPR` spawned one SdfPrimitive carrying the verbatim edit");

    // --- `children:` parented through the kernel's own `ChildOf` seam.
    assert_eq!(probe.brood_sizes, vec![2], "one parent, holding exactly its two declared children");
}

/// §3.7's Before block VERBATIM — the vb_lab compression, as the plan prints it.
///
/// This block exists to be COMPILED (see the module header): the `let … = plane/cube(…)` bindings
/// make the spawn fn demand `NonSendResMut<Assets<MeshGpu>>` and `NonSendRes<GpuDevice>`, and the
/// only thing a headless process can prove about them is that they are real params of a real
/// system over real engine calls. `add_startup_system` proves exactly that, at compile time.
mod vb_lab {
    use super::*;

    aether! {
        plugin VbLab;

        material gold { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }
        material lamp { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }

        scene lab {
            let floor = plane(22.0);
            let block = cube(1.0);

            mesh floor;
            mesh block at Transform { translation: Vec3::new(0.0, 3.0, -4.5),
                                      rotation: Quat::IDENTITY,
                                      scale: Vec3::new(14.0, 6.0, 0.4) };
            mesh block at (-2.4, 0.5, -2.2) { material: gold, casts_shadow };
            mesh block at (-4.4, 1.4, -1.0) { material: lamp };

            sdf SdfEdit::sphere([3.2, 0.85, 1.8], 0.85, sdf_op::UNION, 0.0);

            sun { dir: (-0.42, 0.80, 0.42), color: (1.0, 0.97, 0.92), lux: 3.2 }
            sky { sky: (0.28, 0.36, 0.50), ground: (0.15, 0.14, 0.13) }
        }

        // The heads and the mesh source §3.7's Before block does NOT reach, kept in a second scene
        // so `lab` above stays the plan's example verbatim. Nothing here is decorative: each row
        // is an emission path that, without it, NO compiler ever sees.
        //
        //   * `mesh(&V, &I)`  -> `MeshAssetsExt::register_mesh` (the third mesh source; `plane`
        //                       and `cube` are the only two `lab` uses).
        //   * `spot`          -> `SpotLight::new`'s SEVEN arguments + `Affine3A::look_at_rh` over
        //                       `Vec3 + Vec3`. Seven adjacent scalars is the argument list most
        //                       able to drift silently.
        //   * `casts_shadow` on `spot`/`point` -> `CastsPunctualShadow` (the OTHER `ShadowForm`;
        //                       `lab` only ever reaches `ShadowCaster`).
        //   * `camera`        -> `CameraRig` + `Camera::DEFAULT` + `Projection::Perspective`, a
        //                       four-field struct literal no other scene in the suite builds.
        scene annex {
            let custom = mesh(&annex_vertices(), &annex_indices());
            // §7.2(4), COMPILED: `dev` is one of the four names §3.7's After block gives the
            // generated params. Before they were `__aether_`-prefixed this line shadowed the
            // device param and the scene failed to compile with both labels on the `aether!`
            // token. It now resolves to this binding, and the device is still reachable.
            let dev = cube(0.5);

            mesh custom at (0.0, 0.0, 5.0) { material: gold, casts_shadow };
            mesh dev at (1.0, 0.0, 5.0) { material: lamp };

            spot {
                pos: (3.6, 4.2, 3.2), dir: (-0.6, -0.7, -0.5),
                color: (1.0, 0.85, 0.6),
                power: 6000.0, range: 14.0, inner: 16.0, outer: 26.0,
                casts_shadow
            }

            point { pos: (-1.8, 2.2, 2.4), color: (0.5, 0.7, 1.0), power: 240.0, range: 9.0, casts_shadow }

            camera at (0.0, 2.1, 8.4) { aspect: 1120.0 / 720.0, fov: 52.0, far: 120.0 }

            // The §8 R8 escape, compiled: a bare component expression is inserted AFTER the
            // bundle, so a second camera gets its own draw order without new sugar.
            camera at (0.0, 2.1, -8.4) {
                aspect: 1120.0 / 720.0,
                Camera { order: 1, ..Camera::DEFAULT }
            }
        }
    }

    /// A triangle for the `mesh(&V, &I)` binding — an ordinary fn, so the scene body's `&…`
    /// borrows a temporary that lives to the end of the registration statement.
    fn annex_vertices() -> Vec<Vertex> {
        vec![
            Vertex::new([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0, 1.0]),
            Vertex::new([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0, 1.0]),
            Vertex::new([0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0, 1.0]),
        ]
    }

    /// The index buffer for [`annex_vertices`].
    fn annex_indices() -> Vec<u32> {
        vec![0, 1, 2]
    }

    /// The §8 R4 gate: BOTH scenes' generated fns are legal systems over the REAL engine API, and
    /// the plugin registers them. The app is never updated — a mesh binding needs a live device.
    ///
    /// This is where a changed engine constructor breaks. `add_startup_system` requires
    /// `IntoSystem`, which type-checks the whole generated body: the day `SpotLight::new` grows a
    /// parameter, `Projection::Perspective` gains a field, `CastsPunctualShadow` moves crate, or
    /// `register_mesh` changes its slice types, THIS test stops compiling — in-repo, the same day.
    /// The `aether-lang` token pins cannot do that (that crate has no engine dependency at all);
    /// they gate the expander, this gates the engine seam.
    #[test]
    fn the_vb_lab_compression_registers_as_a_real_startup_system() {
        let mut app = App::new();
        app.add_plugin(VbLab);
        // The material builders are ordinary fns and need no world at all — calling them here
        // proves the scene's hoisted `materials.add(gold())` names something callable.
        assert_eq!(gold().gpu.base_color, [1.0, 0.72, 0.30, 1.0]);
        assert_eq!(lamp().gpu.emissive, [1.6, 0.9, 0.3, 0.0]);
    }
}
