//! The `PostFinal` constant buffer's host mirror (render wave PXe).

use bytemuck::{Pod, Zeroable};

/// Render wave PX1b (`px1b/cut.md` D-R8): bit 1 of [`PostFinalGpu::mode_words`] — `post_final` runs
/// its RCAS sharpen stage at [`PostFinalGpu::sharpness`]. Pinned equal to
/// `boyko_shaderdsl::post::POST_MODE_SHARPEN` by `sharpen_bit_agrees_with_the_edsl`.
pub const POST_MODE_SHARPEN: u32 = 1 << 1;

/// The `post_final` pass's `PostFinal` constant buffer — 32 B, std140, member for member
/// `boyko_shaderdsl::post::POST_FINAL_UBO_LAYOUT`, the table the generator prints the shader's
/// `cbuffer` from and the host oracle reads the receipt's words by (pinned by
/// `post_final_gpu_layout_is_post_final_ubo_layout`). PX1a fills it; nothing does at PXe.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PostFinalGpu {
    /// The mode word: the tonemapper at bits `TONEMAP_MODE_SHIFT..+4` — the field the light
    /// header's word 7 carries ([`crate::light::TONEMAP_MODE_SHIFT`]) — and, since render wave
    /// PX1b, the sharpen bit [`POST_MODE_SHARPEN`].
    pub mode_words: u32,
    /// `E / pre`, the exposure ratio — exactly 1.0 until PX3.
    pub exposure_ratio: f32,
    /// The dispatch extent the shader's guard (and, since render wave PX1b, RCAS's border clamp)
    /// reads.
    pub extent: [u32; 2],
    /// Render wave PX1b: RCAS's `con`, the linear lobe scale in `[0.25, 1]`
    /// (`TaaConfig::rcas_con`), read only under [`POST_MODE_SHARPEN`]; `0.0` when the bit is clear.
    pub sharpness: f32,
    /// Reserved to the 32-byte std140 tail (PX3–PX6 grow into it).
    pub _r: [u32; 3],
}

const _: () = assert!(size_of::<PostFinalGpu>() == 32);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, mode_words) == 0);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, exposure_ratio) == 4);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, extent) == 8);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, sharpness) == 16);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, _r) == 20);

#[cfg(test)]
mod tests {
    use boyko_shaderdsl::post::{
        POST_FINAL_UBO_BYTES, POST_FINAL_UBO_LAYOUT, PostUboKind, TONEMAP_ACES, TONEMAP_AGX,
        TONEMAP_MODE_MASK, TONEMAP_MODE_SHIFT, TONEMAP_NEUTRAL, TONEMAP_REINHARD_JODIE,
    };

    use super::{POST_MODE_SHARPEN, PostFinalGpu};
    use crate::light::{self, Tonemapper};

    /// The host struct's members as `(name, offset, size)`, in declaration order (the reserved
    /// tail as its three `_r<i>` words, `_r1.._r3` — render wave PX1b took `_r0` for `sharpness`).
    fn host_members() -> Vec<(String, usize, usize)> {
        let mut m = vec![
            ("mode_words".to_string(), core::mem::offset_of!(PostFinalGpu, mode_words), 4),
            ("exposure_ratio".to_string(), core::mem::offset_of!(PostFinalGpu, exposure_ratio), 4),
            ("extent".to_string(), core::mem::offset_of!(PostFinalGpu, extent), 8),
            ("sharpness".to_string(), core::mem::offset_of!(PostFinalGpu, sharpness), 4),
        ];
        for i in 0..3 {
            m.push((format!("_r{}", i + 1), core::mem::offset_of!(PostFinalGpu, _r) + 4 * i, 4));
        }
        m
    }

    #[test]
    fn post_final_gpu_layout_is_post_final_ubo_layout() {
        let table: Vec<(String, usize, usize)> = POST_FINAL_UBO_LAYOUT
            .iter()
            .map(|f| {
                let size = match f.kind {
                    PostUboKind::U32 | PostUboKind::F32 => 4,
                    PostUboKind::Uint2 => 8,
                };
                (f.name.to_string(), f.offset as usize, size)
            })
            .collect();
        assert_eq!(host_members(), table, "PostFinalGpu is not POST_FINAL_UBO_LAYOUT member for member");
        assert_eq!(size_of::<PostFinalGpu>(), POST_FINAL_UBO_BYTES as usize, "PostFinalGpu's size");
    }

    /// Every `Tonemapper` variant's wire value is the eDSL word its dispatch reads. The `match` is
    /// exhaustive, so a variant without an eDSL word fails to compile here (render wave PXt).
    #[test]
    fn tonemap_word_agrees_with_the_light_header() {
        assert_eq!(TONEMAP_MODE_SHIFT, light::TONEMAP_MODE_SHIFT, "the tonemapper field's shift");
        assert_eq!(TONEMAP_MODE_MASK, light::TONEMAP_MODE_MASK, "the tonemapper field's mask");
        for curve in [Tonemapper::Aces, Tonemapper::Neutral, Tonemapper::ReinhardJodie, Tonemapper::AgX] {
            let word = match curve {
                Tonemapper::Aces => TONEMAP_ACES,
                Tonemapper::Neutral => TONEMAP_NEUTRAL,
                Tonemapper::ReinhardJodie => TONEMAP_REINHARD_JODIE,
                Tonemapper::AgX => TONEMAP_AGX,
            };
            assert_eq!(word, curve as u32, "{curve:?}'s word");
        }
    }

    /// Render wave PX1b (D-R8): the host's sharpen bit is the eDSL's, and it stays clear of the
    /// tonemapper field.
    #[test]
    fn sharpen_bit_agrees_with_the_edsl() {
        assert_eq!(POST_MODE_SHARPEN, boyko_shaderdsl::post::POST_MODE_SHARPEN, "the sharpen bit drifted from the eDSL's");
        assert_eq!(POST_MODE_SHARPEN & (TONEMAP_MODE_MASK << TONEMAP_MODE_SHIFT), 0, "the sharpen bit overlaps the tonemapper field");
    }
}
