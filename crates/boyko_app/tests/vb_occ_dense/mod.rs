//! **VG R3 piece 4 rung P4-6 — `vb_occ_dense`: `vb_occ_mixed`'s geometry with the hidden set
//! replicated `K` times behind the same slab.**
//!
//! The mixed fixture has EIGHT instances. Whatever the occlusion split saves there is a difference
//! between two four-instance rasters, and the campaign has measured its own GPU-timing floor at
//! 4.7–14.3 % on this box (`docs/VG-DECIDABILITY-FLOOR.md`). A saving that small cannot clear a band
//! computed from a zero control in the same sitting. This fixture exists to make the saving LARGE
//! enough that the question is decidable at all — and to be honest about what a large saving here
//! would and would not mean.
//!
//! # ⚠️ NO GOLDEN PIN, and why that is a decision rather than an omission
//!
//! Nothing pins this fixture's PIXELS. The precedent is `vb_occ_multi` (`vb_occ_split_gate.rs:26-31`):
//! a pin buys a byte-identity claim at the price of a `VB_PINS` name and an owner blessing ceremony,
//! and the byte-identity claim this fixture could make is **plan D12 restated**. On a converged
//! static scene an instance the early phase rejects writes no depth, so the pyramid is a fixed point
//! and the late phase rejects it again; ARMED and FORCE-KEEP therefore produce the same pixels *by
//! theorem*, and hashing them proves the theorem, not the engine.
//!
//! Its correctness oracle is the one `vb_occ_mixed` already runs — [`boyko_render::hzb::occlusion_verdict`]
//! on the host against the DUMPED pyramid, compared against the GPU readback's candidate lists. That
//! oracle is independent of the engine's own decision, and it scales with `K`.
//!
//! It also needs **no density-census row**: `vg_density_census_gate` drives its own fixtures through
//! its own `run_worker` and never reads a pin list, which `vg_density_census.rs:75-84` states as a
//! RULE rather than as a fact about one name. A fixture with no pin at all is even further outside
//! that gate's reach.
//!
//! # The shape
//!
//! Exactly `vb_occ_mixed`'s four singletons — the near cube SLAB (unmarked), the far sphere FILLER
//! (unmarked, off the slab, which is what keeps the EARLY depth non-empty under FORCE-LATE), and the
//! two marked VISIBLE instances whose rects are disjoint from the slab's — plus `K` replicas of the
//! four HIDDEN instances.
//!
//! | role | count | marked | oracle verdict |
//! |---|---|---|---|
//! | `Occluder` | 1 (cube) | no | not tested (the capability is structural) |
//! | `Filler` | 1 (sphere) | no | not tested |
//! | `Hidden` | `4 * K` (2 sphere + 2 cube per replica) | yes | `Reject` |
//! | `Visible` | 2 (1 + 1) | yes | `Keep(NotOccluded)` |
//!
//! **`K = 1` reproduces `vb_occ_mixed` EXACTLY** — same meshes, same roles, same pixels, same view
//! distances, same scales, same spawn order (see [`assert_fixture_invariants`], which checks that
//! against [`crate::vb_occ_mixed_scene::MIXED_INSTANCES`] rather than asserting it in prose). That is
//! what makes the `K = 1` leg of the oracle gate a cross-check against a fixture whose occlusion
//! precondition is independently asserted, analytically and without a GPU, by
//! `vg_occ_verdict_census.rs`.
//!
//! # ⚠️ THE PRECONDITION IS THE SAME ONE, AND IT IS ASSERTED HERE, NOT ASSUMED
//!
//! `vb_occ_mixed_scene`'s module header states the rule that makes a hidden instance rejectable:
//! its projected rect must lie wholly inside ONE `2^(L+1)`-aligned block of width `2^(L+1)` that is
//! itself wholly inside the slab's rect, because [`boyko_render::hzb::select_texels`] returns the
//! ALIGNED expansion and [`boyko_render::hzb::occluder_depth`] folds it with a conservative `min`.
//! One background texel anywhere in the footprint forces KEEP.
//!
//! Replication moves instances, so the rule is re-checked **for every replica** by
//! [`assert_fixture_invariants`], through the engine's own [`boyko_render::hzb::project_aabb`], with
//! no GPU. Two properties do the work:
//!
//! * each replica's screen offset stays inside [`LATTICE_OFFSETS`]'s `±32 px`, and the widest rect
//!   any hidden instance projects to is ~21 px half-width at the NEAREST replica depth, so
//!   `32 + 21 = 53 < 64` — the block's half-side;
//! * the replicas recede in depth ([`HIDDEN_DEPTH_STEP`]), and a perspective rect only SHRINKS with
//!   distance, so replica `0` is the worst case on every axis.
//!
//! The assertion does not rely on either argument: it projects every instance and compares.
//!
//! # ⚠️ WHAT A `Saving` MEASURED ON THIS FIXTURE CANNOT CLAIM
//!
//! * **Not the saving a MOVING camera would realise.** D12's fixed point means the deferred set is
//!   drawn by NEITHER scope here: the late phase re-rejects it. So `−Δ_5` measures the whole cost of
//!   rasterising the hidden set, i.e. an UPPER bound on the split's benefit, while `Overhead` is a
//!   LOWER bound on its cost (a moving camera's late scope draws survivors this one never draws).
//!   The hit rate under motion is piece 3's OQ 3 and stays open.
//! * **Not a fragment-cost saving.** Replicas share four screen neighbourhoods and recede along the
//!   view direction, so under FORCE-KEEP the nearest replica z-rejects most of the fragments behind
//!   it. What the early raster loses when the split defers them is dominated by per-instance and
//!   per-vertex work, not by shading.
//! * **Nothing about PIXELS.** No pin. A defect that produces the oracle's verdicts and the wrong
//!   image is invisible here; pixel correctness stays `[vb_occ_mixed]`'s job, on 8 instances.
//!
//! # Consumers
//!
//! Declared by `vg_occ_split_timing.rs`, which also declares `mod vb_occ_mixed_scene;` — this module
//! reads that one through `crate::` rather than copying its camera, its meshes or its blocks. Two
//! copies of a fixture are two texts that can disagree, and a gate that disagrees with the scene it
//! adjudicates proves nothing.

#![allow(dead_code)]

use boyko_app::prelude::*;
use boyko_render::csm_caster::arvo_transform;
use boyko_render::hzb::{ScreenRect, project_aabb};
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::{Material, MeshAssetsVbExt, MeshGeometryTableSlot, OcclusionCulling};

use crate::vb_occ_mixed_scene::{
    EXTENT, HIDDEN_BLOCKS, HIDDEN_BLOCK_SIDE, MIXED_INSTANCES, MixedMesh, Role,
    SLAB_COVERS_AT_LEAST, SUN_DIR, camera_projection, camera_transform, mesh_geometry,
    mesh_local_bounds, view_proj_rows, world_position,
};

// ===============================================================================================
// The knob, and the replication geometry
// ===============================================================================================

/// The env knob that sets the replication factor `K`.
pub const ENV_K: &str = "BOYKO_VG_OCC_DENSE_K";

/// `K` when [`ENV_K`] is unset — the plan's own default.
pub const DEFAULT_K: u32 = 64;

/// The view distance of replica `0`'s hidden instances — `vb_occ_mixed`'s own `9.0`, so `K = 1`
/// reproduces that fixture exactly.
pub const HIDDEN_BASE_DEPTH: f32 = 9.0;

/// The view-distance increment between consecutive replicas.
///
/// Small enough that `K = 64` reaches `9 + 63 * 0.25 = 24.75`, far inside `vb_occ_mixed_scene`'s
/// `FAR = 100`, and large enough that no two replicas are coincident in depth — the property
/// `vb_occ_mixed_scene::assert_fixture_invariants` names as load-bearing for its own byte-identity
/// pin (a redraw at EQUAL depth is invisible under `VK_COMPARE_OP_GREATER`; geometry REORDERED
/// between the two scopes at equal depth is not).
pub const HIDDEN_DEPTH_STEP: f32 = 0.25;

/// The per-replica screen offsets, in pixels, applied inside each hidden instance's own
/// 128-aligned block.
///
/// ⚠️ **Index 0 is `0.0` on purpose.** Replica `0` then lands on `vb_occ_mixed`'s four hidden
/// centres exactly, which is what makes `K = 1` that fixture.
///
/// `±32` is the whole excursion: the widest hidden rect is ~21 px half-width at the nearest replica
/// depth, so the rect stays `32 + 21 = 53 < 64` from the block centre. The bound is not trusted —
/// [`assert_fixture_invariants`] projects every instance and compares against the block.
pub const LATTICE_OFFSETS: [f32; 4] = [0.0, 32.0, -32.0, 16.0];

/// Hidden instances per replica — one per block in [`crate::vb_occ_mixed_scene::HIDDEN_BLOCKS`].
pub const HIDDEN_PER_REPLICA: usize = 4;

/// Registered meshes, hence `DrawBatch`es — `vb_occ_mixed`'s two.
pub const BATCH_COUNT: usize = 2;

/// The engine's instance ring capacity (`gpu_scene::INSTANCE_CAPACITY`), restated here because it
/// is `pub(crate)` in `boyko_app` and this fixture's whole point is to approach it.
///
/// ⚠️ If the engine's constant ever moves, this one goes stale in the SAFE direction only when it
/// moves UP. [`assert_fixture_invariants`] compares against it, so a fixture that would overrun the
/// ring reds host-side rather than as a GPU count nobody can attribute.
pub const RING_CAPACITY: usize = 1024;

// ===============================================================================================
// The instance table
// ===============================================================================================

/// One instance of the dense scene, authored in SCREEN/VIEW space exactly as `vb_occ_mixed` is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenseInstance {
    pub mesh: MixedMesh,
    pub role: Role,
    /// The replica index for a [`Role::Hidden`] instance; `0` for the four singletons.
    pub replica: u32,
    /// The pixel this instance's CENTRE projects to at [`EXTENT`]².
    pub pixel: [f32; 2],
    /// View-space distance in front of the eye (`clip.w`, before the divide).
    pub view_distance: f32,
    /// Per-axis `Transform` scale.
    pub scale: [f32; 3],
}

impl DenseInstance {
    /// A short name used verbatim in every failure message, so a red names the instance rather than
    /// an index into a `K`-dependent array.
    #[must_use]
    pub fn name(&self) -> String {
        match self.role {
            Role::Hidden => format!("{:?}/hidden/r{}", self.mesh, self.replica),
            other => format!("{:?}/{other:?}", self.mesh),
        }
    }

    /// The 128-aligned block this instance is required to project inside — `None` for every role
    /// but [`Role::Hidden`], which is the only one the precondition constrains.
    #[must_use]
    pub fn hidden_block(&self) -> Option<[u32; 2]> {
        if self.role != Role::Hidden {
            return None;
        }
        HIDDEN_BLOCKS.into_iter().find(|blk| {
            let (bx, by) = (blk[0] as f32, blk[1] as f32);
            let side = HIDDEN_BLOCK_SIDE as f32;
            self.pixel[0] >= bx && self.pixel[0] < bx + side && self.pixel[1] >= by
                && self.pixel[1] < by + side
        })
    }
}

/// `K` from [`ENV_K`], defaulting to [`DEFAULT_K`].
///
/// # Panics
///
/// On a `K` of zero or an unparseable value. A zero `K` would spawn a scene with NO hidden
/// instance, on which `Σ n_defer == 0` is correct — and a correct zero here reads exactly like a
/// cull that stopped deciding, which is the failure this campaign has shipped six times.
#[must_use]
pub fn k_from_env() -> u32 {
    match std::env::var(ENV_K) {
        Err(_) => DEFAULT_K,
        Ok(word) => {
            let k: u32 = word.parse().unwrap_or_else(|_| {
                panic!("{ENV_K}={word:?} is not a replication factor (expected a positive integer)")
            });
            assert!(
                k > 0,
                "{ENV_K}=0 spawns no hidden instance at all. `Σ n_defer == 0` would then be CORRECT, \
                 and a correct zero is indistinguishable from a cull that stopped deciding."
            );
            k
        }
    }
}

/// The hidden BLOCK indices belonging to `mesh` — the sphere owns the `y = 128` row, the cube the
/// `y = 256` row, exactly as `vb_occ_mixed` places its four hidden instances.
const fn hidden_blocks_of(mesh: MixedMesh) -> [usize; 2] {
    match mesh {
        MixedMesh::Sphere => [0, 1],
        MixedMesh::Cube => [2, 3],
    }
}

/// `mesh`'s hidden-instance scale — `vb_occ_mixed`'s own per-mesh value.
const fn hidden_scale(mesh: MixedMesh) -> f32 {
    match mesh {
        MixedMesh::Sphere => 0.55,
        MixedMesh::Cube => 0.68,
    }
}

/// **The whole instance table, in SPAWN order**, for replication factor `k`.
///
/// Grouped by mesh (sphere first, so it claims the lower geometry-table slot and batch 0), and
/// within a mesh: the UNMARKED singleton first, then every HIDDEN replica in replica order, then the
/// marked VISIBLE one. That ordering is load-bearing twice over —
///
/// * per mesh there is exactly ONE unmarked instance, so the ring admits exactly two archetype
///   iteration orders and they are distinguishable by whether batch-local offset `0` is a candidate;
/// * the marked instances share one archetype, so their RELATIVE ring order is their spawn order,
///   which is what lets a candidate offset be mapped back to an instance.
#[must_use]
pub fn dense_instances(k: u32) -> Vec<DenseInstance> {
    let mut out = Vec::with_capacity(4 * k as usize + 4);
    for mesh in [MixedMesh::Sphere, MixedMesh::Cube] {
        // The unmarked singleton: the filler on the sphere, the occluder on the cube.
        out.push(unmarked_of(mesh));
        for r in 0..k {
            for blk in hidden_blocks_of(mesh) {
                out.push(hidden_instance(mesh, blk, r));
            }
        }
        out.push(visible_of(mesh));
    }
    out
}

/// The one hidden instance of `mesh` in `HIDDEN_BLOCKS[blk]` at replica index `r`.
fn hidden_instance(mesh: MixedMesh, blk: usize, r: u32) -> DenseInstance {
    let block = HIDDEN_BLOCKS[blk];
    let half = (HIDDEN_BLOCK_SIDE / 2) as f32;
    let dx = LATTICE_OFFSETS[(r as usize) % LATTICE_OFFSETS.len()];
    let dy = LATTICE_OFFSETS[(r as usize / LATTICE_OFFSETS.len()) % LATTICE_OFFSETS.len()];
    let s = hidden_scale(mesh);
    DenseInstance {
        mesh,
        role: Role::Hidden,
        replica: r,
        pixel: [block[0] as f32 + half + dx, block[1] as f32 + half + dy],
        view_distance: HIDDEN_DEPTH_STEP.mul_add(r as f32, HIDDEN_BASE_DEPTH),
        scale: [s; 3],
    }
}

/// `mesh`'s UNMARKED singleton, copied field-by-field from [`MIXED_INSTANCES`] so the two fixtures
/// cannot drift apart.
fn unmarked_of(mesh: MixedMesh) -> DenseInstance {
    let want = match mesh {
        MixedMesh::Sphere => Role::Filler,
        MixedMesh::Cube => Role::Occluder,
    };
    from_mixed(mesh, want)
}

/// `mesh`'s marked VISIBLE singleton, likewise copied from [`MIXED_INSTANCES`].
fn visible_of(mesh: MixedMesh) -> DenseInstance {
    from_mixed(mesh, Role::Visible)
}

/// The unique [`MIXED_INSTANCES`] entry with this `(mesh, role)`, as a [`DenseInstance`].
///
/// # Panics
///
/// If `vb_occ_mixed` stops having exactly one such instance — which would silently change what this
/// fixture replicates around.
fn from_mixed(mesh: MixedMesh, role: Role) -> DenseInstance {
    let mut found = MIXED_INSTANCES.iter().filter(|i| i.mesh == mesh && i.role == role);
    let inst = found
        .next()
        .unwrap_or_else(|| panic!("vb_occ_mixed has no {role:?} instance on {mesh:?}"));
    assert!(
        found.next().is_none(),
        "vb_occ_mixed has more than one {role:?} instance on {mesh:?}; `vb_occ_dense` copies the \
         singletons by (mesh, role) and would silently pick the first"
    );
    DenseInstance {
        mesh: inst.mesh,
        role: inst.role,
        replica: 0,
        pixel: inst.pixel,
        view_distance: inst.view_distance,
        scale: inst.scale,
    }
}

/// The indices of `instances` belonging to `mesh`, in SPAWN order.
#[must_use]
pub fn indices_of_mesh(instances: &[DenseInstance], mesh: MixedMesh) -> Vec<usize> {
    (0..instances.len()).filter(|&i| instances[i].mesh == mesh).collect()
}

/// The indices of `mesh`'s MARKED instances, in SPAWN order — the order they occupy inside the
/// ring, and therefore inside the candidate list.
#[must_use]
pub fn marked_indices_of_mesh(instances: &[DenseInstance], mesh: MixedMesh) -> Vec<usize> {
    indices_of_mesh(instances, mesh).into_iter().filter(|&i| instances[i].role.is_marked()).collect()
}

/// `batch`'s mesh — batches are emitted in `mesh_id` order and slots are handed out in registration
/// order, which [`spawn_dense`] performs in [`MixedMesh`]'s discriminant order.
///
/// # Panics
///
/// On a batch index this fixture does not have.
#[must_use]
pub fn mesh_of_batch(batch: usize) -> MixedMesh {
    match batch {
        0 => MixedMesh::Sphere,
        1 => MixedMesh::Cube,
        other => panic!("the dense fixture has {BATCH_COUNT} batches; asked for {other}"),
    }
}

/// HIDDEN instances in the whole scene — `Σ n_defer` on the unforced, converged frame.
#[must_use]
pub const fn hidden_total(k: u32) -> usize {
    HIDDEN_PER_REPLICA * k as usize
}

/// MARKED instances in the whole scene — the number `[host] occlusion_instances` must report.
#[must_use]
pub const fn marked_total(k: u32) -> usize {
    hidden_total(k) + 2
}

// ===============================================================================================
// Geometry, and the projection this fixture is measured under
// ===============================================================================================

/// `inst`'s `Transform` — translation from `vb_occ_mixed_scene::world_position`, no rotation.
///
/// No rotation for `vb_occ_mixed`'s own reason: a rotated box's Arvo fold is a strictly larger world
/// AABB than the authored one, so every pixel bound here would become an inequality nobody checked.
#[must_use]
pub fn instance_transform(inst: &DenseInstance) -> Transform {
    Transform {
        translation: world_position(inst.pixel, inst.view_distance),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(inst.scale[0], inst.scale[1], inst.scale[2]),
    }
}

/// `inst`'s WORLD AABB, through the engine's own `arvo_transform` over the PRODUCTION-packed
/// instance row — the same composition `boyko_render::frustum::instance_visible_after_cull`
/// performs and the same one `vb_batch_cull.comp.hlsl` mirrors.
#[must_use]
pub fn instance_world_aabb(inst: &DenseInstance) -> ([f32; 3], [f32; 3]) {
    let (mn, mx) = mesh_local_bounds(inst.mesh);
    let lc = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
    let lh = [(mx[0] - mn[0]) * 0.5, (mx[1] - mn[1]) * 0.5, (mx[2] - mn[2]) * 0.5];
    let row = InstanceModelCol::from_global(&GlobalTransform(instance_transform(inst).to_affine()));
    let (wc, wh) = arvo_transform(&row.rows, lc, lh);
    ([wc[0] - wh[0], wc[1] - wh[1], wc[2] - wh[2]], [wc[0] + wh[0], wc[1] + wh[1], wc[2] + wh[2]])
}

/// `inst`'s projected screen rect at [`EXTENT`]², through the engine's own
/// [`boyko_render::hzb::project_aabb`].
///
/// # Panics
///
/// If the projection refuses the bound. Every instance in this fixture is in front of the eye and
/// inside the frustum, so a refusal is a fixture error and is reported as one.
#[must_use]
pub fn instance_rect(inst: &DenseInstance) -> ScreenRect {
    let (mn, mx) = instance_world_aabb(inst);
    project_aabb(&view_proj_rows(), [EXTENT, EXTENT], mn, mx).unwrap_or_else(|reason| {
        panic!(
            "FIXTURE: `{}` does not project to a screen rect ({reason:?}). Every instance in this \
             fixture is authored in front of the eye through the engine's own camera basis, so a \
             refusal here means the placement arithmetic, not the cull.",
            inst.name()
        )
    })
}

// ===============================================================================================
// The fixture's own invariants — checked WITHOUT a device
// ===============================================================================================

/// Checks everything this module states about itself, at replication factor `k`.
///
/// Runs in a plain `cargo test`: no GPU, no window, no device. A "tidying" edit to
/// [`LATTICE_OFFSETS`], [`HIDDEN_DEPTH_STEP`] or the spawn grouping reds here instead of at the next
/// GPU sitting, where it would arrive as a count nobody can attribute.
///
/// # Panics
///
/// On any violated invariant, naming the instance rather than an index.
pub fn assert_fixture_invariants(k: u32) {
    let instances = dense_instances(k);

    // ---- the ring bound, before anything else ---------------------------------------------------
    assert!(
        instances.len() <= RING_CAPACITY,
        "K={k} spawns {} instances against a ring capacity of {RING_CAPACITY}. Past the cap the \
         upload path panics inside the engine with a message about INSTANCE_CAPACITY, which reads \
         like an engine defect and is a fixture choice.",
        instances.len()
    );
    assert_eq!(
        instances.len(),
        4 * k as usize + 4,
        "the table is 4K hidden plus the four singletons; a different arity means a role was \
         duplicated or dropped"
    );

    // ---- K = 1 IS `vb_occ_mixed` ----------------------------------------------------------------
    //
    // Checked structurally rather than claimed in prose: this is what makes the K=1 leg of the
    // oracle gate a cross-check against a fixture whose precondition `vg_occ_verdict_census.rs`
    // asserts analytically.
    if k == 1 {
        assert_eq!(
            instances.len(),
            MIXED_INSTANCES.len(),
            "at K=1 this fixture must BE `vb_occ_mixed`"
        );
        for (d, m) in instances.iter().zip(MIXED_INSTANCES.iter()) {
            assert!(
                d.mesh == m.mesh
                    && d.role == m.role
                    && d.pixel == m.pixel
                    && d.view_distance == m.view_distance
                    && d.scale == m.scale,
                "at K=1 spawn position {:?} is `{}` but `vb_occ_mixed` has `{}` there. The K=1 \
                 identity is the only independent check this fixture's placement arithmetic has.",
                d.pixel,
                d.name(),
                m.name
            );
        }
    }

    // ---- per-mesh composition: ONE unmarked, the marked ones in spawn order ---------------------
    for mesh in [MixedMesh::Sphere, MixedMesh::Cube] {
        let of_mesh = indices_of_mesh(&instances, mesh);
        assert_eq!(
            of_mesh.len(),
            2 * k as usize + 2,
            "{mesh:?} carries {} instances at K={k}",
            of_mesh.len()
        );
        let unmarked: Vec<usize> =
            of_mesh.iter().copied().filter(|&i| !instances[i].role.is_marked()).collect();
        assert_eq!(
            unmarked.len(),
            1,
            "{mesh:?} has {} unmarked instances. Exactly one is what makes the ring admit only TWO \
             archetype iteration orders, and what makes them distinguishable by whether batch-local \
             offset 0 is a candidate.",
            unmarked.len()
        );
        assert_eq!(
            unmarked[0], of_mesh[0],
            "{mesh:?}'s unmarked instance is not spawned FIRST. The candidate-offset identification \
             reads the two admissible ring layouts off that position."
        );
        let marked = marked_indices_of_mesh(&instances, mesh);
        assert_eq!(
            instances[marked[0]].role,
            Role::Hidden,
            "{mesh:?}'s first MARKED instance is {:?}, not Hidden. The layout identification asks \
             whether offset 0 is a candidate, which is only decisive when the first marked instance \
             is one the oracle rejects.",
            instances[marked[0]].role
        );
        assert_eq!(
            instances[*marked.last().expect("invariant: every mesh marks at least one instance")]
                .role,
            Role::Visible,
            "{mesh:?}'s last MARKED instance must be the Visible one -- the candidate list is then \
             a PREFIX of the marked run, which is the shape a GPU compaction in ring order produces"
        );
    }

    // ---- depth: everything is strictly behind the occluder, and no two replicas coincide --------
    let occluder = instances
        .iter()
        .find(|i| i.role == Role::Occluder)
        .expect("invariant: the dense fixture has exactly one occluder");
    for inst in &instances {
        if inst.role == Role::Occluder {
            continue;
        }
        assert!(
            inst.view_distance > occluder.view_distance,
            "`{}` at view distance {} is not strictly behind the occluder's {}",
            inst.name(),
            inst.view_distance,
            occluder.view_distance
        );
    }

    // ---- THE PRECONDITION: every hidden rect inside ONE 128-aligned block inside the slab --------
    let slab = instance_rect(occluder);
    assert!(
        slab.min[0] <= SLAB_COVERS_AT_LEAST[0]
            && slab.min[1] <= SLAB_COVERS_AT_LEAST[1]
            && slab.max[0] >= SLAB_COVERS_AT_LEAST[2]
            && slab.max[1] >= SLAB_COVERS_AT_LEAST[3],
        "FIXTURE PRECONDITION: the slab projects to [{:?}..{:?}], which does not contain the region \
         {SLAB_COVERS_AT_LEAST:?} the hidden blocks live in. Every hidden instance's rejection rests \
         on the conservative `min` folding only slab texels.",
        slab.min,
        slab.max
    );
    for inst in instances.iter().filter(|i| i.role == Role::Hidden) {
        let block = inst.hidden_block().unwrap_or_else(|| {
            panic!(
                "FIXTURE PRECONDITION: `{}`'s centre {:?} is in none of the four aligned blocks \
                 {HIDDEN_BLOCKS:?}",
                inst.name(),
                inst.pixel
            )
        });
        let rect = instance_rect(inst);
        let hi = [block[0] + HIDDEN_BLOCK_SIDE - 1, block[1] + HIDDEN_BLOCK_SIDE - 1];
        assert!(
            rect.min[0] >= block[0]
                && rect.min[1] >= block[1]
                && rect.max[0] <= hi[0]
                && rect.max[1] <= hi[1],
            "FIXTURE PRECONDITION: `{}` projects to [{:?}..{:?}], which escapes its 128-aligned \
             block [{block:?}..{hi:?}]. `select_texels` returns the ALIGNED expansion, so a rect \
             that straddles a block boundary folds a texel the slab does not own and the instance \
             becomes structurally un-rejectable -- after which `S n_defer` drops for a FIXTURE \
             reason that reads exactly like a cull defect.",
            inst.name(),
            rect.min,
            rect.max
        );
        assert!(
            hi[0] <= SLAB_COVERS_AT_LEAST[2] && hi[1] <= SLAB_COVERS_AT_LEAST[3],
            "FIXTURE PRECONDITION: `{}`'s block [{block:?}..{hi:?}] is not inside the region the \
             slab must cover {SLAB_COVERS_AT_LEAST:?}",
            inst.name()
        );
    }
}

/// **The trap this fixture folds in** — asserts that no PRE-LIGHT consumer is armed on `app`.
///
/// `ResolvedRenderPath::mesh_geo_shade_split` is `VisibilityBuffer && mesh_leg && pre_light`
/// (`boyko_render/src/render_path_config.rs:945-946`), and `pre_light` is the union
/// `ssao || ddgi || shadow_denoise_spatial || shadow_temporal || ssr` (`:918-922`; no `SsrConfig`
/// type exists yet, so the runner threads a literal `false` at `runner.rs:533`). The three
/// Resources below are the whole free variable on a `VisibilityBuffer × Mesh` fixture — the path and
/// the leg are fixed by the worker's own `RenderPathConfig`.
///
/// Why it is here and not left to the engine: `boyko_app::runner`'s bench arming carries a
/// release-live `assert!(!mesh_geo_shade_split, ...)` (`runner.rs:1101-1106`) whose message is about
/// **VB-P1d's published break-even**. A variant of this fixture that armed SSAO would therefore kill
/// the channel-G worker with a message about a different measurement's scope, which reads like an
/// instrument failure and is not one. This assertion fires first, at the fixture, naming the cause.
///
/// # Panics
///
/// If any pre-light consumer Resource is present and enabled.
pub fn assert_no_split_producer(app: &App) {
    let ssao =
        app.world().try_resource::<boyko_render::SsaoConfig>().is_some_and(|c| c.enabled());
    let ddgi =
        app.world().try_resource::<boyko_render::DdgiConfig>().is_some_and(|c| c.enabled());
    let denoise = app
        .world()
        .try_resource::<boyko_render::ShadowDenoiseConfig>()
        .is_some_and(|c| c.spatial_enabled() || c.temporal_enabled());
    assert!(
        !(ssao || ddgi || denoise),
        "FIXTURE: `vb_occ_dense` armed a PRE-LIGHT consumer (ssao={ssao} ddgi={ddgi} \
         shadow_denoise={denoise}), which resolves `mesh_geo_shade_split` and makes \
         `boyko_app::runner`'s bench arming panic at runner.rs:1101-1106 with a message about \
         VB-P1d's break-even. That would read as an instrument failure; it is a fixture \
         configuration. The split lit-producer is out of the VB-P1d bench's scope (its own open \
         question 5), so this fixture must arm none of the three."
    );
}

// ===============================================================================================
// The spawn
// ===============================================================================================

/// Spawns the dense scene at replication factor `k`: two registered meshes, `4K + 4` instances,
/// `4K + 2` of them marked when `marked` — plus `vb_occ_mixed`'s own lights and camera.
///
/// `marked` is a PARAMETER rather than an env read for `spawn_mixed`'s reason: the marked and
/// unmarked legs then differ in exactly one boolean at exactly one site.
///
/// ⚠️ The registration ORDER is load-bearing — the sphere claims the lower geometry-table slot, so
/// batch 0 is the sphere and batch 1 the cube ([`mesh_of_batch`]). ⚠️ `OcclusionCulling` is queued
/// into the SAME command flush as the spawn, never inserted from a later frame, which would arm the
/// split one frame late.
pub fn spawn_dense(
    commands: &mut Commands,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    geo_table: &mut MeshGeometryTableSlot,
    dev: &GpuDevice,
    marked: bool,
    k: u32,
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

    // One material per ROLE — `vb_occ_mixed`'s own four, so a frame dump of either fixture reads
    // the same way.
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

    for inst in dense_instances(k) {
        let e = commands
            .spawn(MeshBundle::new(handles[inst.mesh as usize], instance_transform(&inst)))
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
