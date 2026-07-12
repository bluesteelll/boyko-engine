//! [`PngTextureLoader`] — the in-house PNG texture loader (textured-PBR campaign
//! rung T2), mirroring [`ObjMeshLoader`](crate::loaders::ObjMeshLoader).

use boyko_ecs::ecs::core::asset::{Asset, AssetError, AssetLoader};
use boyko_image::{DecodedImage, decode_png};

use crate::texture::{ColorSpace, TextureGpu};
use crate::texture_data::TextureData;

/// Loads a PNG file into a [`TextureData`] CPU intermediate via
/// [`boyko_image::decode_png`] (the in-house, zero-third-party-dependency PNG
/// decoder, textured-PBR rung T1).
///
/// [`TextureData::color_space`] is set to its default ([`ColorSpace::Srgb`]) — the
/// loader itself has no per-material-slot context; a later rung (T5) picks the
/// color space at the call site the loader is invoked from and overrides this field.
pub struct PngTextureLoader;

impl AssetLoader for PngTextureLoader {
    type Out = TextureGpu;

    const EXTENSIONS: &'static [&'static str] = &["png"];

    fn decode(bytes: &[u8]) -> Result<<Self::Out as Asset>::Cpu, AssetError> {
        let image = decode_png(bytes).map_err(|e| decode_error(e.to_string()))?;
        let width = image.width;
        let height = image.height;
        let rgba8 = narrow_to_rgba8(image);
        Ok(TextureData {
            width,
            height,
            rgba8,
            color_space: ColorSpace::default(),
        })
    }
}

/// Narrows a decoded PNG's RGBA samples to tightly-packed 8-bit-per-channel bytes.
/// `bit_depth == 8` moves `pixels` out unchanged (zero-copy — it is already exactly
/// `width * height * 4` bytes); `bit_depth == 16` keeps only the high byte of each
/// big-endian 16-bit sample (the standard bit-depth-reduction rule).
fn narrow_to_rgba8(image: DecodedImage) -> Vec<u8> {
    debug_assert_eq!(
        image.channels, 4,
        "invariant: decode_png always expands to 4 (RGBA) channels"
    );
    match image.bit_depth {
        8 => image.pixels,
        16 => {
            let mut out = Vec::with_capacity(image.pixels.len() / 2);
            for sample in image.pixels.chunks_exact(2) {
                // Big-endian u16: `sample[0]` is the most-significant byte.
                out.push(sample[0]);
            }
            out
        }
        other => unreachable!(
            "invariant: decode_png only ever returns bit_depth 8 or 16, got {other}"
        ),
    }
}

/// Builds an [`AssetError::Decode`] for a malformed PNG file — the loader's sole
/// error path (mirrors [`ObjMeshLoader`](crate::loaders::ObjMeshLoader)'s
/// `decode_error`).
#[cold]
#[inline(never)]
fn decode_error(msg: String) -> AssetError {
    AssetError::Decode(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // Test-only PNG/DEFLATE construction (mirrors `boyko_image::png`'s own private
    // test helpers, hand-assembled independently per the RFC/PNG spec text — a
    // decoder bug is not masked by testing it against itself). Chunk CRC-32 and the
    // zlib Adler-32 trailer are dummy/zero: both are non-fatal mismatches
    // (`boyko_image::decode_png`'s documented tolerance), so a fixture needs no
    // checksum implementation.
    // -----------------------------------------------------------------

    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + data.len() + 4);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // dummy CRC-32 (tolerated mismatch)
        out
    }

    fn ihdr_chunk(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
        let mut d = Vec::with_capacity(13);
        d.extend_from_slice(&width.to_be_bytes());
        d.extend_from_slice(&height.to_be_bytes());
        d.push(bit_depth);
        d.push(color_type);
        d.push(0); // compression method
        d.push(0); // filter method
        d.push(0); // interlace method (none)
        chunk(b"IHDR", &d)
    }

    /// Wraps a single STORED (uncompressed) DEFLATE block containing `raw` in RFC
    /// 1950 zlib framing, with a dummy Adler-32 trailer.
    fn zlib_stored(raw: &[u8]) -> Vec<u8> {
        assert!(raw.len() <= u16::MAX as usize, "test fixture too large for one stored block");
        let mut out = vec![0x78, 0x01]; // CMF=8(deflate)/CINFO=7, FLG s.t. header % 31 == 0
        out.push(0b0000_0001); // BFINAL=1, BTYPE=00 (stored)
        let len = raw.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(raw);
        out.extend_from_slice(&[0, 0, 0, 0]); // dummy Adler-32 (tolerated mismatch)
        out
    }

    /// Assembles a minimal, well-formed truecolor+alpha (color type 6, 8-bit) PNG of
    /// `width`x`height`, filled with `rgba` repeated across every texel (filter type
    /// 0 / "None" per scanline).
    fn build_rgba8_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let row_bytes = width as usize * 4;
        let mut raw_scanlines = Vec::with_capacity(height as usize * (1 + row_bytes));
        for _ in 0..height {
            raw_scanlines.extend_from_slice(&[0u8]); // filter type 0 (None)
            for _ in 0..width {
                raw_scanlines.extend_from_slice(&rgba);
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&PNG_SIGNATURE);
        out.extend_from_slice(&ihdr_chunk(width, height, 8, 6));
        out.extend_from_slice(&chunk(b"IDAT", &zlib_stored(&raw_scanlines)));
        out.extend_from_slice(&chunk(b"IEND", &[]));
        out
    }

    #[test]
    fn decode_solid_color_png_matches_dimensions_and_pixels() {
        let bytes = build_rgba8_png(4, 3, [0x11, 0x22, 0x33, 0xFF]);
        let data = PngTextureLoader::decode(&bytes).expect("a well-formed PNG must decode");

        assert_eq!(data.width, 4);
        assert_eq!(data.height, 3);
        assert_eq!(data.rgba8.len(), 4 * 3 * 4, "tightly-packed RGBA8");
        assert!(
            data.rgba8.chunks_exact(4).all(|px| px == [0x11, 0x22, 0x33, 0xFF]),
            "every texel must decode to the fill color"
        );
    }

    #[test]
    fn decode_defaults_color_space_to_srgb() {
        let bytes = build_rgba8_png(1, 1, [0, 0, 0, 0]);
        let data = PngTextureLoader::decode(&bytes).expect("a well-formed PNG must decode");
        assert_eq!(data.color_space, ColorSpace::Srgb);
    }

    #[test]
    fn decode_malformed_bytes_is_decode_error() {
        let result = PngTextureLoader::decode(b"not a png");
        assert!(matches!(result, Err(AssetError::Decode(_))), "got {result:?}");
    }
}
