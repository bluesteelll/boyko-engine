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

use boyko_ecs::ecs::core::iters::query::{Changed, Or};
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_macros::Resource;

use crate::light::{
    DirectionalLight, GpuLight, LightHeaderGpu, LightingConfig, MAX_LIGHTS, PointLight, SkyLight,
    SpotLight,
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
    let mut off = LIGHT_HEADER_BYTES;
    let mut l0a_count: u32 = 0;
    for d in directionals {
        write_pod(dst, off, &GpuLight::from_directional(d));
        off += GPU_LIGHT_BYTES;
        l0a_count += 1;
    }
    for s in skies {
        write_pod(dst, off, &GpuLight::from_sky(s));
        off += GPU_LIGHT_BYTES;
        l0a_count += 1;
    }
    let mut point_spot_count: u32 = 0;
    for p in points {
        write_pod(dst, off, &GpuLight::from_point(p));
        off += GPU_LIGHT_BYTES;
        point_spot_count += 1;
    }
    for s in spots {
        write_pod(dst, off, &GpuLight::from_spot(s));
        off += GPU_LIGHT_BYTES;
        point_spot_count += 1;
    }
    debug_assert!(
        l0a_count + point_spot_count <= MAX_LIGHTS,
        "invariant: live light count must not exceed MAX_LIGHTS"
    );

    // Backfill the header now that the counts are known. The header region [0..HEADER)
    // is disjoint from the body, so this is byte-identical to writing it up front.
    let header = LightHeaderGpu::new(l0a_count, point_spot_count, cfg);
    write_pod(dst, 0, &header);
    off
}

/// Copies a `#[repr(C)]` POD's bytes into `dst` at `off`. The POD types are
/// `align(16)` / `align(4)` with no padding holes the GPU reads differently, so a
/// byte-copy of `size_of::<T>()` bytes reproduces the std430 element.
#[inline]
fn write_pod<T: Copy>(dst: &mut [u8], off: usize, value: &T) {
    let size = core::mem::size_of::<T>();
    debug_assert!(off + size <= dst.len(), "invariant: POD write stays within the scratch");
    // SAFETY: `value` is a `Copy` POD of `size` bytes; `src` reads exactly those bytes.
    // The `debug_assert` above (and the `Default` worst-case sizing) guarantee
    // `dst[off..off+size]` is in bounds; the two regions never overlap (distinct
    // allocations). No `T` invariants are violated by reading its raw bytes (it is a
    // plain-data std430 element).
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
#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]
pub fn collect_lights(
    changed_dir: Query<&DirectionalLight, Changed<DirectionalLight>>,
    changed_sky: Query<&SkyLight, Changed<SkyLight>>,
    changed_point: Query<&PointLight, Changed<PointLight>>,
    changed_spot: Query<&SpotLight, Changed<SpotLight>>,
    all_directionals: Query<&DirectionalLight>,
    all_skies: Query<&SkyLight>,
    all_points: Query<&PointLight>,
    all_spots: Query<&SpotLight>,
    cfg: Res<LightingConfig>,
    mut staging: ResMut<LightTableStaging>,
) {
    // Change gate: if no light's change tick advanced this frame, do nothing. Each
    // `Changed` query yields zero rows on a static frame, so the collection is skipped
    // entirely (no rebuild, no dirty flip). NOTE: a despawn/add still needs to be
    // observed; the caller pairs this with the structural-change path when one exists.
    let changed = changed_dir.iter().next().is_some()
        || changed_sky.iter().next().is_some()
        || changed_point.iter().next().is_some()
        || changed_spot.iter().next().is_some();
    if !changed {
        return;
    }

    // Rebuild the full table from the live lights (the table is small — MAX_LIGHTS ≤
    // 1024 — so a full rebuild on change is cheaper than a delta map; Decision 4
    // trade-off). Fold the unfiltered queries DIRECTLY into the preallocated scratch —
    // the worst-case-sized `scratch` is the SOLE sink, no per-frame `Vec` (Principle 1/5).
    let staging = &mut *staging;
    let used = fold_light_table(
        &mut staging.scratch,
        all_directionals.iter(),
        all_skies.iter(),
        all_points.iter(),
        all_spots.iter(),
        &cfg,
    );
    staging.used_bytes = used;
    staging.dirty = true;
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
}
