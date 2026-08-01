//! CSM Increment 2 — the ECS-native shadow-caster gather (the cascade depth-pass
//! caster batches).
//!
//! This is the Principle-0 production caster-selection path: the CSM depth pass draws
//! the casters of SPAWNED ENTITIES — every visible `(MeshHandle, InstanceModelCol)`
//! that ALSO carries the structural [`ShadowCaster`](crate::csm_marker::ShadowCaster)
//! marker — read through an ECS [`Query`](boyko_ecs::ecs::core::iters::query::Query), NOT
//! a hand-built inline batch. It mirrors
//! the mesh foundation's main instance gather
//! ([`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws)) exactly, with ONE
//! structural difference: a `With<ShadowCaster>` term on the filter, so a non-caster
//! row never enters a cascade bucket.
//!
//! # Reuse, not duplication (REUSE `gather_mixed_into`)
//!
//! The count → prefix-sum → scatter core is NOT re-implemented here — it is
//! [`MeshRenderScratch::gather_mixed_into`](crate::mesh_draw::MeshRenderScratch::gather_mixed_into)
//! called VERBATIM (all rows take the `None` pair branch — casters are static), with
//! the `With<ShadowCaster>`-filtered query passed as the
//! re-iteration closure. [`CsmCasterScratch`] is a newtype over
//! [`MeshRenderScratch`](crate::mesh_draw::MeshRenderScratch) so the caster batches +
//! ring live in a SEPARATE [`Resource`](boyko_macros::Resource) from the main gather's
//! (they must not collide:
//! the main pass draws ALL visible meshes, the depth pass draws ONLY casters — a
//! different bucket set with different `base_instance`s), while sharing the foundation's
//! cleared-not-reallocated, grow-POW2 discipline (Principle 5) byte-for-byte.
//!
//! # The output seam (how the caster batches reach the recorder)
//!
//! [`gather_shadow_casters`] fills `CsmCasterScratch`'s
//! [`batches`](crate::mesh_draw::MeshRenderScratch::batches) (one [`DrawBatch`] per
//! caster mesh — Principle-1 one `vkCmdDrawIndexed` per mesh) +
//! [`ring`](crate::mesh_draw::MeshRenderScratch::ring) (the contiguous caster
//! instance ring the depth VS indexes by `base_instance + SV_InstanceID`). That pair is
//! exactly the shape `record_csm_depth` consumes for each cascade — the SAME shape the
//! inline CSM demo hand-builds. A real app reaches the recorder via the render scene
//! (the extract path): `gather_shadow_casters` → `CsmCasterScratch.batches`/`.ring` →
//! extract → the recorder. `boyko_rhi_vulkan` CANNOT depend on `boyko_render`, so the
//! app/extract layer passes the gathered batches across the crate boundary — exactly as
//! the main `MeshRenderScratch` batches reach the gbuffer recorder.
//!
//! # C2 — the caster ⇄ SDF/MDF-occluder exclusivity (scene-authoring contract)
//!
//! A `ShadowCaster` raster mesh must NOT also be an SDF/MDF occluder: were a single mesh
//! both, its shadow would be double-counted (once by this raster depth pass, once by the
//! field marcher) and a hard/soft penumbra seam would appear where the two estimators
//! disagree. The critic's C2 exclusivity is structural — keyed by COMPONENT PRESENCE,
//! exactly as
//! [`csm_marker`](crate::csm_marker) documents. But SDF/MDF occluders are NOT ECS
//! components: they are an EDIT LIST
//! (`boyko_sdf_math`'s `SdfEditField` / `sdf_edit_list`), a separate authority the field
//! marcher folds — there is no occluder Component to test against, so the exclusivity
//! CANNOT be a runtime `debug_assert!` in this gather. It is therefore a SCENE-AUTHORING
//! CONTRACT: the [`ShadowCaster`](crate::csm_marker::ShadowCaster) marker is added ONLY
//! to raster meshes that have NO SDF/MDF twin in the edit list. When SDF occluders later
//! become ECS components (a `SdfOccluder` marker), this gather should gain a
//! `debug_assert!` that no gathered caster also carries that marker (and the query a
//! `Without<SdfOccluder>` term to make the exclusion structural, not just asserted).

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::iters::query::{Query, With};
use boyko_ecs::ecs::core::system::{NonSendRes, Res, ResMut};
use boyko_macros::{Resource, SystemSet};
use boyko_scene::ViewUniform;
use boyko_scene::render_caps::{MeshHandle, RenderEnabled};

use crate::csm_config::{CsmCasterBounds, CsmConfig, CsmFitMode, ResolvedCsm};
use crate::csm_marker::ShadowCaster;
use crate::instance_model::InstanceModelCol;
use crate::light::{LightTableDirty, LightingConfig};
use crate::mesh::MeshGpu;
use crate::mesh_assets::MeshAssetsExt;
use crate::mesh_draw::{DrawBatch, MeshRenderScratch, PerInstanceMaterial};

/// The reused per-frame shadow-caster gather scratch (CSM Inc 2) — a SEPARATE
/// [`Resource`] from the main [`MeshRenderScratch`]
/// so the cascade depth-pass caster batches do not collide with the gbuffer pass's
/// batches.
///
/// A newtype over [`MeshRenderScratch`]: it REUSES
/// the foundation's `gather_mixed_into` core, its per-mesh lanes + instance ring, and its
/// cleared-not-reallocated grow-POW2 discipline (Principle 5) VERBATIM — only the
/// resource IDENTITY differs (the ECS keys a `Resource` by type, so the wrapper gives
/// the caster gather its own slot). The gather is filtered on
/// [`ShadowCaster`], so the batches + ring hold ONLY
/// the structural casters.
#[derive(Resource, Default)]
pub struct CsmCasterScratch(pub MeshRenderScratch);

impl CsmCasterScratch {
    /// The number of distinct caster meshes with at least one visible instance this
    /// frame (`batches.len()`) — the Principle-1 one-draw-per-caster-mesh count.
    #[inline]
    pub fn batch_count(&self) -> usize {
        self.0.batch_count()
    }

    /// The total number of scattered caster instances (== the caster ring length).
    #[inline]
    pub fn instance_count(&self) -> usize {
        self.0.instance_count()
    }

    /// The emitted per-caster-mesh [`DrawBatch`]es (one per non-empty caster mesh, in
    /// mesh-id order) — the depth-pass recorder issues one
    /// `vkCmdDrawIndexed` per batch into each cascade.
    #[inline]
    pub fn batches(&self) -> &[DrawBatch] {
        self.0.batches.as_read_slice()
    }

    /// The contiguous caster instance ring — every gathered caster's 48-byte
    /// [`InstanceModelCol`] scattered into its mesh's bucket. The depth pass uploads
    /// this slice into ONE shared instance SSBO bound once for the whole caster batch
    /// list; the depth VS indexes `ring[base_instance + SV_InstanceID]`.
    #[inline]
    pub fn ring(&self) -> &[InstanceModelCol] {
        self.0.ring.as_read_slice()
    }
}

/// The ECS-native CSM Inc-2 shadow-caster gather SYSTEM: buckets every visible
/// `(MeshHandle, InstanceModelCol)` entity that ALSO carries
/// [`ShadowCaster`] into per-caster-mesh
/// [`DrawBatch`]es + the shared caster instance ring, reusing the [`CsmCasterScratch`]
/// resource (Principle 0 — casters from spawned entities via the query, not an inline
/// batch).
///
/// # The structural filter
///
/// The query filter is `(Enabled<RenderEnabled>, With<ShadowCaster>)` — a tuple-AND:
/// - `Enabled<RenderEnabled>` is the `Visibility::Hidden` per-row gate (the SAME term
///   [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) uses), so a hidden caster
///   never enters a cascade bucket.
/// - `With<ShadowCaster>` is the structural caster term — a non-caster row is excluded at
///   iteration (capability-is-presence), so the depth pass draws ONLY casters. This is
///   the WHOLE difference from the main gather.
///
/// # Reuse of the foundation core
///
/// The count → prefix-sum → scatter is
/// [`MeshRenderScratch::gather_mixed_into`](crate::mesh_draw::MeshRenderScratch::gather_mixed_into)
/// called verbatim: the `With<ShadowCaster>`-filtered `q.iter()` is the re-iteration
/// closure, the world's `Assets<MeshGpu>` table supplies the mesh count (sizes the
/// lanes, O2) + each batch's `(index_count, index_type)`. One `vkCmdDrawIndexed` per
/// caster mesh (Principle 1).
///
/// # 0%-gate
///
/// A world with no `ShadowCaster` row (or no `InstanceModelCol` column) yields zero
/// matching rows, so the gather emits zero caster batches + an empty caster ring — the
/// depth pass then draws nothing, byte-identical to a CSM-disabled frame.
///
/// # Registration — unwired-API (matches `gather_mesh_draws`)
///
/// This system is NOT registered in [`CsmPlugin`](crate::csm_plugin::CsmPlugin) (nor any
/// plugin), exactly as
/// [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) is an unwired exported API:
/// it requires the world's `Assets<MeshGpu>` `NonSend` resource + the
/// `InstanceModelCol`/`MeshHandle` columns the inline CSM demos do not yet spawn, and
/// its output must be co-registered
/// `.before` the depth-pass consumer at the OWNING app's call site (so the
/// `.before(record_csm_depth)` edge is expressible there — the same add-order discipline
/// `CsmPlugin` documents for the resolve/consumer ordering). The app registers it
/// alongside [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) and inserts the
/// [`CsmCasterScratch`] resource when it wires the real CSM caster path.
// The `Query<D, F>` IS the declarative system signature; the `(Enabled<RenderEnabled>,
// With<ShadowCaster>)` tuple-AND filter is the whole point of this gather (the structural
// caster term), so factoring it behind a `type` alias would hide the load-bearing intent.
#[allow(clippy::type_complexity, clippy::needless_pass_by_value)]
pub fn gather_shadow_casters(
    q: Query<(&MeshHandle, &InstanceModelCol), (Enabled<RenderEnabled>, With<ShadowCaster>)>,
    mesh_assets: NonSendRes<Assets<MeshGpu>>,
    mut scratch: ResMut<CsmCasterScratch>,
) {
    // asset-streaming plan F5: `high_water()`, not `len()` — a live `MeshHandle.0` can
    // exceed the live COUNT once a hole exists; see `mesh_draw::gather_mesh_draws`'s
    // identical fix for the full rationale.
    let mesh_count = mesh_assets.high_water();
    // The caster gather is ALL-STATIC (the CSM depth pass reads the caster affines from
    // this scratch's `batches`, never an interpolated ring), so every row takes the
    // `None` pair branch of the unified gather — `pair_ring` / `pair_out_slot` stay empty
    // and inert on the caster scratch. Reuses the one gather core (refined-B).
    scratch.0.gather_mixed_into(
        mesh_count,
        // INVARIANT (asset-streaming plan F6 FIX-2): never dereference a non-Loaded
        // slot — see `mesh_draw::gather_mesh_draws`'s identical fix for the full
        // rationale (a graceful skipped-batch, not a dependence on
        // `validate_asset_refs` having caught this mesh's retire in time).
        |mesh_id| {
            let m = mesh_assets.try_get(MeshHandle(mesh_id))?;
            Some((m.index_count, m.index_type))
        },
        // slot resolved by index; staleness is caught by validate_asset_refs earlier this frame (apply→validate→gather)
        // The caster gather has no material dimension (the CSM depth pass reads only
        // `.batches`/`.ring`, never `.material_ids`) — a constant default payload feeds the
        // shared gather core's material lane inertly (asset-streaming plan F8+).
        || q.iter().map(|(h, col)| (h.0, col, None, PerInstanceMaterial::default())),
    );
}

/// CSM auto-fit plan (`docs/CSM-AUTOFIT-PLAN.md`) rung C2 — the cross-plugin ordering
/// seam for [`reduce_caster_bounds`]. Mirrors
/// [`DdgiResolveSet`](crate::ddgi_config::DdgiResolveSet) /
/// [`PunctualResolveSet`](crate::shadow_atlas::PunctualResolveSet): a set-to-set edge
/// pinned BY NAME holds regardless of plugin add-order, where a per-system `.after(key)`
/// edge cannot cross a plugin boundary.
///
/// Dark this rung: nothing joins or orders against this set yet. A future fit (rung C3)
/// pins its resolve `.after_set(CsmFitSet)`; the owning app (rung C5) puts
/// [`reduce_caster_bounds`] `.in_set(CsmFitSet)` at its registration site.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CsmFitSet;

/// The pure, World-free core of the caster-bounds fold (`docs/CSM-AUTOFIT-PLAN.md`
/// Decisions D4/D7, algorithm A) — unit-testable without an ECS, mirroring the
/// closure-meta idiom [`gather_shadow_casters`] itself uses just above (`|mesh_id| {
/// .. }`).
///
/// Folds `batches` + `ring` (a caster gather's OUTPUT — see [`CsmCasterScratch`]) into a
/// [`CsmCasterBounds`]: the per-instance view-space depth extreme (`raw_far`) plus the
/// union world AABB (`world_min`/`world_max`, kept only for a future sun-axis term, D5).
/// A batch whose `mesh_aabb(mesh_id)` is `None` — the mesh has not resolved `Loaded` yet
/// (the F6 never-deref invariant, mirrored from [`gather_shadow_casters`]'s own
/// `try_get` above) — is SKIPPED and NOT counted as resolved. The SAME skip applies to a
/// `Loaded` mesh whose local AABB is the INVERTED sentinel a zero-vertex `MeshGpu` folds
/// to (`local_min[i] > local_max[i]` on every axis — see `MeshGpu::local_min`'s doc):
/// its centre would compute to NaN, which must never poison `raw_far`/`world_min`/
/// `world_max`, so it is treated exactly like a non-`Loaded` slot rather than dereferenced.
///
/// # D4 — per instance, never a projected union AABB
///
/// `raw_far` is the max, OVER INSTANCES, of that instance's own world-AABB extreme along
/// `forward` — never the projection of the union AABB. Two casters at the same depth but
/// opposite lateral extremes would otherwise inflate `raw_far` by their lateral
/// separation (the union-AABB error D4 refutes: e.g. `world x = ±50` at `|fwd.x| = 0.5`
/// would add ~25 of spurious depth). Each instance's world AABB is the Arvo abs-matrix
/// transform of its mesh's local AABB through [`InstanceModelCol::rows`] (3×4
/// row-major): `wc[r] = Σⱼ rows[r][j]·lc[j] + rows[r][3]`, `wh[r] = Σⱼ |rows[r][j]|·lh[j]`
/// — exact for any linear map, including shear, and strictly dominates a
/// bounding-sphere route (no sqrt, no √3 circumscription loss, and a sphere via
/// max-column-norm underestimates under shear).
///
/// # Cost
///
/// O(instances + batches), cold, no allocation. One `Option`/inverted-box branch per
/// BATCH; the per-instance inner loop is branch-free (`min`/`max`/`abs` only).
/// The Arvo abs-matrix transform of a local AABB — centre `lc`, half-extent `lh` — through one
/// instance's row-major 3×4 affine. Returns `(world_centre, world_half_extent)`.
///
/// `wc[r] = Σⱼ rows[r][j]·lc[j] + rows[r][3]` and `wh[r] = Σⱼ |rows[r][j]|·lh[j]`: exact for any
/// linear map INCLUDING shear, and strictly better than a bounding-sphere route (no `sqrt`, no √3
/// circumscription loss, and a sphere via max-column-norm underestimates under shear). Branch-free
/// — `abs`/`+`/`*` only.
///
/// # Why this is a shared primitive
///
/// Two consumers now fold the SAME transform to different shapes: [`reduce_bounds_into`] unions it
/// across every instance of every caster batch (and takes its depth extreme PER INSTANCE — the D4
/// fix), while [`batch_world_aabb`] unions it within ONE batch for the VG rung-R2c draw cull. The
/// FOLDS legitimately differ; the TRANSFORM must not. Two hand-copies of this arithmetic that drift
/// by one `abs` would put the shadow fit and the draw cull on different geometry — a class this
/// repository has already paid for elsewhere — so the arithmetic lives here once.
#[inline]
#[must_use]
pub fn arvo_transform(rows: &[[f32; 4]; 3], lc: [f32; 3], lh: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let mut wc = [0.0f32; 3];
    let mut wh = [0.0f32; 3];
    for r in 0..3 {
        let row = rows[r];
        wc[r] = row[0] * lc[0] + row[1] * lc[1] + row[2] * lc[2] + row[3];
        wh[r] = row[0].abs() * lh[0] + row[1].abs() * lh[1] + row[2].abs() * lh[2];
    }
    (wc, wh)
}

/// VG rung R2c: ONE batch's world-space AABB — the union of [`arvo_transform`] over that batch's
/// slice of the instance ring. `None` when the batch has no instances, or when `mesh_aabb` is the
/// C0 zero-vertex sentinel (an INVERTED box, `min > max`), which is skipped for the same reason
/// [`reduce_bounds_into`] skips it: its centre is NaN and it would poison the fold.
///
/// This is the DRAW cull's geometry, and its error direction is fixed by construction: the returned
/// box CONTAINS every vertex the batch can rasterize, so a frustum test that rejects only a box
/// wholly outside can never cull something visible. Over-inclusion costs a wasted draw.
///
/// # Panics / bounds
///
/// Debug-asserts that the batch's `[base_instance, base_instance + instance_count)` range fits
/// `ring`; in release an out-of-range batch returns `None` rather than reading past the slice.
#[must_use]
pub fn batch_world_aabb(
    batch: &DrawBatch,
    ring: &[InstanceModelCol],
    mesh_aabb: ([f32; 3], [f32; 3]),
) -> Option<([f32; 3], [f32; 3])> {
    let (mn, mx) = mesh_aabb;
    if mn[0] > mx[0] || mn[1] > mx[1] || mn[2] > mx[2] {
        return None;
    }
    let base = batch.base_instance as usize;
    let count = batch.instance_count as usize;
    debug_assert!(
        base.saturating_add(count) <= ring.len(),
        "batch_world_aabb: batch range [{base}, {}) exceeds the ring's {} instances",
        base + count,
        ring.len()
    );
    if count == 0 || base.saturating_add(count) > ring.len() {
        return None;
    }

    let lc = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
    let lh = [(mx[0] - mn[0]) * 0.5, (mx[1] - mn[1]) * 0.5, (mx[2] - mn[2]) * 0.5];

    let mut world_min = [f32::INFINITY; 3];
    let mut world_max = [f32::NEG_INFINITY; 3];
    for inst in &ring[base..base + count] {
        let (wc, wh) = arvo_transform(&inst.rows, lc, lh);
        for r in 0..3 {
            world_min[r] = world_min[r].min(wc[r] - wh[r]);
            world_max[r] = world_max[r].max(wc[r] + wh[r]);
        }
    }
    Some((world_min, world_max))
}

pub fn reduce_bounds_into(
    batches: &[DrawBatch],
    ring: &[InstanceModelCol],
    eye: [f32; 3],
    forward: [f32; 3],
    mesh_aabb: impl Fn(u32) -> Option<([f32; 3], [f32; 3])>,
) -> CsmCasterBounds {
    let total_batches = batches.len() as u32;
    let mut resolved_batches: u32 = 0;
    let mut raw_far = f32::NEG_INFINITY;
    let mut world_min = [f32::INFINITY; 3];
    let mut world_max = [f32::NEG_INFINITY; 3];

    for batch in batches {
        let Some((mn, mx)) = mesh_aabb(batch.mesh_id) else {
            continue; // not yet Loaded (F6 invariant) — skip, do not count as resolved.
        };
        // C0's zero-vertex sentinel is an INVERTED box (min > max on every axis); its
        // centre is NaN, so it is skipped exactly like a non-Loaded slot instead of
        // poisoning the fold.
        if mn[0] > mx[0] || mn[1] > mx[1] || mn[2] > mx[2] {
            continue;
        }
        resolved_batches += 1;

        let lc = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
        let lh = [(mx[0] - mn[0]) * 0.5, (mx[1] - mn[1]) * 0.5, (mx[2] - mn[2]) * 0.5];

        let base = batch.base_instance as usize;
        let count = batch.instance_count as usize;
        debug_assert!(
            base + count <= ring.len(),
            "reduce_bounds_into: batch range [{base}, {}) exceeds the ring's {} instances",
            base + count,
            ring.len()
        );

        for inst in &ring[base..base + count] {
            let (wc, wh) = arvo_transform(&inst.rows, lc, lh);

            for r in 0..3 {
                world_min[r] = world_min[r].min(wc[r] - wh[r]);
                world_max[r] = world_max[r].max(wc[r] + wh[r]);
            }

            // The per-instance view-space depth extreme along `forward` — the D4 fix:
            // this is `max`'d PER INSTANCE, so a laterally spread caster set can never
            // inflate `raw_far` the way projecting the union AABB would.
            let d_center = forward[0] * (wc[0] - eye[0])
                + forward[1] * (wc[1] - eye[1])
                + forward[2] * (wc[2] - eye[2]);
            let d_half =
                forward[0].abs() * wh[0] + forward[1].abs() * wh[1] + forward[2].abs() * wh[2];
            raw_far = raw_far.max(d_center + d_half);
        }
    }

    if resolved_batches == 0 {
        return CsmCasterBounds { total_batches, ..CsmCasterBounds::EMPTY };
    }

    CsmCasterBounds { raw_far, world_min, world_max, resolved_batches, total_batches }
}

/// The cold caster-bounds fold SYSTEM (`docs/CSM-AUTOFIT-PLAN.md` rung C2/C3) — folds
/// [`CsmCasterScratch`]'s `batches()` + `ring()` (the shadow-caster gather's OUTPUT, NOT
/// a second query — D7) into [`CsmCasterBounds`] via [`reduce_bounds_into`].
///
/// # The `fit_mode` 0%-gate (algorithm A step 1, rung C3)
///
/// Under the default [`CsmFitMode::Fixed`] the fold never runs: `out` is written to
/// [`CsmCasterBounds::EMPTY`] and the per-instance abs-matrix walk is skipped entirely —
/// the same 0-ns default [`CsmConfig`] already guarantees for
/// [`resolve_csm_cascades`](crate::csm_config::resolve_csm_cascades)'s side of the gate.
///
/// # Registration — unwired-API (matches [`gather_shadow_casters`])
///
/// This system is NOT registered in [`CsmPlugin`](crate::csm_plugin::CsmPlugin) (nor any
/// plugin), exactly as [`gather_shadow_casters`] is an unwired exported API: it requires
/// the world's `Assets<MeshGpu>` `NonSend` resource and [`CsmCasterScratch`] (itself
/// unwired — the owning app inserts + populates it), so the owning app co-registers this
/// system `.after(gather_shadow_casters)` and `.in_set(CsmFitSet)` when it wires the real
/// CSM caster path (rung C5).
///
/// **Without registration, [`CsmCasterBounds`] never leaves the
/// [`CsmPlugin`](crate::csm_plugin::CsmPlugin)-inserted [`CsmCasterBounds::EMPTY`]** —
/// nothing folds it, so it can never become `is_usable()`, so the fit (rung C3) never
/// latches, so every non-`Fixed` mode renders as `Fixed`: today's picture, silently, at
/// zero cost.
///
/// # `NonSend`
///
/// Reads `NonSendRes<Assets<MeshGpu>>` (the same class [`gather_shadow_casters`] reads),
/// so this system runs main-thread-only. The pin does NOT propagate to the fit: it reads
/// only `Res<CsmCasterBounds>`, staying thread-agnostic (D7, `docs/CSM-AUTOFIT-PLAN.md`
/// §7).
#[allow(clippy::needless_pass_by_value)]
pub fn reduce_caster_bounds(
    cfg: Res<CsmConfig>,
    view: Res<ViewUniform>,
    scratch: Res<CsmCasterScratch>,
    mesh_assets: NonSendRes<Assets<MeshGpu>>,
    mut out: ResMut<CsmCasterBounds>,
) {
    if cfg.fit_mode == CsmFitMode::Fixed {
        *out = CsmCasterBounds::EMPTY;
        return;
    }

    let eye = view.camera_pos.xyz();
    let forward = view.cam_forward.xyz();
    *out = reduce_bounds_into(
        scratch.batches(),
        scratch.ring(),
        [eye.x, eye.y, eye.z],
        [forward.x, forward.y, forward.z],
        // F6 invariant: never dereference a non-Loaded slot (mirrors
        // gather_shadow_casters's own try_get above).
        |mesh_id| {
            let m = mesh_assets.try_get(MeshHandle(mesh_id))?;
            Some((m.local_min, m.local_max))
        },
    );
}

/// Keeps the light-header CSM sample gate ([`LightingConfig::csm_shadows`] → header
/// word 7 bit [`CSM_MODE_BIT`](crate::light::CSM_MODE_BIT)) in LOCK-STEP with the
/// cascade depth-pass activation predicate (host plan R4):
///
/// ```text
/// gate = ResolvedCsm.csm_mode_word == 1  AND  CsmCasterScratch has >= 1 caster batch
/// ```
///
/// which is EXACTLY the predicate the windowed host arms `GBufferScene::csm` with —
/// one predicate, two consumers, no drift. On a flip the light table is marked dirty
/// ([`LightTableDirty`]) so `collect_lights` rebuilds the header with the new gate word
/// and the staged-table generation advances (the host re-uploads both ring slots).
///
/// # Why the lock-step is layout-sound under ordering staggers (review R4-W1)
///
/// This system's ordering against `resolve_csm_cascades` / `collect_lights` is
/// registration-site-dependent (cross-plugin edges are not expressible), so the header
/// gate can lag the predicate by a frame in EITHER direction — and because this
/// system's `ResolvedCsm` term can itself be one frame stale, a multi-coincidence
/// exists (the sun unfits exactly as casters first appear, latching the gate ON from
/// stale terms; then re-fits exactly as they vanish, inside the header-flip lag) in
/// which the resolve sees the gate ON and an armed cascade UBO on a frame whose depth
/// pass did not record — in the extreme, on a stream where it NEVER recorded.
/// Soundness therefore does NOT rest on this system's timing; it rests on two host
/// guarantees:
///
/// 1. **The never-rendered class is closed by the BOOT LAYOUT**: the windowed host
///    one-shot-transitions the cascade array (and shadow atlas) to
///    `SHADER_READ_ONLY_OPTIMAL` at scene boot, so a gate-ON resolve on a stream
///    where the depth pass never ran samples undefined VALUES at a DEFINED layout —
///    a benign 1–2 frame shadow transient, never an invalid access.
/// 2. **Stale divergence (1–2 frames) is benign**: the host uploads the CURRENT
///    `ResolvedCsm` into the fenced cascade-UBO slot every frame, so a DISABLED fit
///    reaches the resolve as `active_count == 0` (the shader's early-out — no sample
///    at all), and a stale-armed fit samples a valid-layout cascade whose content is
///    at worst one re-render old.
///
/// The gate/dirty mechanics below therefore only bound WHEN the header bit flips
/// (within 1–2 frames of the predicate), not the safety of any interleaving.
///
/// # Value-gated write
///
/// `cfg.csm_shadows` is written only on an actual flip, so a static frame does zero
/// work and never dirties the light table.
///
/// # Registration — app-wired (matches [`gather_shadow_casters`])
///
/// NOT registered by any plugin here: it bridges `CsmPlugin`'s [`ResolvedCsm`] and
/// `LightingPlugin`'s [`LightingConfig`] / [`LightTableDirty`], so only the composing
/// app (which adds BOTH plugins) may register it — `.after(gather_shadow_casters)` in
/// the same builder closure, so the caster half of the predicate is this frame's.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_csm_light_gate(
    resolved: Res<ResolvedCsm>,
    casters: Res<CsmCasterScratch>,
    mut cfg: ResMut<LightingConfig>,
    mut dirty: ResMut<LightTableDirty>,
) {
    let on = resolved.csm_mode_word == 1 && casters.batch_count() > 0;
    // Value gate BEFORE the `DerefMut`: flip-only write, flip-only table dirtying.
    if cfg.csm_shadows != on {
        cfg.csm_shadows = on;
        dirty.0 = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_rhi::enums::IndexType;

    /// A distinct-per-instance affine whose translation encodes `(mesh_id, ordinal)` so
    /// a misplaced scatter is detectable by value (mirrors the `mesh_draw` test scaffold).
    fn affine(mesh_id: u32, ordinal: u32) -> InstanceModelCol {
        InstanceModelCol {
            rows: [
                [1.0, 0.0, 0.0, mesh_id as f32],
                [0.0, 1.0, 0.0, ordinal as f32],
                [0.0, 0.0, 1.0, 0.0],
            ],
        }
    }

    /// A fake registry `meta`: mesh `m` has `index_count = 6 * (m + 1)` and alternating
    /// index width — identical to the `mesh_draw` scaffold so the caster gather is proven
    /// to carry the same O3 mixed-width batch fields.
    fn meta(mesh_id: u32) -> Option<(u32, IndexType)> {
        let width = if mesh_id.is_multiple_of(2) {
            IndexType::Uint16
        } else {
            IndexType::Uint32
        };
        Some((6 * (mesh_id + 1), width))
    }

    /// One `(mesh_id, &InstanceModelCol)` input plus its `is_caster` structural flag —
    /// the test stands in for the `With<ShadowCaster>` archetypal filter by emitting ONLY
    /// the casters into the re-iteration closure (the gather never sees a non-caster row,
    /// exactly as the real query's filter excludes it at iteration).
    struct Row {
        mesh_id: u32,
        col: InstanceModelCol,
        is_caster: bool,
    }

    /// Runs the SAME unified gather core the system runs, fed ONLY the rows whose
    /// `is_caster` is set — the CPU mirror of `Query<.., With<ShadowCaster>>`. All-static
    /// (casters have no interpolation pair), so every row takes the `None` branch.
    fn gather_casters(scratch: &mut CsmCasterScratch, mesh_count: usize, rows: &[Row]) {
        scratch.0.gather_mixed_into(mesh_count, meta, || {
            rows.iter()
                .filter(|r| r.is_caster)
                .map(|r| (r.mesh_id, &r.col, None, PerInstanceMaterial::default()))
        });
    }

    /// THE Inc-2 gate: a mixed set where SOME entities carry `ShadowCaster` and some do
    /// not. The gather must produce caster batches for ONLY the casters — the non-casters
    /// are EXCLUDED by the structural filter — with correct
    /// `base_instance`/`instance_count`/contiguity and one batch per caster mesh.
    #[test]
    fn casters_only_excludes_non_casters_contiguous() {
        // Mesh 0: 2 casters + 1 non-caster.  Mesh 1: 1 caster + 1 non-caster.
        // Mesh 2: 0 casters (1 non-caster only) — it must produce NO batch.
        let m0c0 = affine(0, 0);
        let m0c1 = affine(0, 1);
        let m0n = affine(0, 99); // non-caster — must be excluded
        let m1c0 = affine(1, 0);
        let m1n = affine(1, 99); // non-caster — must be excluded
        let m2n = affine(2, 99); // non-caster on a mesh with NO casters — no batch

        // Interleave casters + non-casters so the scatter (not the input order) produces
        // contiguous buckets, and a non-caster between casters cannot leak into a bucket.
        let rows = [
            Row { mesh_id: 0, col: m0c0, is_caster: true },
            Row { mesh_id: 0, col: m0n, is_caster: false },
            Row { mesh_id: 1, col: m1c0, is_caster: true },
            Row { mesh_id: 2, col: m2n, is_caster: false },
            Row { mesh_id: 0, col: m0c1, is_caster: true },
            Row { mesh_id: 1, col: m1n, is_caster: false },
        ];

        let mut scratch = CsmCasterScratch::default();
        gather_casters(&mut scratch, 3, &rows);

        // Only meshes 0 and 1 have casters => exactly 2 batches (mesh 2 excluded — it has
        // only a non-caster instance).
        assert_eq!(
            scratch.batch_count(),
            2,
            "only ShadowCaster meshes produce batches; the non-caster-only mesh is excluded"
        );

        // Batch 0 = caster mesh 0: base 0, 2 caster instances (the non-caster NOT counted).
        let b0 = scratch.batches()[0];
        assert_eq!(b0.mesh_id, 0);
        assert_eq!(b0.base_instance, 0, "first caster bucket => base 0");
        assert_eq!(b0.instance_count, 2, "2 casters of mesh 0 (the non-caster excluded)");
        assert_eq!(b0.index_count, 6);
        assert_eq!(b0.index_type, IndexType::Uint16);

        // Batch 1 = caster mesh 1: base == count(mesh-0 casters) == 2 (NONZERO), 1 caster.
        let b1 = scratch.batches()[1];
        assert_eq!(b1.mesh_id, 1);
        assert_eq!(b1.base_instance, 2, "mesh 1's base == caster count of mesh 0 == 2");
        assert_eq!(b1.instance_count, 1, "1 caster of mesh 1 (its non-caster excluded)");
        assert_eq!(b1.index_count, 12);
        assert_eq!(b1.index_type, IndexType::Uint32);

        // The caster ring holds exactly the 3 casters (no non-caster value present).
        assert_eq!(scratch.instance_count(), 3, "the ring holds only the 3 casters");
        // Mesh 0's 2 casters contiguous at [0..2), mesh 1's 1 caster at [2..3) — each slot
        // holds the EXPECTED caster (its translation encodes (mesh, ord)), proving no
        // non-caster (ordinal 99) leaked in.
        assert_eq!(scratch.ring()[0], affine(0, 0));
        assert_eq!(scratch.ring()[1], affine(0, 1));
        assert_eq!(scratch.ring()[2], affine(1, 0));
        // The excluded non-caster value (ordinal 99) is nowhere in the ring.
        assert!(
            !scratch.ring().contains(&affine(0, 99))
                && !scratch.ring().contains(&affine(1, 99))
                && !scratch.ring().contains(&affine(2, 99)),
            "no non-caster instance leaked into the caster ring"
        );
    }

    /// A world with ZERO casters (every entity is a non-caster) emits zero caster batches
    /// + an empty ring — the depth-pass 0%-gate (byte-identical to a CSM-disabled frame).
    #[test]
    fn no_casters_yields_no_batches() {
        let n0 = affine(0, 0);
        let n1 = affine(1, 0);
        let rows = [
            Row { mesh_id: 0, col: n0, is_caster: false },
            Row { mesh_id: 1, col: n1, is_caster: false },
        ];
        let mut scratch = CsmCasterScratch::default();
        gather_casters(&mut scratch, 2, &rows);
        assert_eq!(scratch.batch_count(), 0, "no ShadowCaster => no caster batches");
        assert_eq!(scratch.instance_count(), 0, "no ShadowCaster => empty caster ring");
    }

    /// Every entity is a caster — the caster gather degenerates to the SAME result the
    /// main `gather_mesh_draws` would produce (the `With<ShadowCaster>` term is a no-op
    /// when all rows carry the marker), proving the reuse of `gather_mixed_into` is faithful.
    #[test]
    fn all_casters_matches_unfiltered_gather() {
        let a0 = affine(0, 0);
        let a1 = affine(0, 1);
        let b0 = affine(1, 0);
        let rows = [
            Row { mesh_id: 0, col: a0, is_caster: true },
            Row { mesh_id: 1, col: b0, is_caster: true },
            Row { mesh_id: 0, col: a1, is_caster: true },
        ];

        let mut casters = CsmCasterScratch::default();
        gather_casters(&mut casters, 2, &rows);

        // The same inputs through the foundation's unified gather directly (no filter).
        let mut main = MeshRenderScratch::default();
        main.gather_mixed_into(2, meta, || {
            rows.iter().map(|r| (r.mesh_id, &r.col, None, PerInstanceMaterial::default()))
        });

        assert_eq!(casters.batch_count(), main.batch_count());
        assert_eq!(casters.batches(), main.batches.as_read_slice());
        assert_eq!(casters.ring(), main.ring.as_read_slice());
    }

    /// Re-running the caster gather REUSES the scratch's capacity (Principle 5): a large
    /// frame then a smaller one yields the correct smaller result without losing the
    /// reserved ring capacity.
    #[test]
    fn caster_gather_reuses_capacity_across_frames() {
        let mut scratch = CsmCasterScratch::default();

        // Frame 1: 4 casters across 2 meshes.
        let big: Vec<InstanceModelCol> = (0..4).map(|i| affine(i % 2, i)).collect();
        let big_rows: Vec<Row> = big
            .iter()
            .enumerate()
            .map(|(i, &c)| Row { mesh_id: (i as u32) % 2, col: c, is_caster: true })
            .collect();
        gather_casters(&mut scratch, 2, &big_rows);
        assert_eq!(scratch.instance_count(), 4);
        let ring_cap_after_big = scratch.0.ring.capacity();

        // Frame 2: 1 caster of mesh 0.
        let small = affine(0, 0);
        let small_rows = [Row { mesh_id: 0, col: small, is_caster: true }];
        gather_casters(&mut scratch, 2, &small_rows);
        assert_eq!(scratch.batch_count(), 1);
        assert_eq!(scratch.instance_count(), 1);
        assert!(
            scratch.0.ring.capacity() >= ring_cap_after_big,
            "the caster ring retains its reserved capacity across a smaller frame"
        );
    }

    // ---- reduce_bounds_into (rung C2, docs/CSM-AUTOFIT-PLAN.md) -----------------------

    /// An identity-rotation, unit-scale instance at world translation `t` — the caster-
    /// bounds analogue of [`affine`] (no mesh-id/ordinal encoding needed here; only the
    /// world position matters).
    fn identity_instance_at(t: [f32; 3]) -> InstanceModelCol {
        InstanceModelCol {
            rows: [
                [1.0, 0.0, 0.0, t[0]],
                [0.0, 1.0, 0.0, t[1]],
                [0.0, 0.0, 1.0, t[2]],
            ],
        }
    }

    /// T13 — a batch whose mesh has not resolved `Loaded` (`mesh_aabb -> None`, the F6
    /// invariant) is SKIPPED: it must not be counted as resolved, and the fold must not
    /// panic.
    #[test]
    fn reduce_skips_non_loaded_mesh() {
        let batches = [
            DrawBatch {
                mesh_id: 0,
                index_count: 6,
                index_type: IndexType::Uint16,
                base_instance: 0,
                instance_count: 1,
            },
            DrawBatch {
                mesh_id: 1,
                index_count: 6,
                index_type: IndexType::Uint16,
                base_instance: 1,
                instance_count: 1,
            },
        ];
        let ring = [
            identity_instance_at([0.0, 0.0, 0.0]),
            identity_instance_at([0.0, 0.0, 5.0]),
        ];

        // Mesh 0 resolves; mesh 1 simulates a mesh still streaming in (not yet Loaded).
        let bounds = reduce_bounds_into(&batches, &ring, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], |mesh_id| {
            if mesh_id == 0 {
                Some(([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]))
            } else {
                None
            }
        });

        assert_eq!(bounds.total_batches, 2, "the gather emitted 2 batches this frame");
        assert_eq!(
            bounds.resolved_batches, 1,
            "the non-Loaded mesh's batch must be skipped, not counted as resolved"
        );
        assert!(
            !bounds.is_usable(),
            "an incomplete fold (resolved < total) must not be usable as a fit input"
        );
    }

    /// The C0 zero-vertex sentinel is an INVERTED box (`local_min = [+inf;3]`, `local_max
    /// = [-inf;3]`) — its centre computes to NaN. `reduce_bounds_into` must treat it
    /// EXACTLY like a non-Loaded slot (skip, do not count as resolved), never dereference
    /// it into a NaN-poisoned fold. Reachable in practice: `tests/asset_streaming_f5_validation.rs`
    /// constructs such a dummy `MeshGpu`.
    #[test]
    fn reduce_skips_inverted_box_like_non_loaded_mesh() {
        let batches = [DrawBatch {
            mesh_id: 0,
            index_count: 6,
            index_type: IndexType::Uint16,
            base_instance: 0,
            instance_count: 1,
        }];
        let ring = [identity_instance_at([1.0, 2.0, 3.0])];

        let bounds = reduce_bounds_into(&batches, &ring, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], |_| {
            Some(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]))
        });

        assert_eq!(bounds.total_batches, 1);
        assert_eq!(
            bounds.resolved_batches, 0,
            "an inverted (zero-vertex sentinel) box must be skipped, not resolved"
        );
        assert!(!bounds.is_usable());
        assert_eq!(
            bounds.raw_far, 0.0,
            "an all-skipped fold must equal EMPTY, not a NaN-poisoned value"
        );
        assert!(bounds.raw_far.is_finite());
        assert!(bounds.world_min.iter().all(|v| v.is_finite()));
        assert!(bounds.world_max.iter().all(|v| v.is_finite()));
    }

    /// T20 — D4's exactness/conservativeness: a rotated, non-uniformly-scaled, AND
    /// SHEARED instance (not a pure rotation/scale, so a bounding-sphere route would
    /// underestimate). The abs-matrix (Arvo) transform must produce a world AABB that
    /// contains all 8 manually-transformed local-box corners.
    #[test]
    fn reduce_matches_manual_transform_for_sheared_instance() {
        let a = [[1.5_f32, 0.6, -0.3], [-0.2, 2.0, 0.4], [0.1, -0.5, 0.8]];
        let t = [3.0_f32, -2.0, 5.0];

        let inst = InstanceModelCol {
            rows: [
                [a[0][0], a[0][1], a[0][2], t[0]],
                [a[1][0], a[1][1], a[1][2], t[1]],
                [a[2][0], a[2][1], a[2][2], t[2]],
            ],
        };

        let local_min = [-1.0_f32, -2.0, -0.5];
        let local_max = [3.0_f32, 1.0, 2.0];

        let batches = [DrawBatch {
            mesh_id: 0,
            index_count: 6,
            index_type: IndexType::Uint16,
            base_instance: 0,
            instance_count: 1,
        }];
        let ring = [inst];

        let bounds = reduce_bounds_into(&batches, &ring, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], |_| {
            Some((local_min, local_max))
        });

        const EPS: f32 = 1.0e-4;
        for &lx in &[local_min[0], local_max[0]] {
            for &ly in &[local_min[1], local_max[1]] {
                for &lz in &[local_min[2], local_max[2]] {
                    let w = [
                        a[0][0] * lx + a[0][1] * ly + a[0][2] * lz + t[0],
                        a[1][0] * lx + a[1][1] * ly + a[1][2] * lz + t[1],
                        a[2][0] * lx + a[2][1] * ly + a[2][2] * lz + t[2],
                    ];
                    for r in 0..3 {
                        assert!(
                            w[r] >= bounds.world_min[r] - EPS && w[r] <= bounds.world_max[r] + EPS,
                            "corner ({lx},{ly},{lz}) world coord {w:?} axis {r} must land \
                             inside [world_min, world_max] = [{:?}, {:?}]",
                            bounds.world_min,
                            bounds.world_max
                        );
                    }
                }
            }
        }
    }

    /// T21 — the union-AABB error (D). Two caster instances at the SAME view-space depth
    /// (~3) but opposite lateral extremes (`world x = +-50`), viewed along an oblique
    /// `forward` with `|fwd.x| = 0.5`. Projecting a UNION AABB would add ~`0.5 * 50 = 25`
    /// of spurious depth (raw_far -> ~28); the per-instance reduction must not.
    #[test]
    fn laterally_spread_casters_do_not_inflate_raw_far() {
        let fwd = [0.5_f32, 0.0, -(0.75_f32).sqrt()];
        let target_depth = 3.0_f32;
        // Solve for the z that places each instance at exactly `target_depth` along `fwd`
        // from the origin eye, so the ONLY thing that differs between the two instances
        // is their lateral (x) position.
        let z_for = |x: f32| (target_depth - fwd[0] * x) / fwd[2];

        let xa = 50.0_f32;
        let xb = -50.0_f32;
        let inst_a = identity_instance_at([xa, 0.0, z_for(xa)]);
        let inst_b = identity_instance_at([xb, 0.0, z_for(xb)]);

        let batches = [
            DrawBatch {
                mesh_id: 0,
                index_count: 6,
                index_type: IndexType::Uint16,
                base_instance: 0,
                instance_count: 2,
            },
        ];
        let ring = [inst_a, inst_b];
        // A small local box so d_half is negligible next to the depth assertion's tolerance.
        let local_min = [-0.05_f32; 3];
        let local_max = [0.05_f32; 3];

        let bounds =
            reduce_bounds_into(&batches, &ring, [0.0, 0.0, 0.0], fwd, |_| Some((local_min, local_max)));

        assert!(
            (bounds.raw_far - target_depth).abs() < 0.5,
            "raw_far ({}) must stay near the true per-instance depth (~{target_depth}), \
             not the union-AABB error (~28)",
            bounds.raw_far
        );
        assert!(
            bounds.raw_far < 15.0,
            "raw_far ({}) must not inflate toward the union-AABB projection (~28)",
            bounds.raw_far
        );
    }
}
