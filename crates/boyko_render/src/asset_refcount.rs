//! [`apply_refcount_deltas`] — the per-frame fold from
//! [`RefcountDeltas`](boyko_scene::RefcountDeltas) (pushed by the
//! `MeshHandle`/`MaterialHandle` carrier hooks in `boyko_scene::render_caps`)
//! into the two GPU asset tables (asset-streaming plan F2 §1/§3, gen-checked
//! as of F5), plus [`validate_asset_refs`](crate::asset_refcount::validate_asset_refs)
//! (the two-way [`RenderStale`] / [`MaterialStale`] staleness oracle, decoupled
//! from user visibility and pinned before every gather via [`AssetValidateSet`]),
//! [`retire_deferred_frees`] (F6's fence-gated device-free drain), and the
//! [`AssetRefcountPlugin`] that wires the resources + both systems into the
//! app schedule.

use boyko_ecs::ecs::core::app::{App, Plugin};
use boyko_ecs::ecs::core::asset::{AssetBacking, AssetLoadState, Assets, GEN_UNSYNCED};
use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::filter_enable::{Disabled, Enabled};
use boyko_ecs::ecs::core::iters::query::{Query, Without};
use boyko_ecs::ecs::core::system::{Commands, NonSendRes, NonSendResMut, Res, ResMut};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Component, Resource, SystemSet};
use boyko_rhi::RhiDevice;
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::swapchain::FRAMES_IN_FLIGHT;
use boyko_scene::{
    AssetRefKind, DeferredFree, FreeEntry, MaterialHandle, MaterialRefGen, MeshHandle, MeshRefGen,
    RefcountDeltas,
};

use crate::bindless::BindlessTextureTable;
use crate::material::Material;
use crate::mesh::MeshGpu;
use crate::mesh_assets::OrphanedMeshGpu;
use crate::retired_gpu_buffers::RetiredGpuBuffers;
use crate::texture::OrphanedTextureGpu;

/// The fence-gated retire delay (asset-streaming plan F6 Decision 1): a row
/// enqueued at submission-epoch `N` is safe to free once the host observes
/// `epoch >= N + RETIRE_DELAY`. Pinned to [`FRAMES_IN_FLIGHT`] — the exact
/// fence horizon `wait_frame_in_flight` guarantees (see
/// [`retire_deferred_frees`]'s doc for the full proof). If the TLAS ever
/// becomes persistent/compacted across frames, or an async-compute queue with
/// an independent fence references BLAS/buffers, `RETIRE_DELAY` MUST grow
/// beyond `FRAMES_IN_FLIGHT` to match the new horizon.
pub const RETIRE_DELAY: u64 = FRAMES_IN_FLIGHT as u64;

/// The host-published fence clock (asset-streaming plan F6 Decision 1):
/// [`Renderer::submission_epoch`](boyko_rhi_vulkan::swapchain::Renderer::submission_epoch)
/// mirrored into a world resource BEFORE `app.update_with_delta` each frame,
/// so [`apply_refcount_deltas`] can stamp a real
/// `retire_frame = epoch + `[`RETIRE_DELAY`] on every newly-`Retiring` row —
/// counts committed GPU submits (the ring-advance site), NOT the runner's
/// jitter `frame_index` (which can advance on a pre-acquire recreate-skip
/// where nothing submitted, over-counting relative to true GPU progress).
#[derive(Debug, Default, Clone, Copy, Resource)]
pub struct RenderEpoch(pub u64);

/// Drains [`RefcountDeltas`] and folds each delta into the matching
/// `Assets<T>` table's refcount (asset-streaming plan F2 §1, gen-checked as of
/// F5): `+1` calls [`Assets::inc_ref`] and (regardless of its result — see
/// `apply_one`'s doc) re-syncs the carrier's `MeshRefGen`/`MaterialRefGen`
/// lane via a deferred, generation-checked `SyncRefGenCommand` (churn-safe —
/// see that command's doc for why a plain `Commands::entity(...).insert(...)`
/// is unsound here); `-1` calls [`Assets::dec_ref`] with the delta's captured
/// bind-generation. A `dec_ref` that returns a retire ticket is enqueued into
/// [`DeferredFree`] with a placeholder `retire_frame = 0` (F6 sets the real
/// fence-gated value; F5 only enqueues — nothing drains this queue yet).
///
/// Any other `delta` magnitude is unreachable from the hook wiring (every
/// pushed [`RefDelta`](boyko_scene::RefDelta) is `+1` or `-1`); a `debug_assert`
/// catches a future hook regression without costing anything in release.
///
/// # Schedule placement
///
/// Registered by [`AssetRefcountPlugin`] `.before(validate_asset_refs)` (both
/// systems share one [`App::add_systems_cfg`] closure, so the edge is
/// expressible — see the plugin's doc for how the further edge to the mesh/CSM
/// gathers is pinned by name through [`AssetValidateSet`]). The lane writes ride `Commands`,
/// flushed by the PER-SYSTEM apply window immediately after this system's body
/// returns (`boyko_ecs::ecs::core::schedule::schedule::Schedule::run`'s
/// dispatch loop calls `system.apply(world)` right after each system
/// completes — the concurrent-dispatch site at `schedule.rs:724` and the
/// solo/dispatcher-exclusive site at `schedule.rs:1130` both do this; NEITHER
/// batches every system's commands into one end-of-frame barrier), so
/// `validate_asset_refs` always observes THIS frame's lane writes. A future
/// stage-boundary-flush refactor (batching `Commands` across a whole stage
/// before any apply) would break this same-frame lane visibility — anyone
/// touching that dispatch loop should re-verify this contract.
pub fn apply_refcount_deltas(
    mut deltas: ResMut<RefcountDeltas>,
    mut free: ResMut<DeferredFree>,
    mut material_assets: ResMut<Assets<Material>>,
    mut mesh_assets: NonSendResMut<Assets<MeshGpu>>,
    epoch: Res<RenderEpoch>,
    mut cmd: Commands,
) {
    if deltas.is_empty() {
        return;
    }
    let epoch = epoch.0;
    for delta in deltas.drain() {
        match delta.kind {
            AssetRefKind::Mesh => {
                if let Some(g) = apply_one(
                    &mut mesh_assets,
                    &mut free,
                    delta.kind,
                    delta.slot,
                    delta.gen_,
                    delta.delta,
                    epoch,
                ) {
                    cmd.add(SyncRefGenCommand { entity: delta.entity, kind: delta.kind, gen_: g });
                }
            }
            AssetRefKind::Material => {
                if let Some(g) = apply_one(
                    &mut material_assets,
                    &mut free,
                    delta.kind,
                    delta.slot,
                    delta.gen_,
                    delta.delta,
                    epoch,
                ) {
                    cmd.add(SyncRefGenCommand { entity: delta.entity, kind: delta.kind, gen_: g });
                }
            }
        }
    }
}

/// Folds one delta into `assets`, routing a resulting retire ticket into
/// `free`. Generic over the two concrete `AssetBacking` types
/// (`MeshGpu`/`Material`) so [`apply_refcount_deltas`] shares one body for
/// both branches — monomorphized, no dynamic dispatch. Returns `Some(gen)` on
/// a `+1` delta (the attach-time generation the caller stamps into the
/// type-specific lane — this function cannot name `MeshRefGen`/
/// `MaterialRefGen` itself, since both branches share one generic body);
/// `None` on a `-1` delta (nothing to stamp) or an unreachable magnitude.
///
/// # Unconditional lane stamp on a refused `+1` (F5 blocker fix)
///
/// [`Assets::inc_ref`] refuses (returns `false`, no mutation) a carrier that
/// binds an already-`Retiring` slot — the sole resurrection hazard (see its
/// doc). This function returns `Some(generation)` regardless of that bool.
/// Leaving the lane at [`GEN_UNSYNCED`] instead would (a) make
/// `validate_asset_refs` SKIP the carrier (it trusts `GEN_UNSYNCED` as
/// "freshly bound"), never disabling a carrier that in fact bound a dead
/// slot, AND (b) make the carrier's EVENTUAL `-1` decrement also carry
/// `GEN_UNSYNCED`, bypassing `dec_ref`'s gen-check and corrupting whichever
/// tenant has since reused the slot. Stamping unconditionally closes both:
/// `validate_asset_refs` sees `state_of_index != Loaded` and disables the
/// carrier; the eventual `-1` carries the ATTACH-time generation, which
/// mismatches the reused slot's current one, so `dec_ref` suppresses it. The
/// slot's real refcount still never rose (`inc_ref` refused) — the F5/F6
/// boundary stays airtight (see `Assets::inc_ref`'s doc for the full
/// argument). `try_generation` (not the panicking `generation`) guards the
/// OOR case — a malformed carrier holding a never-minted index must not panic.
///
/// # `retire_frame` stamp (asset-streaming plan F6)
///
/// A `-1` delta that reaches a real zero-crossing enqueues a [`FreeEntry`]
/// stamped `retire_frame = epoch + `[`RETIRE_DELAY`] — `epoch` is this
/// frame's [`RenderEpoch`] (the submission-epoch observed BEFORE this frame's
/// submit), so the row is fence-safe to free once the host later observes
/// `epoch' >= epoch + RETIRE_DELAY` (see [`retire_deferred_frees`]'s doc for
/// the full fence-gate proof).
#[inline]
fn apply_one<T: AssetBacking>(
    assets: &mut Assets<T>,
    free: &mut DeferredFree,
    kind: AssetRefKind,
    slot: u32,
    gen_: u32,
    delta: i32,
    epoch: u64,
) -> Option<u32> {
    match delta {
        1 => {
            let _incremented = assets.inc_ref(slot);
            assets.try_generation(slot)
        }
        -1 => {
            if assets.dec_ref(slot, gen_).is_some() {
                free.push(FreeEntry { kind, slot, retire_frame: epoch + RETIRE_DELAY });
            }
            None
        }
        other => {
            debug_assert!(
                false,
                "apply_refcount_deltas: RefDelta magnitude must be +1/-1, got {other} \
                 (a hook regression — every carrier hook pushes exactly +1 or -1)"
            );
            None
        }
    }
}

/// Deferred, generation-checked re-sync of a carrier's
/// `MeshRefGen`/`MaterialRefGen` lane (asset-streaming plan F5; F6 churn-safety
/// fix) — the stale-tolerant counterpart of a plain
/// `Commands::entity(entity).insert(...)`.
///
/// # Why not a plain `Commands::entity(...).insert(...)`
///
/// `apply_refcount_deltas` enqueues this stamp for every `+1` delta —
/// including one whose `entity` gets despawned, and its id RECYCLED with a
/// bumped generation, before this system's own queued command reaches its
/// apply turn: the concurrent dispatcher drains worker completions (and
/// therefore calls each finished system's `apply`) in COMPLETION order, not
/// as an atomic body-then-apply pair per system (`schedule.rs`'s dispatch
/// loop applies one finished system's commands per drained completion,
/// interleaved with every other finished system's), so an unrelated
/// concurrently-scheduled despawn's apply can land strictly between this
/// system's body finishing and this stamp's own apply. A plain
/// `InsertCommand` hitting that window trips its general
/// `debug_assert!(false, "stale entity ...")` (a correct guard against an
/// actual hook-wiring bug elsewhere) even though a departed carrier here is a
/// normal, expected outcome of churn — not a bug. This command performs the
/// SAME generation check [`EcsMaster::get_component_mut`] already does
/// internally (`inland.generation() == entity.generation()`, mirroring
/// `InsertCommand::apply`'s own guard) and silently no-ops on a mismatch —
/// no `debug_assert`, because a departed carrier IS the expected shape here.
///
/// # Byte-identity on a non-churning (golden) scene
///
/// Every `MeshHandle`/`MaterialHandle` carrier structurally `#[require]`s its
/// `MeshRefGen`/`MaterialRefGen` sibling (see `render_caps.rs`'s "Generation
/// lanes" doc), so a LIVE `entity` always already hosts the lane component —
/// `get_component_mut` resolves it and this stamp writes the identical value
/// (`MeshRefGen(gen_)` / `MaterialRefGen(gen_)`) the prior `InsertCommand`
/// fast (replace-in-place) path would have written, with the same
/// unconditional `changed_tick` bump (`Mut::deref_mut`, mirroring
/// `ComponentPool::write_changed_tick`). Neither lane type has a registered
/// `on_insert`/`on_replace` hook or observer (grep-confirmed against
/// `render_caps.rs`), so skipping the structural-insert machinery loses no
/// hook/observer fire on the live path.
struct SyncRefGenCommand {
    /// The (possibly stale-by-apply-time) carrier entity to re-stamp.
    entity: Entity,
    /// Which lane to write — routes to `MeshRefGen` or `MaterialRefGen`.
    kind: AssetRefKind,
    /// The generation value to stamp.
    gen_: u32,
}

impl Command for SyncRefGenCommand {
    fn apply(self, world: &mut EcsMaster) {
        match self.kind {
            AssetRefKind::Mesh => {
                if let Some(mut lane) = world.get_component_mut::<MeshRefGen>(self.entity) {
                    *lane = MeshRefGen(self.gen_);
                }
            }
            AssetRefKind::Material => {
                if let Some(mut lane) = world.get_component_mut::<MaterialRefGen>(self.entity) {
                    *lane = MaterialRefGen(self.gen_);
                }
            }
        }
    }
}

/// The store epochs [`validate_asset_refs`] last observed — the O(1) early-out
/// oracle (asset-streaming plan F5 Decision 6, widened by the "HARD PREREQ"
/// items (a)/(d)) — plus the (e) orphan counter. `Default` starts every epoch at
/// 0, BELOW a booted store's `install_epoch` (every `add` bumps it), so the first
/// run after boot performs one full compare pass that emits nothing on a
/// well-formed scene and then parks the cursor on the live values.
///
/// # Why FOUR epochs (prereq (a))
///
/// `free_epoch` alone (the F5 shape) only advances on a free / a `Retiring`
/// transition — the transitions that can make a live row STALE. `fill`
/// (Loading → Loaded), a free-list-reuse `add` and `retire` bump
/// [`Assets::install_epoch`] — the transitions that can make a stale row LIVE
/// again (a carrier bound while its asset was Loading). Watching both per store
/// is what gives validate its enable arm without a kernel edit; a churn-free
/// (golden) frame still costs exactly four `u64` loads + compares.
#[derive(Debug, Default, Clone, Copy, Resource)]
pub struct ValidateCursor {
    /// Last `Assets::<MeshGpu>::free_epoch()` observed.
    pub mesh_free: u64,
    /// Last `Assets::<MeshGpu>::install_epoch()` observed (fill / add / retire).
    pub mesh_install: u64,
    /// Last `Assets::<Material>::free_epoch()` observed.
    pub mat_free: u64,
    /// Last `Assets::<Material>::install_epoch()` observed.
    pub mat_install: u64,
    /// Prereq (e): cumulative count of lane-less carriers (a `MeshHandle` row
    /// without `MeshRefGen`, or a `MaterialHandle` row without `MaterialRefGen`
    /// — the schema-older-save shape `load_archetype` cannot repair) that the
    /// orphan pass marked stale. The test-readable signal; a `boyko_log` code
    /// for it is a follow-up outside this crate.
    pub orphan_rows_flagged: u64,
}

/// Bit SET == this row's `MeshHandle` is STALE — a generation mismatch against
/// its bound store row, a non-`Loaded` slot state, or a missing `MeshRefGen`
/// lane (prereq (e)). Written ONLY by [`validate_asset_refs`] (mark AND clear);
/// read by every raw-`MeshHandle.0` consumer as a `Disabled<RenderStale>` query
/// term (`gather_mesh_draws`, `gather_shadow_casters`).
///
/// Prereq (b): staleness is DECOUPLED from user visibility. The
/// `boyko_scene::RenderEnabled` bit belongs to `visibility_sync` and the user;
/// validate never touches it, so a hidden row's stale bit is maintained while
/// hidden and a stale row stays undrawn no matter how visibility toggles.
///
/// Bitset storage: an O(1) toggle with no archetype migration (the instance ring
/// order changes only on a real transition), and a stale-free world allocates
/// NO page — `Disabled<RenderStale>` is then a per-row bit test over an absent
/// page, always true, which is the golden byte-identity argument for the gathers.
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct RenderStale;

/// Material twin of [`RenderStale`]: bit SET == this row's `MaterialHandle` is
/// stale (gen mismatch / non-`Loaded` / lane-less). A stale MATERIAL does not
/// drop the instance — `gather_mesh_draws` draws it with the pinned default
/// material (slot 0) instead (prereq (d), asset-streaming plan task #13).
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct MaterialStale;

/// Prereq (c): the by-name ordering seam between [`validate_asset_refs`] (its
/// sole member, joined in [`AssetRefcountPlugin::build`]) and every raw-slot
/// consumer. A consumer registered through
/// [`add_gather_mesh_draws`](crate::mesh_draw::add_gather_mesh_draws) /
/// [`add_gather_shadow_casters`](crate::csm_caster::add_gather_shadow_casters)
/// is pinned `.after_set(AssetValidateSet)` — an edge that holds regardless of
/// plugin add-order and across the plugin boundary a `SystemKey` cannot cross
/// (the same seam `CsmFitSet` / `PunctualResolveSet` / `LightCollectSet` use).
/// A host that registers the gathers without `AssetRefcountPlugin` gets the
/// scheduler's memberless-set warning — correct: it has no validate.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct AssetValidateSet;

/// Deferred toggle of [`RenderStale`] / [`MaterialStale`] for a row
/// [`validate_asset_refs`] found to have TRANSITIONED this frame — keyed by
/// [`EntityId`], mirroring `boyko_scene::visibility_sync`'s
/// `SetRenderEnabledById`: a read-only query yields only `EntityId`, so the
/// live, generation-correct `Entity` is re-resolved at apply time via
/// [`EcsMaster::get_entity`]. A dead/stale id (a despawn racing this frame) is
/// a silent no-op — the same contract as the kernel's own `EnableTagCommand`.
#[repr(C)]
struct SetStaleById {
    /// The row's entity id, read from the matched archetype's entity-id column.
    id: EntityId,
    /// Which tag to toggle — `Mesh` ⇒ [`RenderStale`], `Material` ⇒ [`MaterialStale`].
    kind: AssetRefKind,
    /// `true` ⇒ set the bit (mark stale); `false` ⇒ clear it (row is valid again).
    stale: bool,
}

impl Command for SetStaleById {
    fn apply(self, world: &mut EcsMaster) {
        let Some(entity) = world.get_entity(self.id) else {
            return;
        };
        match (self.kind, self.stale) {
            (AssetRefKind::Mesh, true) => world.enable::<RenderStale>(entity),
            (AssetRefKind::Mesh, false) => world.disable::<RenderStale>(entity),
            (AssetRefKind::Material, true) => world.enable::<MaterialStale>(entity),
            (AssetRefKind::Material, false) => world.disable::<MaterialStale>(entity),
        }
    }
}

/// Staleness oracle for `MeshHandle`/`MaterialHandle` carriers (asset-streaming
/// plan F5 Decision 6, completed by the "HARD PREREQ before async streaming"
/// items (a) (b) (d) (e)) — the SOLE backstop for a bare-slot carrier that has
/// fallen out of sync with its bound store row. A well-formed carrier's own
/// refcount keeps its slot alive and never goes stale (see
/// `boyko_scene::render_caps`'s "Refcount hook wiring" doc); the carriers this
/// system catches are contract violations (the W1 rebind gap, a stale weak
/// `Handle` copy held outside a carrier), an in-flight async load (a carrier bound
/// to a `reserve()`d slot that has not been `fill`ed yet), or a schema-older save's
/// lane-less row. The durable guards — refcount (F2) + the `dec_ref` gen-check (F5
/// Decision 4) — already render every one of these non-corrupting; this system
/// decides what is DRAWN.
///
/// # Two-way (mark + clear), decoupled from visibility — prereqs (a) and (b)
///
/// The verdict lives in two bitset tags this system alone writes:
/// [`RenderStale`] (mesh) and [`MaterialStale`] (material). It never touches
/// `boyko_scene::RenderEnabled` — that bit belongs to `visibility_sync` and the
/// user, and the F5 shape (validate `disable::<RenderEnabled>`) fought it: both
/// drove one bit, and a hidden-while-stale row shown later was drawn stale. Now a
/// row is drawn iff `Enabled<RenderEnabled> && Disabled<RenderStale>` — two
/// independent bits with two independent owners — and a stale MATERIAL row is
/// drawn with the pinned default slot 0 (`Enabled<MaterialStale>`, prereq (d)).
///
/// Transitions are found by COMPLEMENTARY queries per store: rows under
/// `Disabled<Tag>` that are now stale get the bit SET; rows under `Enabled<Tag>`
/// that are now valid get it CLEARED. Each row is visited exactly once, the
/// current bit is read by the filter (no per-row world probe), and a command is
/// emitted ONLY on a transition — a churn frame over N unchanged rows emits zero
/// commands. Neither query filters on `RenderEnabled`, so a hidden row's bit is
/// maintained while hidden. (Bevy PR #18734 is the same-frame handle-swap race
/// this whole mechanism defends against.)
///
/// # Four-epoch early-out — O(1) on every churn-free/golden frame
///
/// Four `u64` loads + compares against [`ValidateCursor`]: both stores'
/// `free_epoch` (bumped on `remove` / a `dec_ref` zero-crossing — a row can turn
/// STALE) and `install_epoch` (bumped on `fill` / `add` / `retire` — a stale row
/// can turn LIVE: prereq (a)'s enable path). If none advanced, this returns
/// without iterating. A static/golden scene never advances any of them after its
/// boot adds, so its per-row loop never runs — the byte-identity argument's
/// load-bearing fact. An `add` frame (a runtime mint) costs one compare pass that
/// emits nothing; `reserve` / `fail` bump nothing (neither makes a row drawable).
///
/// # On a churn frame — O(carriers) dense `u32`-compares, no random access
///
/// One pass per query over `(Handle, RefGen)` (dense, archetype-order,
/// L1-resident): `RefGen(GEN_UNSYNCED)` means "bound this frame, not yet synced"
/// and is trusted (skipped) in BOTH arms — the sibling `apply_refcount_deltas`
/// system, `.before` this one with its per-system flush, guarantees a real
/// binding is NEVER left at `GEN_UNSYNCED` past this point (see `apply_one`'s
/// doc); otherwise the stale predicate is `try_generation(slot) != Some(g) ||
/// state_of_index(slot) != Some(Loaded)`. Material slots widen via `u32::from`.
///
/// # Prereq (e): a lane-less carrier is FLAGGED and COUNTED, never drawn
///
/// `#[require(MeshRefGen)]` / `#[require(MaterialRefGen)]` materialize the lane on
/// every `Commands::spawn`/`insert` path but NOT on the raw archetype-deserialize
/// path (`boyko_ecs::ecs::core::serialize::load_writer::load_archetype` builds
/// the archetype from the FILE's saved component-id list; no require expansion
/// runs), so a `MeshHandle` row from a schema-older save lacks its lane and is
/// AND-filtered out of the two-column queries above — the F5 hole ("silently
/// skipped"). The orphan pass closes it: `Query<&MeshHandle, (Without<MeshRefGen>,
/// Disabled<RenderStale>)>` (and the material twin) marks every match stale and
/// bumps [`ValidateCursor::orphan_rows_flagged`]. It runs UNCONDITIONALLY, outside
/// the early-out — a load bumps no store epoch, so an orphan placed under the
/// early-out would be drawn until the next churn — and it is free in a
/// well-formed world: `Without` over a TABLE component is archetypal, so the
/// matched set is computed at structural change and a world with no lane-less
/// archetype iterates zero rows. The `Disabled<..>` term makes each orphan cost
/// one command, once. Semantics: un-refcounted + unvalidatable ⇒ conservative
/// (never drawn). The REPAIR (insert the lane, rebind `+1`) is kernel work:
/// `load_archetype` replays no `on_insert` hook, so a loaded carrier never pushes
/// `+1` either (pinned by `load_world_fires_no_carrier_hooks`) — the plan's B1.
///
/// # Raw carrier-index read sites downstream of this system
///
/// Every raw `MeshHandle.0` / `MaterialHandle.0` read site in the render crate
/// (`mesh_draw.rs`, `csm_caster.rs`) filters on `Disabled<RenderStale>` (and, for
/// the material lane, splits on `MaterialStale`) and is registered
/// `.after_set(`[`AssetValidateSet`]`)` through the consumer-side helpers — so it
/// reads THIS frame's bits (prereq (c)). The gathers additionally keep their own
/// construction guards (`try_get` for the mesh, `get_by_index` → `None ⇒ 0` for
/// the material), so a carrier bound to a `reserve()`d slot on its spawn frame —
/// no epoch bump, this system does not run — still never dereferences a hole.
// SystemParams are consumed by-value by the SystemParam contract; the six
// read-only queries + two stores + cursor + Commands ARE the system's access
// contract (the scheduler reads the signature), so no argument bundling.
#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments, clippy::type_complexity)]
pub fn validate_asset_refs(
    q_mesh_ok: Query<(&MeshHandle, &MeshRefGen), Disabled<RenderStale>>,
    q_mesh_stale: Query<(&MeshHandle, &MeshRefGen), Enabled<RenderStale>>,
    q_mat_ok: Query<(&MaterialHandle, &MaterialRefGen), Disabled<MaterialStale>>,
    q_mat_stale: Query<(&MaterialHandle, &MaterialRefGen), Enabled<MaterialStale>>,
    q_mesh_orphan: Query<&MeshHandle, (Without<MeshRefGen>, Disabled<RenderStale>)>,
    q_mat_orphan: Query<&MaterialHandle, (Without<MaterialRefGen>, Disabled<MaterialStale>)>,
    mesh_assets: NonSendRes<Assets<MeshGpu>>,
    material_assets: Res<Assets<Material>>,
    mut cursor: ResMut<ValidateCursor>,
    mut cmd: Commands,
) {
    // Prereq (e): the orphan pass — unconditional, BEFORE the early-out (see the
    // doc). Zero matched archetypes in a well-formed world => zero rows touched.
    for (id, _) in q_mesh_orphan.iter_entities() {
        cmd.add(SetStaleById { id, kind: AssetRefKind::Mesh, stale: true });
        cursor.orphan_rows_flagged += 1;
    }
    for (id, _) in q_mat_orphan.iter_entities() {
        cmd.add(SetStaleById { id, kind: AssetRefKind::Material, stale: true });
        cursor.orphan_rows_flagged += 1;
    }

    let mesh_free = mesh_assets.free_epoch();
    let mesh_install = mesh_assets.install_epoch();
    let mat_free = material_assets.free_epoch();
    let mat_install = material_assets.install_epoch();
    debug_assert!(
        mesh_free >= cursor.mesh_free
            && mesh_install >= cursor.mesh_install
            && mat_free >= cursor.mat_free
            && mat_install >= cursor.mat_install,
        "invariant: Assets::free_epoch / install_epoch are monotonic non-decreasing \
         (observed mesh {mesh_free}/{mesh_install}, material {mat_free}/{mat_install}; \
         cursor {:?})",
        *cursor
    );
    if mesh_free == cursor.mesh_free
        && mesh_install == cursor.mesh_install
        && mat_free == cursor.mat_free
        && mat_install == cursor.mat_install
    {
        return;
    }

    // Mesh lane — mark arm, then clear arm (complementary filters, see the doc).
    for (id, (&MeshHandle(slot), &MeshRefGen(g))) in q_mesh_ok.iter_entities() {
        if g != GEN_UNSYNCED && is_stale(&mesh_assets, slot, g) {
            cmd.add(SetStaleById { id, kind: AssetRefKind::Mesh, stale: true });
        }
    }
    for (id, (&MeshHandle(slot), &MeshRefGen(g))) in q_mesh_stale.iter_entities() {
        if g != GEN_UNSYNCED && !is_stale(&mesh_assets, slot, g) {
            cmd.add(SetStaleById { id, kind: AssetRefKind::Mesh, stale: false });
        }
    }

    // Material lane — the same two arms over the `u16` carrier, widened.
    for (id, (&MaterialHandle(slot), &MaterialRefGen(g))) in q_mat_ok.iter_entities() {
        if g != GEN_UNSYNCED && is_stale(&material_assets, u32::from(slot), g) {
            cmd.add(SetStaleById { id, kind: AssetRefKind::Material, stale: true });
        }
    }
    for (id, (&MaterialHandle(slot), &MaterialRefGen(g))) in q_mat_stale.iter_entities() {
        if g != GEN_UNSYNCED && !is_stale(&material_assets, u32::from(slot), g) {
            cmd.add(SetStaleById { id, kind: AssetRefKind::Material, stale: false });
        }
    }

    cursor.mesh_free = mesh_free;
    cursor.mesh_install = mesh_install;
    cursor.mat_free = mat_free;
    cursor.mat_install = mat_install;
}

/// The stale predicate (asset-streaming plan F5 Decision 6, unchanged): a
/// carrier whose lane-stamped generation `g` no longer matches the slot's
/// current one (the slot was retired and reused under it), or whose slot is not
/// `Loaded` (Loading / Failed / Retiring / Vacant / out of range). Generic over
/// the two concrete `AssetBacking` types so both lanes share one monomorphized
/// body — no dynamic dispatch.
#[inline]
fn is_stale<T: AssetBacking>(assets: &Assets<T>, slot: u32, g: u32) -> bool {
    assets.try_generation(slot) != Some(g)
        || assets.state_of_index(slot) != Some(AssetLoadState::Loaded)
}

/// Wires the asset-streaming refcount pipeline into the app schedule
/// (asset-streaming plan F2 §1/§3, F5's validation, F6's fence gate): inserts
/// the queue resources ([`RefcountDeltas`], [`DeferredFree`],
/// [`ValidateCursor`], [`RenderEpoch`]) the carrier hooks and both systems
/// share, and registers [`apply_refcount_deltas`] `.before(validate_asset_refs)`
/// with `validate_asset_refs` joining [`AssetValidateSet`].
/// `RenderEpoch` starts at `0`, matching a fresh [`Renderer`](boyko_rhi_vulkan::swapchain::Renderer)'s
/// `submission_epoch` before the first submit; the host overwrites it every
/// frame BEFORE `app.update_with_delta` (`boyko_app::runner`'s boot-ordering
/// contract), so this only matters for a `apply_refcount_deltas` run before
/// the first host publish (none exists in-tree today).
///
/// # The apply → validate edge is a `SystemKey`; the validate → gather edge is a SET
///
/// Both systems are registered in the SAME [`App::add_systems_cfg`] closure
/// here, so the `SystemKey`-based `.before` edge between them is directly
/// expressible. The FURTHER edge — validation before
/// `boyko_render::gather_mesh_draws` / `gather_shadow_casters`, registered by a
/// LATER, separate closure in the composing host
/// (`boyko_app::plugins::EnginePlugins::build`) — cannot be a `SystemKey` edge
/// (a key for a system that does not exist yet at this plugin's build time is
/// unobtainable, and `SystemKey` is unnameable outside `boyko_ecs`). Prereq (c)
/// therefore pins it BY NAME: `validate_asset_refs` is the sole member of
/// [`AssetValidateSet`], and every consumer is registered through
/// [`add_gather_mesh_draws`](crate::mesh_draw::add_gather_mesh_draws) /
/// [`add_gather_shadow_casters`](crate::csm_caster::add_gather_shadow_casters),
/// which chain `.after_set(AssetValidateSet)` before handing the caller its
/// `SystemConfig` (the `add_gpu_transform_pack` shape). The edge holds
/// regardless of add-order and is provable in a bare `App` through the builder's
/// cycle detection (`tests/asset_validate_schedule_edge.rs`) — a mutation that
/// drops the `.after_set` turns that test red, which an add-order pin could never
/// offer.
#[derive(Default)]
pub struct AssetRefcountPlugin;

impl Plugin for AssetRefcountPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RefcountDeltas::default());
        app.insert_resource(DeferredFree::default());
        app.insert_resource(ValidateCursor::default());
        app.insert_resource(RenderEpoch::default());
        app.add_systems_cfg(|b| {
            let apply = b.add_system(apply_refcount_deltas).key();
            b.add_system(validate_asset_refs).after(apply).in_set(AssetValidateSet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::AssetRefcountPlugin"
    }
}

/// Fence-gated drain of [`DeferredFree`] + [`OrphanedMeshGpu`] +
/// [`RetiredGpuBuffers`] at `epoch` (asset-streaming plan F6, extended by F7):
/// actually retires every store slot whose `retire_frame <= epoch` (moving its
/// value out via [`Assets::retire`](boyko_ecs::ecs::core::asset::Assets::retire))
/// and frees the device resources ([`MeshGpu`]'s vertex/index buffers, and its
/// BLAS under `hwrt`) it held — then does the same for every fill-rejected
/// orphan past its gate, then drains every F7 grow-and-defer-old buffer past
/// its gate (`RetiredGpuBuffers::drain_ready`). Textured-PBR rung T6b extends
/// this with [`BindlessTextureTable::retire_ready_slots`] (the O1 bindless-slot
/// fence-gated recycle, P1-5) and [`OrphanedTextureGpu::drain_ready`] — both
/// under the SAME `epoch` gate, guarded on the table's presence (see the texture
/// block below for why: `BindlessTextureTable::new` is a fallible boot step).
/// There is no `Assets<TextureGpu>` refcount-driven retire branch yet: no
/// `TextureHandle` carrier / refcount-hook producer exists in-tree (T7) to ever
/// enqueue one — see this function's implementation comment at the texture block
/// for the full argument.
///
/// # Caller contract — MUST run after `wait_frame_in_flight` for THIS `epoch`
///
/// `epoch` MUST be
/// [`Renderer::submission_epoch`](boyko_rhi_vulkan::swapchain::Renderer::submission_epoch)
/// read AFTER this frame's
/// [`wait_frame_in_flight`](boyko_rhi_vulkan::swapchain::Renderer::wait_frame_in_flight)
/// call (`boyko_app::runner`, immediately after `let s = token.slot();`, BEFORE
/// any per-frame upload/draw-assembly reads a mesh handle). The fence-gate
/// proof:
///
/// Let a row `S` be enqueued (`RefDelta` -1 reaching zero) at submission-epoch
/// `N` (stamped `retire_frame = N + `[`RETIRE_DELAY`]` by [`apply_one`]). This
/// fn frees `S` at the first `epoch M` with `N + RETIRE_DELAY <= M`. The SAME
/// frame `N` that enqueued `S` also ran `validate_asset_refs` (after `apply`,
/// before any gather) — `dec_ref`'s `Retiring` transition already bumped
/// `free_epoch`, so validation (same frame) or the entity's own despawn
/// disables every carrier of `S` before frame `N`'s own gather/submit runs;
/// `S`'s LAST possible GPU reference is therefore submit `<= N - 1 < N`, i.e.
/// `<= M - RETIRE_DELAY` for any `M` this fn is called at. Since `RETIRE_DELAY
/// == FRAMES_IN_FLIGHT` and `wait_frame_in_flight` at epoch `M` waits the ring
/// slot last used by submit `M - FRAMES_IN_FLIGHT`'s fence, ALL submits up to
/// and including `M - FRAMES_IN_FLIGHT` are GPU-complete once this fn runs —
/// covering `S`'s last reference with a full frame of margin. Reused-slot
/// resurrection cannot reopen this: [`Assets::inc_ref`] permanently refuses a
/// `Retiring`/`Vacant` slot, and `dec_ref` is idempotent on `Retiring`, so
/// `S`'s refcount is provably 0 and exactly one `FreeEntry`/orphan exists per
/// value — `retire`/`OrphanedMeshGpu::drain_ready` free it exactly once.
///
/// `debug_assert_eq!(RETIRE_DELAY, FRAMES_IN_FLIGHT as u64)` below pins the
/// invariant this proof depends on; if the TLAS ever becomes
/// persistent/compacted (rather than rebuilt whole every frame) or an
/// independent-fence async-compute queue references BLAS/buffers,
/// `RETIRE_DELAY` must grow to match the new horizon — see [`RETIRE_DELAY`]'s
/// doc.
///
/// # Golden byte-identity (O(1) early-out)
///
/// A scene that never lets any asset's refcount reach zero (the golden scene:
/// every load is held for the run's duration) never enqueues a `FreeEntry` or
/// an orphan, and a scene whose GPU mirrors never outgrow their boot capacity
/// never pushes a [`RetiredGpuBuffers`] entry — `free.is_empty() &&
/// orphans_empty && retired_empty` (F7 C2: the early-out is extended, not
/// narrowed — a growth-only frame with both F6 queues empty must still drain
/// this queue) short-circuits with three `bool` reads and zero world mutation,
/// zero device calls. The rewrite below is idempotent in any case (same store
/// state -> same device calls), so this never perturbs a rendered frame.
///
/// `scratch` is a host-owned, reusable buffer (parked across frames) — zero
/// steady-state allocation.
pub fn retire_deferred_frees(
    world: &mut EcsMaster,
    ctx: &VulkanContext,
    epoch: u64,
    scratch: &mut Vec<FreeEntry>,
) {
    debug_assert_eq!(
        RETIRE_DELAY,
        FRAMES_IN_FLIGHT as u64,
        "invariant: RETIRE_DELAY must equal the ring's fence horizon (FRAMES_IN_FLIGHT) — \
         see retire_deferred_frees' fence-gate proof"
    );

    let free_empty = world.resource::<DeferredFree>().is_empty();
    let orphans_empty = world.non_send_resource::<OrphanedMeshGpu>().is_empty();
    // `RetiredGpuBuffers::is_empty()` covers BOTH its lanes: the buffer lane (`entries`,
    // F7) AND, under `hwrt`, the TLAS lane (`tlases`, F7-hwrt task#11) — a growth-only
    // frame (both F6 queues empty) must still drain a pending grow-and-defer-old
    // buffer OR TLAS; narrowing this to `free_empty && orphans_empty` would leak either
    // (both are decoupled from refcount churn).
    let retired_empty = world.non_send_resource::<RetiredGpuBuffers>().is_empty();
    // Textured-PBR T6b: two more independent queues, the same F7-C2 shape — a
    // texture-only frame (every queue above empty) must still drain a pending
    // fill-reject orphan OR a fence-staged bindless-slot recycle; narrowing this
    // early-out would leak either. `OrphanedTextureGpu` is unconditional infra
    // (inserted before the fallible `BindlessTextureTable::new` boot step, see
    // `run_windowed`), always present here. `BindlessTextureTable` itself may be
    // ABSENT — its creation is fallible, and a failed boot force-drains this
    // queue (via `teardown`'s unconditional `retire_deferred_frees(..., u64::MAX,
    // ..)` call) BEFORE the table was ever inserted; absence reads as "empty"
    // (nothing could be staged in a table that was never created).
    let tex_orphans_empty = world.non_send_resource::<OrphanedTextureGpu>().is_empty();
    let bindless_recycle_empty = !world.contains_non_send_resource::<BindlessTextureTable>()
        || world.non_send_resource::<BindlessTextureTable>().is_empty();
    if free_empty && orphans_empty && retired_empty && tex_orphans_empty && bindless_recycle_empty {
        return;
    }

    world.resource_mut::<DeferredFree>().drain_ready(epoch, scratch);

    if !scratch.is_empty() {
        {
            let mesh_assets = world.non_send_resource_mut::<Assets<MeshGpu>>();
            for entry in scratch.iter().filter(|e| e.kind == AssetRefKind::Mesh) {
                // INVARIANT: `retire` targets a Retiring row by construction — every
                // entry here came from a `RetireTicket` `dec_ref` issued exactly once
                // (F5 Decision 5: resurrection is impossible once Retiring), so no
                // recheck is needed here (see `Assets::retire`'s own debug assert).
                if let Some(mesh) = mesh_assets.retire(entry.slot) {
                    // R2a-3 (P0-3): free the AS FIRST — its memory lives in its backing
                    // buffer, which must outlive it (mirrors `MeshAssetsExt::destroy`).
                    #[cfg(feature = "hwrt")]
                    if let Some(b) = mesh.blas {
                        // SAFETY: the fence for this `epoch` has been waited via
                        // `wait_frame_in_flight` (the caller's contract above), so every
                        // submit that could reference this mesh's BLAS is complete — the
                        // per-resource form of the device-idle contract this fn's own
                        // fence-gate proof establishes.
                        unsafe { boyko_rhi_vulkan::accel_build::destroy_blas(ctx, b) };
                    }
                    // SAFETY: same fence-gate contract as the BLAS destroy above —
                    // `epoch`'s fence was waited via `wait_frame_in_flight` before this
                    // call, so no submit can still reference `mesh`'s buffers; `retire`
                    // guarantees this value is moved out exactly once.
                    unsafe {
                        ctx.destroy_buffer(mesh.vertex_buffer);
                        ctx.destroy_buffer(mesh.index_buffer);
                    }
                }
            }
        }
        {
            let material_assets = world.resource_mut::<Assets<Material>>();
            for entry in scratch.iter().filter(|e| e.kind == AssetRefKind::Material) {
                // `Material` is `NEEDS_TEARDOWN = false` (device-free POD) — no
                // destroy call needed, only the store-slot retire.
                let _ = material_assets.retire(entry.slot);
            }
        }
    }

    world.non_send_resource_mut::<OrphanedMeshGpu>().drain_ready(epoch, ctx);

    // SAFETY: `epoch`'s fence was waited via `wait_frame_in_flight` (this fn's caller
    // contract above) — the same fence-gate precondition the two drains above rely on.
    unsafe {
        world.non_send_resource_mut::<RetiredGpuBuffers>().drain_ready(epoch, ctx);
    }

    // Textured-PBR rung T6b: the bindless-slot fence-gated recycle (P1-5) +
    // `OrphanedTextureGpu`'s fill-reject teardown, under the SAME `epoch` fence-gate
    // contract as every drain above. Guarded on presence (see the early-out's doc
    // above): a failed `BindlessTextureTable::new` boot step never inserted the
    // table, and there is nothing staged for a table that was never created.
    //
    // `BindlessTextureTable` is temporarily taken OUT of the World (rather than
    // fetched alongside `OrphanedTextureGpu` via two simultaneous
    // `non_send_resource_mut` calls, which the World's non-send API has no helper
    // for) so it can be threaded as `OrphanedTextureGpu::drain_ready`'s
    // `aux: &mut BindlessTextureTable` — mirrors `MaterialTable::boot_seed`'s
    // "take out, use, reinsert" shape (`run_windowed`, boyko_app).
    //
    // No `Assets<TextureGpu>` refcount-driven retire branch runs here (unlike the
    // mesh/material scratch loop above): that would need a `TextureHandle` carrier
    // component and a `boyko_scene::AssetRefKind::Texture` producer feeding
    // `DeferredFree` — neither exists in-tree yet (no material references a
    // texture until T7) — so no `FreeEntry` for a texture can ever be enqueued.
    // Wiring a scratch-filtered branch that can never execute would be dead code
    // masquerading as a real guard; this is deliberately left for the rung that
    // adds the carrier + hook wiring.
    if let Some(mut bindless_table) = world.remove_non_send_resource::<BindlessTextureTable>() {
        bindless_table.retire_ready_slots(epoch);
        world
            .non_send_resource_mut::<OrphanedTextureGpu>()
            .drain_ready(epoch, ctx, &mut bindless_table);
        world.insert_non_send_resource(bindless_table);
    }
}

#[cfg(test)]
mod tests {
    // Test oracle model: the `HashSet<(slot, generation)>` below is the REFERENCE ledger the
    // VM-native `Assets`/`DeferredFree` slot recycler is differentially verified against
    // (no-leak / no-double-free / retire-horizon). Compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

    use super::*;

    /// `apply_one` is generic over `AssetBacking`; `Material` (a local,
    /// device-free `AssetBacking` type — `NEEDS_TEARDOWN = false`) exercises it
    /// without any device dependency. `MeshGpu` cannot be used here: it needs a
    /// real device to construct a value, which this unit test has none of.
    ///
    /// `Assets::add` mints refcount 0 (an unattached load has no owner yet —
    /// see `Assets::add`'s doc), so the sequence below `inc_ref`s TWICE before
    /// decrementing, to reach the SAME 1->2->1->0 trace the name describes
    /// without ever calling `dec_ref` on an already-0 row (which would trip
    /// its `debug_assert!(count > 0, ...)` — a real caller-precondition
    /// violation, not a property this test should exercise).
    #[test]
    fn apply_one_inc_then_dec_to_zero_enqueues_a_free_entry() {
        let mut assets = Assets::<Material>::with_reserved(4);
        let handle = assets.add(Material::default());
        let slot = handle.index();
        let mut free = DeferredFree::default();
        const EPOCH: u64 = 10;

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, 1, EPOCH);
        assert!(free.is_empty(), "refcount 0->1 must not enqueue a retire");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, 1, EPOCH);
        assert!(free.is_empty(), "refcount 1->2 must not enqueue a retire");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1, EPOCH);
        assert!(free.is_empty(), "refcount 2->1 must not enqueue a retire");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1, EPOCH);
        assert_eq!(free.entries().len(), 1, "refcount 1->0 must enqueue exactly one retire");
        assert_eq!(free.entries()[0].slot, slot);
        assert_eq!(
            free.entries()[0].retire_frame,
            EPOCH + RETIRE_DELAY,
            "the enqueued entry must stamp the real fence-gated retire_frame, not a placeholder"
        );
    }

    /// `Assets::add` mints refcount 0 (see the sibling test's doc) — `inc_ref`
    /// once first so the first `dec_ref` below is the genuine 1->0
    /// zero-crossing decrement this test targets, not an underflow on an
    /// already-0 row.
    #[test]
    fn apply_one_double_dec_past_zero_is_idempotent() {
        let mut assets = Assets::<Material>::with_reserved(4);
        let handle = assets.add(Material::default());
        let slot = handle.index();
        let mut free = DeferredFree::default();
        const EPOCH: u64 = 10;

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, 1, EPOCH);

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1, EPOCH);
        assert_eq!(free.entries().len(), 1, "the first zero-crossing decrement enqueues once");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1, EPOCH);
        assert_eq!(
            free.entries().len(),
            1,
            "a second decrement on an already-Retiring slot must not enqueue again"
        );
    }

    /// A tiny deterministic xorshift32 PRNG — reproducible churn without a new
    /// `rand`/`proptest` dev-dependency (`boyko_render` has neither today).
    struct Xorshift32(u32);
    impl Xorshift32 {
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    /// CPU churn stress (asset-streaming plan F6): a long-running simulated
    /// spawn/despawn churn over `Assets<Material>`, driving the EXACT same
    /// `apply_one` (enqueue) -> `DeferredFree::drain_ready` (fence-gate) ->
    /// `Assets::retire` (terminal free) pipeline
    /// [`retire_deferred_frees`]'s material branch runs — MINUS the actual
    /// device call (`Material` is `NEEDS_TEARDOWN = false`; its branch in
    /// `retire_deferred_frees` is ALREADY just `let _ = material_assets.retire(..)`
    /// with no device touch, so this test exercises that branch's real logic
    /// verbatim, not a stand-in). `retire_deferred_frees` itself cannot be
    /// called from a unit test — it takes `&VulkanContext`, which has no
    /// public/testable constructor outside a real device boot (see this
    /// module's sibling suites, which likewise never call it directly); the
    /// mesh side's BLAS-before-buffer destroy CALL ORDER therefore needs a
    /// real device and is covered instead by the `#[ignore]`d windowed churn
    /// test (`boyko_app`'s `asset_streaming_f6_churn_headless.rs`).
    ///
    /// Over many simulated epochs: some live slots are despawned (a `-1` that
    /// reaches zero, enqueuing a `FreeEntry` stamped `epoch + RETIRE_DELAY`),
    /// fresh slots are spawned (`add` + a `+1`), and every epoch drains
    /// whatever is fence-ready. Asserts, across the WHOLE run:
    /// - **No leak / no double-free**: every `add` ends up EITHER still alive
    ///   (a live slot at the end) OR retired EXACTLY ONCE — never both, never
    ///   neither (`create_count == destroy_count + assets.len()`).
    /// - **No retire before its horizon**: every drained entry's
    ///   `retire_frame <= `the epoch it drained at (the same property test
    ///   (a) proves for `drain_ready` in isolation, re-checked here under
    ///   sustained multi-slot churn rather than a hand-picked few entries).
    /// - **Free-list reuse actually happens**: `high_water()` stays far below
    ///   `create_count` (a broken reuse path would grow it 1:1 with every
    ///   spawn).
    #[test]
    fn material_churn_over_many_epochs_never_leaks_or_double_frees_and_respects_the_horizon() {
        use std::collections::HashSet;

        const INITIAL_SLOTS: usize = 32;
        const CHURN_EPOCHS: u64 = 400;
        const DRAIN_TAIL_EPOCHS: u64 = 16; // >> RETIRE_DELAY — drains every straggler.
        const MAX_CHURN_PER_EPOCH: usize = 3;

        let mut assets = Assets::<Material>::with_reserved(INITIAL_SLOTS);
        let mut free = DeferredFree::default();
        let mut scratch: Vec<FreeEntry> = Vec::new();
        let mut rng = Xorshift32(0xC0FF_EE01);

        let mut live_slots: Vec<u32> = Vec::new();
        // Keyed by (slot, generation-at-retire), not bare slot — see the drain loops'
        // comments for why bare-slot double-insert would be a false positive.
        let mut destroyed: HashSet<(u32, u32)> = HashSet::new();
        let mut create_count: u64 = 0;
        let mut destroy_count: u64 = 0;

        let spawn = |assets: &mut Assets<Material>,
                     free: &mut DeferredFree,
                     epoch: u64,
                     live_slots: &mut Vec<u32>,
                     create_count: &mut u64| {
            let handle = assets.add(Material::default());
            let slot = handle.index();
            *create_count += 1;
            // A `+1` delta's return is the lane-stamp generation (see `apply_one`'s
            // doc), not an enqueue signal — a fresh 0->1 increment never enqueues a
            // retire; `free` staying whatever it already was is the real assertion,
            // implicitly proven by every `free.entries()` check elsewhere in this test.
            let _ = apply_one(assets, free, AssetRefKind::Material, slot, GEN_UNSYNCED, 1, epoch);
            live_slots.push(slot);
        };

        for epoch in 0..CHURN_EPOCHS {
            // Despawn up to MAX_CHURN_PER_EPOCH live slots (a -1 that may or may not
            // reach zero — every slot here has exactly one virtual owner, so it always
            // reaches zero and enqueues).
            let despawn_n = (rng.next_u32() as usize % (MAX_CHURN_PER_EPOCH + 1)).min(live_slots.len());
            for _ in 0..despawn_n {
                let pick = rng.next_u32() as usize % live_slots.len();
                let slot = live_slots.swap_remove(pick);
                let gen_ = assets.generation(slot);
                let enqueued_before = free.entries().len();
                // A `-1` delta's return is always `None` regardless of enqueue (see
                // `apply_one`'s match arm) — the enqueue side-effect is only observable
                // via `free.entries()`, checked below.
                let _ =
                    apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, gen_, -1, epoch);
                assert_eq!(
                    free.entries().len(),
                    enqueued_before + 1,
                    "the sole owner's despawn must retire the slot (single-owner model)"
                );
            }

            // Spawn up to MAX_CHURN_PER_EPOCH fresh slots.
            let spawn_n = (rng.next_u32() as usize % (MAX_CHURN_PER_EPOCH + 1)) + 1;
            for _ in 0..spawn_n {
                spawn(&mut assets, &mut free, epoch, &mut live_slots, &mut create_count);
            }

            // Drain whatever is fence-ready THIS epoch and retire it — mirrors
            // `retire_deferred_frees`'s material branch exactly (see this test's doc).
            free.drain_ready(epoch, &mut scratch);
            for entry in &scratch {
                assert!(
                    entry.retire_frame <= epoch,
                    "drain_ready must never yield an entry before its own fence horizon"
                );
                // Keyed by (slot, generation-AT-retire-time), not bare slot: the same
                // numeric slot legitimately gets retired MULTIPLE times over the run
                // (LIFO reuse re-tenants it) — a real double-free is retiring the SAME
                // tenancy (slot, generation) twice, not merely reusing an index.
                let gen_before_retire = assets.generation(entry.slot);
                assert!(
                    destroyed.insert((entry.slot, gen_before_retire)),
                    "slot {} generation {} must be retired at most once — a repeat means a \
                     double-free of the SAME tenancy",
                    entry.slot,
                    gen_before_retire
                );
                let taken = assets.retire(entry.slot);
                assert!(taken.is_some(), "a Loaded->Retiring Material row always holds a value");
                destroy_count += 1;
            }
        }

        // Seed INITIAL_SLOTS extra rows too (via `add`, no owner) so the store starts
        // from a nontrivial base, matching a real boot (slot 0 pinned defaults, etc.) —
        // these are never churned, only counted, proving churn coexists with a stable
        // baseline population without cross-contamination.
        for _ in 0..INITIAL_SLOTS {
            let h = assets.add(Material::default());
            create_count += 1;
            live_slots.push(h.index());
        }

        // Tail: no more churn, just drain every straggler past its horizon.
        for epoch in CHURN_EPOCHS..(CHURN_EPOCHS + DRAIN_TAIL_EPOCHS) {
            free.drain_ready(epoch, &mut scratch);
            for entry in &scratch {
                let gen_before_retire = assets.generation(entry.slot);
                assert!(
                    destroyed.insert((entry.slot, gen_before_retire)),
                    "slot {} generation {} must be retired at most once — a repeat means a \
                     double-free of the SAME tenancy",
                    entry.slot,
                    gen_before_retire
                );
                let taken = assets.retire(entry.slot);
                assert!(taken.is_some());
                destroy_count += 1;
            }
        }

        assert!(
            free.is_empty(),
            "every enqueued retire must have fully drained by the tail's end \
             (DRAIN_TAIL_EPOCHS >> RETIRE_DELAY)"
        );

        // The conservation law: every `add` ends up EITHER still alive OR retired
        // exactly once — never both, never neither.
        assert_eq!(
            create_count,
            destroy_count + assets.len() as u64,
            "no leak, no double-free: create_count must equal destroy_count + still-alive count"
        );
        assert_eq!(
            assets.len(),
            live_slots.len(),
            "the store's live count must match the model's still-owned slot set"
        );

        // Free-list reuse actually happened: high_water stays far below the total
        // number of `add` calls (a broken reuse path would grow it 1:1 with churn).
        assert!(
            (assets.high_water() as u64) < create_count,
            "high_water ({}) must stay below create_count ({}) — free-list reuse must have \
             recycled retired slots, not appended a fresh row for every spawn",
            assets.high_water(),
            create_count
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // F7 C2 regression: the early-out must require ALL THREE queues empty.
    // ════════════════════════════════════════════════════════════════════════

    /// Regression guard for the F7 C2 blocker fix: `retire_deferred_frees`'s
    /// early-out is `free_empty && orphans_empty && retired_empty` — a
    /// `free_empty && orphans_empty`-only guard (the pre-C2 shape) would skip
    /// draining a pending grow-and-defer-old buffer whenever ordinary refcount
    /// churn happens to be quiet, leaking it forever. `retire_deferred_frees`
    /// itself cannot be called here (it takes `&VulkanContext` — see this
    /// module's sibling churn-stress test doc), so this test reproduces the
    /// EXACT guard expression verbatim against real `DeferredFree`/
    /// `OrphanedMeshGpu`/`RetiredGpuBuffers` values.
    #[test]
    fn c2_early_out_requires_all_three_queues_empty_not_just_free_and_orphans() {
        let free = DeferredFree::default();
        let orphans = OrphanedMeshGpu::default();
        let mut retired = RetiredGpuBuffers::default();

        // All three empty: the golden early-out must fire.
        assert!(
            free.is_empty() && orphans.is_empty() && retired.is_empty(),
            "test precondition: every queue starts empty"
        );

        // ONLY `retired` gains an entry (a growth-only frame with no refcount
        // churn at all) — the pre-C2 `free_empty && orphans_empty`-only guard
        // would still see `true && true` and wrongly early-out, leaking this
        // pending grow-and-defer-old buffer forever.
        retired.push(
            boyko_rhi_vulkan::memory::BoundBuffer {
                buffer: boyko_rhi_vulkan::ffi::VkBuffer::NULL,
                offset: 0,
                size: 0,
                mapped: None,
                block: 0,
            },
            0,
        );

        let free_empty = free.is_empty();
        let orphans_empty = orphans.is_empty();
        let retired_empty = retired.is_empty();
        assert!(free_empty, "test precondition: DeferredFree stays empty");
        assert!(orphans_empty, "test precondition: OrphanedMeshGpu stays empty");
        assert!(!retired_empty, "test precondition: RetiredGpuBuffers now holds one entry");

        assert!(
            !(free_empty && orphans_empty && retired_empty),
            "F7 C2: the extended early-out condition must NOT fire while a RetiredGpuBuffers \
             entry is pending, even though the OTHER two queues are empty — a \
             `free_empty && orphans_empty`-only guard (pre-C2) would leak it"
        );
    }
}
