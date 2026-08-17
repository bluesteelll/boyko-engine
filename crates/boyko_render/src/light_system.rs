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

use boyko_ecs::ecs::constants::pool_reserve_rows;
use boyko_ecs::ecs::core::asset::register_asset_layout;
use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, IsEnabled, Or, Query};
use boyko_ecs::ecs::core::system::into_system::IntoSystem;
use boyko_ecs::ecs::core::system::system::System;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_log::codes::{OnceSite, W2201, W2204};
use boyko_macros::{Resource, SystemSet};

use crate::light::{
    DirectionalLight, GpuLight, LightEnabled, LightHeaderGpu, LightTableDirty, LightingConfig,
    MAX_LIGHTS, PointLight, SkyLight, SpotLight,
};
use crate::shadow_atlas::{PunctualSlotAssignment, SLOT_NONE, pack_atlas_slot};

/// The byte size of the light SSBO's leading header region (`LightHeaderGpu`, 64 B).
pub const LIGHT_HEADER_BYTES: usize = core::mem::size_of::<LightHeaderGpu>();
/// The byte size of one `GpuLight` table element (48 B).
pub const GPU_LIGHT_BYTES: usize = core::mem::size_of::<GpuLight>();

/// `boyko-W2201`'s per-site `Once` latch — the light-table overflow report.
///
/// A `Once` latch is PROCESS state, so both of this module's latches are named module-level
/// `static`s rather than `static`s tucked inside their reporters: an observer must be able to
/// reset one, or its green only means "nothing else in this binary tripped this condition first".
/// Four sibling tests here fold NaN lights for reasons of their own, and they measurably did.
/// See [`boyko_log::codes::OnceSite::reset`].
pub(crate) static W2201_SITE: OnceSite = OnceSite::new();

/// `boyko-W2204`'s per-site `Once` latch — the non-finite-light drop report. Separate from
/// [`W2201_SITE`] because the two are separate codes; see that static's doc.
pub(crate) static W2204_SITE: OnceSite = OnceSite::new();

/// The staged-light-table WRITE GENERATION (host plan D5) — a monotonic counter
/// bumped by [`collect_lights`] exactly once per ACTUAL staging rewrite (the rebuild is
/// `Changed` + [`LightTableDirty`]-gated, so a static frame never bumps it).
///
/// # The host protocol (writer-side, deterministic)
///
/// A ringed host keeps `light_uploaded_gen: [u64; FRAMES_IN_FLIGHT]` (seeded
/// `u64::MAX`) and rewrites in-flight slot `s`'s staging iff
/// `light_uploaded_gen[s] != generation` — so BOTH slots catch up over the two frames
/// following a change and an unchanged frame writes nothing. `u64` never wraps in
/// practice (2⁶⁴ rebuilds); `wrapping_add` documents the intent regardless.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightTableGeneration(pub u64);

/// The reused light-table staging scratch + the on-change dirty flag (Principle 0).
///
/// `scratch` holds the contiguous `[LightHeaderGpu || GpuLight[]]` bytes the GPU table
/// mirrors; it is a [`ScratchColumn<u8>`] (the same `ComponentPool`-backed, VM-native
/// transient-scratch primitive [`MeshRenderScratch`](crate::mesh_draw::MeshRenderScratch)
/// uses — Principle 0, not a `std::Vec` side store) fixed, once at construction, to exactly
/// `LIGHT_HEADER_BYTES + MAX_LIGHTS * GPU_LIGHT_BYTES` LIVE elements — never cleared or
/// resized afterward, because [`fold_light_table_slotted`] writes THROUGH the whole buffer
/// at arbitrary offsets (`write_pod` at a running byte cursor), not via append-only `push`.
/// Refilled IN PLACE on a change — no per-frame allocation. `dirty` is set by
/// [`collect_lights`] on a change and cleared by [`Self::mark_uploaded`] after the recorder
/// copies the bytes.
#[derive(Resource)]
pub struct LightTableStaging {
    /// `[LightHeaderGpu || GpuLight[]]` host bytes; the GPU table is its mirror. Always
    /// exactly `LIGHT_HEADER_BYTES + MAX_LIGHTS * GPU_LIGHT_BYTES` live elements (see the
    /// struct doc).
    scratch: ScratchColumn<u8>,
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
        let byte_id = register_asset_layout::<u8>(None);
        let mut scratch = ScratchColumn::new(byte_id, pool_reserve_rows(core::mem::size_of::<u8>()));
        {
            // Fill to the fixed worst-case capacity ONCE, at setup: `fold_light_table_slotted`
            // writes through the WHOLE `cap`-sized buffer at arbitrary offsets (not via `push`),
            // so the column must present `cap` live (zeroed) elements before any fold runs.
            // Mirrors `fit_len` (mesh_draw.rs) — a push loop over the already-reserved
            // `pool_reserve_rows`-class backing, never a realloc.
            let mut view = scratch.build_view();
            for _ in 0..cap {
                view.push(0u8);
            }
        }
        // Seed with an empty default table (count 0, identity exposure) so a never-changed
        // world still has a valid header to seed the device buffer with.
        let used = write_light_table(
            scratch.build_view().as_mut_slice(),
            &[],
            &[],
            &[],
            &[],
            &LightingConfig::default(),
        );
        Self { scratch, used_bytes: used, dirty: true, seeded: false }
    }
}

impl LightTableStaging {
    /// The currently-valid table bytes (`[header || GpuLight[]]`).
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.scratch.as_read_slice()[..self.used_bytes]
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
    // The un-slotted path: no punctual light carries an atlas base, so every point/spot row keeps
    // the raw kind word `from_point` / `from_spot` produce (`SLOT_NONE` base ⇒ no pack). Byte-
    // identical to the pre-Inc-1-GPU fold — the slice unit tests + `write_light_table` are pinned
    // on this signature.
    fold_light_table_slotted(
        dst,
        directionals,
        skies,
        points.map(|p| (SLOT_NONE, p)),
        spots.map(|s| (SLOT_NONE, s)),
        cfg,
    )
}

/// The slot-aware fold — the Inc-1-GPU light-table assembly. Identical to [`fold_light_table`]
/// except each point/spot row is tagged with its RESOLVED atlas base (`base`): a real base
/// (`base != SLOT_NONE`) is packed into that light's kind word via [`pack_atlas_slot`] so the
/// shader's `light_atlas_slot(L.kind)` decodes the light's OWN cube/perspective base; a `SLOT_NONE`
/// base leaves the kind word UNTOUCHED (byte-identical to the un-slotted path — the analytic
/// fallback), which is why a non-`CastsPunctualShadow` light and a slot-loser produce the SAME
/// bytes as before the wiring.
///
/// The per-light base comes from the entity-keyed [`PunctualSlotAssignment`] handoff the
/// [`resolve_shadow_atlas`](crate::shadow_atlas::resolve_shadow_atlas) publishes; the caller
/// ([`collect_lights`]) resolves it per row via `PunctualSlotAssignment::base_for` before the fold.
///
/// # Punctual validity gate (release-safe)
///
/// Every point/spot row is checked by [`punctual_row_is_cullable`] BEFORE it is written; a row
/// that fails is DROPPED (not written, and not counted in the header's `point_spot_count`) but it
/// IS tallied, and the first fold that drops anything reports `boyko-W2204` with the tally. This
/// sentence used to end "and the first drop logs once", which was true and useless: the count the
/// one report carried was always one, because the reporter ran per drop. This is the
/// second release-visible gate on this path, and it exists for the same reason as the
/// `written == MAX_LIGHTS` one: the value it rejects would otherwise reach a consumer that
/// cannot defend itself. See that predicate's doc for what a non-finite centre does to the
/// clustered cull.
///
/// Caller guarantees `dst` is sized for the worst case (`Default` does this). The iterators are
/// walked exactly once each, in table order (directionals → sky → point → spot).
pub fn fold_light_table_slotted<'a>(
    dst: &mut [u8],
    directionals: impl Iterator<Item = &'a DirectionalLight>,
    skies: impl Iterator<Item = &'a SkyLight>,
    points: impl Iterator<Item = (u32, &'a PointLight)>,
    spots: impl Iterator<Item = (u32, &'a SpotLight)>,
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
    // Rung L8a: the non-finite drops are COUNTED here and reported once at the end of the fold,
    // rather than each calling a `#[cold]` reporter that a latch then threw away. The increment
    // sits on the `continue` arm the row was already taking, so the straight-line cost of a fold
    // that drops nothing is one `u32` compare after the loops.
    let mut dropped: u32 = 0;
    for (base, p) in points {
        if written == MAX_LIGHTS {
            if dropped != 0 {
                report_dropped_non_finite_lights(dropped);
            }
            return finish_folded_overflow(dst, l0a_count, point_spot_count, off, cfg);
        }
        if !punctual_row_is_cullable(p.position, p.range) {
            dropped += 1;
            continue;
        }
        write_pod(dst, off, &slot_pack(GpuLight::from_point(p), base));
        off += GPU_LIGHT_BYTES;
        written += 1;
        point_spot_count += 1;
    }
    for (base, s) in spots {
        if written == MAX_LIGHTS {
            if dropped != 0 {
                report_dropped_non_finite_lights(dropped);
            }
            return finish_folded_overflow(dst, l0a_count, point_spot_count, off, cfg);
        }
        if !punctual_row_is_cullable(s.position, s.range) {
            dropped += 1;
            continue;
        }
        write_pod(dst, off, &slot_pack(GpuLight::from_spot(s), base));
        off += GPU_LIGHT_BYTES;
        written += 1;
        point_spot_count += 1;
    }
    if dropped != 0 {
        report_dropped_non_finite_lights(dropped);
    }
    debug_assert!(
        l0a_count + point_spot_count <= MAX_LIGHTS,
        "invariant: live light count must not exceed MAX_LIGHTS"
    );

    finish_folded(dst, l0a_count, point_spot_count, off, cfg)
}

/// `true` iff a punctual light's cull sphere is a well-defined one: a FINITE centre and a
/// non-NaN radius. The gate [`fold_light_table_slotted`] drops a point/spot row on.
///
/// # What a non-finite row did before this gate (traced into the cull)
///
/// `cluster_cull.hlsl`'s `sq_dist_point_aabb` is `d = max(max(aabb_min - c, c - aabb_max), 0)`
/// then `sd = d·d`, and DXC lowers those `max`es to GLSL.std.450 **`NMax`** — measured on the
/// committed module, which carries 18 `NMax` / 8 `NMin` and **zero** `FMax`/`FMin`. `NMax`
/// returns the NON-NaN operand when exactly one operand is NaN, so:
///
/// * **NaN centre.** Both inner operands on that axis are NaN; whatever the (spec-undefined)
///   both-NaN inner result is, the outer `NMax(·, 0.0)` has a non-NaN operand and yields `0.0`.
///   The axis drops out of the distance entirely. With all three components NaN, `sd == 0.0`,
///   so `sd <= r·r` holds for **every** light/froxel pair and the row is appended to **every
///   froxel**. At the default 16×9×24 grid that is 3456 `LightIndexList` entries for ONE bad
///   light — 21 % of `INDEX_LIST_CAP` — and they compete for the O2 clamp-and-drop caps, so a
///   single NaN-positioned light does not merely mis-light: it **evicts correct lights**.
///   Downstream the shading loop then computes `l = L.pos - P` = NaN on every pixel it reaches.
/// * **±inf centre.** `aabb_min - inf = -inf` and `inf - aabb_max = +inf`, so `NMax` gives
///   `+inf`, `sd = +inf`, and the ordered `sd <= r·r` is false — the row is rejected
///   everywhere. Cheap in the index list, but it is exactly the case that costs the
///   hierarchical arm its byte-identity with the flat arm (plan §5.2 Premise F, Case B).
/// * **NaN radius.** `sd <= NaN` is false at both levels, so the arms still agree and the cull
///   drops it — but the FLAT (non-clustered) shading paths still read the row and fold a NaN
///   attenuation into the pixel.
///
/// Both centre cases apply identically to the base and hierarchical arms; neither is a memory-
/// safety issue (`ps_n` bounds every read and `fi < capacity` every write, and neither involves
/// the centre). This predicate is the closure of that premise, discharged where the plan filed
/// it: on the host, one gate, before the row is ever written.
///
/// # Policy: reject-and-skip, not clamp, not panic
///
/// **Not clamp** — there is no meaningful clamp of a NaN centre. Substituting the origin
/// teleports the light into the middle of the scene and INVENTS lighting the author never
/// wrote; a missing light is a visible, debuggable absence, an invented one is not.
/// **Not panic** — this is live, per-frame, gameplay-authored ECS data (one bad
/// `GlobalTransform`, one divide-by-zero in a gameplay curve), not a build-time configuration
/// invariant. Killing the process on a transient data glitch is not a policy this path takes
/// anywhere else; the SAME function already answers its other release-visible gate — the
/// `MAX_LIGHTS` overflow — with drop-and-report-once (`boyko-W2201`), and this matches it in
/// shape. The two carry DIFFERENT codes because they need different fixes: one says the scene
/// has too many lights, the other says one of them has a NaN.
///
/// # Cost
///
/// Four ordered compares per point/spot row (`is_finite` / `is_nan` each lower to a single
/// compare), on a path that already writes 48 bytes per row, and the branch resolves the same
/// way on every row of every well-formed frame — so it is perfectly predicted. Directional and
/// sky rows are NOT checked: they carry no cull centre (`LightElem::pos` is the sky's ground
/// colour on a sky row), so there is nothing here to validate.
///
/// An INFINITE radius is deliberately accepted: `r·r = +inf` is a totally-ordered comparand,
/// both cull levels agree on it, and "a light that reaches everywhere" is a coherent (if
/// unwise) authoring choice — unlike a NaN, it is not a broken value.
#[inline]
fn punctual_row_is_cullable(position: [f32; 3], range: f32) -> bool {
    position[0].is_finite()
        && position[1].is_finite()
        && position[2].is_finite()
        && !range.is_nan()
}

/// Reports `boyko-W2204` once: how many punctual lights this fold dropped for carrying a
/// non-finite position or a NaN range.
///
/// **The count is the point, and it is why this moved to the END of the fold.** The migration
/// ledger's row for these sites promised "the dropped count is now reported, which the one-shot
/// latch never did", and the old shape could not deliver it: the reporter was called once per
/// dropped light, so the first call — the only one the latch let through — always meant "one".
/// Accumulating a `u32` in the fold and reporting after the loops costs one increment on a branch
/// that was already taken and one compare per fold call, instead of one `#[cold]` call per
/// dropped light, so it is cheaper on the path that actually drops.
///
/// `#[cold]` + `#[inline(never)]` for the same reason as [`finish_folded_overflow`]: only the
/// four compares of [`punctual_row_is_cullable`] stay on the hot fold's straight-line code.
#[cold]
#[inline(never)]
fn report_dropped_non_finite_lights(dropped: u32) {
    if W2204_SITE.claim() {
        boyko_log::warn!(
            boyko_log::Render,
            W2204,
            "dropped {} point/spot light(s) with a non-finite position (or a NaN range) from \
             the GPU light table; a NaN centre would otherwise be culled INTO every froxel",
            dropped
        );
    }
}

/// Packs a resolved atlas `base` into a punctual [`GpuLight`]'s kind word, but ONLY when `base` is
/// a real layer — a `SLOT_NONE` base returns the light UNCHANGED (byte-identical to the un-slotted
/// fold). Guarding the `SLOT_NONE` case is what preserves the 0%-gate byte-identity:
/// `pack_atlas_slot(kind, SLOT_NONE)` would WRITE the `0x1F` slot field (functionally the analytic
/// fallback, but a different `dir_kind.w`), so the un-slotted rows must skip the pack entirely.
#[inline]
fn slot_pack(mut light: GpuLight, base: u32) -> GpuLight {
    if base != SLOT_NONE {
        light.dir_kind[3] = f32::from_bits(pack_atlas_slot(light.dir_kind[3].to_bits(), base));
    }
    light
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
    if W2201_SITE.claim() {
        // WHAT THIS CANNOT SAY, and why it does not try. The ledger's row for this site asked for
        // a dropped count. There is none to give: `fold_light_table_slotted` takes `impl
        // Iterator`s, its doc pins "walked exactly once each", and this function is reached by an
        // early `return` — so at the moment of the report nothing has looked at the remainder, and
        // producing a count would mean draining iterators the contract says are not drained, on
        // the overflow path, purely to make a number. What the site DOES know is the cap it hit
        // and the rows that made it, so that is what it reports. `boyko-W2204` carries a real
        // count because at that site one exists.
        boyko_log::warn!(
            boyko_log::Render,
            W2201,
            "light table overflow -- more than MAX_LIGHTS ({}) enabled lights; the {} rows \
             already written are kept and every later light is dropped from the GPU table",
            MAX_LIGHTS,
            l0a_count + point_spot_count
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

/// The `Main`-schedule ordering seam that makes [`collect_lights`] visible to a
/// cross-plugin `.before_set(LightCollectSet)` edge — the light-table-FOLD analogue of
/// [`PunctualResolveSet`](crate::shadow_atlas::PunctualResolveSet) (which lets a
/// DIFFERENT plugin publish BEFORE the fold reads it).
///
/// # Why a named set, not add-order
///
/// `collect_lights` is registered inside [`LightingPlugin`](crate::light_plugin::LightingPlugin)'s
/// OWN builder closure (`light_plugin.rs`), so its `SystemKey` is a closure-local variable —
/// invisible to any OTHER plugin's registration site. A writer that feeds the fold from a
/// different plugin (or, like [`sync_cluster_light_gate`](crate::light::sync_cluster_light_gate),
/// from the composing app) cannot express `.before(collect_lights)` directly; it targets THIS
/// set instead, exactly as `resolve_shadow_atlas` targets `PunctualResolveSet`.
///
/// # Why this edge is load-bearing for the cluster lane (VB-P1b-0 C1)
///
/// Unlike the CSM/punctual/SSAO `sync_*_light_gate`s (whose worst case under a stale-by-one-frame
/// header is a wrong SCALAR BIT — benign), [`sync_cluster_light_gate`](crate::light::sync_cluster_light_gate)
/// feeds a GPU BUFFER INDEX: on the very first frame `clusters_enabled` goes `true`, an unordered
/// fold could pack `clusters_enabled=1` together with `dims=0` (the gate hasn't run yet), and the
/// froxel resolve's `cluster_z_slice`/`cluster_linear_index` (`light_table.hlsli`) would then
/// underflow to a huge, out-of-bounds `ClusterGrid` index. That WAS real GPU UB with
/// `robust_buffer_access` disabled; as of VB-P1k all four `ClusterGrid` readers
/// (`vb_resolve`/`vb_shade`/`deferred_pbr`/`forward_opaque`) reject a zero-dims — or
/// over-capacity — header and fall back to the in-bounds flat light scan, so what survives is a
/// one-frame LIGHTING artefact rather than a device fault. This edge is therefore a CORRECTNESS
/// edge now, not the only line against UB, and it stays for that reason.
/// `sync_cluster_light_gate` joins THIS set with `.before_set(LightCollectSet)` so the header
/// always carries valid dims the SAME frame the enabled bit goes hot.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct LightCollectSet;

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
    // The four `Changed`-filtered rebuild-gate probes, grouped into ONE nested-tuple
    // `SystemParam` slot so the system stays within the 12-param arity limit (the entity-keyed
    // `PunctualSlotAssignment` read added a 13th param). A tuple is itself a `SystemParam`, so this
    // is a pure regrouping — the per-query access + `Changed` semantics are unchanged.
    changed: (
        Query<&DirectionalLight, Changed<DirectionalLight>>,
        Query<&SkyLight, Changed<SkyLight>>,
        Query<&PointLight, Changed<PointLight>>,
        Query<&SpotLight, Changed<SpotLight>>,
    ),
    all_directionals: Query<(&DirectionalLight, IsEnabled<LightEnabled>)>,
    all_skies: Query<(&SkyLight, IsEnabled<LightEnabled>)>,
    all_points: Query<(&PointLight, IsEnabled<LightEnabled>)>,
    all_spots: Query<(&SpotLight, IsEnabled<LightEnabled>)>,
    cfg: Res<LightingConfig>,
    assignment: Res<PunctualSlotAssignment>,
    mut staging: ResMut<LightTableStaging>,
    mut dirty: ResMut<LightTableDirty>,
    mut generation: ResMut<LightTableGeneration>,
) {
    // Rebuild gate: rebuild iff a light component's Changed tick advanced OR the
    // structural `LightTableDirty` channel is set. Changed alone CANNOT see two events:
    // (1) an O(1) LightEnabled toggle (enable_id/disable_id bumps no tick), and (2) a
    // removed/despawned light (the departed row advances no surviving tick). Both mark
    // LightTableDirty — toggles via the set-light-enabled surface, removals/despawns via
    // the on_remove hook registered first in LightingPlugin::build — so this gate evicts
    // them next frame. The dirty bit is consumed unconditionally after every rebuild (no
    // early-return in between).
    let (changed_dir, changed_sky, changed_point, changed_spot) = &changed;
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
    // `fold_light_table_slotted`) stay correct.
    //
    // Point + spot rows are iterated WITH their `EntityId` (`iter_entities`) so each is tagged
    // with its resolved atlas base from the entity-keyed `PunctualSlotAssignment` the shadow
    // resolve published. `base_for` returns `SLOT_NONE` for a light that won no slot (or when the
    // resolve is disabled — the empty handoff), so `fold_light_table_slotted` leaves that row's
    // kind word UNTOUCHED (byte-identical to the pre-wiring path); a real base is packed via
    // `pack_atlas_slot` so the shader decodes the light's OWN cube/perspective base.
    let assign = &*assignment;
    let staging = &mut *staging;
    // Disjoint field-projection borrow (mesh_draw.rs precedent): `scratch_view` borrows only
    // `staging.scratch`, so the plain `staging.used_bytes = used;` write below (a distinct
    // field) needs no explicit drop.
    let mut scratch_view = staging.scratch.build_view();
    let used = fold_light_table_slotted(
        scratch_view.as_mut_slice(),
        all_directionals.iter().filter_map(|(l, en)| en.then_some(l)),
        all_skies.iter().filter_map(|(l, en)| en.then_some(l)),
        all_points
            .iter_entities()
            .filter_map(|(id, (l, en))| en.then_some((assign.base_for(id), l))),
        all_spots
            .iter_entities()
            .filter_map(|(id, (l, en))| en.then_some((assign.base_for(id), l))),
        &cfg,
    );
    staging.used_bytes = used;
    staging.dirty = true;
    // Consume the structural-change signal — always reached on every rebuild, so a set
    // bit is never stranded (W2).
    dirty.0 = false;
    // Host plan D5: the writer-side deterministic generation — bumped exactly once per
    // actual staging rewrite (this line is reached only past the rebuild gate above),
    // so the host's per-slot `light_uploaded_gen` compare is exact, never a hash.
    generation.0 = generation.0.wrapping_add(1);
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
    /// Reused id scratch — cleared and refilled each non-static pass (Principle 5). A
    /// [`ScratchColumn<EntityId>`] (Principle 0, not a `std::Vec` side store): unlike
    /// [`MeshRenderScratch`](crate::mesh_draw::MeshRenderScratch)'s fields this state is
    /// closure-captured cross-frame data, not an ECS `Resource` — but `ScratchColumn`
    /// needs no `Resource`/`World` home to construct (it is a bare `ComponentPool`-backed
    /// struct built from a registered [`ComponentId`](boyko_ecs::ecs::identifiers::primitives::ComponentId)),
    /// so it drops in here identically.
    ids: ScratchColumn<EntityId>,
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
        ids: ScratchColumn::new(
            register_asset_layout::<EntityId>(None),
            pool_reserve_rows(core::mem::size_of::<EntityId>()),
        ),
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
        // Each cached system still returns an owned `Vec<EntityId>` (the `System::Out`
        // contract, unchanged by this scratch conversion) — `extend_from_slice` appends its
        // bytes into the reused `ids` column, then the temporary `Vec` drops as before.
        let dir = world.run_cached_system(&mut self.all_dir);
        self.ids.build_view().extend_from_slice(&dir);
        let sky = world.run_cached_system(&mut self.all_sky);
        self.ids.build_view().extend_from_slice(&sky);
        let point = world.run_cached_system(&mut self.all_point);
        self.ids.build_view().extend_from_slice(&point);
        let spot = world.run_cached_system(&mut self.all_spot);
        self.ids.build_view().extend_from_slice(&spot);
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
    fn run_added<Sys>(
        sys: &mut Sys,
        world: &mut EcsMaster,
        this_run: Tick,
        out: &mut ScratchColumn<EntityId>,
    ) where
        Sys: System<Out = Vec<EntityId>>,
    {
        // `initialize` is idempotent (FS1); the first call seeds the window, after which
        // `meta().this_run()` reads back the PREVIOUS pass's `this_run` to use as the new
        // `last_run` — the same prev/cur snapshot the schedule dispatch performs.
        sys.initialize(world);
        let prev_this_run = sys.meta().this_run();
        sys.set_change_ticks(prev_this_run, this_run);
        let added = world.run_cached_system(sys);
        out.build_view().extend_from_slice(&added);
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
        self.ids.build_view().clear();
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
            let id = self.ids.as_read_slice()[i];
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
        // Drives an emitting fold, so it joins this module's serialized set: `boyko-W2201`
        // and `boyko-W2204` are per-SITE `Once` latches, which is PROCESS state, and a
        // sibling that spends one inside the observer's window makes the observer read zero.
        // Resetting fixes ORDER; the lock fixes CONCURRENCY; both are needed.
        let _observe = boyko_log::probe::observe_lock();
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
        // Drives an emitting fold, so it joins this module's serialized set: `boyko-W2201`
        // and `boyko-W2204` are per-SITE `Once` latches, which is PROCESS state, and a
        // sibling that spends one inside the observer's window makes the observer read zero.
        // Resetting fixes ORDER; the lock fixes CONCURRENCY; both are needed.
        let _observe = boyko_log::probe::observe_lock();
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

    // ---- Inc-1-GPU slot-pack wiring (the punctual-shadow regression) ------------------

    use crate::shadow_atlas::light_atlas_slot;

    /// Reads back a table row's kind word (`dir_kind.w`, bit-cast `u32`) at `elem`.
    fn row_kind_word(bytes: &[u8], elem: usize) -> u32 {
        // Each row is `GPU_LIGHT_WORDS` (12) words; `dir_kind.w` is word 3 of the row. The body
        // starts at `LIGHT_HEADER_WORDS`.
        let word = LIGHT_HEADER_WORDS + elem * (GPU_LIGHT_BYTES / 4) + 3;
        let off = word * 4;
        u32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap())
    }

    /// Mirrors `resolve_shadow_atlas`'s assignment-publish loop: builds the entity-keyed handoff
    /// from the per-source resolved slots. `SLOT_NONE` slots (losers) are NOT recorded — their
    /// absence makes `base_for` return `SLOT_NONE`.
    fn assignment_from(spots: &[(EntityId, u32)], points: &[(EntityId, u32)]) -> PunctualSlotAssignment {
        let mut a = PunctualSlotAssignment::EMPTY;
        for &(id, slot) in spots.iter().chain(points.iter()) {
            if slot != SLOT_NONE {
                a = a.with_winner(id, slot);
            }
        }
        a
    }

    /// The masked bug: a 2-source scene (one SPOT + one POINT) where the POINT does NOT get base
    /// 0 (the spot wins base 0, the point lands at base 1). The fold must decode EACH light's kind
    /// word to its REAL base — before the fix, every punctual light decoded to base 0, so the point
    /// sampled the spot's map.
    #[test]
    fn slotted_fold_packs_each_light_its_own_resolved_base() {
        let spot_ent = EntityId(10);
        let point_ent = EntityId(20);
        // The resolve assigned: spot → layer 0, point → cube base 1 (a point costs 6 layers, so
        // base 1 covers layers 1..7; here we assert the packed base index, not the layer count).
        let assign = assignment_from(&[(spot_ent, 0)], &[(point_ent, 1)]);

        let pt = PointLight::new([0.0, 1.0, 0.0], [1.0, 1.0, 1.0], 300.0, 9.0);
        let sp = SpotLight::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 200.0, 8.0, 20.0, 30.0);

        let mut scratch = vec![0u8; LIGHT_HEADER_BYTES + 4 * GPU_LIGHT_BYTES];
        let used = fold_light_table_slotted(
            &mut scratch,
            [].iter(),
            [].iter(),
            core::iter::once((assign.base_for(point_ent), &pt)),
            core::iter::once((assign.base_for(spot_ent), &sp)),
            &LightingConfig::default(),
        );
        assert_eq!(used, LIGHT_HEADER_BYTES + 2 * GPU_LIGHT_BYTES);

        // Table order is point (elem 0) then spot (elem 1).
        let point_kind = row_kind_word(&scratch, 0);
        let spot_kind = row_kind_word(&scratch, 1);

        // The POINT decodes to its OWN base 1 (NOT 0 — the bug) and carries the casts bit.
        assert_eq!(light_atlas_slot(point_kind), 1, "point must decode to its resolved base 1");
        assert_ne!(point_kind & crate::shadow_atlas::CASTS_SHADOW_BIT, 0, "point casts bit set");
        // The kind tag survives the pack.
        assert_eq!(point_kind & 0xFFFF, crate::light::LIGHT_KIND_POINT);

        // The SPOT decodes to its own base 0 and carries the casts bit.
        assert_eq!(light_atlas_slot(spot_kind), 0, "spot must decode to its resolved base 0");
        assert_ne!(spot_kind & crate::shadow_atlas::CASTS_SHADOW_BIT, 0, "spot casts bit set");
        assert_eq!(spot_kind & 0xFFFF, crate::light::LIGHT_KIND_SPOT);
    }

    /// A `CastsPunctualShadow` light that won NO slot (over budget) must pack `SLOT_NONE` — the
    /// analytic fallback — never a stale base 0. Modelled as an entity absent from the assignment
    /// (`base_for` returns `SLOT_NONE`), which the fold leaves UNPACKED.
    #[test]
    fn slotted_fold_loser_packs_slot_none_not_zero() {
        let winner = EntityId(1);
        let loser = EntityId(2);
        let assign = assignment_from(&[(winner, 0)], &[]); // only `winner` got a slot

        let sp_win = SpotLight::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 200.0, 8.0, 20.0, 30.0);
        let sp_lose = SpotLight::new([5.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 200.0, 8.0, 20.0, 30.0);

        let mut scratch = vec![0u8; LIGHT_HEADER_BYTES + 4 * GPU_LIGHT_BYTES];
        fold_light_table_slotted(
            &mut scratch,
            [].iter(),
            [].iter(),
            core::iter::empty(),
            [(assign.base_for(winner), &sp_win), (assign.base_for(loser), &sp_lose)].into_iter(),
            &LightingConfig::default(),
        );

        let win_kind = row_kind_word(&scratch, 0);
        let lose_kind = row_kind_word(&scratch, 1);
        assert_eq!(light_atlas_slot(win_kind), 0, "winner decodes to base 0");
        assert_ne!(win_kind & crate::shadow_atlas::CASTS_SHADOW_BIT, 0, "winner casts bit set");
        // The loser is left UNPACKED (a `base_for` of `SLOT_NONE` skips the pack), so its kind word
        // is byte-identical to the raw kind: slot field 0 AND casts bit CLEAR — the shader takes the
        // analytic fallback off the CLEAR casts bit (never a stale slot-0 SAMPLE, the masked bug).
        assert_eq!(lose_kind & crate::shadow_atlas::CASTS_SHADOW_BIT, 0, "loser casts bit clear");
        assert_eq!(lose_kind, crate::light::LIGHT_KIND_SPOT, "loser kind word untouched (raw)");
    }

    /// The 0%-gate: a point light with NO atlas base (the empty assignment — no
    /// `CastsPunctualShadow`, or the shadow resolve disabled) folds to a kind word BYTE-IDENTICAL
    /// to the pre-wiring path (`fold_light_table`), no slot bits set.
    #[test]
    fn slotted_fold_no_assignment_is_byte_identical_to_unslotted() {
        let pt = PointLight::new([0.0, 1.0, 0.0], [1.0, 1.0, 1.0], 300.0, 9.0);
        let sp = SpotLight::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 200.0, 8.0, 20.0, 30.0);
        let cfg = LightingConfig::default();

        // The un-slotted reference table.
        let mut reference = vec![0u8; LIGHT_HEADER_BYTES + 4 * GPU_LIGHT_BYTES];
        let r_used =
            fold_light_table(&mut reference, [].iter(), [].iter(), [pt].iter(), [sp].iter(), &cfg);

        // The slotted table with the EMPTY assignment (`base_for` == SLOT_NONE for both).
        let empty = PunctualSlotAssignment::EMPTY;
        let mut slotted = vec![0u8; LIGHT_HEADER_BYTES + 4 * GPU_LIGHT_BYTES];
        let s_used = fold_light_table_slotted(
            &mut slotted,
            [].iter(),
            [].iter(),
            core::iter::once((empty.base_for(EntityId(0)), &pt)),
            core::iter::once((empty.base_for(EntityId(0)), &sp)),
            &cfg,
        );

        assert_eq!(r_used, s_used);
        assert_eq!(reference, slotted, "empty-assignment slotted fold is byte-identical");
    }

    /// The host `pack_atlas_slot(kind, base)` must produce the SAME `dir_kind.w` the golden's
    /// `with_atlas_slot(base)` encodes (they share the identical shift/mask/casts-bit encoding).
    /// Reproduces the golden's `with_atlas_slot` arithmetic locally and asserts byte-equality for a
    /// few bases + kinds.
    #[test]
    fn pack_atlas_slot_matches_golden_with_atlas_slot_encoding() {
        // The golden's `with_atlas_slot(slot)` on a `dir_kind[3]` bit-pattern `kind` (goldens.rs):
        // clear the slot field + casts bit, write the masked slot, set casts iff slot != NONE.
        fn golden_with_atlas_slot(kind: u32, slot: u32) -> u32 {
            use crate::shadow_atlas::{ATLAS_SLOT_MASK, ATLAS_SLOT_SHIFT, CASTS_SHADOW_BIT, SLOT_NONE};
            let base = kind & !(ATLAS_SLOT_MASK << ATLAS_SLOT_SHIFT) & !CASTS_SHADOW_BIT;
            let with_slot = base | ((slot & ATLAS_SLOT_MASK) << ATLAS_SLOT_SHIFT);
            if slot == SLOT_NONE { with_slot } else { with_slot | CASTS_SHADOW_BIT }
        }
        for &kind in &[crate::light::LIGHT_KIND_POINT, crate::light::LIGHT_KIND_SPOT] {
            for base in [0u32, 1, 6, 10, SLOT_NONE] {
                assert_eq!(
                    pack_atlas_slot(kind, base),
                    golden_with_atlas_slot(kind, base),
                    "pack_atlas_slot must match golden with_atlas_slot for kind={kind}, base={base}"
                );
            }
        }
    }

    // ---- The punctual validity gate (plan §5.2 Premise F closure) ---------------------

    /// Reads a table row's `pos_range.xyz` (words 4..6 of the row) back out of the bytes.
    fn row_pos(bytes: &[u8], elem: usize) -> [f32; 3] {
        let base_word = LIGHT_HEADER_WORDS + elem * (GPU_LIGHT_BYTES / 4) + 4;
        let mut out = [0.0f32; 3];
        for (i, o) in out.iter_mut().enumerate() {
            let off = (base_word + i) * 4;
            *o = f32::from_ne_bytes(bytes[off..off + 4].try_into().unwrap());
        }
        out
    }

    /// A finite point light at `x` on the X axis.
    fn point_at_x(x: f32) -> PointLight {
        PointLight::new([x, 0.0, 0.0], [1.0, 1.0, 1.0], 300.0, 9.0)
    }

    /// **The RED-mutation gate.** Remove the `punctual_row_is_cullable` guard from
    /// [`fold_light_table_slotted`] and this test fails on `point_spot_count == 3` and on a
    /// NaN in row 1's position — the very row that, uploaded, is culled INTO every froxel.
    #[test]
    fn a_nan_positioned_point_is_dropped_and_the_finite_rows_close_up() {
        // Drives an emitting fold, so it joins this module's serialized set: `boyko-W2201`
        // and `boyko-W2204` are per-SITE `Once` latches, which is PROCESS state, and a
        // sibling that spends one inside the observer's window makes the observer read zero.
        // Resetting fixes ORDER; the lock fixes CONCURRENCY; both are needed.
        let _observe = boyko_log::probe::observe_lock();
        let good_a = point_at_x(1.0);
        let bad = PointLight::new([f32::NAN, 0.0, 0.0], [1.0, 1.0, 1.0], 300.0, 9.0);
        let good_b = point_at_x(3.0);

        let mut scratch = vec![0u8; LIGHT_HEADER_BYTES + 8 * GPU_LIGHT_BYTES];
        let used = write_light_table(
            &mut scratch,
            &[],
            &[],
            &[good_a, bad, good_b],
            &[],
            &LightingConfig::default(),
        );

        assert_eq!(
            used,
            LIGHT_HEADER_BYTES + 2 * GPU_LIGHT_BYTES,
            "the NaN row must not occupy a table slot"
        );
        let h = read_header(&scratch);
        assert_eq!(h.light_count(), 2);
        assert_eq!(h.point_spot_count(), 2);
        // The survivors CLOSE UP — the table has no hole where the dropped row was, so the
        // per-froxel index lists the cull emits still address contiguous rows.
        assert_eq!(row_pos(&scratch, 0)[0], 1.0);
        assert_eq!(row_pos(&scratch, 1)[0], 3.0);
    }

    /// The other three rejected shapes, one per row: a `+inf` component, a `-inf` component,
    /// and a NaN radius — on both punctual kinds.
    #[test]
    fn infinite_positions_and_a_nan_range_are_dropped_on_both_punctual_kinds() {
        // Drives an emitting fold, so it joins this module's serialized set: `boyko-W2201`
        // and `boyko-W2204` are per-SITE `Once` latches, which is PROCESS state, and a
        // sibling that spends one inside the observer's window makes the observer read zero.
        // Resetting fixes ORDER; the lock fixes CONCURRENCY; both are needed.
        let _observe = boyko_log::probe::observe_lock();
        let cfg = LightingConfig::default();
        let keep_pt = point_at_x(1.0);
        let keep_sp =
            SpotLight::new([2.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 3], 200.0, 8.0, 20.0, 30.0);

        let bad_points = [
            PointLight::new([f32::INFINITY, 0.0, 0.0], [1.0; 3], 300.0, 9.0),
            PointLight::new([0.0, f32::NEG_INFINITY, 0.0], [1.0; 3], 300.0, 9.0),
            PointLight::new([0.0, 0.0, 0.0], [1.0; 3], 300.0, f32::NAN),
        ];
        let bad_spots = [
            SpotLight::new(
                [0.0, 0.0, f32::INFINITY],
                [0.0, 0.0, 1.0],
                [1.0; 3],
                200.0,
                8.0,
                20.0,
                30.0,
            ),
            SpotLight::new([f32::NAN; 3], [0.0, 0.0, 1.0], [1.0; 3], 200.0, 8.0, 20.0, 30.0),
        ];

        for bad in &bad_points {
            let mut scratch = vec![0u8; LIGHT_HEADER_BYTES + 8 * GPU_LIGHT_BYTES];
            let used =
                write_light_table(&mut scratch, &[], &[], &[keep_pt, *bad], &[keep_sp], &cfg);
            assert_eq!(
                used,
                LIGHT_HEADER_BYTES + 2 * GPU_LIGHT_BYTES,
                "point row {bad:?} must be rejected"
            );
            assert_eq!(read_header(&scratch).point_spot_count(), 2);
        }
        for bad in &bad_spots {
            let mut scratch = vec![0u8; LIGHT_HEADER_BYTES + 8 * GPU_LIGHT_BYTES];
            let used =
                write_light_table(&mut scratch, &[], &[], &[keep_pt], &[keep_sp, *bad], &cfg);
            assert_eq!(
                used,
                LIGHT_HEADER_BYTES + 2 * GPU_LIGHT_BYTES,
                "spot row {bad:?} must be rejected"
            );
            assert_eq!(read_header(&scratch).point_spot_count(), 2);
        }
    }

    /// Golden neutrality, stated as a property rather than asserted in prose: the gate is
    /// INERT on every well-formed row, including the awkward-but-valid ones (a zero radius, an
    /// INFINITE radius, coordinates at the f32 extremes, a light exactly at the origin). If
    /// this ever reddens, some real scene just lost a light and a golden is about to move.
    #[test]
    fn the_validity_gate_is_inert_on_every_well_formed_row() {
        let valid_positions = [
            [0.0, 0.0, 0.0],
            [-1.5, 2.25, 1e-30],
            [f32::MAX, -f32::MAX, 0.0],
            [f32::MIN_POSITIVE, 0.0, -0.0],
        ];
        // `+inf` range is deliberately ACCEPTED — a totally-ordered comparand both cull levels
        // agree on, i.e. a coherent authoring choice, unlike a NaN.
        let valid_ranges = [0.0f32, 1e-6, 9.0, f32::MAX, f32::INFINITY];

        for pos in valid_positions {
            for range in valid_ranges {
                assert!(
                    punctual_row_is_cullable(pos, range),
                    "well-formed row (pos {pos:?}, range {range}) must NOT be dropped"
                );
            }
        }
    }

    /// Keeps the gate above from being vacuous: the predicate really does reject, so a green
    /// inertness sweep means "nothing valid is rejected", not "nothing is ever rejected".
    #[test]
    fn the_validity_gate_is_not_vacuous() {
        assert!(!punctual_row_is_cullable([f32::NAN, 0.0, 0.0], 1.0));
        assert!(!punctual_row_is_cullable([0.0, f32::INFINITY, 0.0], 1.0));
        assert!(!punctual_row_is_cullable([0.0, 0.0, f32::NEG_INFINITY], 1.0));
        assert!(!punctual_row_is_cullable([0.0, 0.0, 0.0], f32::NAN));
    }

    /// A dropped row must not consume the `MAX_LIGHTS` budget either — the two release-visible
    /// gates compose, they do not shadow each other.
    #[test]
    fn a_dropped_row_does_not_spend_the_max_lights_budget() {
        // Drives an emitting fold, so it joins this module's serialized set: `boyko-W2201`
        // and `boyko-W2204` are per-SITE `Once` latches, which is PROCESS state, and a
        // sibling that spends one inside the observer's window makes the observer read zero.
        // Resetting fixes ORDER; the lock fixes CONCURRENCY; both are needed.
        let _observe = boyko_log::probe::observe_lock();
        let cfg = LightingConfig::default();
        let bad = PointLight::new([f32::NAN; 3], [1.0; 3], 300.0, 9.0);
        let mut points = vec![bad];
        points.extend((0..MAX_LIGHTS).map(|i| point_at_x(i as f32)));

        let mut scratch =
            vec![0u8; LIGHT_HEADER_BYTES + (MAX_LIGHTS as usize + 2) * GPU_LIGHT_BYTES];
        let used = write_light_table(&mut scratch, &[], &[], &points, &[], &cfg);

        // The bad row is skipped WITHOUT advancing `written`, so all MAX_LIGHTS valid lights fit.
        assert_eq!(used, LIGHT_HEADER_BYTES + MAX_LIGHTS as usize * GPU_LIGHT_BYTES);
        assert_eq!(read_header(&scratch).point_spot_count(), MAX_LIGHTS);
        assert_eq!(row_pos(&scratch, 0)[0], 0.0, "the first survivor is the first VALID light");
    }
}

#[cfg(test)]
mod l8a_light_codes {
    use super::*;
    use boyko_log::probe::{watch, watch_any, watched};

    use crate::log_probe::arm;

    /// A punctual light the validity gate must reject.
    fn nan_point() -> PointLight {
        PointLight {
            position: [f32::NAN, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            power: 100.0,
            range: 1.0,
        }
    }

    fn good_point() -> PointLight {
        PointLight {
            position: [0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0],
            power: 100.0,
            range: 1.0,
        }
    }

    fn scratch() -> Vec<u8> {
        vec![0u8; LIGHT_HEADER_BYTES + (MAX_LIGHTS as usize) * GPU_LIGHT_BYTES]
    }

    fn fold(points: &[PointLight]) -> usize {
        let mut dst = scratch();
        fold_light_table_slotted(
            &mut dst,
            core::iter::empty(),
            core::iter::empty(),
            points.iter().map(|p| (SLOT_NONE, p)),
            core::iter::empty::<(u32, &SpotLight)>(),
            &LightingConfig::default(),
        )
    }

    /// The two light-table codes, in ONE test, because a `Once` latch is process state.
    ///
    /// This began as two tests and they failed each other: the overflow case's fixture also
    /// contains a NaN light, so whichever ran first spent BOTH latches and the other observed
    /// zero. Resetting fixes that between tests — but not *within* a sequence, because a fold that
    /// overflows spends `W2201` whether or not anyone is watching. So the only sound way to assert
    /// on a sequence of first-occurrences is to own the whole sequence; splitting it into two
    /// `#[test]` fns hands the ordering to the harness, which is not a thing a test may assume.
    #[test]
    fn w2201_and_w2204_are_separate_codes_and_each_fires_once() {
        let _observe = boyko_log::probe::observe_lock();
        arm();
        // Both latches are PROCESS state and four sibling tests in this binary fold NaN lights
        // for reasons of their own. Resetting is what makes this test independent of whatever ran
        // before it -- without it the first assertion below observed `left: 0, right: 1` on every
        // run, and the fix is not "lock harder": a spent latch cannot be un-spent by waiting.
        W2204_SITE.reset();
        W2201_SITE.reset();

        // 1. Three NaN lights in ONE fold: one record carrying THREE, not three records carrying
        //    one. This is the claim the migration ledger asked for and the old shape could not
        //    make -- its reporter ran per dropped light, so the single record a latch let through
        //    always described exactly one drop.
        watch(b'W', W2204.number());
        let _ = fold(&[nan_point(), nan_point(), nan_point()]);
        assert_eq!(watched(), 1, "one W2204 record per fold, not one per dropped light");
        //    AND the record says THREE. The count assertion above cannot see that: reverting this
        //    site to its pre-migration shape -- a reporter called once per dropped light behind
        //    the same latch -- leaves it green, because "one record saying three" and "one record
        //    saying one" are both one record. The ledger's claim is about the PAYLOAD, so the
        //    payload is what this line reads.
        let msg = boyko_log::probe::last_message();
        assert!(
            msg.contains("dropped 3 point/spot light(s)"),
            "the record must carry the tally, not merely exist: {msg}"
        );

        // 2. ONE fold that both overflows AND drops a NaN. `boyko-W2204`'s latch is spent by step
        //    1, so the drop is silent; `boyko-W2201` has its own latch and fires. Watching W2201
        //    across this fold therefore counts exactly one record, and THAT is the separation:
        //    under a single code covering both conditions this count would be zero, because the
        //    latch step 1 spent would be the same latch.
        //
        //    The two must be observed in the SAME fold, not in two. A fold that overflows spends
        //    W2201 whether or not anyone is watching, so a first pass "to check W2204 is quiet"
        //    would consume the very occurrence the next assertion is about -- which is exactly how
        //    the first draft of this test failed, `left: 0, right: 1`, deterministically.
        let mut lights: Vec<PointLight> = (0..MAX_LIGHTS + 4).map(|_| good_point()).collect();
        lights[0] = nan_point();
        watch(b'W', W2201.number());
        let _ = fold(&lights);
        assert_eq!(watched(), 1, "W2201 has its own latch and fires on its first overflow");

        // 3. Both spent: silence, and `watch_any` means silence about EVERY code, so a third
        //    reporter appearing on this path would redden this line rather than hide behind it.
        watch_any();
        let _ = fold(&lights);
        assert_eq!(watched(), 0, "both Once latches are spent");
    }

    #[test]
    fn a_clean_fold_reports_nothing() {
        // The positive control: a fold with no NaN and no overflow must be silent, whatever the
        // latches' state. Without it, a fold that reported unconditionally would satisfy the
        // assertions above.
        arm();

        watch_any();
        let _ = fold(&[good_point(), good_point()]);
        assert_eq!(watched(), 0, "a clean fold is silent");
    }
}
