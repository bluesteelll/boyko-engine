//! Lighting L0 collection: fold the ECS light components into the GPU light table.
//!
//! The authoritative store is the ECS light columns ([`DirectionalLight`] etc.);
//! [`LightTableStaging`] is a *reused staging scratch* (Principle 0 — NOT a durable
//! side-store): one preallocated `Vec<u8>` holding `[LightHeaderGpu || GpuLight[]]`,
//! refilled in place on a change. [`collect_lights`] is `Changed`-gated, so a static
//! scene does zero work and an idle frame records nothing (rung L0-r0).
//!
//! # The upload (rung L0-r0)
//!
//! On a changed frame [`collect_lights`] rewrites the scratch + sets the dirty flag.
//! The per-frame GPU recorder reads [`LightTableStaging::pending_upload`] and, on a
//! dirty frame, records a fence-free staging→device `copy_buffer` + a
//! TRANSFER_WRITE→SHADER_READ buffer barrier BEFORE the marcher dispatch, then clears
//! the flag via [`LightTableStaging::mark_uploaded`]. The FIRST seed uses the
//! fence-waited `upload_initial`; only the on-change re-upload is async.

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, IsEnabled, Or, Query};
use boyko_ecs::ecs::core::system::into_system::IntoSystem;
use boyko_ecs::ecs::core::system::system::System;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Resource;

use crate::light::{
    DirectionalLight, GpuLight, LightEnabled, LightHeaderGpu, LightTableDirty, LightingConfig,
    MAX_LIGHTS, PointLight, SkyLight, SpotLight,
};

/// The byte size of the light SSBO's leading header region (`LightHeaderGpu`, 64 B).
pub const LIGHT_HEADER_BYTES: usize = core::mem::size_of::<LightHeaderGpu>();
/// The byte size of one `GpuLight` table element (48 B).
pub const GPU_LIGHT_BYTES: usize = core::mem::size_of::<GpuLight>();

/// The reused light-table staging scratch + the on-change dirty flag (Principle 0).
///
/// `scratch` holds the contiguous `[LightHeaderGpu || GpuLight[]]` bytes the GPU table
/// mirrors; it is sized once to `LIGHT_HEADER_BYTES + MAX_LIGHTS * GPU_LIGHT_BYTES` and
/// refilled in place — no per-frame allocation. `dirty` is set by [`collect_lights`] on
/// a change and cleared by [`Self::mark_uploaded`] after the recorder copies the bytes.
#[derive(Resource)]
pub struct LightTableStaging {
    /// `[LightHeaderGpu || GpuLight[]]` host bytes; the GPU table is its mirror.
    scratch: Vec<u8>,
    /// Valid byte length in `scratch` (`LIGHT_HEADER_BYTES + light_count * GPU_LIGHT_BYTES`).
    used_bytes: usize,
    /// Set on a changed frame; the recorder records the copy + barrier when set.
    dirty: bool,
    /// `true` until the FIRST seed upload has run (so the seed path can be distinguished
    /// from the async on-change path).
    seeded: bool,
}

impl Default for LightTableStaging {
    #[inline]
    fn default() -> Self {
        // Preallocate the worst-case table once: header + MAX_LIGHTS elements. The
        // collection refills this in place (Principle 5 — no frame-path alloc).
        let cap = LIGHT_HEADER_BYTES + (MAX_LIGHTS as usize) * GPU_LIGHT_BYTES;
        let mut scratch = vec![0u8; cap];
        // Seed with an empty default table (count 0, identity exposure) so a never-changed
        // world still has a valid header to seed the device buffer with.
        let used = write_light_table(&mut scratch, &[], &[], &[], &[], &LightingConfig::default());
        Self { scratch, used_bytes: used, dirty: true, seeded: false }
    }
}

impl LightTableStaging {
    /// The currently-valid table bytes (`[header || GpuLight[]]`).
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.scratch[..self.used_bytes]
    }

    /// The pending on-change upload bytes if a change is queued, else `None` (idle frame
    /// → the recorder records nothing).
    #[inline]
    pub fn pending_upload(&self) -> Option<&[u8]> {
        if self.dirty && self.seeded {
            Some(self.bytes())
        } else {
            None
        }
    }

    /// Whether the FIRST seed upload still needs to run (the fence-waited setup path).
    #[inline]
    pub fn needs_seed(&self) -> bool {
        !self.seeded
    }

    /// Marks the FIRST seed upload done (called after the setup `upload_initial`).
    #[inline]
    pub fn mark_seeded(&mut self) {
        self.seeded = true;
        self.dirty = false;
    }

    /// Clears the dirty flag after the recorder has copied the staging bytes.
    #[inline]
    pub fn mark_uploaded(&mut self) {
        self.dirty = false;
    }
}

/// Writes `[LightHeaderGpu || GpuLight[]]` into `dst`, returning the valid byte length.
///
/// The no-`P` front block (directionals, then sky) is laid out first so the L0a resolve
/// can loop `[0..l0a_count)` without touching the point/spot rows that need `gViewT`/`P`
/// (L0b). Point + spot follow (their `from_*` bakes the L0b intensity). A pure function
/// over the component slices — host-testable without an ECS world. Caller guarantees
/// `dst` is sized for the worst case (`Default` does this).
///
/// Thin slice-wrapper over [`fold_light_table`]; the per-frame collection feeds the ECS
/// query iterators to [`fold_light_table`] directly (no intermediate `Vec`).
pub fn write_light_table(
    dst: &mut [u8],
    directionals: &[DirectionalLight],
    skies: &[SkyLight],
    points: &[PointLight],
    spots: &[SpotLight],
    cfg: &LightingConfig,
) -> usize {
    fold_light_table(
        dst,
        directionals.iter(),
        skies.iter(),
        points.iter(),
        spots.iter(),
        cfg,
    )
}

/// Folds the live lights — taken as four borrowing iterators — directly into `dst` as
/// `[LightHeaderGpu || GpuLight[]]`, returning the valid byte length.
///
/// The single buffer is the SOLE sink (Principle 1/5 — no intermediate `Vec`): each
/// `&Light` is converted via the matching `GpuLight::from_*` and written straight into
/// `dst` while the running byte offset and the two header counts are tracked (`l0a_count`
/// counts directionals plus sky, `point_spot_count` counts point plus spot). The body is
/// written at [`LIGHT_HEADER_BYTES`] and the header backfilled once the counts are known —
/// byte-identical to writing the header up front, since the two regions are disjoint.
/// Layout order is preserved: directionals → sky → point → spot.
///
/// Caller guarantees `dst` is sized for the worst case (`Default` does this). The
/// iterators are walked exactly once each, in the table-order above.
///
/// # Overflow clamp (release-safe)
///
/// The table is hard-capped at [`MAX_LIGHTS`] rows: `dst`'s body region holds exactly
/// `MAX_LIGHTS` `GpuLight` slots. A live count above the cap is clamped here in ALL build
/// profiles — a saturating gate (`written == MAX_LIGHTS`) is checked once before each
/// write, so both the write AND the count stop at the cap. This is the sole bounds
/// enforcement for the `write_pod` `copy_nonoverlapping` (whose own check is a
/// `debug_assert` that compiles out in release), so it must NOT be a debug-only guard.
/// The gate adds one predictable compare per light (never taken until the cap); the
/// dropped-lights branch is `#[cold]`. The returned length therefore never exceeds
/// `LIGHT_HEADER_BYTES + MAX_LIGHTS * GPU_LIGHT_BYTES`, and the header counts sum to at
/// most `MAX_LIGHTS`.
pub fn fold_light_table<'a>(
    dst: &mut [u8],
    directionals: impl Iterator<Item = &'a DirectionalLight>,
    skies: impl Iterator<Item = &'a SkyLight>,
    points: impl Iterator<Item = &'a PointLight>,
    spots: impl Iterator<Item = &'a SpotLight>,
    cfg: &LightingConfig,
) -> usize {
    // The body starts after the header region; walk the iterators in table order,
    // converting + writing each light in place and counting the two header blocks.
    // `written` is the running total across all four kinds — the single quantity the
    // saturating cap gates on, so no write can spill past the `MAX_LIGHTS`-slot body.
    let mut off = LIGHT_HEADER_BYTES;
    let mut written: u32 = 0;
    let mut l0a_count: u32 = 0;
    for d in directionals {
        if written == MAX_LIGHTS {
            return finish_folded_overflow(dst, l0a_count, 0, off, cfg);
        }
        write_pod(dst, off, &GpuLight::from_directional(d));
        off += GPU_LIGHT_BYTES;
        written += 1;
        l0a_count += 1;
    }
    for s in skies {
        if written == MAX_LIGHTS {
            return finish_folded_overflow(dst, l0a_count, 0, off, cfg);
        }
        write_pod(dst, off, &GpuLight::from_sky(s));
        off += GPU_LIGHT_BYTES;
        written += 1;
        l0a_count += 1;
    }
    let mut point_spot_count: u32 = 0;
    for p in points {
        if written == MAX_LIGHTS {
            return finish_folded_overflow(dst, l0a_count, point_spot_count, off, cfg);
        }
        write_pod(dst, off, &GpuLight::from_point(p));
        off += GPU_LIGHT_BYTES;
        written += 1;
        point_spot_count += 1;
    }
    for s in spots {
        if written == MAX_LIGHTS {
            return finish_folded_overflow(dst, l0a_count, point_spot_count, off, cfg);
        }
        write_pod(dst, off, &GpuLight::from_spot(s));
        off += GPU_LIGHT_BYTES;
        written += 1;
        point_spot_count += 1;
    }
    debug_assert!(
        l0a_count + point_spot_count <= MAX_LIGHTS,
        "invariant: live light count must not exceed MAX_LIGHTS"
    );

    finish_folded(dst, l0a_count, point_spot_count, off, cfg)
}

/// Backfills the light-table header once the two block counts are known and returns the
/// valid byte length. Split out of [`fold_light_table`] so the cap-reached early exit
/// (which stops writing rows mid-iteration) closes the table identically to the full walk:
/// the header region `[0..LIGHT_HEADER_BYTES)` is disjoint from the body, so writing it
/// last is byte-identical to writing it up front.
#[inline]
fn finish_folded(
    dst: &mut [u8],
    l0a_count: u32,
    point_spot_count: u32,
    off: usize,
    cfg: &LightingConfig,
) -> usize {
    let header = LightHeaderGpu::new(l0a_count, point_spot_count, cfg);
    write_pod(dst, 0, &header);
    off
}

/// The cap-reached table close: logs the overflow once (rate-limited to the first
/// offender), then backfills the header for the `MAX_LIGHTS` rows already written.
///
/// Marked `#[cold]` + `#[inline(never)]` so the drop path stays entirely off the hot fold's
/// straight-line code; only the single `written == MAX_LIGHTS` compare per light remains on
/// the hot path.
#[cold]
#[inline(never)]
fn finish_folded_overflow(
    dst: &mut [u8],
    l0a_count: u32,
    point_spot_count: u32,
    off: usize,
    cfg: &LightingConfig,
) -> usize {
    use core::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    // Relaxed: this is a best-effort one-shot log guard, not a synchronization edge — a
    // rare double-log under a race is harmless and no data is published through this flag.
    if !LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "boyko_render: light table overflow — more than MAX_LIGHTS ({MAX_LIGHTS}) \
             enabled lights; extras are dropped from the GPU table"
        );
    }
    finish_folded(dst, l0a_count, point_spot_count, off, cfg)
}

/// Copies a `#[repr(C)]` POD's bytes into `dst` at `off`. The POD types are
/// `align(16)` / `align(4)` with no padding holes the GPU reads differently, so a
/// byte-copy of `size_of::<T>()` bytes reproduces the std430 element.
#[inline]
fn write_pod<T: Copy>(dst: &mut [u8], off: usize, value: &T) {
    let size = core::mem::size_of::<T>();
    debug_assert!(off + size <= dst.len(), "invariant: POD write stays within the scratch");
    // SAFETY: `value` is a `Copy` POD of `size` bytes; `src` reads exactly those bytes.
    // `dst[off..off+size]` is in bounds in ALL build profiles: `dst` is sized to
    // `LIGHT_HEADER_BYTES + MAX_LIGHTS * GPU_LIGHT_BYTES` (`LightTableStaging::default`),
    // the header write is at `off == 0`, and every body write is gated by the release-safe
    // `written == MAX_LIGHTS` cap in `fold_light_table` — so `off` never exceeds
    // `LIGHT_HEADER_BYTES + (MAX_LIGHTS - 1) * GPU_LIGHT_BYTES` for a `GpuLight` write. The
    // `debug_assert` above is a redundant witness of that gate, not the enforcement. The two
    // regions never overlap (distinct allocations). No `T` invariants are violated by
    // reading its raw bytes (it is a plain-data std430 element).
    unsafe {
        let src = (value as *const T).cast::<u8>();
        core::ptr::copy_nonoverlapping(src, dst.as_mut_ptr().add(off), size);
    }
}

/// The L0 collection system (Decision 4) — `Changed`-gated.
///
/// On a frame where any light component or [`LightingConfig`] changed, folds the live
/// lights into [`LightTableStaging`] and sets the dirty flag; an unchanged frame returns
/// immediately (the static-scene fast path — zero work, zero allocation). The recorder
/// consumes the dirty flag (rung L0-r0).
///
/// L0a resolves directionals + sky; the point/spot rows are collected here too (their
/// resolve path is L0b) so the table is complete the moment L0b wires `gViewT`.
//
// `clippy::needless_pass_by_value`: `Res<_>` / `ResMut<_>` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive the physics systems carry.
// `clippy::too_many_arguments`: an ECS system's arguments ARE its `SystemParam`s; the
// param-injection protocol cannot read a struct of params, so the per-light-kind queries
// (changed + full, four kinds) + the config + the staging are necessarily separate.
#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments, clippy::type_complexity)]
pub fn collect_lights(
    changed_dir: Query<&DirectionalLight, Changed<DirectionalLight>>,
    changed_sky: Query<&SkyLight, Changed<SkyLight>>,
    changed_point: Query<&PointLight, Changed<PointLight>>,
    changed_spot: Query<&SpotLight, Changed<SpotLight>>,
    all_directionals: Query<(&DirectionalLight, IsEnabled<LightEnabled>)>,
    all_skies: Query<(&SkyLight, IsEnabled<LightEnabled>)>,
    all_points: Query<(&PointLight, IsEnabled<LightEnabled>)>,
    all_spots: Query<(&SpotLight, IsEnabled<LightEnabled>)>,
    cfg: Res<LightingConfig>,
    mut staging: ResMut<LightTableStaging>,
    mut dirty: ResMut<LightTableDirty>,
) {
    // Rebuild gate: rebuild iff a light component's Changed tick advanced OR the
    // structural `LightTableDirty` channel is set. Changed alone CANNOT see two events:
    // (1) an O(1) LightEnabled toggle (enable_id/disable_id bumps no tick), and (2) a
    // removed/despawned light (the departed row advances no surviving tick). Both mark
    // LightTableDirty — toggles via the set-light-enabled surface, removals/despawns via
    // the on_remove hook registered first in LightingPlugin::build — so this gate evicts
    // them next frame. The dirty bit is consumed unconditionally after every rebuild (no
    // early-return in between).
    let changed = changed_dir.iter().next().is_some()
        || changed_sky.iter().next().is_some()
        || changed_point.iter().next().is_some()
        || changed_spot.iter().next().is_some();
    if !changed && !dirty.0 {
        return;
    }

    // Rebuild the full table from the live lights (the table is small — MAX_LIGHTS ≤
    // 1024 — so a full rebuild on change is cheaper than a delta map; Decision 4
    // trade-off). Fold the queries DIRECTLY into the preallocated scratch — the
    // worst-case-sized `scratch` is the SOLE sink, no per-frame `Vec` (Principle 1/5).
    // The per-row `IsEnabled<LightEnabled>` bit is `filter_map`'d so a disabled light is
    // dropped BEFORE the fold sees it — the header counts (incremented per write inside
    // `fold_light_table`) stay correct, and the `impl Iterator<Item = &T>` signature is
    // byte-identical (`write_light_table` + its slice unit tests are untouched).
    let staging = &mut *staging;
    let used = fold_light_table(
        &mut staging.scratch,
        all_directionals.iter().filter_map(|(l, en)| en.then_some(l)),
        all_skies.iter().filter_map(|(l, en)| en.then_some(l)),
        all_points.iter().filter_map(|(l, en)| en.then_some(l)),
        all_spots.iter().filter_map(|(l, en)| en.then_some(l)),
        &cfg,
    );
    staging.used_bytes = used;
    staging.dirty = true;
    // Consume the structural-change signal — always reached on every rebuild, so a set
    // bit is never stranded (W2).
    dirty.0 = false;
}

/// Cross-frame state for the light-seed pass ([`seed`](Self::seed)): the eight CACHED
/// light-id systems plus the first-run flag and a reused scratch buffer.
///
/// # Why cached systems (W1)
///
/// The per-row entity id is only reachable through a `Query` system context
/// (`Query::iter_entities`; the exclusive `QueryView` does NOT expose ids — verified). A
/// naive seed would call [`EcsMaster::run_system`] each frame, which rebuilds a fresh
/// `FunctionSystem` and re-runs `initialize` (query-state allocation + archetype matching)
/// on EVERY invocation — four uncached system builds+inits per steady-state frame, which
/// contradicts Principle 1/5 (no per-frame setup/allocation on the hot path).
///
/// Instead this struct OWNS the eight `FunctionSystem` values (the four `Added`-filtered
/// twins for the steady state, the four unfiltered twins for the first run) and runs them
/// through [`EcsMaster::run_cached_system`], so `initialize` is paid ONCE (idempotent FS1).
/// The steady-state cost is therefore four cached-system executions + four archetype scans
/// (each yielding zero rows on a static frame) — no system rebuild, no init, and no
/// allocation when the scans are empty (an empty `Vec` does not allocate).
///
/// The eight system types are unnameable (closure-backed `FunctionSystem`s), so the struct
/// is generic over them; construct it via [`light_seed_state`] and let inference name the
/// type at the (single) capture site.
pub struct LightSeedState<A, S, P, T, Aa, Sa, Pa, Ta> {
    added_dir: A,
    added_sky: S,
    added_point: P,
    added_spot: T,
    all_dir: Aa,
    all_sky: Sa,
    all_point: Pa,
    all_spot: Ta,
    first_run: bool,
    /// Reused id scratch — cleared and refilled each non-static pass (Principle 5).
    ids: Vec<EntityId>,
}

/// Builds the cross-frame [`LightSeedState`] with its eight cached light-id systems.
///
/// The `Added`-filtered systems drive the steady state (zero rows on a static frame); the
/// unfiltered systems drive the one-time first-run full scan (pre-plugin / pre-existing
/// lights). The return type is `impl`-named because the eight `FunctionSystem` types are
/// closure-backed and unnameable; capture it once (e.g. in the plugin's seed closure).
#[expect(
    clippy::type_complexity,
    reason = "eight closure-backed FunctionSystem type params are unnameable; \
              the type is constructed once via this helper and never spelled by callers"
)]
pub fn light_seed_state() -> LightSeedState<
    impl System<Out = Vec<EntityId>>,
    impl System<Out = Vec<EntityId>>,
    impl System<Out = Vec<EntityId>>,
    impl System<Out = Vec<EntityId>>,
    impl System<Out = Vec<EntityId>>,
    impl System<Out = Vec<EntityId>>,
    impl System<Out = Vec<EntityId>>,
    impl System<Out = Vec<EntityId>>,
> {
    LightSeedState {
        added_dir: IntoSystem::into_system(
            |q: Query<&DirectionalLight, Added<DirectionalLight>>| {
                q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
            },
        ),
        added_sky: IntoSystem::into_system(|q: Query<&SkyLight, Added<SkyLight>>| {
            q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
        }),
        added_point: IntoSystem::into_system(|q: Query<&PointLight, Added<PointLight>>| {
            q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
        }),
        added_spot: IntoSystem::into_system(|q: Query<&SpotLight, Added<SpotLight>>| {
            q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
        }),
        all_dir: IntoSystem::into_system(|q: Query<&DirectionalLight>| {
            q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
        }),
        all_sky: IntoSystem::into_system(|q: Query<&SkyLight>| {
            q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
        }),
        all_point: IntoSystem::into_system(|q: Query<&PointLight>| {
            q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
        }),
        all_spot: IntoSystem::into_system(|q: Query<&SpotLight>| {
            q.iter_entities().map(|(id, _)| id).collect::<Vec<_>>()
        }),
        first_run: false,
        ids: Vec::new(),
    }
}

impl<A, S, P, T, Aa, Sa, Pa, Ta> LightSeedState<A, S, P, T, Aa, Sa, Pa, Ta>
where
    A: System<Out = Vec<EntityId>>,
    S: System<Out = Vec<EntityId>>,
    P: System<Out = Vec<EntityId>>,
    T: System<Out = Vec<EntityId>>,
    Aa: System<Out = Vec<EntityId>>,
    Sa: System<Out = Vec<EntityId>>,
    Pa: System<Out = Vec<EntityId>>,
    Ta: System<Out = Vec<EntityId>>,
{
    /// Collects the ids of every light (all four kinds) into `self.ids` (first-run scan).
    ///
    /// Runs the four cached unfiltered systems via [`EcsMaster::run_cached_system`] so
    /// `initialize` is paid once. The world borrow is internal to each call and dropped
    /// before the caller `enable`s the bits.
    fn collect_all_light_ids(&mut self, world: &mut EcsMaster) {
        self.ids.extend(world.run_cached_system(&mut self.all_dir));
        self.ids.extend(world.run_cached_system(&mut self.all_sky));
        self.ids.extend(world.run_cached_system(&mut self.all_point));
        self.ids.extend(world.run_cached_system(&mut self.all_spot));
    }

    /// Collects the ids of every NEWLY-added light into `self.ids` (steady-state scan).
    ///
    /// `Added<*Light>`-filtered twins of [`collect_all_light_ids`](Self::collect_all_light_ids)
    /// — zero rows on a static frame, so the seed is a no-op there (and the empty system
    /// outputs do not allocate).
    ///
    /// # Per-pass change-tick advance (W1)
    ///
    /// [`EcsMaster::run_cached_system`] runs `initialize` (idempotent — seeds the change-tick
    /// window ONCE to the degenerate pre-first-run value) + `run_unsafe`, but it NEVER calls
    /// [`System::set_change_ticks`]. Only the schedule's dispatch loop advances a system's
    /// `(last_run, this_run]` window (schedule.rs:339-340). Because these sub-systems run
    /// INSIDE the seed's exclusive body (not as schedule-dispatched systems), their window
    /// would otherwise stay frozen at the initialize-time value for the life of the process —
    /// so `Added` would NOT mean "added since the previous seed pass" (it would either never
    /// re-report a light spawned after the first frame, or re-report every light every frame
    /// and defeat the static fast path).
    ///
    /// We therefore stamp each sub-system's window before each run, mirroring the schedule's
    /// per-frame snapshot exactly: the sub-system's PREVIOUS `this_run` becomes its new
    /// `last_run`, and its new `this_run` is the world's current frame tick. The seed is an
    /// exclusive system scheduled `.before(collect_lights)` WITHIN `Schedule::run`, which has
    /// already bumped the frame tick (schedule.rs:286), so `world.current_tick()` is the
    /// frame's `this_run`. After this stamp `Added<*Light>` correctly observes only the rows
    /// whose add-tick falls in `(prev_this_run, this_run]` — exactly the lights added since
    /// the previous seed pass (zero on a static frame).
    fn collect_added_light_ids(&mut self, world: &mut EcsMaster) {
        let this_run = world.current_tick();
        Self::run_added(&mut self.added_dir, world, this_run, &mut self.ids);
        Self::run_added(&mut self.added_sky, world, this_run, &mut self.ids);
        Self::run_added(&mut self.added_point, world, this_run, &mut self.ids);
        Self::run_added(&mut self.added_spot, world, this_run, &mut self.ids);
    }

    /// Advances `sys`'s change-tick window to `(prev_this_run, this_run]` (matching
    /// schedule.rs:339-340), then runs it cached and extends `out` with its ids.
    ///
    /// Factored out so the four `Added` sub-systems share one stamp+run path; the
    /// per-pass window advance is what makes `Added` mean "since the previous seed pass"
    /// (see [`collect_added_light_ids`](Self::collect_added_light_ids)).
    fn run_added<Sys>(sys: &mut Sys, world: &mut EcsMaster, this_run: Tick, out: &mut Vec<EntityId>)
    where
        Sys: System<Out = Vec<EntityId>>,
    {
        // `initialize` is idempotent (FS1); the first call seeds the window, after which
        // `meta().this_run()` reads back the PREVIOUS pass's `this_run` to use as the new
        // `last_run` — the same prev/cur snapshot the schedule dispatch performs.
        sys.initialize(world);
        let prev_this_run = sys.meta().this_run();
        sys.set_change_ticks(prev_this_run, this_run);
        out.extend(world.run_cached_system(sys));
    }

    /// Exclusive seed pass (`&mut EcsMaster`): enables the [`LightEnabled`] bit on lights
    /// that have not been seeded yet, flipping bits IMMEDIATELY (not via deferred
    /// `Commands`).
    ///
    /// Scheduled `.before(collect_lights)` so the freshly-enabled bits AND the
    /// [`LightTableDirty`] mark are visible to `collect_lights` in the SAME schedule pass
    /// (W2 — zero added latency; a deferred seed would apply only after `collect_lights`
    /// already folded, hiding the bit for one frame).
    ///
    /// - **First run** (`!self.first_run`): scan every light (the four kinds) and `enable`
    ///   `LightEnabled` on each — catching pre-plugin / pre-existing lights and test scenes
    ///   that spawned lights before this system. Sets `self.first_run = true`.
    /// - **Subsequent runs**: scan `Added<*Light>` rows only (zero on a static frame) and
    ///   `enable` each.
    ///
    /// If any row was enabled, marks [`LightTableDirty`] so the same-pass `collect_lights`
    /// rebuilds with the newly-visible lights.
    ///
    /// # Cost (corrected, W1)
    ///
    /// Exclusive (not `Commands`) because the immediate `&mut self` `enable` is the only way
    /// to make the bit live in-pass. The steady-state pass is NOT free: it runs four CACHED
    /// systems (`initialize` already paid — see [`LightSeedState`]) over four archetype
    /// scans. On a static / no-new-light frame each scan yields zero rows, so no bit flip,
    /// no dirty mark, and no allocation occur; only the four cached executions + the
    /// exclusive serialization point remain — bounded and constant.
    pub fn seed(&mut self, world: &mut EcsMaster) {
        // Collect the ids first (the query view borrows the world), then enable (needs
        // `&mut`). `self.ids` is the reused scratch (cleared here, refilled below).
        self.ids.clear();
        if self.first_run {
            self.collect_added_light_ids(world);
        } else {
            self.collect_all_light_ids(world);
            self.first_run = true;
        }

        if self.ids.is_empty() {
            return;
        }

        // Index-walk (not a drain): `self.ids` and the `&mut world` arg are disjoint borrows,
        // so we can read an id out of `self.ids` and call `world.enable` in the same loop body
        // without aliasing. The buffer keeps its capacity for reuse — it is `clear()`ed at the
        // top of the NEXT pass (line above), not emptied here.
        for i in 0..self.ids.len() {
            let id = self.ids[i];
            // `get_entity` resolves the live `Entity` (with generation); a stale / dead id
            // is a `None` no-op. `enable` is the O(1) immediate `&mut self` bit flip.
            if let Some(entity) = world.get_entity(id) {
                world.enable::<LightEnabled>(entity);
            }
        }

        // At least one row was (re)enabled this pass — mark the table for an in-pass
        // rebuild.
        if let Some(d) = world.try_resource_mut::<LightTableDirty>() {
            d.0 = true;
        }
    }
}

/// Enables/disables a light's [`LightEnabled`] bit at runtime and marks the light table
/// for rebuild — the IMMEDIATE (`&mut EcsMaster`) entry point.
///
/// Use this from setup / test code (and the seed uses the same `enable`/`disable`); for
/// in-system gameplay use the deferred [`SetLightEnabledById`] command. Marking the table
/// dirty in the SAME call is mandatory: the bit flip bumps no `Changed` tick (Decision 2),
/// so without the dirty mark `collect_lights` would never observe the toggle.
///
/// The `&mut self` exclusive borrow is the documented soundness ground for the bitset's
/// `Relaxed` atomics (do NOT relax to `&self`). A dead / stale `entity` is a silent no-op
/// (the underlying `set_enable_bit` no-ops), so a still-set dirty mark merely triggers one
/// idempotent rebuild.
pub fn set_light_enabled_now(world: &mut EcsMaster, entity: Entity, enabled: bool) {
    debug_assert!(
        world.get_entity(entity.id()).is_some(),
        "invariant: set_light_enabled_now expects a live entity (no-ops on a dead one)"
    );
    if enabled {
        world.enable::<LightEnabled>(entity);
    } else {
        world.disable::<LightEnabled>(entity);
    }
    if let Some(d) = world.try_resource_mut::<LightTableDirty>() {
        d.0 = true;
    }
}

/// Deferred [`Command`] twin of [`set_light_enabled_now`] — toggles a light's
/// [`LightEnabled`] bit and marks the table dirty under the `&mut` apply window.
///
/// Enqueue from an in-system context via `commands.add(SetLightEnabledById { entity, enabled })`.
/// Like the immediate path, it marks [`LightTableDirty`] in the same apply because the bit
/// flip is tickless (Decision 2).
pub struct SetLightEnabledById {
    /// The light entity to toggle.
    pub entity: Entity,
    /// `true` enables the light, `false` disables it.
    pub enabled: bool,
}

impl Command for SetLightEnabledById {
    #[inline]
    fn apply(self, world: &mut EcsMaster) {
        set_light_enabled_now(world, self.entity, self.enabled);
    }
}

/// Gate-5 eviction hook: marks [`LightTableDirty`] when a light DATA component is removed
/// (the entity survives) or the whole entity is despawned (despawn fires `on_remove` per
/// component too — so this single `on_remove` registration subsumes both classes).
///
/// Registered on the four light data components (a bitset tag rejects hooks). A removed /
/// despawned light advances no surviving row's `Changed` tick, so `collect_lights`'s
/// Changed gate alone would never evict it; this hook sets the structural-change channel
/// so the next `collect_lights` rebuilds with the departed light gone.
///
/// Declared `unsafe fn` only to match the [`HookFn`] signature; its body calls ONLY the
/// safe `resource_mut` — there is no `unsafe` block inside.
///
/// [`HookFn`]: boyko_ecs::ecs::core::component::hooks::HookFn
///
/// # Safety
///
/// The caller is always a `trigger_on_remove` dispatch that fires synchronously under the
/// outermost apply's `&mut EcsMaster` (the single-threaded apply window). `resource_mut`
/// returns a `&mut LightTableDirty` into resource storage, which is disjoint from every
/// archetype / pool buffer — so this never aliases the apply's component reborrows. It does
/// no structural mutation, enqueues no commands, and is non-re-entrant (the canonical
/// `on_remove` resource-mark pattern; Phase-14a/19 Miri-TB-proven surface).
pub unsafe fn evict_light(mut dm: DeferredEcsMaster<'_>, _ctx: HookContext) {
    if let Some(d) = dm.resource_mut::<LightTableDirty>() {
        d.0 = true;
    }
}

/// A convenience change filter alias for the full collection path: a light is
/// (re)collected when any light component is `Changed` (which `Added` is a subset of in
/// the engine's change model). Exposed for the scheduler-side wiring.
pub type LightChanged = Or<(
    Changed<DirectionalLight>,
    Changed<SkyLight>,
    Changed<PointLight>,
    Changed<SpotLight>,
)>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::{LightHeaderGpu, LIGHT_HEADER_WORDS};

    /// Reads `LightHeaderGpu` back out of the staging bytes.
    fn read_header(bytes: &[u8]) -> LightHeaderGpu {
        assert!(bytes.len() >= LIGHT_HEADER_BYTES);
        // SAFETY: `bytes` is sized for at least one header (asserted) and the source is a
        // `repr(C, align(16))` POD written via `write_pod`; read it back unaligned-safe.
        let mut h = core::mem::MaybeUninit::<LightHeaderGpu>::uninit();
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                h.as_mut_ptr().cast::<u8>(),
                LIGHT_HEADER_BYTES,
            );
            h.assume_init()
        }
    }

    #[test]
    fn empty_table_is_header_only() {
        let mut scratch = vec![0u8; LIGHT_HEADER_BYTES + 4 * GPU_LIGHT_BYTES];
        let used = write_light_table(&mut scratch, &[], &[], &[], &[], &LightingConfig::default());
        assert_eq!(used, LIGHT_HEADER_BYTES);
        let h = read_header(&scratch);
        assert_eq!(h.light_count(), 0);
        assert_eq!(h.l0a_count(), 0);
        assert_eq!(h.point_spot_count(), 0);
        assert_eq!(h.counts_exposure[1], 1.0);
    }

    #[test]
    fn front_block_counts_directionals_plus_sky() {
        let dirs = [DirectionalLight::new([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0)];
        let skies = [SkyLight::new([0.1, 0.1, 0.12], [0.1, 0.1, 0.12])];
        let pts = [PointLight::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 50.0, 5.0)];
        let mut scratch = vec![0u8; LIGHT_HEADER_BYTES + 8 * GPU_LIGHT_BYTES];
        let used = write_light_table(&mut scratch, &dirs, &skies, &pts, &[], &LightingConfig::default());
        assert_eq!(used, LIGHT_HEADER_BYTES + 3 * GPU_LIGHT_BYTES);
        let h = read_header(&scratch);
        assert_eq!(h.light_count(), 3);
        assert_eq!(h.l0a_count(), 2); // 1 directional + 1 sky
        assert_eq!(h.point_spot_count(), 1);
    }

    #[test]
    fn front_block_is_laid_out_directional_then_sky() {
        let dirs = [DirectionalLight::new([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0)];
        let skies = [SkyLight::new([0.2, 0.2, 0.2], [0.1, 0.1, 0.1])];
        let words = LIGHT_HEADER_WORDS + 2 * (GPU_LIGHT_BYTES / 4);
        let mut scratch = vec![0u8; words * 4];
        write_light_table(&mut scratch, &dirs, &skies, &[], &[], &LightingConfig::default());
        // element 0's kind tag is at word HEADER_WORDS + 3 (dir_kind.w).
        let kind0_off = (LIGHT_HEADER_WORDS + 3) * 4;
        let kind0 = u32::from_ne_bytes(scratch[kind0_off..kind0_off + 4].try_into().unwrap());
        assert_eq!(kind0, crate::light::LIGHT_KIND_DIRECTIONAL);
        // element 1's kind tag is at word HEADER_WORDS + 12 + 3.
        let kind1_off = (LIGHT_HEADER_WORDS + (GPU_LIGHT_BYTES / 4) + 3) * 4;
        let kind1 = u32::from_ne_bytes(scratch[kind1_off..kind1_off + 4].try_into().unwrap());
        assert_eq!(kind1, crate::light::LIGHT_KIND_SKY);
    }

    #[test]
    fn staging_default_seeds_a_valid_empty_table_and_is_dirty() {
        let s = LightTableStaging::default();
        // The seed has not run yet, so there is no async pending upload.
        assert!(s.needs_seed());
        assert!(s.pending_upload().is_none());
        let h = read_header(s.bytes());
        assert_eq!(h.light_count(), 0);
    }

    /// The worst-case scratch capacity `Default` allocates: header + `MAX_LIGHTS` slots.
    const WORST_CASE_CAP: usize = LIGHT_HEADER_BYTES + (MAX_LIGHTS as usize) * GPU_LIGHT_BYTES;

    #[test]
    fn overflow_of_a_single_kind_clamps_to_max_lights_without_writing_past_scratch() {
        // `MAX_LIGHTS + 1` enabled point lights folded into the exact `Default`-sized
        // scratch: the +1 light must be dropped, and no byte may be written past the cap.
        let over = (MAX_LIGHTS as usize) + 1;
        let pts: Vec<PointLight> =
            (0..over).map(|_| PointLight::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 50.0, 5.0)).collect();

        // Guard bytes past the worst case would catch a spill; sizing `dst` to the exact
        // worst case means any write past it is an OOB the test allocator/Miri would flag.
        let mut scratch = vec![0u8; WORST_CASE_CAP];
        let used = fold_light_table(
            &mut scratch,
            [].iter(),
            [].iter(),
            pts.iter(),
            [].iter(),
            &LightingConfig::default(),
        );

        // The returned length is exactly the full table (header + MAX_LIGHTS rows) — never
        // more — so the recorder never copies past the SSBO mirror.
        assert_eq!(used, WORST_CASE_CAP);
        let h = read_header(&scratch);
        assert_eq!(h.light_count(), MAX_LIGHTS);
        assert_eq!(h.l0a_count(), 0);
        assert_eq!(h.point_spot_count(), MAX_LIGHTS);
    }

    #[test]
    fn overflow_across_kinds_gates_on_the_running_total_not_per_kind() {
        // The cap gates on the running total across all four kinds: MAX_LIGHTS directionals
        // fill the table, then a sky, points, and spots must ALL be dropped — proving the
        // gate is a single cross-kind counter, not a per-loop reset.
        let dirs: Vec<DirectionalLight> = (0..MAX_LIGHTS)
            .map(|_| DirectionalLight::new([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0))
            .collect();
        let skies = [SkyLight::new([0.1, 0.1, 0.12], [0.1, 0.1, 0.12])];
        let pts = [PointLight::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 50.0, 5.0)];
        let spots = [SpotLight::new(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            50.0,
            5.0,
            20.0,
            30.0,
        )];

        let mut scratch = vec![0u8; WORST_CASE_CAP];
        let used = fold_light_table(
            &mut scratch,
            dirs.iter(),
            skies.iter(),
            pts.iter(),
            spots.iter(),
            &LightingConfig::default(),
        );

        assert_eq!(used, WORST_CASE_CAP);
        let h = read_header(&scratch);
        assert_eq!(h.light_count(), MAX_LIGHTS);
        // All MAX_LIGHTS rows are directionals; the sky/point/spot beyond the cap are gone.
        assert_eq!(h.l0a_count(), MAX_LIGHTS);
        assert_eq!(h.point_spot_count(), 0);
    }
}
