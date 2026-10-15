//! The `PostFinal` constant buffer's host mirror (render wave PXe).

use bytemuck::{Pod, Zeroable};

/// The `post_final` pass's `PostFinal` constant buffer — 32 B, std140, member for member
/// `boyko_shaderdsl::post::POST_FINAL_UBO_LAYOUT`, the table the generator prints the shader's
/// `cbuffer` from and the host oracle reads the receipt's words by (pinned by
/// `post_final_gpu_layout_is_post_final_ubo_layout`). PX1a fills it; nothing does at PXe.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PostFinalGpu {
    /// The mode word: the tonemapper at bits `TONEMAP_MODE_SHIFT..+4` — the field the light
    /// header's word 7 carries ([`crate::light::TONEMAP_MODE_SHIFT`]).
    pub mode_words: u32,
    /// `E / pre`, the exposure ratio — exactly 1.0 until PX3.
    pub exposure_ratio: f32,
    /// The dispatch extent the shader's guard reads.
    pub extent: [u32; 2],
    /// Reserved to the 32-byte std140 tail (PX3–PX6 grow into it).
    pub _r: [u32; 4],
}

const _: () = assert!(size_of::<PostFinalGpu>() == 32);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, mode_words) == 0);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, exposure_ratio) == 4);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, extent) == 8);
const _: () = assert!(core::mem::offset_of!(PostFinalGpu, _r) == 16);

#[cfg(test)]
mod tests {
    use boyko_shaderdsl::post::{
        POST_FINAL_UBO_BYTES, POST_FINAL_UBO_LAYOUT, PostUboKind, TONEMAP_ACES, TONEMAP_MODE_MASK,
        TONEMAP_MODE_SHIFT, TONEMAP_NEUTRAL, TONEMAP_REINHARD_JODIE,
    };

    use super::PostFinalGpu;
    use crate::light::{self, Tonemapper};

    /// The host struct's members as `(name, offset, size)`, in declaration order (the reserved
    /// tail as its four `_r<i>` words).
    fn host_members() -> Vec<(String, usize, usize)> {
        let mut m = vec![
            ("mode_words".to_string(), core::mem::offset_of!(PostFinalGpu, mode_words), 4),
            ("exposure_ratio".to_string(), core::mem::offset_of!(PostFinalGpu, exposure_ratio), 4),
            ("extent".to_string(), core::mem::offset_of!(PostFinalGpu, extent), 8),
        ];
        for i in 0..4 {
            m.push((format!("_r{i}"), core::mem::offset_of!(PostFinalGpu, _r) + 4 * i, 4));
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

    #[test]
    fn tonemap_word_agrees_with_the_light_header() {
        assert_eq!(TONEMAP_MODE_SHIFT, light::TONEMAP_MODE_SHIFT, "the tonemapper field's shift");
        assert_eq!(TONEMAP_MODE_MASK, light::TONEMAP_MODE_MASK, "the tonemapper field's mask");
        assert_eq!(TONEMAP_ACES, Tonemapper::Aces as u32, "ACES's word");
        assert_eq!(TONEMAP_NEUTRAL, Tonemapper::Neutral as u32, "Neutral's word");
        assert_eq!(TONEMAP_REINHARD_JODIE, Tonemapper::ReinhardJodie as u32, "Reinhard-Jodie's word");
    }
}
