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
//!
//! # Textured-PBR rung T5 — `Material` splits into `{ gpu, textures }`
//!
//! [`Material`] is no longer a bare alias for [`MaterialGpu`]: it is the CPU authority
//! struct `{ gpu: MaterialGpu, textures: MaterialTextures }`. [`MaterialGpu`] KEEPS its
//! FROZEN 48-byte std430 layout unchanged — it is still the exact element the device SSBO
//! ([`MaterialTable`](crate::material_table::MaterialTable)) uploads. [`MaterialTextures`]
//! is a CPU-only, POD sidecar of five bindless texture slots (textured-PBR T2's
//! [`BindlessTextureTable`](crate::bindless::BindlessTextureTable) indices), read by the
//! per-instance texture gather (T5's dormant [`mesh_draw::PerInstanceMaterialTex`] lane;
//! T6 wires it into a shader pipeline). `Asset`/`AssetBacking`/`HasLoaders` moved from
//! `MaterialGpu` to `Material` — `Assets<Material>` (not `Assets<MaterialGpu>`) is now the
//! world-global CPU authority every call site mints/edits.

use boyko_ecs::ecs::core::asset::{Asset, Handle, HasLoaders, LoaderEntry};

use crate::loaders::RonMaterialLoader;

/// The GPU material-table element — a std430-compatible POD, uploaded once / on-change.
///
/// Three 16-byte `vec4` lanes (the shader's `MaterialGpu`):
///
/// - `base_color` (off 0): `rgb` = LINEAR base color, `w` = alpha / cutoff.
/// - `mrr` (off 16): `[metallic, roughness, reflectance, bitcast<f32>(flags)]` — the
///   metallic-roughness parameters packed into one lane. `flags` is reserved (0 in MVP-2).
/// - `emissive` (off 32): `rgb` = LINEAR emissive radiance, `w` unused.
///
/// All values are LINEAR. PBR P0-C: the deferred resolve (`deferred_pbr.hlsl`) Hill
/// ACES-fits the exposed radiance THEN manually gamma-2.2 encodes it once, right before the
/// single `gLit` store — `gLit` (R8G8B8A8_UNORM) and the swapchain (`pick_surface_format`
/// prefers `*_UNORM` over `*_SRGB`) are linear UNORM end to end, so nothing downstream
/// hardware-encodes sRGB and the OETF must be manual.
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

/// A bit in [`MaterialGpu::mrr`]'s bitcast `flags` lane (`mrr[3]`): set iff the
/// material's [`MaterialTextures`] carries at least one non-zero bindless slot
/// (textured-PBR rung T5). A material minted via [`Material::new`] / [`Material::default`]
/// / [`From<MaterialGpu>`](Material) never sets this bit (`flags` stays whatever the
/// caller passed, `0` in every non-textured constructor) — so a non-textured material's
/// `MaterialGpu` bytes are BYTE-IDENTICAL to the pre-T5 shape, and every in-tree golden
/// holds. The shader-side READ of this bit is a later rung (T6).
///
/// AUTHORITATIVE derivation happens at the host→GPU upload boundary
/// ([`MaterialTable::seed_rows`](crate::material_table::MaterialTable), grooming item
/// B): every copy of a row's bytes into the device SSBO re-derives this bit from
/// `textures.any()` — ORed in when a slot is bound, ANDed out otherwise — regardless of
/// whatever value `gpu.mrr[3]` already carries, so direct field mutation of `gpu` or
/// `textures` after construction can never desync the two on the device.
/// [`Material::with_textures`] also sets it CPU-side, as a convenience mirror for a
/// reader that inspects `gpu.mrr[3]` before the next upload — not the sole source of
/// truth.
pub const MATERIAL_FLAG_TEXTURED: u32 = 1;

/// The CPU-only sidecar of bindless texture slots a [`Material`] carries alongside its
/// [`MaterialGpu`] SSBO element (textured-PBR rung T5).
///
/// Each field is a RESOLVED bindless index into
/// [`BindlessTextureTable`](crate::bindless::BindlessTextureTable) — `0` means "no
/// texture, fall back to the matching [`MaterialGpu`] scalar/vector parameter" (documented
/// per-field below). Slots are set by textured-PBR T7 (upload texture → register in the
/// bindless table → store the returned slot here); this type is POD infrastructure only —
/// it depends on NEITHER `crate::texture::TextureGpu` NOR
/// `Handle<TextureGpu>` (a trivial `u32`-only gather, zero dependency on the T2 texture
/// asset types, HYBRID-perf: the per-instance texture gather stays a flat struct copy).
///
/// # Append-only-texture caveat
///
/// A cached slot here is valid ONLY while the world's texture table is APPEND-ONLY (never
/// frees/recycles a bindless slot) — identical to `MeshHandle` / [`MaterialId`]'s
/// truncation/staleness caveat. A streaming-free texture table (where a slot could be
/// recycled out from under a cached `MaterialTextures`) is DEFERRED, mirroring
/// asset-streaming plan task#13 (`MaterialStale` substitution) for the material side.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialTextures {
    /// Bindless slot of the albedo (base-color) texture; `0` = none, use
    /// [`MaterialGpu::base_color`].
    pub albedo: u32,
    /// Bindless slot of the tangent-space normal map; `0` = none, use the mesh's vertex
    /// normal unperturbed.
    pub normal: u32,
    /// Bindless slot of the packed metallic-roughness texture; `0` = none, use
    /// [`MaterialGpu::mrr`]'s `x`/`y` (metallic/roughness) scalars.
    pub metal_rough: u32,
    /// Bindless slot of the ambient-occlusion texture; `0` = none, AO defaults to `1`
    /// (no occlusion).
    pub ao: u32,
    /// Bindless slot of the emissive texture; `0` = none, use [`MaterialGpu::emissive`].
    pub emissive: u32,
}

impl MaterialTextures {
    /// No bindless textures bound — every field `0`. The non-textured default every
    /// [`Material::new`] / [`Material::default`] / [`From<MaterialGpu>`](Material) mint
    /// carries.
    pub const NONE: Self = Self { albedo: 0, normal: 0, metal_rough: 0, ao: 0, emissive: 0 };

    /// `true` iff at least one slot is bound (non-zero) — the gate
    /// [`Material::with_textures`] consults to set [`MATERIAL_FLAG_TEXTURED`].
    #[inline]
    pub fn any(&self) -> bool {
        self.albedo | self.normal | self.metal_rough | self.ao | self.emissive != 0
    }
}

/// The CPU material authority (textured-PBR rung T5): the FROZEN 48-byte
/// [`MaterialGpu`] SSBO element plus its CPU-only [`MaterialTextures`] sidecar.
///
/// `Assets<Material>` (not `Assets<MaterialGpu>`) is the world-global CPU authority —
/// mint via `Assets::add`, edit via `Assets::get_mut`. The device SSBO
/// ([`MaterialTable`](crate::material_table::MaterialTable)) mirrors ONLY `gpu` (the
/// `textures` sidecar is host-side, consumed by the per-instance texture gather, never
/// uploaded as a table row itself).
///
/// # [`MATERIAL_FLAG_TEXTURED`] is derived at upload, not trusted from `gpu`
///
/// `gpu.mrr[3]`'s [`MATERIAL_FLAG_TEXTURED`] bit is RE-DERIVED from
/// `textures.`[`any`](MaterialTextures::any)`()` at every host→GPU copy
/// ([`MaterialTable::seed_rows`](crate::material_table::MaterialTable), the sole site
/// that packs a row's bytes into the device SSBO) — set when a slot is bound, cleared
/// otherwise — so direct field mutation of either `gpu` or `textures` after
/// construction (e.g. `mat.textures.albedo = slot`) can never desync the two on the
/// device; only the value staged for the next upload is authoritative.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// The 48-byte std430 SSBO element the device table mirrors.
    pub gpu: MaterialGpu,
    /// The CPU-only bindless-texture sidecar (textured-PBR T5/T7).
    pub textures: MaterialTextures,
}

impl Material {
    /// Builds a NON-TEXTURED material from LINEAR parameters — identical signature to
    /// [`MaterialGpu::new`], which this delegates to; `textures` starts at
    /// [`MaterialTextures::NONE`]. `flags` is passed through verbatim (byte-identical
    /// `gpu` to a direct `MaterialGpu::new` call — no [`MATERIAL_FLAG_TEXTURED`] bit is
    /// ever set here).
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
            gpu: MaterialGpu::new(base_color, metallic, roughness, reflectance, emissive, flags),
            textures: MaterialTextures::NONE,
        }
    }

    /// Builds a material from an existing [`MaterialGpu`] plus a [`MaterialTextures`]
    /// sidecar, setting [`MATERIAL_FLAG_TEXTURED`] in `gpu.mrr[3]` iff
    /// `textures.any()`. The ONLY constructor that can produce a TEXTURED material
    /// (textured-PBR T7's authoring entry point).
    #[inline]
    pub fn with_textures(mut gpu: MaterialGpu, textures: MaterialTextures) -> Self {
        if textures.any() {
            gpu.mrr[3] = f32::from_bits(gpu.mrr[3].to_bits() | MATERIAL_FLAG_TEXTURED);
        }
        Self { gpu, textures }
    }
}

impl From<MaterialGpu> for Material {
    /// Wraps a bare [`MaterialGpu`] with [`MaterialTextures::NONE`] — byte-identical
    /// `gpu` bytes, no [`MATERIAL_FLAG_TEXTURED`] bit set.
    #[inline]
    fn from(gpu: MaterialGpu) -> Self {
        Self { gpu, textures: MaterialTextures::NONE }
    }
}

impl Default for Material {
    /// The engine default material (table slot 0): [`MaterialGpu::default`] with no
    /// textures.
    #[inline]
    fn default() -> Self {
        Self { gpu: MaterialGpu::default(), textures: MaterialTextures::NONE }
    }
}

impl Asset for Material {
    /// [`Material`] is its own decoded CPU form — no separate loader/decode step exists
    /// yet beyond [`RonMaterialLoader`] (materials are authored directly in Rust or via
    /// the in-house `.mat` text format).
    type Cpu = Material;
}

// Asset-streaming plan F1 / textured-PBR T5: `Assets<Material>`'s store-owned
// `ComponentPool` needs `Material: AssetBacking` to obtain its layout/`ComponentId`.
// `Material` is plain-old-data (`#[repr(C)]`, `Copy`, no `Drop`, no device handle — both
// `MaterialGpu` and `MaterialTextures` are POD) — the POD macro path
// (`NEEDS_TEARDOWN = false`, no `drop_fn`) fits it exactly.
boyko_ecs::impl_asset_pod_backing!(Material);

impl HasLoaders for Material {
    /// One entry: the in-house `.mat` text-format loader. Asset-streaming
    /// plan F3 — a compile-time-static table, no runtime registration.
    const LOADERS: &'static [LoaderEntry<Self>] = &[LoaderEntry::of::<RonMaterialLoader>()];
}

/// A material-table index handed to the G-buffer. 16-bit range (the `gNormal.BA` pack +
/// the `SdfEdit.center.w` carrier); 65 536 materials. `0` is the engine default material.
///
/// SEALED (asset-system rung A1): the field is private. A live, table-resolvable id is
/// minted ONLY from a fresh [`Assets<Material>`](boyko_ecs::ecs::core::asset::Assets)
/// [`Handle<Material>`](boyko_ecs::ecs::core::asset::Handle) via
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
    /// resolves to a real `Assets<Material>` row.
    #[inline]
    pub(crate) fn from_index(index: u16) -> Self {
        Self(index)
    }

    /// Mints the render carrier for a freshly-added
    /// [`Assets<Material>`](boyko_ecs::ecs::core::asset::Assets) row — the ONE
    /// legitimate cross-crate conversion from a `Handle<Material>` to this type.
    /// Truncates the handle's `u32` row index to this type's 16-bit width: the asset
    /// table permits up to `u32::MAX` rows, but the G-buffer / `SdfEdit` carrier only
    /// reserves 16 bits, so growing an `Assets<Material>` table past 65 536 rows and
    /// then minting a carrier for a high row silently aliases ids. `debug_assert!`s the
    /// index is in range; at asset-system rung A1 the only caller mints exactly one row
    /// (index 0), far under the limit.
    #[inline]
    pub fn from_handle(handle: Handle<Material>) -> Self {
        let index = handle.index();
        debug_assert!(
            index <= u16::MAX as u32,
            "invariant: the material table fits the 16-bit MaterialId range (65 536 slots)"
        );
        Self::from_index(index as u16)
    }

    /// Test/golden-harness escape hatch: mints a `MaterialId` OUTSIDE a real
    /// [`Assets<Material>`](boyko_ecs::ecs::core::asset::Assets) table (a harness
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

#[cfg(test)]
mod tests {
    use super::*;

    /// [`Material::new`] must produce a `gpu` byte-identical to a direct
    /// [`MaterialGpu::new`] call with the same parameters, plus
    /// [`MaterialTextures::NONE`] — the byte-identity contract T5's plan depends on.
    #[test]
    fn material_new_matches_material_gpu_new_and_carries_no_textures() {
        let base_color = [0.1, 0.2, 0.3, 1.0];
        let emissive = [0.05, 0.1, 0.2];
        let mat = Material::new(base_color, 0.6, 0.25, 0.7, emissive, 0);
        let gpu = MaterialGpu::new(base_color, 0.6, 0.25, 0.7, emissive, 0);

        assert_eq!(mat.gpu, gpu, "Material::new's gpu field must byte-match MaterialGpu::new");
        assert_eq!(mat.textures, MaterialTextures::NONE, "a non-textured mint carries no textures");
        assert!(!mat.textures.any());
    }

    /// [`Material::default`] must match [`MaterialGpu::default`] plus no textures.
    #[test]
    fn material_default_matches_material_gpu_default_and_carries_no_textures() {
        let mat = Material::default();
        assert_eq!(mat.gpu, MaterialGpu::default());
        assert_eq!(mat.textures, MaterialTextures::NONE);
    }

    /// [`From<MaterialGpu>`] wraps the value verbatim with no textures — the conversion
    /// used by every pre-T5 call site that only ever produced a bare `MaterialGpu`.
    #[test]
    fn from_material_gpu_preserves_gpu_bytes_with_no_textures() {
        let gpu = MaterialGpu::new([0.4, 0.5, 0.6, 1.0], 1.0, 0.1, 0.5, [0.0; 3], 0);
        let mat: Material = gpu.into();
        assert_eq!(mat.gpu, gpu);
        assert_eq!(mat.textures, MaterialTextures::NONE);
    }

    /// [`Material::with_textures`] sets [`MATERIAL_FLAG_TEXTURED`] in `gpu.mrr[3]` when
    /// `textures.any()`, while preserving `base_color` / `mrr.x` (metallic) / `mrr.y`
    /// (roughness) / `emissive`.
    #[test]
    fn with_textures_sets_the_textured_flag_and_preserves_scalar_params() {
        let base_color = [0.7, 0.2, 0.2, 1.0];
        let emissive = [0.0, 0.3, 0.0];
        let gpu = MaterialGpu::new(base_color, 1.0, 0.4, 0.5, emissive, 0);
        let textures = MaterialTextures { albedo: 3, normal: 0, metal_rough: 0, ao: 0, emissive: 0 };

        let mat = Material::with_textures(gpu, textures);

        assert_eq!(
            mat.gpu.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED,
            MATERIAL_FLAG_TEXTURED,
            "with_textures must set the TEXTURED bit when any texture slot is bound"
        );
        assert_eq!(mat.gpu.base_color, base_color, "base_color must be preserved verbatim");
        assert_eq!(mat.gpu.metallic(), gpu.metallic(), "metallic must be preserved verbatim");
        assert_eq!(mat.gpu.roughness(), gpu.roughness(), "roughness must be preserved verbatim");
        assert_eq!(mat.gpu.emissive, gpu.emissive, "emissive must be preserved verbatim");
        assert_eq!(mat.textures, textures);
    }

    /// [`Material::with_textures`] with an all-zero [`MaterialTextures`] must NOT set the
    /// TEXTURED flag — byte-identical to a plain [`From<MaterialGpu>`] conversion.
    #[test]
    fn with_textures_leaves_the_flag_clear_when_no_slot_is_bound() {
        let gpu = MaterialGpu::default();
        let mat = Material::with_textures(gpu, MaterialTextures::NONE);
        assert_eq!(mat.gpu, gpu, "an all-zero textures sidecar must not perturb gpu bytes");
        assert_eq!(mat.gpu.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED, 0);
    }

    /// [`MaterialTextures::any`] is `false` only for [`MaterialTextures::NONE`]; any single
    /// non-zero slot flips it `true`.
    #[test]
    fn material_textures_any_is_true_iff_a_slot_is_bound() {
        assert!(!MaterialTextures::NONE.any());
        assert!(MaterialTextures { albedo: 1, ..MaterialTextures::NONE }.any());
        assert!(MaterialTextures { normal: 1, ..MaterialTextures::NONE }.any());
        assert!(MaterialTextures { metal_rough: 1, ..MaterialTextures::NONE }.any());
        assert!(MaterialTextures { ao: 1, ..MaterialTextures::NONE }.any());
        assert!(MaterialTextures { emissive: 1, ..MaterialTextures::NONE }.any());
    }
}
