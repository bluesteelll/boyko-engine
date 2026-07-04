//! SDFDDGI I2 — the probe-update pass's host-side data + policy: the b6 update UBO
//! byte-mirror ([`DdgiUpdateUbo`]), the owner-set update knobs ([`DdgiUpdateConfig`]),
//! the device-caps degrade gate ([`DdgiCaps`]), the boot Fibonacci ray-table builder
//! ([`fill_fibonacci_ray_table`]), and the round-robin dispatch/UBO packing helpers the
//! host reads each frame to arm `scene.ddgi_update`.
//!
//! # Principle 0
//!
//! Durable data is ECS-native: [`DdgiUpdateConfig`] / [`DdgiCaps`] are `#[derive(Resource)]`
//! singletons (cold owner/boot state, NOT a side `std::Vec`/`HashMap`), the ray table is a
//! DEVICE buffer (an FFI/GPU-contiguity carrier the host owns alongside `DdgiAtlas`), and the
//! UBO is a host-coherent DEVICE buffer. The CPU Fibonacci precompute writes into a
//! caller-supplied slice (no hidden allocation), boot-uploaded ONCE — never a per-frame host
//! `Vec`.
//!
//! # The b6 update UBO (`DdgiUpdateUbo`)
//!
//! A `#[repr(C)]` 48-byte mirror of the committed `sdf_probe_update.comp.hlsl` `cbuffer
//! DdgiUpdate` (in field order): `float4 origin` (xyz = grid world origin, w = probe spacing),
//! `uint4 grid_dims` (xyz = probes per axis), `uint frame_index`, `uint subset_n`, `uint
//! rays_per_probe`, `uint light_count`. The dims ride as bit-cast `u32` lanes exactly as
//! [`ResolvedDdgi`](crate::ddgi_config::ResolvedDdgi) packs its own dims — the SINGLE way the
//! host packs grid geometry.
//!
//! # The GI-OFF 0%-gate
//!
//! [`DdgiUpdateConfig::default`] carries the plan §6 placeholder knobs (`rays = 64`, `subset_n
//! = 4`, `gi_max_it = 64`), but the update pass is ARMED only when
//! [`ResolvedDdgi::enabled()`](crate::ddgi_config::ResolvedDdgi::enabled) — the same predicate
//! driving the LightBuf GI gate. With the default (disabled) [`DdgiConfig`] the host leaves
//! `scene.ddgi_update = None`, so the pass is never recorded (byte-identical command stream).
//! The degrade gate ([`DdgiCaps`]) forces DDGI permanently disabled on a device lacking
//! B10G11R11/RG16F storage (plan §3), so an unsupported device also boots into the 0%-gate.

use core::f32::consts::PI;

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

use boyko_rhi_vulkan::ddgi::{DDGI_GRID_DIM_X, DDGI_GRID_DIM_Y, DDGI_GRID_DIM_Z, DDGI_PROBE_COUNT};

use crate::ddgi_config::{DdgiConfig, ResolvedDdgi};

// ---- constants -----------------------------------------------------------------------

/// The default Fibonacci rays per probe — bench-finalized from `ddgi_probe_gi_cost` (plan §5).
/// One `RayTable` `float4` per ray; the update UBO's `rays_per_probe` field. MUST be
/// `<= GI_MAX_RAYS` (the shader's groupshared cache bound). `64` gives good angular coverage well
/// within budget: on an RTX 3060 the full-grid (subset 1), 64-ray, `GI_MAX_IT=64`, one-shadowed-
/// -light update measures ~1.2 ms p95 — comfortably under the ~3 ms ceiling with headroom for
/// temporal accumulation (I4) to integrate more effective samples over frames.
pub const DEFAULT_RAYS_PER_PROBE: u32 = 64;

/// The default round-robin subset divisor `N` (Decision D3) — bench-finalized: each frame updates
/// `DDGI_PROBE_COUNT / N` probes. MUST divide `DDGI_PROBE_COUNT` (a power of two does —
/// `DDGI_PROBE_COUNT = 2048 = 2^11`, plan §4 P1-5). `2` (not the conservative placeholder `4`):
/// the `ddgi_probe_gi_cost` bench found the whole update is far under budget (even subset 1 /
/// full-grid / max-quality / shadow-on is 2.48 ms p95 on an RTX 3060), so a low divisor is
/// affordable. `2` gives 2-frame convergence (responsive for *dynamic* GI) while keeping ~2×
/// per-frame margin for multi-light / fully-occluded scenes (the bench's shadow cost is a lower
/// bound — sky-escaping probes pay no shadow march). Raise it for very heavy scenes; drop to `1`
/// for instant convergence on light scenes — a runtime config knob, not a rebuild.
pub const DEFAULT_SUBSET_N: u32 = 2;

/// The shader's groupshared ray-cache bound (`GI_MAX_RAYS` in `sdf_probe_update.comp.hlsl`) —
/// the maximum `rays_per_probe` the pass can march (the LDS cache is sized for this). The bench
/// sweeps `rays_per_probe ∈ {16, 32, 64, 128}`, all `<= 128`.
pub const GI_MAX_RAYS: u32 = 128;

/// The default temporal-hysteresis blend factor `α` (SDFDDGI I4) — the fraction of the PREVIOUS
/// atlas value kept per update: the shader writes `lerp(fresh, prev, α)`. `0.95` gives an effective
/// window of `1/(1-α) ≈ 20` frames — stable for *dynamic* runtime GI once the smoothly-advancing
/// per-frame ray rotation feeds it decorrelated samples (a per-frame random rotation would need a
/// LOWER α; ours is smooth, so `0.95` is safe). A one-shot static capture wants a LOWER α (≈0.9)
/// so it converges in fewer frames. Rides the update UBO's `grid_dims.w` lane as a bit-cast `f32`;
/// clamped to `[0, 1)` where packed (a `1.0` would freeze the field, never integrating new light).
pub const DEFAULT_HYSTERESIS: f32 = 0.95;

// ---- DdgiUpdateUbo (the b6 cbuffer byte-mirror) --------------------------------------

/// The SDFDDGI I2 probe-update parameter UBO — a `#[repr(C)]` byte-mirror of the committed
/// `sdf_probe_update.comp.hlsl` `cbuffer DdgiUpdate : register(b6)` (plan §2.3). Bound at the
/// update set @6 (a single host-coherent DEVICE buffer — the grid is world-fixed, Decision D1,
/// and I2 ships identity ray-rotation, so the UBO is effectively static → NO per-FIF ring).
///
/// Field order + the bit-cast dims packing MUST match the shader exactly; a drift silently
/// mis-parses every param (the M2 dead-branch class). The `grid_dims` lane is the shader's
/// `uint4 grid_dims`, NOT an inverse spacing (the plan P2-1 fix — the misleading
/// `inv_spacing_dims` name held dims, not an inverse; here the field is named `grid_dims`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct DdgiUpdateUbo {
    /// `origin.xyz` = the grid world origin (probe `(0,0,0)`'s minimum corner, Decision D1),
    /// `origin.w` = the probe spacing (world units between adjacent probes). The shader's
    /// `probe_world_pos` = `origin.xyz + float3(coord) * origin.w`.
    pub origin: [f32; 4],
    /// `grid_dims.xyz` = the probes per axis as bit-cast `u32` — packed like
    /// [`ResolvedDdgi`](crate::ddgi_config::ResolvedDdgi)'s dims lanes. The shader reads
    /// `grid_dims.x/.y/.z` as `uint` (its `probe_coord` decomposition divisors). **`.w` carries the
    /// temporal-hysteresis `α` as a bit-cast `f32`** (SDFDDGI I4 — the shader reads
    /// `asfloat(grid_dims.w)`), NOT a free/pad lane: this transports `α` without disturbing the
    /// pinned 48-byte layout. A reader wiring dims must NOT treat `.w` as a dimension.
    pub grid_dims: [u32; 4],
    /// The host-frame-derived round-robin phase (`frame_index % subset_n` selects the subset).
    /// I2 ships identity ray-rotation, so this only phases the subset (the quaternion rotate is
    /// I4); it may stay `0` for a static single-subset dispatch.
    pub frame_index: u32,
    /// The round-robin subset divisor `N` — each frame updates `DDGI_PROBE_COUNT / N` probes.
    /// MUST divide `DDGI_PROBE_COUNT` (plan §4 P1-5; guarded in [`ddgi_update_dispatch_groups`]).
    pub subset_n: u32,
    /// The Fibonacci ray count per probe (`== RayTable length`). `<= GI_MAX_RAYS`.
    pub rays_per_probe: u32,
    /// The light-table entry count the per-ray direct-light shade loop iterates.
    pub light_count: u32,
}

// Layout pin: 16 (origin vec4) + 16 (grid_dims uvec4) + 4·4 (the four trailing u32) = 48 B.
// A change is a deliberate decision (the GPU cbuffer reads this stride at b6). The offsets
// mirror the shader's field order.
const _: () = assert!(size_of::<DdgiUpdateUbo>() == 48);
const _: () = assert!(core::mem::offset_of!(DdgiUpdateUbo, origin) == 0);
const _: () = assert!(core::mem::offset_of!(DdgiUpdateUbo, grid_dims) == 16);
const _: () = assert!(core::mem::offset_of!(DdgiUpdateUbo, frame_index) == 32);
const _: () = assert!(core::mem::offset_of!(DdgiUpdateUbo, subset_n) == 36);
const _: () = assert!(core::mem::offset_of!(DdgiUpdateUbo, rays_per_probe) == 40);
const _: () = assert!(core::mem::offset_of!(DdgiUpdateUbo, light_count) == 44);

/// The byte size of the probe-update UBO (`size_of::<DdgiUpdateUbo>()`, 48 B). Hosts size their
/// update UBO from THIS constant (single source — no hand-copied `48`).
pub const DDGI_UPDATE_UBO_BYTES: usize = size_of::<DdgiUpdateUbo>();

impl DdgiUpdateUbo {
    /// The all-zero UBO (the boot seed on the OFF path — bound-but-unread). `frame_index` and
    /// every param zero; the dispatch never runs while disabled, so the contents are inert.
    pub const ZERO: Self = Self {
        origin: [0.0; 4],
        grid_dims: [0; 4],
        frame_index: 0,
        subset_n: 0,
        rays_per_probe: 0,
        light_count: 0,
    };

    /// This UBO as its 48 raw bytes — the exact host-coherent write into the update UBO buffer
    /// (`ptr::copy_nonoverlapping` at offset 0). `#[repr(C)]` makes the transmute layout-stable.
    #[inline]
    pub fn as_bytes(&self) -> [u8; DDGI_UPDATE_UBO_BYTES] {
        // SAFETY: `DdgiUpdateUbo` is `#[repr(C)]` with no padding beyond the pinned 48-byte
        // layout (const-asserted above), and every field is a POD (`f32`/`u32`) — all bit
        // patterns are valid, so the transmute to a byte array reads only initialized bytes.
        unsafe { core::mem::transmute::<Self, [u8; DDGI_UPDATE_UBO_BYTES]>(*self) }
    }
}

// ---- DdgiUpdateConfig (the owner-set update knobs) -----------------------------------

/// The owner-set probe-update knobs (SDFDDGI I2) — a `World`-singleton `#[derive(Resource)]`
/// carrying the Fibonacci ray count, the round-robin subset divisor, and the shipped
/// `GI_MAX_IT` variant. These are the bench-derived cadence knobs (plan §5): `rays_per_probe`
/// and `subset_n` ride the UBO; `gi_max_it` selects the pre-compiled pipeline variant.
///
/// Enablement is NOT stored here — the update pass is gated on
/// [`ResolvedDdgi::enabled()`](crate::ddgi_config::ResolvedDdgi::enabled) (the
/// capability-is-structural principle + the SAME predicate driving the LightBuf GI gate). This
/// config only shapes HOW the enabled pass runs. The defaults are the plan §6 placeholders
/// (`rays = 64`, `subset_n = 4`, `gi_max_it = 64`); the orchestrator finalizes them from the
/// `ddgi_probe_gi_cost` bench.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct DdgiUpdateConfig {
    /// The Fibonacci rays per probe (the UBO `rays_per_probe` + the ray-table length). Clamped
    /// `[1, GI_MAX_RAYS]` where consumed.
    pub rays_per_probe: u32,
    /// The round-robin subset divisor `N` (the UBO `subset_n`). MUST divide `DDGI_PROBE_COUNT`
    /// (guarded by a `debug_assert!` in [`ddgi_update_dispatch_groups`]).
    pub subset_n: u32,
    /// The shipped `GI_MAX_IT` sphere-trace trip count — one of
    /// [`GI_MAX_IT_VARIANTS`](boyko_rhi_vulkan::compute::GI_MAX_IT_VARIANTS). Selects the
    /// pre-compiled `sdf_probe_update_it*.comp.spv` pipeline (measured==shipped, plan §5).
    pub gi_max_it: u32,
    /// The temporal-hysteresis blend factor `α` (SDFDDGI I4) — the fraction of the previous atlas
    /// value the update KEEPS each frame (`lerp(fresh, prev, α)`). Rides the UBO `grid_dims.w` lane
    /// as a bit-cast `f32`; clamped to `[0, 1)` in [`pack_ddgi_update_ubo`] (a `1.0` never
    /// integrates new light). Default [`DEFAULT_HYSTERESIS`].
    pub hysteresis: f32,
}

impl Default for DdgiUpdateConfig {
    #[inline]
    fn default() -> Self {
        Self {
            rays_per_probe: DEFAULT_RAYS_PER_PROBE,
            subset_n: DEFAULT_SUBSET_N,
            gi_max_it: boyko_rhi_vulkan::compute::GI_MAX_IT_DEFAULT,
            hysteresis: DEFAULT_HYSTERESIS,
        }
    }
}

// ---- DdgiCaps (the device-storage degrade gate — plan §3) -----------------------------

/// The device-storage capability gate (SDFDDGI I2 / plan §3) — a `World`-singleton
/// `#[derive(Resource)]` the host inserts at device boot from
/// [`DeviceCaps::ddgi_storage_ok()`](boyko_rhi_vulkan's `DeviceCaps`). When `false`, the device
/// lacks B10G11R11/RG16F STORAGE, so the atlas was created WITHOUT the storage bit and the
/// update pass CANNOT write it — [`resolve_ddgi_grid_gated`] then clamps [`ResolvedDdgi`] to
/// DISABLED regardless of the owner's [`DdgiConfig::ddgi_indirect`]. DDGI is opt-in (unlike the
/// always-used `gViewT`), so an unsupported device DEGRADES (boots into the 0%-gate), never
/// fail-fasts (plan §3, the P1-4 fix).
///
/// [`Default`] is `storage_ok = true` (the common case — most desktop GPUs support both
/// formats; an offscreen bench or a host that never queries assumes supported). The host
/// OVERRIDES it at boot with the real `ddgi_storage_ok()` query.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DdgiCaps {
    /// Whether the device supports BOTH B10G11R11 (irradiance) and RG16F (depth) STORAGE
    /// images (`DeviceCaps::ddgi_storage_ok()`). `false` ⇒ DDGI permanently disabled (the
    /// degrade path — [`resolve_ddgi_grid_gated`] clamps the resolve to all-zero).
    pub storage_ok: bool,
}

impl Default for DdgiCaps {
    #[inline]
    fn default() -> Self {
        // Assume supported until the host inserts the real boot query — a host that DOES query
        // overrides this; a bench that never queries wants the enabled path.
        Self { storage_ok: true }
    }
}

impl DdgiCaps {
    /// Builds the caps from a device `ddgi_storage_ok()` query result (the host boot seam).
    #[inline]
    pub const fn new(storage_ok: bool) -> Self {
        Self { storage_ok }
    }
}

// ---- the boot Fibonacci ray table (CPU precompute — Principle 0) ----------------------

/// Fills `out` with unit spherical-Fibonacci ray directions (xyz per `[f32; 4]`, `.w = 0`) —
/// the boot ray-table CPU precompute (plan §2.3). `out.len()` rays are laid on the sphere by
/// the golden-angle spiral (uniform-area distribution): for ray `i` of `n`,
/// `z = 1 - 2·(i + 0.5)/n`, `r = sqrt(1 - z²)`, `phi = i · goldenAngle`, so
/// `(x, y, z) = (r·cos phi, r·sin phi, z)` — every direction unit-length.
///
/// Uploaded ONCE to the DEVICE ray-table buffer at boot (the `MeshSdfTexture::upload_region`
/// boot-submit shape); per-frame decorrelation is an in-shader quaternion rotate (identity at
/// I2, D6). Principle 0: writes into the caller's slice (no hidden `Vec`), the slice being the
/// mapped staging bytes or a boot scratch. Testable in isolation (the caller supplies `out`).
///
/// A zero-length `out` is a no-op. Caller SHOULD size `out.len() == rays_per_probe`.
pub fn fill_fibonacci_ray_table(out: &mut [[f32; 4]]) {
    let n = out.len();
    if n == 0 {
        return;
    }
    // The golden angle in radians: π · (3 − √5) ≈ 2.399963. Successive rays advance by this
    // azimuth, the spiral that spreads points evenly over the sphere.
    let golden_angle = PI * (3.0 - 5.0_f32.sqrt());
    let inv_n = 1.0 / n as f32;
    for (i, ray) in out.iter_mut().enumerate() {
        // z sweeps (1 → −1) over the equal-area bands; the +0.5 centers each band.
        let z = 1.0 - 2.0 * (i as f32 + 0.5) * inv_n;
        // r = the band radius; clamp the sqrt argument against tiny negatives from rounding.
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = i as f32 * golden_angle;
        *ray = [r * phi.cos(), r * phi.sin(), z, 0.0];
    }
}

// ---- the resolve degrade clamp (plan §3) ----------------------------------------------

/// The device-caps-gated DDGI grid resolve — the pure decision that CLAMPS the grid to DISABLED
/// when the device lacks storage (plan §3 degrade), else defers to
/// [`resolve_ddgi`](crate::ddgi_config::resolve_ddgi). Unit-testable (no `Res` wrappers).
///
/// When `!caps.storage_ok` the atlas was created WITHOUT STORAGE and the update pass cannot
/// write it, so DDGI MUST be off regardless of `cfg.ddgi_indirect` — returns
/// [`ResolvedDdgi::DISABLED`](crate::ddgi_config::ResolvedDdgi::DISABLED) (all-zero, the
/// 0%-gate). Otherwise the normal resolve runs.
#[inline]
pub fn resolve_ddgi_grid_clamped(cfg: &DdgiConfig, caps: &DdgiCaps) -> ResolvedDdgi {
    if !caps.storage_ok {
        return ResolvedDdgi::DISABLED;
    }
    crate::ddgi_config::resolve_ddgi(cfg)
}

/// The cold single-writer of [`ResolvedDdgi`] that FOLDS IN the device-storage degrade gate
/// (SDFDDGI I2 / plan §3) — the storage-aware analogue of
/// [`resolve_ddgi_grid`](crate::ddgi_config::resolve_ddgi_grid). Reads the cold [`DdgiConfig`]
/// AND the boot [`DdgiCaps`]; when the caps report no storage it writes the all-zero DISABLED
/// carrier, so a device lacking B10G11R11/RG16F storage runs the whole GI path into the
/// 0%-gate (the update pass is never armed, the resolve never samples). CAMERA-INDEPENDENT
/// (Decision D1): no view read, no per-FIF ring.
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s read/written
// through reborrows — the same false-positive `resolve_ddgi_grid` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_ddgi_grid_gated(
    cfg: Res<DdgiConfig>,
    caps: Res<DdgiCaps>,
    mut out: ResMut<ResolvedDdgi>,
) {
    *out = resolve_ddgi_grid_clamped(&cfg, &caps);
}

// ---- the per-frame dispatch + UBO packing helpers -------------------------------------

/// The round-robin dispatch BLOCK count (`groups_x`) for the update pass this frame — one
/// `[numthreads(64,1,1)]` block per ACTIVE probe in the current subset:
/// `DDGI_PROBE_COUNT / subset_n` (exact division; `subset_n` divides `DDGI_PROBE_COUNT`, plan
/// §4 P1-5). `cmd_dispatch(groups, 1, 1)`. A `subset_n` of `0` or one that does not divide the
/// probe count is a config bug (debug-asserted); release clamps `subset_n` to `>= 1`.
#[inline]
pub fn ddgi_update_dispatch_groups(subset_n: u32) -> u32 {
    debug_assert!(subset_n >= 1, "invariant: DDGI subset_n must be >= 1");
    debug_assert!(
        DDGI_PROBE_COUNT.is_multiple_of(subset_n),
        "invariant: DDGI subset_n ({subset_n}) must divide DDGI_PROBE_COUNT ({DDGI_PROBE_COUNT})"
    );
    let n = subset_n.max(1);
    DDGI_PROBE_COUNT / n
}

/// Packs the b6 [`DdgiUpdateUbo`] for a frame from the resolved grid + the update knobs + the
/// dynamic frame/light state (plan §2.3/§4). The grid `origin.xyz`/spacing come from `resolved`
/// (`ResolvedDdgi` already carries `origin` and the bit-cast dims); `origin.w` reconstructs the
/// spacing from `inv_spacing` (the resolve stores `1/spacing` for the div-free read path, so
/// the UBO's `spacing = 1/inv_spacing`, guarded against a zero). `rays_per_probe`/`subset_n`
/// come from `cfg` (clamped to sane ranges); `frame_index`/`light_count` are the caller's
/// dynamic per-frame values.
///
/// The caller writes [`DdgiUpdateUbo::as_bytes`] into the host-coherent update UBO before
/// arming `scene.ddgi_update`.
#[inline]
pub fn pack_ddgi_update_ubo(
    resolved: &ResolvedDdgi,
    cfg: &DdgiUpdateConfig,
    frame_index: u32,
    light_count: u32,
) -> DdgiUpdateUbo {
    // `ResolvedDdgi` packs `origin.xyz` + `inv_spacing` (in `inv_spacing_dims[0]`) + the three
    // bit-cast dims (`inv_spacing_dims[1..4]`). Reconstruct the spacing for the UBO (the update
    // shader multiplies `coord * spacing`, NOT `coord / inv_spacing`). Guard a zero inv_spacing
    // (the resolve's degenerate) → spacing 0 (a benign collapse; the pass never runs disabled).
    let inv_spacing = resolved.inv_spacing_dims[0];
    let spacing = if inv_spacing > 0.0 { 1.0 / inv_spacing } else { 0.0 };
    // `grid_dims.w` (SDFDDGI I4) carries the temporal-hysteresis `α` as a bit-cast `f32`. Clamp to
    // `[0, 1)`: a NaN/inf would reach the shader `lerp`, and `1.0` would freeze the field (never
    // integrating new light). `0.999` is the practical ceiling.
    let hysteresis = cfg.hysteresis.clamp(0.0, 0.999);
    let dims = [
        resolved.inv_spacing_dims[1].to_bits(),
        resolved.inv_spacing_dims[2].to_bits(),
        resolved.inv_spacing_dims[3].to_bits(),
        hysteresis.to_bits(),
    ];
    let rays = cfg.rays_per_probe.clamp(1, GI_MAX_RAYS);
    let subset_n = cfg.subset_n.max(1);
    DdgiUpdateUbo {
        origin: [resolved.origin[0], resolved.origin[1], resolved.origin[2], spacing],
        grid_dims: dims,
        frame_index,
        subset_n,
        rays_per_probe: rays,
        light_count,
    }
}

/// The owner-locked default grid dims as a `[u32; 3]` — a convenience for the ray-table /
/// dispatch sizing that does not want to reach into `DdgiConfig`. Mirrors the
/// `boyko_rhi_vulkan::ddgi` owner-locked `[16, 8, 16]`.
pub const DDGI_DEFAULT_DIMS: [u32; 3] = [DDGI_GRID_DIM_X, DDGI_GRID_DIM_Y, DDGI_GRID_DIM_Z];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_ubo_size_is_pinned() {
        assert_eq!(DDGI_UPDATE_UBO_BYTES, 48);
        assert_eq!(size_of::<DdgiUpdateUbo>(), 48);
    }

    #[test]
    fn zero_ubo_is_all_zero_bytes() {
        assert!(DdgiUpdateUbo::ZERO.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn fibonacci_rays_are_unit_length() {
        let mut table = [[0.0f32; 4]; 64];
        fill_fibonacci_ray_table(&mut table);
        for r in &table {
            let len2 = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
            assert!((len2 - 1.0).abs() < 1e-4, "ray not unit-length: len² = {len2}");
            assert_eq!(r[3], 0.0, "the .w lane must be zero-padded");
        }
    }

    #[test]
    fn dispatch_groups_divide_the_probe_count() {
        for n in [1, 2, 4, 8] {
            assert_eq!(ddgi_update_dispatch_groups(n), DDGI_PROBE_COUNT / n);
        }
    }

    #[test]
    fn caps_off_clamps_to_disabled_regardless_of_config() {
        let cfg = DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() };
        let off = resolve_ddgi_grid_clamped(&cfg, &DdgiCaps::new(false));
        assert_eq!(off, ResolvedDdgi::DISABLED, "no-storage device must degrade to disabled");
        // With storage the enabled config resolves the real grid.
        let on = resolve_ddgi_grid_clamped(&cfg, &DdgiCaps::new(true));
        assert_eq!(on.ddgi_mode_word, 1);
    }

    #[test]
    fn packed_ubo_reconstructs_spacing_and_dims() {
        let cfg = DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() };
        let resolved = crate::ddgi_config::resolve_ddgi(&cfg);
        let update = DdgiUpdateConfig::default();
        let ubo = pack_ddgi_update_ubo(&resolved, &update, 3, 5);
        // Spacing round-trips from the resolve's inv_spacing.
        assert!((ubo.origin[3] - cfg.spacing).abs() < 1e-5);
        // Dims match the config (bit-cast lanes) in xyz; .w carries the hysteresis alpha.
        assert_eq!(
            [ubo.grid_dims[0], ubo.grid_dims[1], ubo.grid_dims[2]],
            [cfg.dims[0], cfg.dims[1], cfg.dims[2]]
        );
        assert_eq!(ubo.grid_dims[3], update.hysteresis.clamp(0.0, 0.999).to_bits());
        assert_eq!(ubo.frame_index, 3);
        assert_eq!(ubo.light_count, 5);
        assert_eq!(ubo.rays_per_probe, update.rays_per_probe);
        assert_eq!(ubo.subset_n, update.subset_n);
    }

    #[test]
    fn packed_ubo_clamps_hysteresis_into_the_w_lane() {
        let cfg = DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() };
        let resolved = crate::ddgi_config::resolve_ddgi(&cfg);
        // An out-of-range alpha (>= 1.0 would freeze the field / a NaN would poison the lerp) is
        // clamped to 0.999 before it reaches `grid_dims.w`.
        let update = DdgiUpdateConfig { hysteresis: 1.5, ..DdgiUpdateConfig::default() };
        let ubo = pack_ddgi_update_ubo(&resolved, &update, 0, 1);
        assert_eq!(f32::from_bits(ubo.grid_dims[3]), 0.999);
        // The in-range default round-trips exactly.
        let ok = DdgiUpdateConfig { hysteresis: 0.9, ..DdgiUpdateConfig::default() };
        let ubo_ok = pack_ddgi_update_ubo(&resolved, &ok, 0, 1);
        assert_eq!(f32::from_bits(ubo_ok.grid_dims[3]), 0.9);
    }
}
