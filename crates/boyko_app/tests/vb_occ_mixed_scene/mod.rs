//! **VG R3 piece 3 step P3-8 — `vb_occ_mixed`: the fixture that actually occludes.**
//!
//! One scene definition, four consumers: `vb_mesh.rs` (which renders the four golden pins behind
//! `BOYKO_VG_SCENE=mixed`), `vb_occ_mixed.rs` (gates G-P3-A/B/C), `vg_occ_verdict_census.rs` (the
//! analytic fixture precondition) and `hzb_engine_pyramid_gate.rs` (G-P3-E's FORCE-LATE leg). Two
//! copies of a fixture are two texts that can disagree, and a gate that disagrees with the scene it
//! adjudicates proves nothing.
//!
//! # Why this scene exists at all
//!
//! Every VB fixture in the tree stands its geometry side by side against the sky, so nothing lies
//! behind anything and the conservative min-over-footprint test rejects **zero** instances. On such
//! a scene "the cull decided correctly" and "the cull decided nothing" are the same pixels and the
//! same counts. `vb_occ_mixed` is the first fixture in which `Σ n_defer > 0` is reachable, which is
//! what makes the plan's non-vacuity clause an assertion rather than a hope.
//!
//! # The shape, and the four roles
//!
//! **TWO registered meshes** — batches bucket per `MeshHandle` (`boyko_render::mesh_draw`), so
//! `draw_batches >= 2` is delivered by registration or by nothing — with FOUR instances each:
//!
//! | role | count | marked | what it is for |
//! |---|---|---|---|
//! | [`Role::Occluder`] | 1 (cube) | no | the near slab; its silhouette is what hides the others |
//! | [`Role::Filler`] | 1 (sphere) | no | populates the EARLY depth under FORCE-LATE, where every marked instance is deferred and the early scope would otherwise draw an empty depth — which trips `hzb_engine_pyramid_gate.rs`'s SHIPPED non-vacuity clauses |
//! | [`Role::Hidden`] | 4 (2 + 2) | yes | wholly inside the slab's silhouette AND inside one 128-aligned block, so the pyramid test must REJECT them |
//! | [`Role::Visible`] | 2 (1 + 1) | yes | rects disjoint from the slab's, so the pyramid test must KEEP them |
//!
//! # ⚠️ FIXTURE PRECONDITION VG-P3-MIXED-OCCLUDES — "behind the silhouette" is necessary, NOT sufficient
//!
//! [`boyko_render::hzb::select_texels`] returns the **ALIGNED** expansion
//! (`containing_texel(t, level) = t >> level`) and [`boyko_render::hzb::occluder_depth`] folds all
//! four texels with a conservative `min`. With the reverse-Z clear at `0.0`, **one background texel
//! anywhere in the footprint forces KEEP** — and at 512² a rect that merely straddles `x = 256`
//! selects level 8, whose footprint is the whole image. "Wholly behind the slab" would therefore be
//! satisfied by a fixture that is one transform away from structurally-cannot-defer.
//!
//! The rule that makes it hold: **each hidden instance's projected rect lies wholly inside ONE
//! `2^(L+1)`-aligned block of width `2^(L+1)` that is itself wholly inside the slab's rect.** Then
//! `msb(tx0 ^ tx1) <= L`, the 2×2 footprint IS that block, and every texel in it belongs to the
//! slab. The four blocks are [`HIDDEN_BLOCKS`] and `L` is [`MIXED_MAX_LEVEL`].
//!
//! Both halves are ASSERTED — analytically in `vg_occ_verdict_census.rs` (no GPU) and against the
//! DUMPED pyramid in `vb_occ_mixed.rs` (clause 0 of G-P3-B) — and both red with the word FIXTURE, so
//! a fixture error can never be read as a cull defect.
//!
//! # ⚠️ THE PLACEMENTS ARE IN SCREEN/VIEW SPACE, NOT WORLD SPACE
//!
//! Every instance is authored as `(pixel centre, view distance, scale)` and converted to world by
//! [`world_position`] through the engine's OWN `Affine3A::look_at_rh` basis. The alternative —
//! committing eight world triples — would hide the design intent behind numbers nobody can check,
//! and would silently stop meaning what it meant the moment the camera moved. Here the camera IS the
//! conversion, so a camera edit moves the geometry with it and the census reds by name if the
//! resulting rects stop satisfying the precondition.
//!
//! # ⚠️ THE RING ORDER, and the ONE property that depends on it
//!
//! Marking a strict subset splits each mesh family into two archetypes, and the gather scatters with
//! a per-mesh cursor over the query's iteration order — so the ABSOLUTE ring index of any one
//! instance depends on which archetype the query yields first, which this file deliberately does not
//! predict. What it DOES fix is the RELATIVE order of the marked instances: all six share ONE
//! archetype (`MeshBundle` + `MaterialHandle` + `OcclusionCulling`), so within a mesh they appear in
//! SPAWN order, and [`spawn_mixed`] spawns them `Hidden, Hidden, Visible`. Hence
//! **`MARKED_ROLES_IN_SPAWN_ORDER`**: candidate `j` of batch `b` is that batch's `j`-th marked
//! instance, and the oracle's per-candidate AABB follows. Nothing else in this fixture depends on
//! the ring order, and the counts, the set relations and the ascending-uniqueness clause are all
//! order-INDEPENDENT by construction.

#![allow(dead_code)]

use boyko_app::prelude::*;
use boyko_render::csm_caster::arvo_transform;
use boyko_render::frustum::{FRUSTUM_PLANE_COUNT, Plane, frustum_planes_from_push_bytes};
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::mesh::Vertex;
use boyko_render::{Material, MeshAssetsVbExt, MeshGeometryTableSlot, OcclusionCulling, generate_tangents};
use boyko_scene::ViewUniform;

/// The env knob that selects this scene SHAPE inside `vb_mesh.rs` (plan D9). ORTHOGONAL to
/// `BOYKO_VG_OCC`, which keeps its shipped `== "1"` marker predicate — that orthogonality is what
/// makes the `vb_occ_mixed_off` baseline producible at all (round 2's table folded the two into one
/// variable and the four-pin equality was then unsatisfiable).
pub const ENV_SCENE: &str = "BOYKO_VG_SCENE";

/// The one value [`ENV_SCENE`] recognises. Any other value — including a plausible-looking
/// `"MIXED"` — selects nothing, exactly as `BOYKO_VG_OCC`'s `== "1"` does.
pub const SCENE_MIXED: &str = "mixed";

/// `true` iff this process was asked for the mixed scene.
#[must_use]
pub fn scene_is_mixed() -> bool {
    std::env::var(ENV_SCENE).is_ok_and(|v| v == SCENE_MIXED)
}

/// The window client extent every pin and every gate on this fixture renders at — `[vb_mesh]`'s own
/// 512², so `prev_pow2(512) == 512`, level 0 of the pyramid IS the pixel grid, and
/// `HzbAxis::texel_of` is the identity. Every pixel number in this file is stated against that.
pub const EXTENT: u32 = 512;

/// The camera eye — `vb_mesh.rs`'s, verbatim.
pub const EYE: [f32; 3] = [0.0, 1.1, 7.8];
/// The camera target — `vb_mesh.rs`'s, verbatim.
pub const TARGET: [f32; 3] = [0.0, 0.55, 0.0];
/// Vertical field of view in radians — `vb_mesh.rs`'s 52°.
pub const FOV_Y: f32 = 52.0 * core::f32::consts::PI / 180.0;
/// Near plane — `vb_mesh.rs`'s.
pub const NEAR: f32 = 0.1;
/// Far plane — `vb_mesh.rs`'s.
pub const FAR: f32 = 100.0;

/// The sun direction TO the light (byte-identical to `vb_mesh.rs`'s).
pub const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// The coarsest pyramid level any HIDDEN instance may select.
///
/// The four blocks below are 128 = `2^7` wide and 128-ALIGNED, so two level-0 texels inside one of
/// them agree on every bit above 6 and `msb(tx0 ^ tx1) <= 6`. A selection above this means the rect
/// straddled a block boundary, the aligned 2×2 footprint spilled outside the slab, and the
/// conservative `min` folded a background texel — after which the instance CANNOT be deferred and
/// every count clause below it would red on a correct engine.
pub const MIXED_MAX_LEVEL: u32 = 6;

/// The 128-aligned, 128-wide pixel blocks the four hidden instances live inside, one each. All four
/// are strictly inside the slab's projected rect.
pub const HIDDEN_BLOCKS: [[u32; 2]; 4] = [[128, 128], [256, 128], [128, 256], [256, 256]];

/// The side of a block in [`HIDDEN_BLOCKS`], `2^(MIXED_MAX_LEVEL + 1)`.
pub const HIDDEN_BLOCK_SIDE: u32 = 1 << (MIXED_MAX_LEVEL + 1);

const _: () = assert!(
    HIDDEN_BLOCK_SIDE == 128,
    "the block side and MIXED_MAX_LEVEL are two spellings of one number; a block wider than \
     2^(L+1) admits a rect whose aligned footprint escapes it"
);

/// The pixel rect (inclusive) the four hidden blocks together occupy — the region the slab's own
/// rect must CONTAIN for the precondition to hold.
pub const HIDDEN_BLOCK_UNION: [u32; 4] = [128, 128, 383, 383];

/// The rect the plan states the slab covers, `[64, 448)²`, as an inclusive pixel box. The measured
/// slab rect must CONTAIN this; containment (rather than equality) is the assertion because slack in
/// this direction can only make the precondition safer, and pinning four exact pixel bounds would
/// red on a driver-independent rounding change that costs the fixture nothing.
pub const SLAB_COVERS_AT_LEAST: [u32; 4] = [64, 64, 447, 447];

// ===============================================================================================
// The instance table
// ===============================================================================================

/// Which registered mesh an instance belongs to. The discriminant IS the registration order, and
/// therefore the geometry-table slot order, and therefore the `DrawBatch` order — batches are
/// emitted in `mesh_id` order by the gather's prefix sum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MixedMesh {
    /// The UV sphere — `vb_mesh.rs`'s own `uv_sphere(0.62, 28, 40)`, registered FIRST.
    Sphere = 0,
    /// The unit cube, registered SECOND. A second `MeshHandle` is the ONLY thing that can produce a
    /// second `DrawBatch`; nothing about instance counts can.
    Cube = 1,
}

/// What an instance is for. The role decides the marking AND the verdict the oracle must produce,
/// so a gate never has to restate either.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// UNMARKED, near, large: the occluding slab.
    Occluder,
    /// UNMARKED, far, outside the slab's rect: the instance that keeps the EARLY depth non-empty
    /// under FORCE-LATE.
    Filler,
    /// MARKED, wholly behind the slab and inside one [`HIDDEN_BLOCKS`] block — the oracle must
    /// REJECT it.
    Hidden,
    /// MARKED, rect disjoint from the slab's — the oracle must KEEP it with
    /// `KeepReason::NotOccluded`.
    Visible,
}

impl Role {
    /// `true` iff [`spawn_mixed`] puts `OcclusionCulling` in this instance's flush.
    #[must_use]
    pub const fn is_marked(self) -> bool {
        matches!(self, Role::Hidden | Role::Visible)
    }
}

/// One instance of the mixed scene, authored in SCREEN/VIEW space (see the module header).
#[derive(Clone, Copy, Debug)]
pub struct MixedInstance {
    /// A short name, used verbatim in every failure message so a red names the instance rather than
    /// an index.
    pub name: &'static str,
    pub mesh: MixedMesh,
    pub role: Role,
    /// The pixel this instance's CENTRE projects to at [`EXTENT`]².
    pub pixel: [f32; 2],
    /// View-space distance in front of the eye (`clip.w`, before the divide).
    pub view_distance: f32,
    /// Per-axis `Transform` scale. Uniform everywhere except the slab, which is a slab because its
    /// Z scale is small.
    pub scale: [f32; 3],
}

/// **The eight instances, in SPAWN order.**
///
/// Grouped by mesh, and within a mesh: the unmarked one first, then `Hidden, Hidden, Visible`. That
/// last ordering is load-bearing — see the module header's ring-order section — and
/// [`assert_fixture_invariants`] checks it rather than trusting this array's layout.
pub const MIXED_INSTANCES: [MixedInstance; 8] = [
    // ---- mesh A: the sphere ------------------------------------------------------------------
    MixedInstance {
        name: "A0/filler",
        mesh: MixedMesh::Sphere,
        role: Role::Filler,
        pixel: [24.0, 487.0],
        view_distance: 16.0,
        scale: [0.45; 3],
    },
    MixedInstance {
        name: "A1/hidden",
        mesh: MixedMesh::Sphere,
        role: Role::Hidden,
        pixel: [192.0, 192.0],
        view_distance: 9.0,
        scale: [0.55; 3],
    },
    MixedInstance {
        name: "A2/hidden",
        mesh: MixedMesh::Sphere,
        role: Role::Hidden,
        pixel: [320.0, 192.0],
        view_distance: 9.0,
        scale: [0.55; 3],
    },
    MixedInstance {
        name: "A3/visible",
        mesh: MixedMesh::Sphere,
        role: Role::Visible,
        pixel: [24.0, 24.0],
        view_distance: 12.0,
        scale: [0.35; 3],
    },
    // ---- mesh B: the cube --------------------------------------------------------------------
    MixedInstance {
        name: "B0/occluder",
        mesh: MixedMesh::Cube,
        role: Role::Occluder,
        pixel: [256.0, 256.0],
        view_distance: 4.0,
        // A SLAB: wide and tall enough for its rect to contain `[64, 448)²`, thin in Z so its front
        // face is close to a constant-depth plane and the conservative `min` over any 128-block
        // stays far above every hidden instance's `depth_near`.
        scale: [3.05, 3.05, 0.30],
    },
    MixedInstance {
        name: "B1/hidden",
        mesh: MixedMesh::Cube,
        role: Role::Hidden,
        pixel: [192.0, 320.0],
        view_distance: 9.0,
        scale: [0.68; 3],
    },
    MixedInstance {
        name: "B2/hidden",
        mesh: MixedMesh::Cube,
        role: Role::Hidden,
        pixel: [320.0, 320.0],
        view_distance: 9.0,
        scale: [0.68; 3],
    },
    MixedInstance {
        name: "B3/visible",
        mesh: MixedMesh::Cube,
        role: Role::Visible,
        pixel: [487.0, 24.0],
        view_distance: 12.0,
        scale: [0.42; 3],
    },
];

/// Instances per mesh — and therefore per `DrawBatch`.
pub const INSTANCES_PER_MESH: usize = 4;
/// Registered meshes, hence `DrawBatch`es. Asserted from the probe's `[host]` table, never assumed.
pub const BATCH_COUNT: usize = 2;
/// MARKED instances per mesh: two [`Role::Hidden`] plus one [`Role::Visible`].
pub const MARKED_PER_MESH: usize = 3;
/// Marked instances in the whole scene — the number `host.occlusion_instances` must report.
pub const MARKED_TOTAL: usize = BATCH_COUNT * MARKED_PER_MESH;
/// HIDDEN instances in the whole scene — `Σ n_defer` on the UNFORCED, converged pin.
pub const HIDDEN_TOTAL: usize = 4;
/// VISIBLE marked instances — `Σ n_keep` under FORCE-LATE.
pub const VISIBLE_MARKED_TOTAL: usize = 2;

const _: () = assert!(MIXED_INSTANCES.len() == BATCH_COUNT * INSTANCES_PER_MESH);
const _: () = assert!(
    HIDDEN_TOTAL + VISIBLE_MARKED_TOTAL == MARKED_TOTAL,
    "every marked instance is either Hidden or Visible; a third marked role would have no stated \
     oracle verdict and clause 7's two-sided bound would stop being derivable"
);
const _: () = assert!(
    VISIBLE_MARKED_TOTAL > 0 && VISIBLE_MARKED_TOTAL < MARKED_TOTAL,
    "G-P3-B clause 7 asserts `0 < S|K_b| < S n_defer` under FORCE-LATE; that is only a real claim \
     when the late test both KEEPS something and REJECTS something"
);

/// The roles of one mesh's MARKED instances, in the order [`spawn_mixed`] spawns them — which is
/// the order they occupy in the ring, and therefore in the candidate list.
///
/// The oracle's per-candidate AABB is read through this array and through nothing else.
pub const MARKED_ROLES_IN_SPAWN_ORDER: [Role; MARKED_PER_MESH] =
    [Role::Hidden, Role::Hidden, Role::Visible];

/// **The TWO ring layouts this fixture admits, and the reason it admits exactly two.**
///
/// Each row maps a batch-local RING SLOT to the SPAWN position (within that mesh) of the instance
/// occupying it. Marking a strict subset puts each mesh family in two archetypes — the six marked
/// entities in one, the two unmarked in the other — and the gather scatters with a per-mesh cursor
/// over the query's iteration order, so the only free variable is WHICH archetype the query yields
/// first. Within an archetype the rows are in spawn order, and this fixture spawns each mesh's four
/// instances as `unmarked, Hidden, Hidden, Visible`:
///
/// * `[0, 1, 2, 3]` — the UNMARKED archetype is yielded first: ring `[U, H, H, V]`.
/// * `[1, 2, 3, 0]` — the MARKED archetype is yielded first: ring `[H, H, V, U]`.
///
/// ⚠️ **The layout is DERIVED from the observed candidate set and then checked, never assumed.** The
/// two rows produce DISJOINT candidate-offset sets in both regimes — `{1,2}` vs `{0,1}` unforced,
/// `{1,2,3}` vs `{0,1,2}` under FORCE-LATE — so a gate can identify which one the engine produced
/// and red by name if it produced neither. Predicting one of them would have made a kernel
/// iteration-order change read as a cull defect; enumerating both makes it read as itself.
pub const RING_SLOT_TO_SPAWN: [[usize; INSTANCES_PER_MESH]; 2] = [[0, 1, 2, 3], [1, 2, 3, 0]];

/// The [`MIXED_INSTANCES`] index occupying batch-local ring slot `slot` of `mesh`, under ring
/// layout `layout` (an index into [`RING_SLOT_TO_SPAWN`]).
pub fn ring_slot_instance(mesh: MixedMesh, layout: usize, slot: usize) -> usize {
    instances_of(mesh)[RING_SLOT_TO_SPAWN[layout][slot]]
}

/// The batch-local ring slots whose instance has `role`, under ring layout `layout`, ASCENDING.
///
/// This is the shape a candidate list must have: the GPU compacts in ring order, so a candidate list
/// is always an ascending run of slots.
pub fn slots_with_role(mesh: MixedMesh, layout: usize, roles: &[Role]) -> Vec<usize> {
    (0..INSTANCES_PER_MESH)
        .filter(|&s| roles.contains(&MIXED_INSTANCES[ring_slot_instance(mesh, layout, s)].role))
        .collect()
}

/// The six PRODUCTION frustum planes for this fixture at [`EXTENT`]².
///
/// Every step is the engine's own — `look_at_rh` → `Transform::to_affine` →
/// `ViewUniform::from_camera` → `forward_gbuffer_push_from_view` → `frustum_planes_from_push_bytes`
/// — the route `vg_cull_granularity_census.rs` documents in full and the same one the armed cull's
/// host half takes. Nothing is re-derived: a hand-built matrix risks an OpenGL-style near plane
/// against this engine's reverse-Z, which silently rejects geometry IN FRONT of the camera.
pub fn frustum_planes() -> [Plane; FRUSTUM_PLANE_COUNT] {
    let view = ViewUniform::from_camera(camera_transform().to_affine(), camera_projection());
    // `instanced = true`: this fixture always submits a non-empty batch list. It selects the VS arm
    // at push byte 84 and cannot touch bytes 0..64.
    let push = boyko_render::view::forward_gbuffer_push_from_view(&view, EXTENT, EXTENT, true);
    frustum_planes_from_push_bytes(
        push[0..64].try_into().expect("invariant: the raster push's leading 64 bytes are view_proj"),
    )
}

/// The number of this fixture's instances the FRUSTUM cull keeps — the count G-P3-B clause 2
/// compares `k + n_defer` against. MEASURED through the shipped host oracle rather than asserted to
/// be eight: a clause whose right-hand side is a literal cannot see a fixture that drifted off
/// screen.
pub fn frustum_survivors_of(mesh: MixedMesh) -> usize {
    let planes = frustum_planes();
    instances_of(mesh)
        .into_iter()
        .filter(|&i| {
            let row = InstanceModelCol::from_global(&GlobalTransform(instance_transform(i).to_affine()));
            boyko_render::frustum::instance_visible_after_cull(
                &planes,
                &row,
                mesh_local_bounds(MIXED_INSTANCES[i].mesh),
            )
        })
        .count()
}

// ===============================================================================================
// Geometry
// ===============================================================================================

/// `vb_mesh.rs`'s `uv_sphere`, copied for the reason that file copies it: a fixture whose geometry
/// is compared against a recorded measurement keeps its own mesh generation instead of moving under
/// it when a shared helper is edited for someone else.
///
/// Its local AABB is exactly `±radius` on every axis (every vertex is `unit_normal * radius`, and
/// the poles and the equator reach `±1` on each axis in turn), which is what the placement
/// arithmetic in this module assumes.
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

/// The sphere's radius — `vb_mesh.rs`'s own `0.62`.
pub const SPHERE_RADIUS: f32 = 0.62;
/// The cube's local half-extent: a UNIT cube, `±0.5`.
pub const CUBE_HALF: f32 = 0.5;

/// A unit cube spanning `[-0.5, 0.5]³`, with FLAT per-face normals (24 vertices, 36 indices).
///
/// Flat rather than shared-corner normals because a cube with averaged corner normals shades like a
/// sphere, and this mesh's job in the frame is to be an unmistakable opaque slab. Winding matches
/// [`uv_sphere`]'s (counter-clockwise seen from outside), so both meshes face the camera under the
/// same rasterizer state.
pub fn unit_cube(color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    // (normal, the four corners of that face in CCW order seen from outside)
    const FACES: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]]),
        ([0.0, 0.0, -1.0], [[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]]),
        ([1.0, 0.0, 0.0], [[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, 1.0, 0.0], [[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]]),
    ];
    const UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    let mut verts = Vec::with_capacity(24);
    let mut idx = Vec::with_capacity(36);
    for (f, (n, corners)) in FACES.iter().enumerate() {
        let base = (f * 4) as u32;
        for (c, p) in corners.iter().enumerate() {
            let mut v = Vertex::new(*p, *n, color);
            v.uv = UV[c];
            verts.push(v);
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// `mesh`'s geometry.
pub fn mesh_geometry(mesh: MixedMesh) -> (Vec<Vertex>, Vec<u32>) {
    match mesh {
        MixedMesh::Sphere => uv_sphere(SPHERE_RADIUS, 28, 40, [0.70, 0.70, 0.72, 1.0]),
        MixedMesh::Cube => unit_cube([0.55, 0.56, 0.60, 1.0]),
    }
}

/// `mesh`'s LOCAL-space AABB — the row the geometry table stores for it, computed here from the
/// analytic shape rather than folded from the vertex list.
///
/// Analytic on purpose: this number feeds the census's precondition, and folding it from the same
/// vertex array the fixture uploads would make the census agree with a mesh generator that had
/// drifted. [`assert_fixture_invariants`] folds the ACTUAL vertices and compares — so the two
/// derivations are cross-checked, in a test that needs no GPU.
pub fn mesh_local_bounds(mesh: MixedMesh) -> ([f32; 3], [f32; 3]) {
    let h = match mesh {
        MixedMesh::Sphere => SPHERE_RADIUS,
        MixedMesh::Cube => CUBE_HALF,
    };
    ([-h, -h, -h], [h, h, h])
}

/// The axis-aligned LOCAL bounds of a vertex list — the fold [`assert_fixture_invariants`] checks
/// [`mesh_local_bounds`] against.
pub fn fold_local_bounds(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
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

// ===============================================================================================
// Camera, and the screen-space → world conversion
// ===============================================================================================

/// The camera's world pose, through the engine's own `look_at_rh`.
pub fn camera_pose() -> Affine3A {
    Affine3A::look_at_rh(
        Vec3::new(EYE[0], EYE[1], EYE[2]),
        Vec3::new(TARGET[0], TARGET[1], TARGET[2]),
        Vec3::new(0.0, 1.0, 0.0),
    )
}

/// The camera's `Transform` — what [`spawn_mixed`] puts on the `CameraRig`.
pub fn camera_transform() -> Transform {
    let pose = camera_pose();
    Transform {
        translation: pose.translation,
        rotation: Quat::from_mat3(pose.matrix3),
        scale: Vec3::ONE,
    }
}

/// The camera's projection — `vb_mesh.rs`'s, verbatim.
pub fn camera_projection() -> Projection {
    Projection::Perspective { fov_y: FOV_Y, aspect: 1.0, near: NEAR, far: FAR }
}

/// **The view-projection in MATH-ROW form** — exactly what `boyko_render::hzb::project_aabb` takes,
/// built through the engine's own `ViewUniform::from_camera` → `forward_view_proj_rows` chain at
/// [`EXTENT`]².
///
/// UNJITTERED: the runner jitters only when TAA is armed, and no consumer of this fixture inserts an
/// `AaConfig`. Nothing is hand-built here — a matrix assembled by hand risks an OpenGL-style near
/// plane against this engine's reverse-Z, which silently rejects geometry IN FRONT of the camera.
pub fn view_proj_rows() -> [[f32; 4]; 4] {
    let view = ViewUniform::from_camera(camera_transform().to_affine(), camera_projection());
    boyko_render::forward_view_proj_rows(&view, EXTENT, EXTENT)
}

/// The world position whose CENTRE projects to `pixel` at view distance `view_distance`.
///
/// The inverse of the projection this fixture is measured under, spelled against the camera BASIS
/// rather than against a matrix inverse: for `P = eye + a·right + b·up − d·back`,
/// `clip.w = d`, `clip.x = a / (aspect · tan)` and `clip.y = −b / tan`, so
/// `a = x_ndc · aspect · tan · d` and `b = −y_ndc · tan · d`. Two multiplies, no inversion, and no
/// question about which convention the result is in.
pub fn world_position(pixel: [f32; 2], view_distance: f32) -> Vec3 {
    let pose = camera_pose();
    // `look_at_rh` stores the basis as the COLUMNS of `matrix3`: column 0 is `right`, column 1 the
    // true up, column 2 the BACK vector (the camera's `-Z` points at the target).
    let right = pose.matrix3.mul_vec(Vec3::new(1.0, 0.0, 0.0));
    let up = pose.matrix3.mul_vec(Vec3::new(0.0, 1.0, 0.0));
    let back = pose.matrix3.mul_vec(Vec3::new(0.0, 0.0, 1.0));

    let half = EXTENT as f32 * 0.5;
    let aspect = 1.0; // EXTENT / EXTENT — spelled so a non-square extent would have to be handled.
    let tan = (FOV_Y * 0.5).tan();
    let x_ndc = pixel[0] / half - 1.0;
    let y_ndc = pixel[1] / half - 1.0;

    let a = x_ndc * aspect * tan * view_distance;
    let b = -y_ndc * tan * view_distance;
    pose.translation + right * a + up * b - back * view_distance
}

/// Instance `i`'s `Transform` — translation from [`world_position`], no rotation, its own scale.
///
/// No rotation on purpose: a rotated box's Arvo fold is a strictly larger world AABB than the
/// authored one, so every pixel bound in this file would become an inequality nobody had checked.
pub fn instance_transform(i: usize) -> Transform {
    let inst = MIXED_INSTANCES[i];
    Transform {
        translation: world_position(inst.pixel, inst.view_distance),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(inst.scale[0], inst.scale[1], inst.scale[2]),
    }
}

/// Instance `i`'s WORLD AABB, through the engine's own `arvo_transform` over the PRODUCTION-packed
/// instance row — the same composition `boyko_render::frustum::instance_visible_after_cull` performs
/// and the same one `vb_batch_cull.comp.hlsl` mirrors.
pub fn instance_world_aabb(i: usize) -> ([f32; 3], [f32; 3]) {
    let inst = MIXED_INSTANCES[i];
    let (mn, mx) = mesh_local_bounds(inst.mesh);
    let lc = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
    let lh = [(mx[0] - mn[0]) * 0.5, (mx[1] - mn[1]) * 0.5, (mx[2] - mn[2]) * 0.5];
    let row = InstanceModelCol::from_global(&GlobalTransform(instance_transform(i).to_affine()));
    let (wc, wh) = arvo_transform(&row.rows, lc, lh);
    ([wc[0] - wh[0], wc[1] - wh[1], wc[2] - wh[2]], [wc[0] + wh[0], wc[1] + wh[1], wc[2] + wh[2]])
}

/// The indices of [`MIXED_INSTANCES`] belonging to `mesh`, in SPAWN order.
pub fn instances_of(mesh: MixedMesh) -> Vec<usize> {
    (0..MIXED_INSTANCES.len()).filter(|&i| MIXED_INSTANCES[i].mesh == mesh).collect()
}

/// The indices of `mesh`'s MARKED instances, in SPAWN order — the order they occupy in the ring and
/// therefore in the candidate list (see the module header).
pub fn marked_instances_of(mesh: MixedMesh) -> Vec<usize> {
    instances_of(mesh).into_iter().filter(|&i| MIXED_INSTANCES[i].role.is_marked()).collect()
}

/// The index of the one [`Role::Occluder`] instance.
pub fn occluder_index() -> usize {
    (0..MIXED_INSTANCES.len())
        .find(|&i| MIXED_INSTANCES[i].role == Role::Occluder)
        .expect("invariant: the mixed fixture has exactly one occluder")
}

/// `batch`'s mesh. Batches are emitted in `mesh_id` order and the geometry-table slots are handed
/// out in registration order, which [`spawn_mixed`] performs in [`MixedMesh`]'s discriminant order.
pub fn mesh_of_batch(batch: usize) -> MixedMesh {
    match batch {
        0 => MixedMesh::Sphere,
        1 => MixedMesh::Cube,
        other => panic!("the mixed fixture has {BATCH_COUNT} batches; asked for {other}"),
    }
}

// ===============================================================================================
// The fixture's own invariants — checked WITHOUT a device
// ===============================================================================================

/// Checks the properties the constants above are supposed to produce, so a "tidying" edit reds in a
/// plain `cargo test` rather than at the next GPU run.
///
/// Deliberately NOT the occlusion precondition: that one is `vg_occ_verdict_census.rs`'s whole
/// subject and it needs `boyko_render::hzb`. What is here is the arithmetic this module states about
/// itself.
pub fn assert_fixture_invariants() {
    // The analytic bounds ARE the mesh's bounds.
    for mesh in [MixedMesh::Sphere, MixedMesh::Cube] {
        let (verts, _) = mesh_geometry(mesh);
        let folded = fold_local_bounds(&verts);
        let analytic = mesh_local_bounds(mesh);
        for axis in 0..3 {
            assert!(
                (folded.0[axis] - analytic.0[axis]).abs() <= 1e-6
                    && (folded.1[axis] - analytic.1[axis]).abs() <= 1e-6,
                "{mesh:?}: the folded local bounds {folded:?} disagree with the analytic \
                 {analytic:?} on axis {axis}. Every pixel bound in this fixture is computed from \
                 the analytic pair, so a mesh generator that drifted would move the geometry \
                 without moving a single number in this file."
            );
        }
    }

    // Four instances per mesh, and the marked ones in the order the candidate list will carry.
    for mesh in [MixedMesh::Sphere, MixedMesh::Cube] {
        let all = instances_of(mesh);
        assert_eq!(
            all.len(),
            INSTANCES_PER_MESH,
            "{mesh:?} carries {} instances, not {INSTANCES_PER_MESH} -- `draw_batches == 2` with \
             equal counts is what makes batch 1's `base_instance` a nonzero, checkable number",
            all.len()
        );
        let marked = marked_instances_of(mesh);
        assert_eq!(marked.len(), MARKED_PER_MESH, "{mesh:?} marks {} instances", marked.len());
        let roles: Vec<Role> = marked.iter().map(|&i| MIXED_INSTANCES[i].role).collect();
        assert_eq!(
            roles.as_slice(),
            MARKED_ROLES_IN_SPAWN_ORDER.as_slice(),
            "{mesh:?}'s marked instances are spawned in the order {roles:?}. G-P3-B clauses 4 and 5 \
             read candidate `j` as this mesh's `j`-th MARKED instance -- the six marked entities \
             share one archetype, so their relative ring order IS their spawn order. Reordering \
             this array without reordering the clause's expectation would compare the oracle \
             against the wrong AABB and report it as a cull defect."
        );
    }

    // Exactly one occluder, exactly one filler, and the occluder is the NEAREST thing in the scene.
    let occ = occluder_index();
    assert_eq!(
        MIXED_INSTANCES.iter().filter(|i| i.role == Role::Filler).count(),
        1,
        "the mixed fixture has exactly one filler -- it is what keeps the EARLY depth non-empty \
         under FORCE-LATE, and a second one would be untracked geometry in every count"
    );
    for (i, inst) in MIXED_INSTANCES.iter().enumerate() {
        if i == occ {
            continue;
        }
        assert!(
            inst.view_distance > MIXED_INSTANCES[occ].view_distance,
            "{}: view distance {} is not strictly behind the occluder's {}. The byte-identity pin \
             rests on there being NO coincident geometry: a redraw at EQUAL depth is invisible \
             under VK_COMPARE_OP_GREATER, but geometry at equal depth REORDERED between the two \
             scopes is not, and marking a strict subset reorders the ring.",
            inst.name,
            inst.view_distance,
            MIXED_INSTANCES[occ].view_distance
        );
    }

    // The blocks are aligned and inside the region the slab is required to cover.
    for (b, blk) in HIDDEN_BLOCKS.iter().enumerate() {
        assert!(
            blk[0].is_multiple_of(HIDDEN_BLOCK_SIDE) && blk[1].is_multiple_of(HIDDEN_BLOCK_SIDE),
            "block {b} at {blk:?} is not {HIDDEN_BLOCK_SIDE}-ALIGNED. Alignment is the whole \
             argument: `containing_texel` masks off the low `level` bits, so an unaligned block's \
             2x2 footprint straddles two blocks and folds a texel the slab does not own."
        );
        assert!(
            blk[0] >= SLAB_COVERS_AT_LEAST[0]
                && blk[1] >= SLAB_COVERS_AT_LEAST[1]
                && blk[0] + HIDDEN_BLOCK_SIDE - 1 <= SLAB_COVERS_AT_LEAST[2]
                && blk[1] + HIDDEN_BLOCK_SIDE - 1 <= SLAB_COVERS_AT_LEAST[3],
            "block {b} at {blk:?} is not inside the region the slab must cover \
             {SLAB_COVERS_AT_LEAST:?}"
        );
    }
    assert_eq!(
        [
            HIDDEN_BLOCKS.iter().map(|b| b[0]).min().expect("four blocks"),
            HIDDEN_BLOCKS.iter().map(|b| b[1]).min().expect("four blocks"),
            HIDDEN_BLOCKS.iter().map(|b| b[0]).max().expect("four blocks") + HIDDEN_BLOCK_SIDE - 1,
            HIDDEN_BLOCKS.iter().map(|b| b[1]).max().expect("four blocks") + HIDDEN_BLOCK_SIDE - 1,
        ],
        HIDDEN_BLOCK_UNION,
        "HIDDEN_BLOCK_UNION must be the union of the four blocks, or the census would check the \
         slab's containment against a region no instance lives in"
    );
    assert_eq!(
        HIDDEN_BLOCKS.len(),
        HIDDEN_TOTAL,
        "one block per hidden instance: two hidden instances sharing a block would make the \
         precondition true for a fixture that occludes only one of them"
    );
}

// ===============================================================================================
// The spawn
// ===============================================================================================

/// Spawns the mixed scene: two registered meshes, eight instances, six of them marked when
/// `marked` — plus the shipped lights and camera.
///
/// `marked` is a PARAMETER rather than an env read, so the `off` pin and the three armed pins
/// differ in exactly one boolean at exactly one site.
///
/// ⚠️ The registration ORDER is load-bearing: the sphere claims the lower geometry-table slot, so
/// batch 0 is the sphere and batch 1 the cube ([`mesh_of_batch`]). ⚠️ `OcclusionCulling` is queued
/// into the SAME command flush as the spawn, never inserted from a LATER frame — this kernel has no
/// tuple `Bundle` impl, so it cannot go in the bundle, and an insert from a later frame arms the
/// split one frame late.
pub fn spawn_mixed(
    commands: &mut Commands,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    geo_table: &mut MeshGeometryTableSlot,
    dev: &GpuDevice,
    marked: bool,
) {
    let handles: Vec<MeshHandle> = [MixedMesh::Sphere, MixedMesh::Cube]
        .iter()
        .map(|m| {
            let (verts, idx) = mesh_geometry(*m);
            match geo_table.0.as_mut() {
                Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
                None => meshes.register_mesh(dev.get(), &verts, &idx),
            }
        })
        .collect();

    // One material per ROLE, so a frame dump is readable and so no two instances can be confused
    // for one another by eye. The material travels with the entity, so the archetype split marking
    // causes cannot move a material onto a different instance.
    let mats: Vec<u16> = [
        Material::new([0.55, 0.56, 0.60, 1.0], 0.0, 0.55, 0.5, [0.0; 3], 0), // occluder
        Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0), // filler
        Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0), // hidden
        Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0), // visible
    ]
    .into_iter()
    .map(|m| materials.add(m).index() as u16)
    .collect();
    let mat_of = |role: Role| match role {
        Role::Occluder => mats[0],
        Role::Filler => mats[1],
        Role::Hidden => mats[2],
        Role::Visible => mats[3],
    };

    for (i, inst) in MIXED_INSTANCES.iter().enumerate() {
        let e = commands
            .spawn(MeshBundle::new(handles[inst.mesh as usize], instance_transform(i)))
            .id();
        if marked && inst.role.is_marked() {
            commands.entity(e).insert(OcclusionCulling);
        }
        commands.entity(e).insert(MaterialHandle(mat_of(inst.role)));
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

    commands.spawn(CameraRig {
        transform: camera_transform(),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: camera_projection(),
    });
}
