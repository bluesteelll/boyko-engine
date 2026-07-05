//! HW-RT rung R1 — the dormant unified ray / acceleration-structure backend seam
//! (CPU, unit-testable). This is the zero-cost scaffolding layer: the
//! [`RayBackendConfig`] carrier + its pure single-writer resolve, resolved to
//! all-`Software` for every device tier. No acceleration structure, no RT
//! extension, no shader variant, no rendered-pixel change — the grand_showcase
//! golden stays byte-identical (plan §4).
//!
//! Principle 0: ECS-native — [`RayBackendConfig`] is a `#[derive(Resource)]`
//! singleton (the cold derived carrier, NOT a side `std::Vec`/`HashMap`) written
//! by the cold [`resolve_ray_backend_system`]. This mirrors the DDGI substrate
//! EXACTLY: [`ResolvedDdgi`](crate::ddgi_config::ResolvedDdgi) (the derived
//! carrier) + [`resolve_ddgi`](crate::ddgi_config::resolve_ddgi) (the pure fit) +
//! [`resolve_ddgi_grid_gated`](crate::ddgi_update::resolve_ddgi_grid_gated) (the
//! cold single-writer) + [`DdgiResolveSet`](crate::ddgi_config::DdgiResolveSet)
//! (the by-name ordering seam).
//!
//! # The dormancy anchor (plan D1/D3) + the R2a-4b routing
//!
//! [`resolve_ray_backend`] returns [`RayBackendConfig::DISABLED`] (every cell
//! [`RayBackend::Software`]) for [`RtTier::Absent`] — the byte-identity majority
//! and every non-RT device. R2a-4b brought the `Weak`/`Strong` arms alive: they
//! route the mesh-shadow cell to [`RayBackend::HardwareTri`] (the deferred
//! resolve's `rayQuery` TLAS trace, the FIRST consumer) and keep every other cell
//! software. The `HardwareTri` cell is honored only when the resolve is built under
//! `feature = "hwrt"` + `ctx.ray_query_enabled()`; on a non-hwrt build no consumer
//! reads it, so the config is inert and the render stays byte-identical.

use boyko_macros::{Resource, SystemSet};

use boyko_ecs::ecs::core::system::{Res, ResMut};

use boyko_rhi_vulkan::device::RtTier;

// ---- vocabulary (the stable ABI R2a fills) --------------------------------------------

/// The backend that services a ray workload — the selection each config cell
/// carries. R1 only ever SELECTS [`Software`](RayBackend::Software); the hardware
/// arms are declared so R2a's routing ABI is stable (plan §3 vocab).
///
/// `#[repr(u8)]` for a compact, stable discriminant the config table stores by
/// value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RayBackend {
    /// The CPU / compute software path — the only selection in R1 (the dormancy
    /// anchor). Requires no acceleration structure and no RT extension.
    Software = 0,
    /// Hardware triangle ray tracing (a mesh-only BLAS/TLAS). Reserved for R2a;
    /// never selected in R1.
    HardwareTri = 1,
    /// Hardware mixed ray tracing (triangles + procedural/SDF AABBs in one TLAS).
    /// Reserved for R4; never selected in R1.
    HardwareMixed = 2,
}

impl RayBackend {
    /// The number of [`RayBackend`] variants (the discriminant domain size).
    pub const COUNT: usize = 3;
}

/// A ray-traced workload class — the ROW axis of the backend selection table.
/// Each class may route to a different backend once R2a's routing lands (e.g.
/// shadows on hardware, GI probes on software). `#[repr(usize)]` so a variant
/// indexes the table row directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum RayWorkload {
    /// Shadow-ray visibility (the first hardware candidate in R2a).
    Shadow = 0,
    /// Ambient-occlusion rays.
    Ao = 1,
    /// GI-probe update rays (the SDFDDGI marcher's workload).
    GiProbe = 2,
    /// Reflection rays.
    Reflection = 3,
}

impl RayWorkload {
    /// The number of [`RayWorkload`] variants (the table's row count).
    pub const COUNT: usize = 4;
}

/// The geometry representation a ray traverses — the COLUMN axis of the backend
/// selection table. A workload may pick a different backend per geometry kind
/// (mesh triangles vs SDF fields). `#[repr(usize)]` so a variant indexes the
/// table column directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum RayGeom {
    /// Triangle-mesh geometry (a hardware BLAS candidate).
    Mesh = 0,
    /// Signed-distance-field geometry (the marcher / procedural-AABB path).
    Sdf = 1,
}

impl RayGeom {
    /// The number of [`RayGeom`] variants (the table's column count).
    pub const COUNT: usize = 2;
}

// ---- RayBackendConfig (the derived carrier — mirrors ResolvedDdgi) --------------------

/// The derived ray-backend selection the router reads — the ray analogue of
/// [`ResolvedDdgi`](crate::ddgi_config::ResolvedDdgi). [`resolve_ray_backend_system`]
/// is its SINGLE writer (the one-producer-per-field discipline), recomputing it
/// from the device [`RtTier`] each frame. A cold POD: `#[repr(C)]` for a stable
/// layout (the byte-mirror a later R2a router / push-constant consumes).
///
/// DISABLED == [`Default`] == every cell [`RayBackend::Software`] with a unit
/// budget: the dormancy anchor (plan D3). No consumer reads it in R1, so even a
/// non-default config is inert.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct RayBackendConfig {
    /// The `[workload][geom]` backend-selection table — row per [`RayWorkload`],
    /// column per [`RayGeom`]. In R1 every cell is [`RayBackend::Software`]
    /// (the dormancy anchor); R2a's routing fills the hardware cells.
    pub table: [[RayBackend; RayGeom::COUNT]; RayWorkload::COUNT],
    /// The per-workload ray budget (rays-per-pixel or per-probe cap), indexed by
    /// [`RayWorkload`]. `1` in R1 (the minimum non-zero budget); R2a tunes it.
    pub budget: [u16; RayWorkload::COUNT],
    /// Trailing padding to a stable stride (reserved; zero in R1 — a later rung
    /// may carry per-workload flags here).
    pub _pad: [u8; 8],
}

// Layout pin: 8 (table = 4 rows × 2 cols × 1 B) + 8 (budget = 4 × u16) + 8 (pad)
// = 24 B. A change is a deliberate decision (a later R2a router / push-constant
// reads this stride).
const _: () = assert!(size_of::<RayBackendConfig>() == 24);
const _: () = assert!(core::mem::offset_of!(RayBackendConfig, table) == 0);
const _: () = assert!(core::mem::offset_of!(RayBackendConfig, budget) == 8);
const _: () = assert!(core::mem::offset_of!(RayBackendConfig, _pad) == 16);

/// The byte size of the [`RayBackendConfig`] carrier (24 B) — the single source
/// a later R2a host sizes its router push-constant / UBO from (no hand-copied
/// `24`). Mirrors [`RESOLVED_DDGI_BYTES`](crate::ddgi_config::RESOLVED_DDGI_BYTES).
pub const RAY_BACKEND_CONFIG_BYTES: usize = size_of::<RayBackendConfig>();

impl RayBackendConfig {
    /// The DISABLED selection — every table cell [`RayBackend::Software`], every
    /// budget `1`, padding zero. The resolve of any [`RtTier`] in R1 and the value
    /// [`RayBackendConfig::default`] returns. All-software is load-bearing: the
    /// byte-identity argument (plan §4) rests on DISABLED == Default == no
    /// hardware cell.
    pub const DISABLED: Self = Self {
        table: [[RayBackend::Software; RayGeom::COUNT]; RayWorkload::COUNT],
        budget: [1; RayWorkload::COUNT],
        _pad: [0; 8],
    };
}

impl Default for RayBackendConfig {
    /// The resolve of the dormant seam — [`RayBackendConfig::DISABLED`], so a
    /// never-run policy already carries the all-software selection (the dormancy
    /// anchor).
    #[inline]
    fn default() -> Self {
        Self::DISABLED
    }
}

// ---- RayCaps (the device-tier resource — mirrors DdgiCaps) ----------------------------

/// The device ray-tier capability the resolve reads — the ray analogue of
/// [`DdgiCaps`](crate::ddgi_update::DdgiCaps). A `World`-singleton
/// `#[derive(Resource)]` the host inserts at device boot from
/// [`DeviceCaps::rt_tier()`](boyko_rhi_vulkan's `DeviceCaps`).
///
/// There is NO world-resident `DeviceCaps` resource to read (the DDGI substrate
/// derives its own [`DdgiCaps`] mirror the same way), so R1 carries a ray-specific
/// tier mirror. [`Default`] is [`RtTier::Absent`] (dormant if the host never fills
/// it — a headless bench or a host that never queries stays in the software path);
/// the host OVERRIDES it at boot with the real `rt_tier()` query, at the SAME site
/// [`DdgiCaps`] is filled.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayCaps {
    /// The device ray-tracing tier (`DeviceCaps::rt_tier()`). In R1 this is
    /// [`RtTier::Absent`] on every device (`ray_query` hard-wired `false`), so
    /// [`resolve_ray_backend`] selects the all-software path.
    pub tier: RtTier,
}

impl Default for RayCaps {
    /// The dormant default — [`RtTier::Absent`] (the software path). A host that
    /// queries the device overrides this at boot; a bench that never queries
    /// stays software-only.
    #[inline]
    fn default() -> Self {
        Self { tier: RtTier::Absent }
    }
}

impl RayCaps {
    /// Builds the caps from a device `rt_tier()` query result (the host boot seam).
    #[inline]
    pub const fn new(tier: RtTier) -> Self {
        Self { tier }
    }
}

// ---- the resolve decision (pure — the unit-testable fit) ------------------------------

/// Derives the [`RayBackendConfig`] carrier from the device [`RtTier`] — the PURE,
/// unit-testable ray-backend resolve (the analogue of
/// [`resolve_ddgi`](crate::ddgi_config::resolve_ddgi), the core the cold system
/// wraps). Narrow `RtTier` argument (per-tier testable, plan §5 / open-question 4);
/// R3 widens it to `(RayCalibration, DeviceCaps)`.
///
/// EVERY arm returns [`RayBackendConfig::DISABLED`] in R1 — the all-software
/// dormancy anchor (RISK-2 lock, plan §6). The `Weak`/`Strong` arms are written
/// now so R2a fills them without reshaping the function; matching on `tier`
/// exhaustively keeps the shape R2a diverges from visible.
#[inline]
pub fn resolve_ray_backend(tier: RtTier) -> RayBackendConfig {
    match tier {
        // No hardware ray query — the software path (byte-identity majority + every
        // non-RT device). Every cell stays [`RayBackend::Software`].
        RtTier::Absent => RayBackendConfig::DISABLED,
        // R2a-4b: hardware ray query present (without / with reorder). Route the
        // mesh-shadow workload to the hardware triangle backend — the deferred
        // resolve's `rayQuery` TLAS trace (the FIRST consumer). Every OTHER cell
        // stays software (SDF shadows, AO, GI probes, reflections keep their
        // software paths). `HardwareTri` is honored only when the resolve is built
        // under `feature = "hwrt"` + `ctx.ray_query_enabled()`; on a non-hwrt build
        // the cell is inert (no consumer reads it).
        RtTier::Weak | RtTier::Strong => {
            let mut cfg = RayBackendConfig::DISABLED;
            cfg.table[RayWorkload::Shadow as usize][RayGeom::Mesh as usize] =
                RayBackend::HardwareTri;
            cfg
        }
    }
}

// ---- RayResolveSet + AsBuildSet (the ordering seams) ----------------------------------

/// The `Main`-schedule ordering seam that pins the ray-backend resolve BEFORE its
/// consumers — the ray analogue of
/// [`DdgiResolveSet`](crate::ddgi_config::DdgiResolveSet). [`resolve_ray_backend_system`]
/// joins it via `.in_set(RayResolveSet)`; a later consumer pins itself AFTER via
/// `.after_set(RayResolveSet)`. Set-to-set ordering is add-order-independent and
/// pinned by name (the `DdgiResolveSet` precedent). No consumer exists in R1.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct RayResolveSet;

/// The empty by-name anchor the R2a acceleration-structure build + its
/// `.after_set(AsBuildSet)` consumers hang on — declared now so R2a adds the
/// build system + the `ACCELERATION_STRUCTURE_WRITE→READ` barrier consumers
/// without reshaping the schedule.
///
/// # No `configure_set` in R1 (open-question 3)
///
/// `AsBuildSet` has NO member and NO consumer in R1. The scheduler interns a set
/// by value on first reference — `.in_set`/`.before_set`/`.after_set` all call
/// `set_id_of_value`, exactly how [`DdgiResolveSet`] becomes orderable through its
/// `.in_set` member. So a R2a consumer's `.after_set(AsBuildSet)` interns it then;
/// a `configure_set(AsBuildSet)` in R1 would be an inert no-op (a set with no
/// member and no edges). The derive + the first R2a reference suffices.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct AsBuildSet;

// ---- the cold single-writer system ----------------------------------------------------

/// Writes [`RayBackendConfig`] from the device [`RayCaps`] tier — the SINGLE
/// writer of the ray-backend carrier (the one-producer-per-field discipline), the
/// ray analogue of
/// [`resolve_ddgi_grid_gated`](crate::ddgi_update::resolve_ddgi_grid_gated). It
/// recomputes the selection from the boot tier each frame (a dormant `Absent` tier
/// resolves the all-software carrier).
///
/// The `debug_assert!` is the dormancy tripwire, made TIER-CONDITIONAL in R2a-4b
/// (critic P1-4): when the device has NO hardware ray query ([`RtTier::Absent`] —
/// the byte-identity majority / every non-RT device) every resolved cell MUST STILL
/// be [`RayBackend::Software`] — a hardware cell on an `Absent` device would mean the
/// software-path invariant broke (e.g. `ray_query` spuriously `true`). On a `Weak` /
/// `Strong` device the mesh-shadow cell is legitimately [`RayBackend::HardwareTri`],
/// so the assert is skipped for those tiers.
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive `resolve_ddgi_grid_gated`
// carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_ray_backend_system(caps: Res<RayCaps>, mut out: ResMut<RayBackendConfig>) {
    let resolved = resolve_ray_backend(caps.tier);
    debug_assert!(
        caps.tier != RtTier::Absent
            || resolved
                .table
                .iter()
                .all(|row| row.iter().all(|&b| b == RayBackend::Software)),
        "invariant: an Absent-tier device must resolve every ray-backend cell to Software \
         (the software-path anchor)"
    );
    *out = resolved;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cell of DISABLED (and Default) is `Software`, budget `1` (plan §8).
    #[test]
    fn default_is_disabled_all_software() {
        let cfg = RayBackendConfig::default();
        assert_eq!(cfg, RayBackendConfig::DISABLED);
        for row in &cfg.table {
            for &cell in row {
                assert_eq!(cell, RayBackend::Software);
            }
        }
        assert_eq!(cfg.budget, [1; RayWorkload::COUNT]);
        assert_eq!(cfg._pad, [0; 8]);
    }

    /// The R2a-4b routing contract: an `Absent`-tier device resolves all-software
    /// (the byte-identity anchor); a `Weak` / `Strong` device routes ONLY the
    /// mesh-shadow cell to [`RayBackend::HardwareTri`] and keeps every other cell
    /// software.
    #[test]
    fn resolve_routes_mesh_shadow_on_rt_tiers() {
        // Absent: unchanged all-software dormancy anchor.
        assert_eq!(resolve_ray_backend(RtTier::Absent), RayBackendConfig::DISABLED);

        // Weak / Strong: the mesh-shadow cell is HardwareTri; every OTHER cell is Software.
        for tier in [RtTier::Weak, RtTier::Strong] {
            let cfg = resolve_ray_backend(tier);
            for (w, row) in cfg.table.iter().enumerate() {
                for (g, &cell) in row.iter().enumerate() {
                    let expected = if w == RayWorkload::Shadow as usize
                        && g == RayGeom::Mesh as usize
                    {
                        RayBackend::HardwareTri
                    } else {
                        RayBackend::Software
                    };
                    assert_eq!(cell, expected, "tier {tier:?} cell [{w}][{g}]");
                }
            }
        }
    }

    /// The resolve is a pure function — the same tier resolves the same carrier
    /// (idempotence, plan §8).
    #[test]
    fn resolve_is_idempotent() {
        for tier in [RtTier::Absent, RtTier::Weak, RtTier::Strong] {
            assert_eq!(resolve_ray_backend(tier), resolve_ray_backend(tier));
        }
    }

    /// The layout pin + the vocab COUNTs (3 / 4 / 2), plan §8.
    #[test]
    fn layout_and_counts_are_pinned() {
        assert_eq!(RAY_BACKEND_CONFIG_BYTES, 24);
        assert_eq!(size_of::<RayBackendConfig>(), 24);
        assert_eq!(RayBackend::COUNT, 3);
        assert_eq!(RayWorkload::COUNT, 4);
        assert_eq!(RayGeom::COUNT, 2);
    }

    /// `RayCaps::default()` is the dormant `Absent` tier (the software path).
    #[test]
    fn ray_caps_default_is_absent() {
        assert_eq!(RayCaps::default().tier, RtTier::Absent);
        assert_eq!(RayCaps::new(RtTier::Strong).tier, RtTier::Strong);
    }
}
