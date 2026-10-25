//! IBL1 R1 (`ibl1/cut.md` C11, D-P2b) — the `-D REFL=1` twins of the lit producers and the ONE
//! boot-time selector that picks between a producer's base `.spv` and its REFL twin.
//!
//! REFL is an INTERFACE axis: the twin adds one combined image sampler (the prefiltered
//! environment) and the SKY branch's environment arm, nothing else. An armed boot
//! (`boyko_render::ResolvedEnvironment::armed`) builds the twin INSTEAD of the base for every lit
//! producer it builds (the pipeline count does not change); an unarmed boot builds today's base
//! modules, so its pipelines, layouts and frames are byte-identical by construction. The VIS and
//! VIS+MV deferred stages write no `lit` and have no twin.
//!
//! `boyko_app::gpu_scene` creates every lit-producer module through [`LitProducer::spirv`]; the
//! `ibl_boot_selection` gate checks the truth table and that no boot site names a base accessor
//! directly.

use super::SpirvBlob;

embed_spirv! {
    /// `deferred_pbr.hlsl -D REFL=1` — the software resolve's REFL twin (env @20).
    DEFERRED_PBR_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_refl.comp.spv")
}
embed_spirv! {
    /// `deferred_pbr.hlsl -D TERMINATOR_WRAP=1 -D REFL=1` (env @20).
    DEFERRED_PBR_WRAP_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_wrap_refl.comp.spv")
}
embed_spirv! {
    /// `deferred_pbr.hlsl -T cs_6_5 -D HWRT=1 -D REFL=1` — RESOLVE_INLINE-hwrt (env @22).
    #[cfg(feature = "hwrt")]
    DEFERRED_PBR_HWRT_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_hwrt_refl.comp.spv")
}
embed_spirv! {
    /// `deferred_pbr.hlsl -T cs_6_5 -D HWRT=1 -D SHADOW_STAGE=2 -D REFL=1` — DENOISED (env @23).
    #[cfg(feature = "hwrt")]
    DEFERRED_PBR_HWRT_DENOISED_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/deferred_pbr_hwrt_denoised_refl.comp.spv")
}
embed_spirv! {
    /// `forward_opaque.fs.hlsl -D REFL=1` (env at Set 1 @4).
    FORWARD_OPAQUE_REFL_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/forward_opaque_refl.fs.spv")
}
embed_spirv! {
    /// `forward_opaque.fs.hlsl -D FROXEL=1 -D REFL=1` (env at Set 1 @4).
    FORWARD_OPAQUE_FROXEL_REFL_FS_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/forward_opaque_froxel_refl.fs.spv")
}
embed_spirv! {
    /// `sdf_forward_march.comp.hlsl -D HAS_MESH=1 -D REFL=1` (env at Set 1 @4).
    SDF_FORWARD_MARCH_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_forward_march_refl.comp.spv")
}
embed_spirv! {
    /// `sdf_forward_march.comp.hlsl -D REFL=1` (SDF-only; env at Set 1 @4).
    SDF_FORWARD_MARCH_SDFONLY_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_forward_march_sdfonly_refl.comp.spv")
}
embed_spirv! {
    /// `sdf_forward_march.comp.hlsl -D HAS_MESH=1 -D VIEWT=1 -D REFL=1` (env at Set 1 @4).
    SDF_FORWARD_MARCH_VIEWT_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_forward_march_viewt_refl.comp.spv")
}
embed_spirv! {
    /// `sdf_forward_march.comp.hlsl -D VIEWT=1 -D REFL=1` (env at Set 1 @4).
    SDF_FORWARD_MARCH_SDFONLY_VIEWT_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/sdf_forward_march_sdfonly_viewt_refl.comp.spv")
}
embed_spirv! {
    /// `vb_resolve.comp.hlsl -D REFL=1` (env at Set 1 @4).
    VB_RESOLVE_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_resolve_refl.comp.spv")
}
embed_spirv! {
    /// `vb_resolve.comp.hlsl -D FROXEL=1 -D REFL=1` (env at Set 1 @4).
    VB_RESOLVE_FROXEL_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_resolve_froxel_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade.comp.hlsl -D REFL=1` (env at Set 1 @4).
    VB_SHADE_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade.comp.hlsl -D FROXEL=1 -D REFL=1` (env at Set 1 @4).
    VB_SHADE_FROXEL_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_froxel_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade.comp.hlsl -D TEXTURED=1 -D REFL=1` (env at Set 1 @4).
    VB_SHADE_TEX_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_tex_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade.comp.hlsl -D TEXTURED=1 -D FROXEL=1 -D REFL=1` (env at Set 1 @4).
    VB_SHADE_TEX_FROXEL_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_tex_froxel_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade_split.comp.hlsl -D REFL=1` (env at Set 1 @9).
    VB_SHADE_SPLIT_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_split_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade_split.comp.hlsl -D TEXTURED=1 -D REFL=1` (env at Set 1 @9).
    VB_SHADE_SPLIT_TEX_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_split_tex_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade_split.comp.hlsl -D HWRT=1 -D REFL=1` (env at Set 1 @9).
    #[cfg(feature = "hwrt")]
    VB_SHADE_SPLIT_HWRT_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_split_hwrt_refl.comp.spv")
}
embed_spirv! {
    /// `vb_shade_split.comp.hlsl -D TEXTURED=1 -D HWRT=1 -D REFL=1` (env at Set 1 @9).
    #[cfg(feature = "hwrt")]
    VB_SHADE_SPLIT_TEX_HWRT_REFL_SPV,
    concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/vb_shade_split_tex_hwrt_refl.comp.spv")
}

/// One `lit`-writing producer variant — the 20 modules (16 on `not(hwrt)`) that have a
/// `-D REFL=1` twin (`docs/SHADER-VARIANT-MANIFEST.md`, update U6 §3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LitProducer {
    /// `deferred_pbr.comp` — the software resolve.
    DeferredPbr,
    /// `deferred_pbr_wrap.comp` — TERMINATOR_WRAP.
    DeferredPbrWrap,
    /// `deferred_pbr_hwrt.comp` — RESOLVE_INLINE-hwrt.
    #[cfg(feature = "hwrt")]
    DeferredPbrHwrt,
    /// `deferred_pbr_hwrt_denoised.comp` — DENOISED.
    #[cfg(feature = "hwrt")]
    DeferredPbrHwrtDenoised,
    /// `forward_opaque.fs`.
    ForwardOpaqueFs,
    /// `forward_opaque_froxel.fs`.
    ForwardOpaqueFroxelFs,
    /// `sdf_forward_march.comp` (`HAS_MESH`).
    SdfForwardMarch,
    /// `sdf_forward_march_sdfonly.comp`.
    SdfForwardMarchSdfonly,
    /// `sdf_forward_march_viewt.comp`.
    SdfForwardMarchViewt,
    /// `sdf_forward_march_sdfonly_viewt.comp`.
    SdfForwardMarchSdfonlyViewt,
    /// `vb_resolve.comp`.
    VbResolve,
    /// `vb_resolve_froxel.comp`.
    VbResolveFroxel,
    /// `vb_shade.comp`.
    VbShade,
    /// `vb_shade_froxel.comp`.
    VbShadeFroxel,
    /// `vb_shade_tex.comp`.
    VbShadeTex,
    /// `vb_shade_tex_froxel.comp`.
    VbShadeTexFroxel,
    /// `vb_shade_split.comp`.
    VbShadeSplit,
    /// `vb_shade_split_tex.comp`.
    VbShadeSplitTex,
    /// `vb_shade_split_hwrt.comp`.
    #[cfg(feature = "hwrt")]
    VbShadeSplitHwrt,
    /// `vb_shade_split_tex_hwrt.comp`.
    #[cfg(feature = "hwrt")]
    VbShadeSplitTexHwrt,
}

impl LitProducer {
    /// Every variant this build compiles, in manifest order.
    pub const ALL: &'static [LitProducer] = &[
        LitProducer::DeferredPbr,
        LitProducer::DeferredPbrWrap,
        #[cfg(feature = "hwrt")]
        LitProducer::DeferredPbrHwrt,
        #[cfg(feature = "hwrt")]
        LitProducer::DeferredPbrHwrtDenoised,
        LitProducer::ForwardOpaqueFs,
        LitProducer::ForwardOpaqueFroxelFs,
        LitProducer::SdfForwardMarch,
        LitProducer::SdfForwardMarchSdfonly,
        LitProducer::SdfForwardMarchViewt,
        LitProducer::SdfForwardMarchSdfonlyViewt,
        LitProducer::VbResolve,
        LitProducer::VbResolveFroxel,
        LitProducer::VbShade,
        LitProducer::VbShadeFroxel,
        LitProducer::VbShadeTex,
        LitProducer::VbShadeTexFroxel,
        LitProducer::VbShadeSplit,
        LitProducer::VbShadeSplitTex,
        #[cfg(feature = "hwrt")]
        LitProducer::VbShadeSplitHwrt,
        #[cfg(feature = "hwrt")]
        LitProducer::VbShadeSplitTexHwrt,
    ];

    /// The base (unarmed) module — today's `.spv`.
    #[must_use]
    pub fn base_spirv(self) -> &'static [u32] {
        match self {
            LitProducer::DeferredPbr => super::deferred_pbr_spirv(),
            LitProducer::DeferredPbrWrap => super::deferred_pbr_wrap_spirv(),
            #[cfg(feature = "hwrt")]
            LitProducer::DeferredPbrHwrt => super::deferred_pbr_hwrt_spirv(),
            #[cfg(feature = "hwrt")]
            LitProducer::DeferredPbrHwrtDenoised => super::deferred_pbr_denoised_spirv(),
            LitProducer::ForwardOpaqueFs => super::forward_opaque_fs_spirv(),
            LitProducer::ForwardOpaqueFroxelFs => super::forward_opaque_froxel_fs_spirv(),
            LitProducer::SdfForwardMarch => super::sdf_forward_march_spirv(),
            LitProducer::SdfForwardMarchSdfonly => super::sdf_forward_march_sdfonly_spirv(),
            LitProducer::SdfForwardMarchViewt => super::sdf_forward_march_viewt_spirv(),
            LitProducer::SdfForwardMarchSdfonlyViewt => super::sdf_forward_march_sdfonly_viewt_spirv(),
            LitProducer::VbResolve => super::vb_resolve_spirv(),
            LitProducer::VbResolveFroxel => super::vb_resolve_froxel_spirv(),
            LitProducer::VbShade => super::vb_shade_spirv(),
            LitProducer::VbShadeFroxel => super::vb_shade_froxel_spirv(),
            LitProducer::VbShadeTex => super::vb_shade_tex_spirv(),
            LitProducer::VbShadeTexFroxel => super::vb_shade_tex_froxel_spirv(),
            LitProducer::VbShadeSplit => super::vb_shade_split_spirv(),
            LitProducer::VbShadeSplitTex => super::vb_shade_split_tex_spirv(),
            #[cfg(feature = "hwrt")]
            LitProducer::VbShadeSplitHwrt => super::vb_shade_split_hwrt_spirv(),
            #[cfg(feature = "hwrt")]
            LitProducer::VbShadeSplitTexHwrt => super::vb_shade_split_tex_hwrt_spirv(),
        }
    }

    /// The `-D REFL=1` twin (armed boots).
    #[must_use]
    pub fn refl_spirv(self) -> &'static [u32] {
        match self {
            LitProducer::DeferredPbr => DEFERRED_PBR_REFL_SPV.as_words(),
            LitProducer::DeferredPbrWrap => DEFERRED_PBR_WRAP_REFL_SPV.as_words(),
            #[cfg(feature = "hwrt")]
            LitProducer::DeferredPbrHwrt => DEFERRED_PBR_HWRT_REFL_SPV.as_words(),
            #[cfg(feature = "hwrt")]
            LitProducer::DeferredPbrHwrtDenoised => DEFERRED_PBR_HWRT_DENOISED_REFL_SPV.as_words(),
            LitProducer::ForwardOpaqueFs => FORWARD_OPAQUE_REFL_FS_SPV.as_words(),
            LitProducer::ForwardOpaqueFroxelFs => FORWARD_OPAQUE_FROXEL_REFL_FS_SPV.as_words(),
            LitProducer::SdfForwardMarch => SDF_FORWARD_MARCH_REFL_SPV.as_words(),
            LitProducer::SdfForwardMarchSdfonly => SDF_FORWARD_MARCH_SDFONLY_REFL_SPV.as_words(),
            LitProducer::SdfForwardMarchViewt => SDF_FORWARD_MARCH_VIEWT_REFL_SPV.as_words(),
            LitProducer::SdfForwardMarchSdfonlyViewt => SDF_FORWARD_MARCH_SDFONLY_VIEWT_REFL_SPV.as_words(),
            LitProducer::VbResolve => VB_RESOLVE_REFL_SPV.as_words(),
            LitProducer::VbResolveFroxel => VB_RESOLVE_FROXEL_REFL_SPV.as_words(),
            LitProducer::VbShade => VB_SHADE_REFL_SPV.as_words(),
            LitProducer::VbShadeFroxel => VB_SHADE_FROXEL_REFL_SPV.as_words(),
            LitProducer::VbShadeTex => VB_SHADE_TEX_REFL_SPV.as_words(),
            LitProducer::VbShadeTexFroxel => VB_SHADE_TEX_FROXEL_REFL_SPV.as_words(),
            LitProducer::VbShadeSplit => VB_SHADE_SPLIT_REFL_SPV.as_words(),
            LitProducer::VbShadeSplitTex => VB_SHADE_SPLIT_TEX_REFL_SPV.as_words(),
            #[cfg(feature = "hwrt")]
            LitProducer::VbShadeSplitHwrt => VB_SHADE_SPLIT_HWRT_REFL_SPV.as_words(),
            #[cfg(feature = "hwrt")]
            LitProducer::VbShadeSplitTexHwrt => VB_SHADE_SPLIT_TEX_HWRT_REFL_SPV.as_words(),
        }
    }

    /// The module an armed (`env_armed`) or unarmed boot builds for this producer — the ONE
    /// selection site (`ibl_boot_selection`).
    #[inline]
    #[must_use]
    pub fn spirv(self, env_armed: bool) -> &'static [u32] {
        if env_armed { self.refl_spirv() } else { self.base_spirv() }
    }
}
