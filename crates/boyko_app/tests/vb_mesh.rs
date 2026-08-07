//! Multi-paradigm render-path plan, rung R8 — the VisibilityBuffer v1 (FUSED `vb_resolve`)
//! mesh-only golden dump.
//!
//! Reuses [`grand_showcase_2mat`]'s EXACT five-sphere scene (same mesh, same five materials,
//! same sun/sky, same camera) verbatim — mirrors `forward_mesh.rs`'s own precedent: the dumped
//! BMP is a DIRECT visual comparator against the Deferred `f6147f90` / Forward `f93b5aad`
//! goldens (same geometry/lighting/shadows; only the render PATH differs — VB's id-raster +
//! compute-resolve re-fetch vs Forward's inline raster shade). Byte-identity is NOT expected
//! (the geometry re-fetch's analytic barycentric interpolation is a genuinely different
//! floating-point path than the rasterizer's own hardware interpolation) — the orchestrator
//! compares the two visually.
//!
//! # Decision 9 (VB1) 2-instance fixture
//!
//! [`grand_showcase_2mat`]'s five spheres all reference the SAME [`MeshHandle`] (one
//! `register_mesh_vb` call, five `MeshBundle::new(sphere, ...)` spawns) — `gather_mesh_draws`
//! therefore buckets them into ONE [`DrawBatch`] with `instance_count == 5`, so the SAME
//! `vkCmdDrawIndexed`'s `SV_InstanceID` ranges `0..5` against ONE shared index buffer. This is
//! EXACTLY the fixture Decision 9's `raw_prim_id % tri_count` normalization needs to prove
//! itself against instance > 0 (whichever `SV_PrimitiveID` per-instance semantics the driver
//! implements) — no separate fixture is needed; reusing the existing five-sphere scene already
//! exercises it.
//!
//! # Geometry-table slot claim (rung R8 register_mesh gap, closed)
//!
//! [`MeshAssetsExt::register_mesh`](boyko_render::MeshAssetsExt::register_mesh) does not claim a
//! Decision-0 geometry-table slot AT REGISTRATION (that fn's own doc) — it leaves
//! [`VB_GEOMETRY_RESERVED_SLOT`](boyko_render::mesh_geometry_table::VB_GEOMETRY_RESERVED_SLOT)
//! for `backfill_vb_geometry_slots` to replace later in boot. This test uses the VB-aware sibling
//! [`MeshAssetsVbExt::register_mesh_vb`](boyko_render::MeshAssetsVbExt::register_mesh_vb)
//! instead, threading `NonSendResMut<MeshGeometryTableSlot>` — the World resource
//! `boyko_app::runner` constructs (`Some`-armed) BEFORE `app.finish()` drains this startup
//! system, on EVERY boot (`None` when the table isn't armed, e.g. a device lacking the
//! descriptor-indexing prerequisite — this test falls back to the plain, non-VB-aware
//! `register_mesh` in that case, which still renders correctly under the resolver's
//! `VbDeviceCapMissing` degrade to `Deferred`).
//!
//! Windowed-test conventions (mirrors `forward_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[vb_mesh]` pin (UNBLESSED — the orchestrator renders + blesses).

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

use boyko_app::OcclusionForce;
use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, MeshAssetsVbExt, MeshGeometryTableSlot, OcclusionCulling,
    OcclusionMode, RenderPath, RenderPathConfig,
};

mod occ_fixture;
mod vb_occ_mixed_scene;

use occ_fixture::occ_marked;

/// The env knob that arms the depth pyramid — `[vb_mesh_hzb]`'s own.
///
/// ⚠️ Since VG R3 piece 4 rung P4-4 it is the ONLY route to `HzbConfig` in this fixture. The old
/// `|| occ_marked()` disjunction is gone: the host now plans a pyramid when a PRODUCER asks **or**
/// a CONSUMER needs one (`boyko_app::hzb_plan::hzb_plan_for`), so the fixture-local workaround
/// became a second implementation of a rule that lives in the engine. Every occlusion pin still
/// carries `BOYKO_VG_HZB="1"` in its own `[*.env]` block, so the pinned configurations reach the
/// pyramid by the producer route and are unaffected by which of the two routes exists.
const ENV_HZB: &str = "BOYKO_VG_HZB";

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s /
/// `forward_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Verbatim copy of `grand_showcase_2mat.rs::uv_sphere` (see that file's NOTE for why this is a
/// local copy rather than a shared `tests/common` helper — a pinned-golden scene keeps its exact
/// mesh generation frozen).
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi; // 0..π, north pole to south
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32; // phi / π
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi); // 0..2π
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st]; // unit outward normal
            let u = j as f32 / slices as f32; // theta / 2π
            let mut vertex = Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, color);
            vertex.uv = [u, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// Verbatim copy of `grand_showcase_2mat.rs::setup` — the SAME five-sphere scene, so the dumped
/// BMP is a direct VB-vs-Deferred/Forward visual comparator — with ONE delta: the sphere mesh is
/// registered via [`MeshAssetsVbExt::register_mesh_vb`] (falling back to the plain
/// [`MeshAssetsExt::register_mesh`] when the geometry table is not armed) so it claims a
/// Decision-0 geometry-table slot, without which `vb_geom_fetch.hlsli` could never resolve this
/// mesh's `gMeshVerts[]`/`gMeshIndices[]` entries.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    // VG R3 piece 3 step P3-8 (plan D9): the NEW, ORTHOGONAL scene selector. `BOYKO_VG_SCENE=mixed`
    // replaces the five-sphere loop below with `vb_occ_mixed` — two registered meshes, eight
    // instances, six of them marked when `BOYKO_VG_OCC=1`. It is orthogonal to `BOYKO_VG_OCC`
    // precisely so `[vb_occ_mixed_off]` can render the SAME geometry with NOTHING marked; folding
    // the two into one variable (round 2's table) made that baseline unproducible.
    //
    // ⚠️ The five-sphere scene's "all five or none" rule below is SCOPED to it, not deleted. The
    // mixed scene breaks that rule deliberately and carries its own reorder-safety argument: no two
    // instances share a depth, so a ring reshuffle cannot change which one wins a pixel — and the
    // four-pin byte-identity IS the check on that claim.
    if vb_occ_mixed_scene::scene_is_mixed() {
        vb_occ_mixed_scene::spawn_mixed(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut geo_table,
            &dev,
            occ_marked(),
        );
        return;
    }

    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    // VG R3 piece 2 step P2-6 (gate G1): ALL FIVE spheres, or none. A strict subset would split
    // the mesh family into two archetypes, and the gather walks archetypes in order — so the RING
    // ORDER would change, and with it the order two instances writing the same pixel at exactly
    // equal depth resolve. Marking all five keeps ONE archetype and therefore the exact ring
    // order, which is what makes `[vb_occ_split]`'s byte-identity evidence about the RECORDING
    // path rather than about a reshuffle that happened to be invisible. The mixed-archetype case
    // is gate G2's `vb_occ_multi` fixture, where the gate is a count and an order change cannot
    // produce a false red.
    let occ = occ_marked();
    let spacing = 1.55;
    let materials_row: [Option<u16>; 5] =
        [None, Some(red.index() as u16), Some(green.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];
    for (i, mat) in materials_row.iter().enumerate() {
        let x = (i as f32 - 2.0) * spacing;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        // ⚠️ Queued into the SAME command flush as the spawn — NOT into the bundle. This kernel
        // has no tuple `Bundle` impl (`Bundle` is sealed and implemented per type; the tuple impl
        // was deleted in Phase 8.5), so `spawn((MeshBundle, OcclusionCulling))` does not compile.
        // What the plan forbids is an insert from a LATER frame, which arms the split one frame
        // late; spawn and insert queued together are applied by ONE flush before any gather runs —
        // exactly the route `MaterialHandle` below already takes, and an equally late
        // `MaterialHandle` would be visible in this very pin.
        if occ {
            commands.entity(e).insert(OcclusionCulling);
        }
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }

    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.97, 0.92], 3.1),
    });

    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));

    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 1.1, 7.8),
        Vec3::new(0.0, 0.55, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// **The VisibilityBuffer v1 (fused) mesh-only golden dump.** The SAME five-sphere
/// `grand_showcase_2mat` scene, rendered through `RenderPath::VisibilityBuffer ×
/// GeometryLegs::Mesh` instead of the Deferred default — the owner's RTX visual sign-off gate
/// for rung R8 (`goldens/PINS.toml`'s `[vb_mesh]`).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
///
/// # Seven pins, one test
///
/// Env knobs drive this same code path, and each configuration has its own pin. Three render the
/// five-sphere scene and carry `[vb_mesh]`'s own hash: `BOYKO_VG_HZB=1` → `[vb_mesh_hzb]` (the
/// pyramid, built and read by nothing) and `BOYKO_VG_OCC=1` → `[vb_occ_split]` (the marker, hence
/// the pyramid AND the LATE RASTER SCOPE). Sharing the binary makes each equality an identity
/// rather than a resemblance between two scenes that merely look alike.
///
/// **VG R3 piece 3 step P3-8** adds four more behind `BOYKO_VG_SCENE=mixed` — `[vb_occ_mixed_off]`,
/// `[vb_occ_mixed_keep]`, `[vb_occ_mixed]`, `[vb_occ_mixed_late]` — on `vb_occ_mixed_scene`, the
/// first fixture in the tree where the cull can actually reject something. Their four hashes are the
/// SAME literal, and the ladder between them is what makes each difference attributable:
///
/// | step | what changes | a difference means |
/// |---|---|---|
/// | `off` → `keep` | the split predicate, the late scope's bracket, the late dispatch, the second and third descriptor sets | a **PLUMBING** defect |
/// | `keep` → `mixed` | ONE push-constant bit; `defer` goes from identically-false to computed | a **DECISION** defect |
/// | `keep` → `late` | the other push-constant bit | the late raster path |
///
/// The pyramid's existence is deliberately NOT a variable in that family — `off` carries
/// `BOYKO_VG_HZB=1` too, and the pyramid's byte-neutrality is already pinned by `[vb_mesh]` vs
/// `[vb_mesh_hzb]`.
///
/// ⚠️ **Since VG R3 piece 3 step P3-6 `[vb_occ_split]` is no longer an INERT addition — it is a
/// LIVE occlusion cull, and its hash equality is a claim about the DECISION.** The five spheres
/// stand side by side against the sky, so none of them lies behind another's silhouette and the
/// conservative pyramid test rejects nothing: the early phase defers zero instances, the late
/// scope draws zero, and the frame is `[vb_mesh]`'s. A divergence here says the cull deleted
/// visible geometry — the one failure this pin is uniquely placed to catch.
///
/// ⚠️ **What a green `[vb_occ_split]` still cannot claim: that anything was DEFERRED.** A cull that
/// defers nothing and a cull that is off produce the same pixels. Gate G2
/// (`vb_occ_split_gate.rs`) says the recorder recorded two scopes; the non-vacuity clause
/// (`Σ n_defer > 0`) needs a fixture that actually occludes, which is step P3-8's `vb_occ_mixed`.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the VisibilityBuffer mesh-only screenshot"]
fn vb_mesh_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb mesh-only", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R8: request `VisibilityBuffer × Mesh` — inserted
    // AFTER `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this
    // override wins, mirroring `forward_mesh.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    // VG R3 piece 1 step P1-2: `BOYKO_VG_HZB=1` arms the depth pyramid on THIS scene, THIS
    // binary and THIS test — deliberately not a cloned fixture. On the `BOYKO_VG_HZB`-only leg the
    // pyramid is allocated and read by NOTHING (that leg marks no instance, so the cull binds
    // `hzb_null`), so the armed dump must hash to `[vb_mesh]`'s own pin; sharing the code path
    // makes that an identity rather than a resemblance between two scenes that merely look alike.
    // `goldens/PINS.toml`'s `[vb_mesh_hzb]` is that leg, and it carries the SAME sha256 on
    // purpose: a divergence means the allocation perturbed a render it must not touch.
    //
    // The real prize is the VALIDATION leg (`golden.ps1 -Pin vb_mesh_hzb -ValidationOn`). This is
    // the engine's FIRST storage image with a mip chain — every other `TextureDesc` call site in
    // the tree passes `mip_levels: 1` — so the layer is what proves the image and its per-mip
    // views are legal. A byte-identical dump alone would not: an illegal view that nothing binds
    // changes no pixel.
    //
    // ⚠️ VG R3 piece 4 rung P4-4 DELETED the `|| occ_marked()` half of this branch. Piece 3 needed
    // it because `path_vb_occlusion_split()` had gained an `hzb.is_some()` conjunct while nothing
    // made a consumer's need for a pyramid reach the plan — so a marked run with no `BOYKO_VG_HZB`
    // would have silently stopped splitting. Piece 4 promotes that rule into the HOST
    // (`hzb_plan_for` plans a pyramid iff a producer asks OR a consumer needs), so keeping the
    // fixture-local disjunction would be a second implementation of it, in a fixture, able to
    // disagree with the engine. All five occlusion pins carry `BOYKO_VG_HZB="1"` in their own
    // `[*.env]` blocks, so their configuration is unchanged either way — which is also why no pin
    // can red the host disjunct, and why the non-pinned `vb_occ_probe_dump_marked_no_hzb` leg in
    // `vb_occ_split_gate.rs` exists.
    if std::env::var(ENV_HZB).is_ok_and(|v| v == "1") {
        app.insert_resource(HzbConfig { mode: HzbMode::Build });
    }
    // VG R3 piece 4 rung P4-4: the occlusion CONSUMER knob and its diagnostic regime, from THE
    // single insert site. `BOYKO_VG_OCC=1` ⇒ `OcclusionMode::TwoPhase`; `BOYKO_VG_OCC_FORCE` ⇒ the
    // `keep`/`late` override `[vb_occ_mixed_keep]`/`[vb_occ_mixed_late]` name. Unset ⇒ `Off` +
    // `None`, which is the 0%-gate and what the other twenty-six pins render.
    //
    // Until this rung the regime came from a `BOYKO_VG_OCC_FORCE` read inside `GpuSceneBundles::
    // boot` — shipping code, with a boot panic in it, doing a fixture's job. The pin file did not
    // change: `[*.env]` still carries the same words, and this fixture translates them, exactly as
    // it already translates `BOYKO_VG_HZB` into `HzbConfig`.
    occ_fixture::arm_occlusion(&mut app);
    app.run();
}

// ===============================================================================================
// VG R3 piece 4 rung P4-4 — `vb_mesh_occ_pins_actually_split`: the gate INSIDE the pinned binary
// ===============================================================================================
//
// # Why this gate has to live here and nowhere else
//
// All five occlusion pins render through THIS binary and THIS test (`test_binary = "vb_mesh"`,
// `test_name = "vb_mesh_screenshot_dump"`), and four of them declare themselves byte-identical to
// a pin that never splits. So one edit — deleting the `OcclusionConfig` insert from
// `occ_fixture::arm_occlusion_with` — can silently disarm the split in all five while every hash,
// AND `the_pins_declared_byte_identical_actually_agree` (which compares pins to EACH OTHER), stays
// green. That is not a hypothetical: a scope that draws nothing and a scope that does not exist
// produce the same pixels, which is what the four-hash equality is FOR.
//
// Before this rung the executed evidence lived in other binaries (G2 in `vb_occ_split_gate`,
// G-P3-A/B/C in `vb_occ_mixed`), so an edit to `vb_mesh.rs` reached no gate that executes. This
// one adjudicates the PINNED CONFIGURATIONS — each pin's own `[*.env]` block, verbatim from
// `goldens/PINS.toml`, re-executed through the pin's own binary and test.
//
// # What it CANNOT claim
//
// * It runs PROBE-ON while the pins are PROBE-OFF. The gap is small and named: `vb_probe_dump` is
//   a host-side counter sink that records no command, allocates nothing on the device and is not
//   an input to `path_vb_occlusion_split()`. It is a gap all the same.
// * It cannot distinguish the two FORCE regimes by EFFECT — `keep` and `late` both record two
//   scopes. What it checks is that the regime WORD reached the GPU (`[probe] occ_regime`, stamped
//   from the pushed push-constant) and agrees with the pin's env. Distinguishing them by effect is
//   G-P3-B/C's job, and it reads GPU counters.
// * Nothing about pixels. The hashes are the pixel claim; this is the claim they cannot make.

/// The five occlusion pins, with the raster-scope count each pinned configuration must record.
///
/// `[vb_occ_mixed_off]` is here as a REQUIRED NEGATIVE, not for symmetry: without a leg that must
/// report `1`, a gate asserting `scopes == 2` four times is satisfied by a recorder that always
/// splits, and the four-pin ladder's `off -> keep` step would be unfalsifiable from this side.
const OCC_PINS: [(&str, u32); 5] = [
    ("vb_occ_split", 2),
    ("vb_occ_mixed_off", 1),
    ("vb_occ_mixed_keep", 2),
    ("vb_occ_mixed", 2),
    ("vb_occ_mixed_late", 2),
];

/// The `[*.env]` key this gate REDIRECTS rather than inherits.
///
/// Redirected, never removed: removing it would change the configuration away from the pin's — the
/// screenshot capture is part of what the pinned run does, and it is what makes the run terminate —
/// while leaving it pointed at the blessed BMP would have a gate overwrite the artifact it exists
/// to protect.
const ENV_HOST_DUMP: &str = "BOYKO_HOST_DUMP";

/// The record probe's own knob; this gate's payload.
const ENV_PROBE: &str = "BOYKO_VB_PROBE";

/// The pin-file key whose value is the regime word.
const ENV_OCC_FORCE: &str = "BOYKO_VG_OCC_FORCE";

/// `goldens/PINS.toml`, from this crate's manifest directory.
fn pins_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../goldens/PINS.toml")
}

/// Every `KEY = "VALUE"` pair of `[<pin>.env]`, in file order.
///
/// A LOCAL reader rather than a borrowed one, for the reason `vb_occ_split_gate.rs` gives about its
/// own field reader: a gate that borrows another gate's parser inherits that gate's future edits.
/// It handles the flat, quoted subset `PINS.toml` actually uses, and it UNESCAPES `\\` because the
/// dump paths are Windows paths written as TOML basic strings (`"D:\\tmp\\x.bmp"`) — a reader that
/// passed the escaped form through would hand the child a path with doubled separators.
///
/// An absent or empty table PANICS: a gate that silently ran a pin with no env would render the
/// five-sphere default scene and report it as the mixed one.
fn pin_env(pins: &str, pin: &str) -> Vec<(String, String)> {
    let want = format!("[{pin}.env]");
    let mut inside = false;
    let mut out = Vec::new();
    for line in pins.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.starts_with('[') && l.ends_with(']') {
            if inside {
                break;
            }
            inside = l == want;
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
        {
            let key = k.trim().to_string();
            let val = v.trim().trim_matches('"').replace("\\\\", "\\");
            out.push((key, val));
        }
    }
    assert!(
        !out.is_empty(),
        "goldens/PINS.toml has no `{want}` -- this gate adjudicates the PINNED configurations, so \
         a pin whose env it cannot read is a gate that would render something else and pass"
    );
    out
}

/// The raw right-hand side of `table.key` in the record probe's flat TOML subset. Local, for the
/// same reason [`pin_env`] is.
fn probe_field(src: &str, path: &str, file: &Path) -> String {
    let (table, key) = path.split_once('.').expect("a probe path is `table.key`");
    let mut inside = false;
    for line in src.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.starts_with('[') && l.ends_with(']') {
            inside = l.trim_start_matches('[').trim_end_matches(']') == table;
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return v.trim().trim_matches('"').to_string();
        }
    }
    panic!(
        "the probe file {} has no `{path}` -- a gate that reads a missing field asserts nothing",
        file.display()
    )
}

fn probe_u32(src: &str, path: &str, file: &Path) -> u32 {
    probe_field(src, path, file).parse().unwrap_or_else(|_| panic!("`{path}` is not an integer"))
}

/// Strict: anything that is neither `true` nor `false` PANICS rather than reading as `false`. A
/// silent `false` here would fire the instrument clause and report "this is not a VB frame" about
/// a frame that was one.
fn probe_bool(src: &str, path: &str, file: &Path) -> bool {
    match probe_field(src, path, file).as_str() {
        "true" => true,
        "false" => false,
        other => panic!("`{path}` is `{other}`, which is not a boolean"),
    }
}

/// What one pinned configuration recorded.
struct PinProbe {
    scopes: u32,
    late_draws: u32,
    late_cull_dispatches: u32,
    /// `[probe] occ_regime` — decoded from the word the RECORDER pushed.
    probe_regime: String,
    draw_batches: u32,
    occlusion_instances: u32,
    vb_path: bool,
    mesh_leg: bool,
    /// `[host] occ_mode` / `occ_force` — the host's independent derivation, for the cross-check.
    host_mode: String,
    host_force: String,
}

/// Re-executes `vb_mesh_screenshot_dump` under one pin's env and returns what it recorded.
fn run_pin(pin: &str, pins: &str) -> PinProbe {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let probe_out: PathBuf = std::env::temp_dir().join(format!("boyko_pin_split_{pin}.toml"));
    let bmp_out: PathBuf = std::env::temp_dir().join(format!("boyko_pin_split_{pin}.bmp"));
    // A stale file from a previous run that this run failed to overwrite would be read as this
    // run's evidence -- and "the capture never ran" and "the capture left last run's file" are the
    // same bytes.
    for p in [&probe_out, &bmp_out] {
        let _ = std::fs::remove_file(p);
    }

    let mut cmd = Command::new(&exe);
    cmd.args(["vb_mesh_screenshot_dump", "--ignored", "--exact", "--test-threads=1", "--nocapture"]);
    // The pin's own block, VERBATIM -- that is the whole point: this gate adjudicates the
    // configuration `goldens/PINS.toml` declares, not a lookalike spelled here.
    for (k, v) in pin_env(pins, pin) {
        cmd.env(k, v);
    }
    // ...with exactly two substitutions, both named above.
    cmd.env(ENV_HOST_DUMP, &bmp_out).env(ENV_PROBE, &probe_out);
    // Every UNRELATED capture and bench removed: another armed capture would hold the host loop
    // open (the exit is a conjunction over all armed drivers) and either bench returns before any
    // capture completes.
    cmd.env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VB_BENCH")
        .env_remove("BOYKO_SV0_BENCH");

    let status = cmd.status().expect("invariant: the worker process spawns");
    assert!(status.success(), "the `{pin}` pin worker exited {status}");

    let text = std::fs::read_to_string(&probe_out).unwrap_or_else(|e| {
        panic!(
            "`{pin}`: the worker wrote no probe at {} ({e}). A worker that renders and produces \
             nothing is an instrument failure, not an unsplit frame.",
            probe_out.display()
        )
    });
    PinProbe {
        scopes: probe_u32(&text, "probe.scopes", &probe_out),
        late_draws: probe_u32(&text, "probe.late_draws", &probe_out),
        late_cull_dispatches: probe_u32(&text, "probe.late_cull_dispatches", &probe_out),
        probe_regime: probe_field(&text, "probe.occ_regime", &probe_out),
        draw_batches: probe_u32(&text, "host.draw_batches", &probe_out),
        occlusion_instances: probe_u32(&text, "host.occlusion_instances", &probe_out),
        vb_path: probe_bool(&text, "host.vb_path", &probe_out),
        mesh_leg: probe_bool(&text, "host.mesh_leg", &probe_out),
        host_mode: probe_field(&text, "host.occ_mode", &probe_out),
        host_force: probe_field(&text, "host.occ_force", &probe_out),
    }
}

/// **THE PINNED CONFIGURATIONS ACTUALLY SPLIT** (VG R3 piece 4 rung P4-4).
///
/// Boots each of the five occlusion pins with its own `[*.env]` block and asserts, from the
/// RECORDER's counts, that four of them record two raster scopes and `[vb_occ_mixed_off]` records
/// one. Read `OCC_PINS`'s doc for why the negative leg is required, and this section's header for
/// what the gate cannot claim.
#[test]
#[ignore = "live GPU gate (spawns five windowed workers); the orchestrator runs it with --test-threads=1"]
fn vb_mesh_occ_pins_actually_split() {
    let pins = std::fs::read_to_string(pins_path())
        .expect("invariant: goldens/PINS.toml is in the repository");

    for (pin, want_scopes) in OCC_PINS {
        let env = pin_env(&pins, pin);
        // The pin's own regime word, from the pin FILE -- `none` when the block sets no override.
        // This is the third derivation the leg cross-checks (pin file, host Resource, recorder
        // push), and the only one produced outside the process under test.
        // The default word comes from the SHARED table, not from a literal spelled here: the pin
        // file, the fixture's decode, the host's `[host] occ_force` and the recorder's
        // `[probe] occ_regime` must all be the same three words, and a fourth spelling in a gate
        // is a text that can drift from the thing it adjudicates.
        let want_regime = env
            .iter()
            .find(|(k, _)| k.as_str() == ENV_OCC_FORCE)
            .map_or_else(|| OcclusionForce::None.as_str().to_string(), |(_, v)| v.clone());
        let p = run_pin(pin, &pins);

        // ---- the instrument clause, before any count -------------------------------------------
        assert!(
            p.vb_path && p.mesh_leg,
            "{pin}: the probed frame resolved vb_path={} mesh_leg={} -- it is not a \
             `VisibilityBuffer x Mesh` frame, so its counts say nothing about the split. That is \
             an instrument failure (a device that failed the VB capability probe degrades to \
             Deferred), not a gate result.",
            p.vb_path,
            p.mesh_leg
        );

        // ---- the load-bearing clause -----------------------------------------------------------
        assert_eq!(
            p.scopes, want_scopes,
            "{pin}: the recorder reported {} raster scope(s), expected {want_scopes}. This number \
             is incremented AT the `vkCmdEndRendering` of each scope. The pin's HASH cannot see \
             it: a late scope that draws nothing and a late scope that does not exist produce the \
             same pixels, which is exactly why all these pins share one literal.",
            p.scopes
        );

        let splits = want_scopes == 2;
        // ---- the late scope's draws, and the late cull's dispatch ------------------------------
        if splits {
            assert_eq!(
                p.late_draws, p.draw_batches,
                "{pin}: {} late draws against {} host batches. The two come from different sites \
                 (the recorder's per-draw counter and `mesh_draw.len()`), so this compares two \
                 derivations rather than one with itself.",
                p.late_draws, p.draw_batches
            );
            assert_eq!(
                p.late_cull_dispatches, 1,
                "{pin}: {} late cull dispatches on a SPLIT frame -- exactly one is recorded per \
                 split frame, at the `vkCmdDispatch` itself. A 0 means the phase-1 dispatch was \
                 not recorded, and the late scope would then draw whatever the host seeded \
                 (nothing), which every image gate reads as green.",
                p.late_cull_dispatches
            );
            assert!(
                p.occlusion_instances > 0,
                "{pin}: the split recorded with ZERO marked instances in the ring -- the marker \
                 never reached the gather, so this leg would be adjudicating a fixture failure"
            );
        } else {
            assert_eq!(
                p.late_draws, 0,
                "{pin}: {} late draws recorded with no late scope",
                p.late_draws
            );
            assert_eq!(
                p.late_cull_dispatches, 0,
                "{pin}: {} late cull dispatches on the UNSPLIT baseline -- the split arming itself \
                 on the unarmed path",
                p.late_cull_dispatches
            );
        }

        // ---- the REGIME's provenance, three derivations (VG R3 piece 4 rung P4-4, plan A5) ------
        //
        // `[probe] occ_regime` is decoded from the word `record_vb` PUSHED; `[host] occ_force` is
        // what the runner's live `try_resource` read said on the same frame; `want_regime` is the
        // pin file's own `BOYKO_VG_OCC_FORCE` value. The regime became a live Resource at this
        // rung, replacing a boot-time env read whose justification was that a mid-run knob makes
        // "which regime produced this capture?" unanswerable. Comparing three independently
        // derived answers is how that is answered without asserting constancy on hosts this
        // repository does not own.
        assert_eq!(
            p.host_force, want_regime,
            "{pin}: the host read regime `{}` while `[{pin}.env]` sets `{want_regime}`. The \
             fixture's env decode and the pin file disagree, so the pinned configuration is not \
             what rendered.",
            p.host_force
        );
        // The recorder's word is the one sourced from the GPU-bound push constant rather than from
        // a Resource, so it is the one that can catch a regime that never reached the device. On
        // the unsplit leg no batch-cull push carries a FORCE bit and `none` is the honest word,
        // which is also what that pin's env declares.
        assert_eq!(
            p.probe_regime, want_regime,
            "{pin}: the RECORDER pushed regime `{}` while the pin declares `{want_regime}`",
            p.probe_regime
        );
        let want_mode =
            if splits { OcclusionMode::TwoPhase.as_str() } else { OcclusionMode::Off.as_str() };
        assert_eq!(
            p.host_mode, want_mode,
            "{pin}: the host read `OcclusionMode` `{}` where the pinned configuration needs \
             `{want_mode}`",
            p.host_mode
        );

        println!(
            "vb_mesh occ pin `{pin}`: scopes={} late_draws={} late_cull_dispatches={} \
             (batches={}, marked={}) regime probe={} host={} pin={} mode={}",
            p.scopes,
            p.late_draws,
            p.late_cull_dispatches,
            p.draw_batches,
            p.occlusion_instances,
            p.probe_regime,
            p.host_force,
            want_regime,
            p.host_mode
        );
    }
}
