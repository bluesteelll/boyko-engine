//! **VG rung R2d-5 — the per-INSTANCE cull fixture, its two framings, and the probe-line reader.**
//!
//! Shared by `vb_inst_cull_narrow.rs` (the gate), `vb_inst_cull_wide.rs` (its control),
//! `vb_inst_cull_ids.rs` (the id-mapping gate) and `vb_inst_cull_corpus.rs` (which uses only the
//! probe reader). One scene definition, four consumers: two copies of a fixture are two texts that
//! can disagree, and a fixture that disagrees with its own control proves nothing.
//!
//! # The scene
//!
//! TWO registered meshes — hence two `DrawBatch`es, since `gather_mesh_draws` buckets by `mesh_id`
//! — with [`BATCH_INSTANCES`] spawns each. Five distinct materials over six instances, so one
//! material is shared by two instances: a material id must not be confusable with an instance id,
//! and the id-mapping gate would be weaker if the two were in bijection.
//!
//! # ⚠️ TWO FIXTURE PROPERTIES ARE LOAD-BEARING — do not "tidy" them away
//!
//! Both exist to give the R2d-6 arming rung's gate its teeth. A later edit that simplifies either
//! one leaves every assertion still green while removing what it detects.
//!
//! 1. **The second batch must have `base_instance > 0`.** With `base_instance == 0` for every
//!    batch, `visible[base + id]` and `visible[id]` are the same expression, and a vertex shader
//!    that dropped the base term would render identically. [`BATCH_INSTANCES`] `> 0` with two
//!    batches is what guarantees it: the gather assigns `base_instance = running` in mesh-id order
//!    (`boyko_render::mesh_draw`'s prefix sum), so batch 1's base is [`BATCH_INSTANCES`].
//!
//! 2. **The culled instance must NOT be last in its batch.** If it were, the compacted survivor
//!    list would equal the identity PREFIX `[base, base + k)` — and then a raster that read a STALE
//!    list (the identity this rung writes) and a raster that read the compacted one would produce
//!    the SAME image, as would one that mis-exported the compacted slot instead of the global id.
//!    [`OFFSCREEN_LOCAL_INDEX`] is what makes the compaction observable, and
//!    [`assert_fixture_invariants`] checks it is interior rather than trusting the constant.
//!
//! Both are STRUCTURAL here, not incidental: [`local_x`] derives an instance's world X from
//! [`OFFSCREEN_LOCAL_INDEX`], so moving that constant moves the geometry with it.

#![allow(dead_code)]

use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::frustum::{FRUSTUM_PLANE_COUNT, Plane, frustum_planes_from_push_bytes};
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::mesh::Vertex;
use boyko_render::{Material, MeshAssetsVbExt, MeshGeometryTableSlot, generate_tangents};
use boyko_scene::ViewUniform;

/// The sun direction TO the light — `vb_mesh.rs`'s value, so this fixture lights the same way the
/// pinned VB scene does and a visual glance at it is comparable.
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Draw batches in the fixture: one per registered mesh.
pub const BATCH_COUNT: usize = 2;

/// Instances per batch.
pub const BATCH_INSTANCES: usize = 3;

/// Total instances — the fixture's whole instance ring.
pub const INSTANCE_COUNT: usize = BATCH_COUNT * BATCH_INSTANCES;

/// The LOCAL index (within its batch) of the instance the arming rung must cull.
///
/// INTERIOR by requirement — see this module's header, property 2. `1` of `3` is the smallest
/// arrangement that is interior at all.
pub const OFFSCREEN_LOCAL_INDEX: usize = 1;

/// World X of the off-screen instance. An order of magnitude outside the narrow framing's cone, so
/// the fixture tests the CULL rather than an exact aspect ratio — the same reasoning
/// `vb_cull_offscreen.rs` gives its own choice, and the same value.
pub const OFFSCREEN_X: f32 = 40.0;

/// World X magnitude of the on-screen instances.
pub const ONSCREEN_X: f32 = 1.0;

/// World Y of each batch's row, so the two batches are visually separable in a frame dump.
const BATCH_Y: [f32; BATCH_COUNT] = [0.6, -0.6];

/// Distinct materials the fixture registers. Fewer than [`INSTANCE_COUNT`] on purpose: one material
/// is shared, so nothing downstream can be keyed on a material↔instance bijection.
pub const MATERIAL_COUNT: usize = 5;

/// The window extent both framings render at. 1:1, so the horizontal field equals the vertical one
/// and the fov below is the whole story.
pub const EXTENT: (u32, u32) = (512, 512);

/// One camera framing. The two below differ ONLY in `eye`, which is what makes the wide one a
/// control for the narrow one rather than a second experiment.
pub struct Framing {
    /// Short name, used in probe-worker temp-file tags.
    pub id: &'static str,
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
}

/// **The narrow framing** — the gate. At `eye.z = 6` with a 52° vertical field the half-extent at
/// the instance plane is `6 * tan(26°) ≈ 2.93` world units, so the four instances at
/// `|x| = ONSCREEN_X` are comfortably inside and the two at [`OFFSCREEN_X`] are an order of
/// magnitude outside.
pub const NARROW: Framing = Framing {
    id: "narrow",
    eye: [0.0, 0.0, 6.0],
    target: [0.0, 0.0, 0.0],
    fov_y_degrees: 52.0,
    near: 0.1,
    far: 400.0,
};

/// **The wide framing** — the narrow test's CONTROL. `eye.z = 100` gives a half-extent of
/// `100 * tan(26°) ≈ 48.8`, which contains all six instances including the ones at
/// [`OFFSCREEN_X`]. Everything else — fov, near, far, the scene itself — is identical, so a
/// difference between the two runs can only be the framing.
pub const WIDE: Framing = Framing {
    id: "wide",
    eye: [0.0, 0.0, 100.0],
    target: [0.0, 0.0, 0.0],
    fov_y_degrees: 52.0,
    near: 0.1,
    far: 400.0,
};

/// The world X of the instance at local index `local`.
///
/// The off-screen position is keyed on [`OFFSCREEN_LOCAL_INDEX`] rather than written at a literal
/// slot, so the fixture's "the culled instance is interior" property cannot drift away from the
/// constant that names it.
pub fn local_x(local: usize) -> f32 {
    if local == OFFSCREEN_LOCAL_INDEX {
        OFFSCREEN_X
    } else if local < OFFSCREEN_LOCAL_INDEX {
        -ONSCREEN_X
    } else {
        ONSCREEN_X
    }
}

/// The world position of batch `batch`'s instance at local index `local`.
pub fn instance_position(batch: usize, local: usize) -> [f32; 3] {
    [local_x(local), BATCH_Y[batch], 0.0]
}

/// `true` iff this instance is the one the narrow framing must reject.
pub fn is_offscreen(local: usize) -> bool {
    local == OFFSCREEN_LOCAL_INDEX
}

/// The GLOBAL instance-ring index of batch `batch`'s local instance `local`.
///
/// The gather assigns `base_instance` by a prefix sum over mesh ids, and both meshes carry
/// [`BATCH_INSTANCES`] instances, so batch `b`'s base is `b * BATCH_INSTANCES`. This is the same
/// number `vb_raster.vs.hlsl` exports as `instance_id` (rung R2d-4 exports `global`), which is what
/// makes it the id the census's distinct-instance set is read against.
pub fn global_instance_id(batch: usize, local: usize) -> u32 {
    (batch * BATCH_INSTANCES + local) as u32
}

/// The material slot instance `(batch, local)` carries. Cycles through [`MATERIAL_COUNT`], so the
/// last instance repeats the first material.
pub fn material_index(batch: usize, local: usize) -> usize {
    (batch * BATCH_INSTANCES + local) % MATERIAL_COUNT
}

/// Radius of batch `batch`'s sphere. The two differ so the two registrations are unmistakably
/// distinct geometry rather than an accidental de-duplication.
pub fn batch_radius(batch: usize) -> f32 {
    [0.62f32, 0.70][batch]
}

/// `vb_mesh.rs`'s `uv_sphere`, copied for the reason that file copies it: a fixture scene keeps its
/// own mesh generation, so a later edit to a shared helper cannot silently re-shape it.
pub fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi;
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32;
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi);
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            let u = j as f32 / slices as f32;
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

/// Batch `batch`'s mesh geometry.
pub fn batch_mesh(batch: usize) -> (Vec<Vertex>, Vec<u32>) {
    let (stacks, slices) = [(28u32, 40u32), (24, 32)][batch];
    uv_sphere(batch_radius(batch), stacks, slices, [0.7, 0.7, 0.72, 1.0])
}

/// The axis-aligned LOCAL bounds of a vertex list — the host mirror of the `MeshLocalBounds` row
/// the geometry table stores for the same mesh.
pub fn local_bounds(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v.position[k]);
            hi[k] = hi[k].max(v.position[k]);
        }
    }
    (lo, hi)
}

/// The fixture's instance ring, in RING order: batch-major, spawn order within a batch.
///
/// Packed by the PRODUCTION packer (`InstanceModelCol::from_global`) off the same `Transform`
/// [`spawn_fixture`] spawns, so a CPU-side prediction and the GPU's own row are the same bytes.
pub fn instance_rows() -> Vec<InstanceModelCol> {
    let mut rows = Vec::with_capacity(INSTANCE_COUNT);
    for batch in 0..BATCH_COUNT {
        for local in 0..BATCH_INSTANCES {
            let p = instance_position(batch, local);
            let transform = Transform {
                translation: Vec3::new(p[0], p[1], p[2]),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            };
            rows.push(InstanceModelCol::from_global(&GlobalTransform(transform.to_affine())));
        }
    }
    rows
}

/// The six PRODUCTION frustum planes for `framing` at `width × height`.
///
/// Every step is the engine's own — `look_at_rh` → `Transform::to_affine` →
/// `ViewUniform::from_camera` → `forward_gbuffer_push_from_view` → `frustum_planes_from_push_bytes`
/// — which is the route `vg_cull_granularity_census.rs` documents in full and the same one the
/// armed cull's host half takes. Nothing is re-derived here: hand-building a matrix would risk an
/// OpenGL-style near plane against this engine's reverse-Z, which silently rejects geometry IN
/// FRONT of the camera.
pub fn frustum_planes(framing: &Framing, width: u32, height: u32) -> [Plane; FRUSTUM_PLANE_COUNT] {
    let pose = Affine3A::look_at_rh(
        Vec3::new(framing.eye[0], framing.eye[1], framing.eye[2]),
        Vec3::new(framing.target[0], framing.target[1], framing.target[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    let transform = Transform {
        translation: pose.translation,
        rotation: Quat::from_mat3(pose.matrix3),
        scale: Vec3::ONE,
    };
    let view = ViewUniform::from_camera(transform.to_affine(), framing.projection());
    // `instanced = true`: this fixture always submits a non-empty batch list. It selects the VS arm
    // at push byte 84 and cannot touch bytes 0..64.
    let push = boyko_render::view::forward_gbuffer_push_from_view(&view, width, height, true);
    frustum_planes_from_push_bytes(
        push[0..64].try_into().expect("invariant: the raster push's leading 64 bytes are view_proj"),
    )
}

impl Framing {
    /// The `Projection` this framing spawns. `aspect: 1.0` matches [`EXTENT`] — and is inert
    /// either way, since the raster push derives aspect from the EXTENT, never from this field.
    pub fn projection(&self) -> Projection {
        Projection::Perspective {
            fov_y: self.fov_y_degrees * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: self.near,
            far: self.far,
        }
    }
}

/// Batch `batch`'s mesh-LOCAL AABB, folded from the geometry the fixture actually registers.
pub fn batch_local_bounds(batch: usize) -> ([f32; 3], [f32; 3]) {
    let (verts, _) = batch_mesh(batch);
    local_bounds(&verts)
}

/// Scales a local box's half-extent by `k` about its own centre. `k > 1` INFLATES it, which makes a
/// still-rejected instance rejected by a margin rather than marginally; `k == 0` collapses it to
/// its centre point, which makes a still-kept instance kept by its centre rather than by a corner.
pub fn scaled_bounds(local: ([f32; 3], [f32; 3]), k: f32) -> ([f32; 3], [f32; 3]) {
    let (mn, mx) = local;
    let mut lo = [0.0f32; 3];
    let mut hi = [0.0f32; 3];
    for a in 0..3 {
        let c = (mn[a] + mx[a]) * 0.5;
        let h = (mx[a] - mn[a]) * 0.5 * k;
        lo[a] = c - h;
        hi[a] = c + h;
    }
    (lo, hi)
}

/// The GLOBAL instance ids the PER-INSTANCE cull rejects at `framing`, via the shipped host oracle
/// [`boyko_render::frustum::instance_visible_after_cull`] over the fixture's own ring and bounds.
pub fn instance_rejections(framing: &Framing) -> Vec<u32> {
    let planes = frustum_planes(framing, EXTENT.0, EXTENT.1);
    let ring = instance_rows();
    let mut out = Vec::new();
    for batch in 0..BATCH_COUNT {
        let local_aabb = batch_local_bounds(batch);
        for local in 0..BATCH_INSTANCES {
            let g = global_instance_id(batch, local);
            if !boyko_render::frustum::instance_visible_after_cull(
                &planes,
                &ring[g as usize],
                local_aabb,
            ) {
                out.push(g);
            }
        }
    }
    out
}

/// The BATCH indices the per-BATCH (level-1) cull rejects at `framing` — the union AABB test the
/// shipped rung R2c already performs, evaluated by its own host oracle.
pub fn batch_rejections(framing: &Framing) -> Vec<usize> {
    let planes = frustum_planes(framing, EXTENT.0, EXTENT.1);
    let ring = instance_rows();
    (0..BATCH_COUNT)
        .filter(|&batch| {
            let draw = boyko_render::mesh_draw::DrawBatch {
                mesh_id: batch as u32,
                index_count: batch_mesh(batch).1.len() as u32,
                index_type: boyko_rhi::IndexType::Uint32,
                base_instance: (batch * BATCH_INSTANCES) as u32,
                instance_count: BATCH_INSTANCES as u32,
            };
            boyko_render::frustum::batch_instance_count_after_cull(
                &planes,
                &draw,
                &ring,
                Some(batch_local_bounds(batch)),
            ) == 0
        })
        .collect()
}

// ── The two load-bearing fixture properties, as COMPILE-TIME assertions ────────────────────────
//
// Const-asserted rather than checked in a test: they are relations between constants, so a
// violation is a build error at the edit that causes it rather than a red test somebody has to run.
// (`const` assertions carry no format arguments — formatting is not available in a const context —
// so each message is a plain sentence.)

/// Property 2: the culled instance is INTERIOR to its batch. At the last index the compacted
/// survivor list would equal the identity PREFIX, and then a stale read, a correct read and a
/// mis-exported compacted slot all render identically.
const _: () = assert!(
    OFFSCREEN_LOCAL_INDEX > 0 && OFFSCREEN_LOCAL_INDEX + 1 < BATCH_INSTANCES,
    "the culled instance must be INTERIOR to its batch: at local index 0 or at the last index the \
     compacted survivor list is the identity prefix and the gate cannot distinguish a stale read \
     from a compacted one"
);

/// Property 1: a second batch exists, and it starts past 0 — which is what makes the
/// `visible[base + id]` base term observable at all.
const _: () = assert!(
    BATCH_COUNT >= 2 && BATCH_INSTANCES > 0,
    "a second batch with base_instance > 0 is what makes the `base + id` term observable; batch \
     1's base is BATCH_INSTANCES"
);

/// One material is shared by two instances on purpose: nothing downstream may key on a
/// material-instance bijection.
const _: () = assert!(
    MATERIAL_COUNT < INSTANCE_COUNT,
    "a shared material is deliberate: with one material per instance, an id-mapping gate could be \
     satisfied by the material lane instead of the instance lane"
);

/// Checks the load-bearing fixture properties that are NOT relations between constants — the
/// GEOMETRY the constants above are supposed to produce.
///
/// Called from the non-ignored CPU tests of every consumer, so a "tidying" edit that keeps the
/// constants but moves the geometry reds in CI rather than at the next GPU run.
pub fn assert_fixture_invariants() {
    assert_eq!(
        local_x(OFFSCREEN_LOCAL_INDEX),
        OFFSCREEN_X,
        "the off-screen world X must sit at local index {OFFSCREEN_LOCAL_INDEX}; if it moved, the \
         culled instance is no longer the interior one the const assertion above protects"
    );
    for local in 0..BATCH_INSTANCES {
        if local == OFFSCREEN_LOCAL_INDEX {
            continue;
        }
        assert_eq!(
            local_x(local).abs(),
            ONSCREEN_X,
            "every other instance must sit at the on-screen magnitude"
        );
    }
    assert_ne!(
        global_instance_id(1, 0),
        0,
        "the second batch's base must be nonzero -- with both bases at 0, `visible[base + id]` and \
         `visible[id]` are the same expression"
    );
}

/// Spawns the fixture: two meshes, [`BATCH_INSTANCES`] instances each, the lights, and `framing`'s
/// camera. Called from each consumer's own startup system.
pub fn spawn_fixture(
    commands: &mut Commands,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    geo_table: &mut MeshGeometryTableSlot,
    dev: &GpuDevice,
    framing: &Framing,
) {
    let mats: Vec<u16> = (0..MATERIAL_COUNT)
        .map(|i| {
            let t = i as f32 / MATERIAL_COUNT as f32;
            materials
                .add(Material::new([0.72 - 0.5 * t, 0.10 + 0.6 * t, 0.30, 1.0], t, 0.38, 0.5, [0.0; 3], 0))
                .index() as u16
        })
        .collect();

    let handles: Vec<MeshHandle> = (0..BATCH_COUNT)
        .map(|b| {
            let (verts, idx) = batch_mesh(b);
            match geo_table.0.as_mut() {
                Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
                None => meshes.register_mesh(dev.get(), &verts, &idx),
            }
        })
        .collect();

    // Spawn BATCH-MAJOR and, within a batch, in local-index order. The gather buckets by `mesh_id`
    // and scatters with a per-mesh cursor over the query's iteration order, so this order is the
    // ring order — which is what `global_instance_id` states and what the id-mapping gate reads.
    for batch in 0..BATCH_COUNT {
        for local in 0..BATCH_INSTANCES {
            let p = instance_position(batch, local);
            let e = commands
                .spawn(MeshBundle::new(
                    handles[batch],
                    Transform::from_translation(Vec3::new(p[0], p[1], p[2])),
                ))
                .id();
            commands.entity(e).insert(MaterialHandle(mats[material_index(batch, local)]));
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
        Vec3::new(framing.eye[0], framing.eye[1], framing.eye[2]),
        Vec3::new(framing.target[0], framing.target[1], framing.target[2]),
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
        projection: framing.projection(),
    });
}

/// The startup-system shape every consumer binary wraps — takes the engine's own system params so a
/// consumer's `fn setup(..)` is one call.
pub fn fixture_setup_system(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
    framing: &Framing,
) {
    spawn_fixture(&mut commands, &mut meshes, &mut materials, &mut geo_table, &dev, framing);
}

// ===============================================================================================
// The probe line
// ===============================================================================================

/// One parsed `VB_CULL_READBACK` line (`boyko_app::vb_cull_probe::format_vb_cull_probe_line`).
#[derive(Debug, Clone)]
pub struct CullProbe {
    /// `batches=` — live `DrawBatch` records the host submitted.
    pub batches: usize,
    /// `visible=` — the GPU counter: batches that passed level 1 AND carry ≥ 1 survivor.
    pub visible: u32,
    /// `frame=` — the ENGINE frame the capture came from (VG R3 piece 3 step P3-5). The probe
    /// settles 30 presented frames before capturing, so a small value here means the run captured
    /// an unconverged frame.
    pub frame: u32,
    /// `gpu_frame=` — the frame index the CULL read out of `VbCullUniform`, taken from
    /// `vb_late_count`'s reserved tail slot.
    ///
    /// ⚠️ The shader writes it only under `VB_CULL_OCC_ARMED`, which the host pushes as `0` until
    /// the P3-6 arming commit. Before that it reads the staging's boot prefill, so equality with
    /// [`Self::frame`] (plan D6's control F-M4a) is NOT assertable yet.
    pub gpu_frame: u32,
    /// `list=[..]` — the compacted visible-BATCH indices.
    pub list: Vec<u32>,
    /// `inst=[..]` — post-cull `instanceCount` per drawn batch, in batch order.
    pub inst: Vec<u32>,
    /// `vis=[base:members|..]` — each batch's OWN region of the per-instance survivor list.
    pub vis: Vec<(u32, Vec<u32>)>,
    /// `late_cnt_pre=[..]` — `n_defer` per drawn batch, as the EARLY cull phase wrote it.
    pub late_cnt_pre: Vec<u32>,
    /// `late_cnt_post=[..]` — the same words re-read AFTER the late cull. A difference from
    /// [`Self::late_cnt_pre`] is a clobber (plan A5's first clause).
    pub late_cnt_post: Vec<u32>,
    /// `late_ic=[..]` — `n_keep` per drawn batch: word 1 of each late draw record, whose only
    /// producer is the late cull.
    pub late_ic: Vec<u32>,
    /// `late_cand=[base:members|..]` — each batch's CANDIDATE region, sized by
    /// [`Self::late_cnt_pre`]. Observable only in the PRE-late snapshot: the late phase compacts
    /// the same allocation in place.
    pub late_cand: Vec<(u32, Vec<u32>)>,
    /// `late_surv=[base:members|..]` — each batch's compacted SURVIVOR prefix, sized by
    /// [`Self::late_ic`]. Deliberately not sized by `late_cnt_pre`: what follows the prefix inside
    /// the region is the untouched candidate tail, never a survivor.
    pub late_surv: Vec<(u32, Vec<u32>)>,
    /// The line as written, for assertion messages.
    pub raw: String,
}

impl CullProbe {
    /// Instances the frame actually draws — the sum of the per-record `instanceCount` words, which
    /// is exactly what `vkCmdDrawIndexedIndirect` fetches.
    pub fn drawn_instances(&self) -> u32 {
        self.inst.iter().sum()
    }

    /// `true` iff every batch's survivor region is the IDENTITY run `base, base+1, …` of its own
    /// recorded length.
    ///
    /// That was every region's shape while rung R2d-5's `keep` was hardwired. Since the rung R2d-6
    /// arming it is the shape of a batch with NOTHING rejected — compaction with no rejections is
    /// the identity — so `vb_inst_cull_wide.rs` (the control) asserts it and
    /// `vb_inst_cull_narrow.rs` (the gate, whose culled instance is INTERIOR to its batch) asserts
    /// its negation.
    pub fn regions_are_identity(&self) -> bool {
        self.vis
            .iter()
            .all(|(base, members)| members.iter().enumerate().all(|(i, m)| *m == base + i as u32))
    }
}

fn bracketed(token: &str) -> &str {
    token.trim_start_matches('[').trim_end_matches(']')
}

fn parse_u32_list(body: &str) -> Vec<u32> {
    body.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or_else(|_| panic!("`{s}` is not a probe list entry")))
        .collect()
}

/// Parses a pipe-separated `base:members` group list.
///
/// Each group carries its OWN base rather than being positioned by one, because the per-batch
/// regions need not be contiguous — the frame loop skips batches whose mesh is not `Loaded`, so the
/// bases can leave gaps. This function is therefore the only place that has to know the shape, and
/// the three grouped fields (`vis`, `late_cand`, `late_surv`) all go through it.
fn parse_groups(key: &str, body: &str) -> Vec<(u32, Vec<u32>)> {
    bracketed(body)
        .split('|')
        .filter(|g| !g.is_empty())
        .map(|g| {
            let (base, members) = g
                .split_once(':')
                .unwrap_or_else(|| panic!("a `{key}` group is `base:members` -- got {g:?}"));
            let base: u32 = base
                .parse()
                .unwrap_or_else(|_| panic!("a `{key}` group base is an integer -- got {base:?}"));
            (base, parse_u32_list(members))
        })
        .collect()
}

/// Parses one `VB_CULL_READBACK …` line.
///
/// Key-driven rather than positional: the line's fields are `key=value` with no interior spaces, so
/// a field added later cannot shift the ones already read. VG R3 piece 3 step P3-5 added seven
/// fields on exactly that promise.
///
/// ⚠️ Every `field()` below PANICS on a missing key, and that is deliberate: a probe line without
/// `late_cand=` means the emitter and this reader disagree about the format, and a default would
/// turn that into a green gate over an empty list.
pub fn parse_probe_line(line: &str) -> CullProbe {
    let field = |key: &str| -> String {
        line.split_whitespace()
            .find_map(|t| t.strip_prefix(key))
            .unwrap_or_else(|| {
                panic!("the probe line carries no `{key}` field -- got {line:?}")
            })
            .to_string()
    };
    // Each body is bound before use: `bracketed` borrows its argument, and a `&field("..")`
    // temporary inside the struct literal below would make the borrow's extent a question nobody
    // should have to answer while reading a parser.
    let vis_body = field("vis=");
    let list_body = field("list=");
    let inst_body = field("inst=");
    let cnt_pre_body = field("late_cnt_pre=");
    let cnt_post_body = field("late_cnt_post=");
    let late_ic_body = field("late_ic=");
    let cand_body = field("late_cand=");
    let surv_body = field("late_surv=");
    CullProbe {
        batches: field("batches=").parse().expect("`batches=` is an integer"),
        visible: field("visible=").parse().expect("`visible=` is an integer"),
        frame: field("frame=").parse().expect("`frame=` is an integer"),
        gpu_frame: field("gpu_frame=").parse().expect("`gpu_frame=` is an integer"),
        list: parse_u32_list(bracketed(&list_body)),
        inst: parse_u32_list(bracketed(&inst_body)),
        vis: parse_groups("vis", &vis_body),
        late_cnt_pre: parse_u32_list(bracketed(&cnt_pre_body)),
        late_cnt_post: parse_u32_list(bracketed(&cnt_post_body)),
        late_ic: parse_u32_list(bracketed(&late_ic_body)),
        late_cand: parse_groups("late_cand", &cand_body),
        late_surv: parse_groups("late_surv", &surv_body),
        raw: line.to_string(),
    }
}

/// Boots THIS process with the cull-readback probe armed and returns the parsed line.
///
/// The probe path (`BOYKO_VB_CULL_READBACK=<path>`) makes the runner settle 30 presented frames,
/// copy the cull's DEVICE-LOCAL outputs into host-visible staging on ONE frame, drain 3 more so that
/// frame's slot fence has been re-waited, write one line and stop (VG R3 piece 3 step P3-5; before
/// it, the capture was the FIRST presented frame and the run `return`ed from inside the readback
/// branch). Nothing is relocated for the probe — what is read is a transfer copy of the buffers
/// exactly as they ship, so this observes the cull in the configuration that renders.
///
/// # Panics
///
/// Panics if the run ends without writing the file: a run that renders and produces nothing is an
/// instrument failure, not an empty scene.
pub fn probe_in_process(tag: &str, boot: impl FnOnce()) -> CullProbe {
    let out = std::env::temp_dir().join(format!("boyko_vb_inst_cull_{tag}.txt"));
    let _ = std::fs::remove_file(&out);
    // SAFETY: single-threaded test setup, before any engine thread exists. Windowed tests in this
    // crate run with `--test-threads=1` by convention, so no sibling test observes this write.
    unsafe {
        std::env::set_var("BOYKO_VB_CULL_READBACK", &out);
    }
    boot();
    let line = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "the cull-readback probe wrote no file at {} ({e}). The run ended without reaching the \
             readback, so nothing here is evidence about the cull.",
            out.display()
        )
    });
    parse_probe_line(line.trim())
}

/// Spawns one `#[ignore]`d worker test in a SEPARATE process with the probe armed, and returns its
/// parsed line.
///
/// One process per reading, for the reason `vg_thresholds::run_worker` gives: a windowed boot owns
/// the device singleton and the window, so two framings (or two camera paths) cannot both be
/// rendered by one process.
pub fn run_cull_probe_worker(worker: &str, tag: &str, env: &[(&str, String)]) -> CullProbe {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let out = std::env::temp_dir().join(format!("boyko_vb_inst_cull_{tag}.txt"));
    let _ = std::fs::remove_file(&out);

    let mut cmd = Command::new(&exe);
    cmd.args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env("BOYKO_VB_CULL_READBACK", &out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // The cull readback is the only capture THIS worker is for, and an inherited `BOYKO_HOST_DUMP`
        // or `BOYKO_VG_CENSUS` would render the same frames for no reason while holding the loop
        // open until it too completed.
        //
        // ⚠️ The reason has CHANGED, and the old one is retired here rather than left standing.
        // Until VG R3 piece 3 step P3-5 this comment said a second armed capture "would silently
        // produce no file", because the readback `return`ed out of the frame loop from inside its own
        // branch on the first presented frame. It no longer does: all five drivers now exit through
        // ONE conjunction, so a co-armed capture completes. `BOYKO_HZB_DUMP` is deliberately NOT
        // removed — `vb_cull_hzb_pairing.rs` arms it BESIDE this variable in one process, which is
        // the whole point of that gate.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("invariant: the worker process spawns");
    assert!(status.success(), "cull-probe worker `{tag}` exited {status}");

    let line = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "cull-probe worker `{tag}` wrote no line at {} ({e}). A worker that renders and \
             produces nothing is an instrument failure, not an empty scene.",
            out.display()
        )
    });
    parse_probe_line(line.trim())
}
