//! PBR material table (Render PBR MVP-2).
//!
//! The deferred Cook-Torrance resolve (`deferred_pbr.hlsl`) and the SDF marcher
//! (`sdf_gbuffer_composite.hlsl`) index a GPU-resident `MaterialGpu[]` table by a
//! 16-bit material id (carried per-edit in the `SdfEdit.center.w` free lane, and per
//! G-buffer pixel in `gNormal.BA`). The marcher fetches `base_color`; the resolve
//! fetches the full metallic-roughness parameter set and runs the BRDF.
//!
//! # Layout discipline (the std430 mirror)
//!
//! [`MaterialGpu`] is `#[repr(C, align(16))]` with three clean 16-byte `vec4` lanes —
//! NO mixed-scalar greedy packing — so the std430 mapping the shader reads is
//! unambiguous. A const-assert fingerprint (size / align / every field offset) pins
//! the layout like `SdfEdit`'s §3.8 fingerprint, and [`MATERIAL_GPU_WORDS`] mirrors
//! the shader's `MATERIAL_GPU_WORDS == 12` pin so a host/shader desync is a build
//! error. The total is 48 B / 12 words — one material per 48 B, 65 536 materials in a
//! ~3 MiB SSBO (L2-resident on the target GPUs).
//!
//! # Asset-system rung A1 — the CPU authority moved to `Assets<MaterialGpu>`
//!
//! [`MaterialGpu`] implements [`Asset`] with `Cpu = MaterialGpu` (the GPU layout IS
//! its own decoded form — no separate loader/decode step exists yet). The world-global
//! CPU authority is [`Assets<MaterialGpu>`](boyko_ecs::ecs::core::asset::Assets): mint
//! via `Assets::add`, edit via `Assets::get_mut`. The GPU mirror
//! ([`MaterialTable`](crate::material_table::MaterialTable)) reads that table to
//! seed/refresh the device SSBO — it holds no host authority of its own. This replaces
//! the standalone mesh-materials rung M(-1) `MaterialRegistry`.

use boyko_ecs::ecs::core::asset::{Asset, Handle};

/// The GPU material-table element — a std430-compatible POD, uploaded once / on-change.
///
/// Three 16-byte `vec4` lanes (the shader's `MaterialGpu`):
///
/// - `base_color` (off 0): `rgb` = LINEAR base color, `w` = alpha / cutoff.
/// - `mrr` (off 16): `[metallic, roughness, reflectance, bitcast<f32>(flags)]` — the
///   metallic-roughness parameters packed into one lane. `flags` is reserved (0 in MVP-2).
/// - `emissive` (off 32): `rgb` = LINEAR emissive radiance, `w` unused.
///
/// All values are LINEAR (the resolve tonemaps + applies the OETF once at output).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialGpu {
    /// `rgb` = LINEAR base color, `w` = alpha / cutoff (lane 0, offset 0).
    pub base_color: [f32; 4],
    /// `[metallic, roughness, reflectance, bitcast<f32>(flags)]` (lane 1, offset 16).
    pub mrr: [f32; 4],
    /// `rgb` = LINEAR emissive radiance, `w` unused (lane 2, offset 32).
    pub emissive: [f32; 4],
}

/// Number of `u32` words one [`MaterialGpu`] occupies (`size_of / 4` = 12). Mirrors the
/// shader's `static const uint MATERIAL_GPU_WORDS = 12u`; a desync is a build error
/// host-side (the const-asserts below) and a documented pin shader-side.
pub const MATERIAL_GPU_WORDS: usize = core::mem::size_of::<MaterialGpu>() / 4;

// ---- std430 / repr(C) layout fingerprint (mirrors the shader's MaterialGpu) --------
//
// A mismatch between this Rust struct and the std430 element the shader reads is silent
// GPU corruption neither the validation layer nor a golden diff would localize. These
// const-asserts make any drift a BUILD ERROR, exactly like `SdfEdit`'s §3.8 fingerprint.
const _: () = assert!(
    core::mem::size_of::<MaterialGpu>() == 48,
    "MaterialGpu must be 48 bytes (3 std430 vec4 lanes the shader reads)"
);
const _: () = assert!(
    core::mem::align_of::<MaterialGpu>() == 16,
    "MaterialGpu must be 16-byte aligned (std430 struct alignment)"
);
const _: () = assert!(
    core::mem::offset_of!(MaterialGpu, base_color) == 0,
    "MaterialGpu::base_color must be at offset 0"
);
const _: () = assert!(
    core::mem::offset_of!(MaterialGpu, mrr) == 16,
    "MaterialGpu::mrr must be at offset 16"
);
const _: () = assert!(
    core::mem::offset_of!(MaterialGpu, emissive) == 32,
    "MaterialGpu::emissive must be at offset 32"
);
const _: () = assert!(MATERIAL_GPU_WORDS == 12, "MATERIAL_GPU_WORDS must equal the shader's 12u");

impl Asset for MaterialGpu {
    /// [`MaterialGpu`] is its own decoded CPU form — no separate loader/decode step
    /// exists yet (materials are authored directly in Rust at MVP-2; a future
    /// asset-loading rung may add a file-backed decode).
    type Cpu = MaterialGpu;
}

// Asset-streaming plan F1: `Assets<MaterialGpu>`'s store-owned `ComponentPool`
// needs `MaterialGpu: AssetBacking` to obtain its layout/`ComponentId`.
// `MaterialGpu` is plain-old-data (`#[repr(C, align(16))]`, `Copy`, no `Drop`,
// no device handle) — the POD macro path (`NEEDS_TEARDOWN = false`, no
// `drop_fn`) fits it exactly.
boyko_ecs::impl_asset_pod_backing!(MaterialGpu);

/// The asset-facing name for [`MaterialGpu`] — `Assets<Material>` mint call sites
/// (`Assets::add`) read more naturally under this alias than the raw GPU-layout type
/// name. A plain type alias, not a newtype: both names address the SAME
/// [`Assets<T>`](boyko_ecs::ecs::core::asset::Assets) table.
pub type Material = MaterialGpu;

/// A material-table index handed to the G-buffer. 16-bit range (the `gNormal.BA` pack +
/// the `SdfEdit.center.w` carrier); 65 536 materials. `0` is the engine default material.
///
/// SEALED (asset-system rung A1): the field is private. A live, table-resolvable id is
/// minted ONLY from a fresh [`Assets<MaterialGpu>`](boyko_ecs::ecs::core::asset::Assets)
/// [`Handle<MaterialGpu>`](boyko_ecs::ecs::core::asset::Handle) via
/// [`from_handle`](Self::from_handle) (the render-carrier truncation of the asset
/// table's `u32` row index to this 16-bit width).
/// [`from_raw_for_tests`](Self::from_raw_for_tests) is the ONLY other constructor,
/// reserved for golden/oracle test harnesses that stamp ids without booting a real
/// `Assets` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MaterialId(u16);

impl MaterialId {
    /// The engine default material (table slot 0), used when an edit carries no explicit
    /// id and when an SDF hit's argmin attribution falls through to the fallback.
    pub const DEFAULT: MaterialId = MaterialId(0);

    /// The registry-relative table index (`0` == [`Self::DEFAULT`]).
    #[inline]
    pub fn index(self) -> u16 {
        self.0
    }

    /// Mints a `MaterialId` from a raw table index. Crate-private: the sole legitimate
    /// cross-crate conversion is [`from_handle`](Self::from_handle) — a bare `u16`
    /// constructor exposed crate-wide would let any caller fabricate an id that never
    /// resolves to a real `Assets<MaterialGpu>` row.
    #[inline]
    pub(crate) fn from_index(index: u16) -> Self {
        Self(index)
    }

    /// Mints the render carrier for a freshly-added
    /// [`Assets<MaterialGpu>`](boyko_ecs::ecs::core::asset::Assets) row — the ONE
    /// legitimate cross-crate conversion from a `Handle<MaterialGpu>` to this type.
    /// Truncates the handle's `u32` row index to this type's 16-bit width: the asset
    /// table permits up to `u32::MAX` rows, but the G-buffer / `SdfEdit` carrier only
    /// reserves 16 bits, so growing an `Assets<MaterialGpu>` table past 65 536 rows and
    /// then minting a carrier for a high row silently aliases ids. `debug_assert!`s the
    /// index is in range; at asset-system rung A1 the only caller mints exactly one row
    /// (index 0), far under the limit.
    #[inline]
    pub fn from_handle(handle: Handle<MaterialGpu>) -> Self {
        let index = handle.index();
        debug_assert!(
            index <= u16::MAX as u32,
            "invariant: the material table fits the 16-bit MaterialId range (65 536 slots)"
        );
        Self::from_index(index as u16)
    }

    /// Test/golden-harness escape hatch: mints a `MaterialId` OUTSIDE a real
    /// [`Assets<MaterialGpu>`](boyko_ecs::ecs::core::asset::Assets) table (a harness
    /// that stamps ids without booting one). NEVER call this from production code — an
    /// id minted here may not resolve to a live table row.
    #[doc(hidden)]
    #[inline]
    pub fn from_raw_for_tests(index: u16) -> Self {
        Self(index)
    }

    /// Bit-casts the id into the `f32` an `SdfEdit.center.w` free lane carries (the shader
    /// reads it back via `asuint(Buf[base + 3])`). A round-trip-safe `u16 → u32 → f32`
    /// bit pattern (the value is an integer id, never interpreted as a float arithmetically).
    #[inline]
    pub fn to_center_w_bits(self) -> f32 {
        f32::from_bits(self.0 as u32)
    }

    /// Recovers a [`MaterialId`] from an `SdfEdit.center.w` bit pattern (the inverse of
    /// [`Self::to_center_w_bits`]). Ids above the 16-bit range are truncated to the low
    /// 16 bits (the G-buffer carrier width).
    #[inline]
    pub fn from_center_w_bits(w: f32) -> Self {
        MaterialId((w.to_bits() & 0xFFFF) as u16)
    }
}

impl MaterialGpu {
    /// Builds a metallic-roughness material from LINEAR parameters. `reflectance` is the
    /// dielectric F0 scale (the standard `0.5` → 4% F0); `flags` is reserved (pass `0`).
    #[inline]
    pub fn new(
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
        reflectance: f32,
        emissive: [f32; 3],
        flags: u32,
    ) -> Self {
        Self {
            base_color,
            mrr: [metallic, roughness, reflectance, f32::from_bits(flags)],
            emissive: [emissive[0], emissive[1], emissive[2], 0.0],
        }
    }

    /// The metallic parameter (`mrr.x`).
    #[inline]
    pub fn metallic(&self) -> f32 {
        self.mrr[0]
    }

    /// The perceptual-roughness parameter (`mrr.y`).
    #[inline]
    pub fn roughness(&self) -> f32 {
        self.mrr[1]
    }

    /// The dielectric reflectance parameter (`mrr.z`).
    #[inline]
    pub fn reflectance(&self) -> f32 {
        self.mrr[2]
    }
}

impl Default for MaterialGpu {
    /// The engine default material (table slot 0): a mid-gray dielectric — neutral base
    /// color, non-metal, moderately rough, standard 4% F0, no emission.
    #[inline]
    fn default() -> Self {
        MaterialGpu::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.5, 0.5, [0.0, 0.0, 0.0], 0)
    }
}
