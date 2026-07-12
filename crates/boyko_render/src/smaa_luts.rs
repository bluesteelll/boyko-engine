//! SMAA 1x (AA campaign, Stage 2) — the two precomputed LUT textures the blending-weight
//! calculation pass (`shaders/smaa_weight.fs.hlsl`) samples: `AreaTex` (crossing-edge distance
//! → coverage area) and `SearchTex` (edge-run search-length lookup).
//!
//! ```text
//! SMAA Area & Search lookup textures (AreaTex.bin, SearchTex.bin)
//! ================================================================
//!
//! These two binary lookup tables are the canonical precomputed SMAA textures, extracted
//! byte-for-byte from the reference implementation:
//!
//!     https://github.com/iryoku/smaa  (Textures/AreaTex.h, Textures/SearchTex.h)
//!
//!   AreaTex.bin   — 160 x 560, R8G8 (VK_FORMAT_R8G8_UNORM),   179200 bytes
//!   SearchTex.bin —  64 x  16, R8   (VK_FORMAT_R8_UNORM),       1024 bytes
//!
//! Copyright (C) 2013 Jorge Jimenez, Jose I. Echevarria, Belen Masia, Fernando Navarro,
//! Diego Gutierrez.
//!
//! Released under the MIT license. The MIT license clarification in the SMAA repository
//! states the attribution notice is required in source distributions (which this doc
//! comment + the sibling `assets/smaa/NOTICE` file satisfy) but not in binary/compiled
//! distributions.
//!
//! The paper: "SMAA: Enhanced Subpixel Morphological Antialiasing", Computer Graphics Forum
//! (Proc. EUROGRAPHICS 2012), 31(2), 2012.
//! ```
//!
//! # No Y-flip (the one silent-wrong-output risk)
//!
//! The bytes below are the RAW extracted header payload — no row-flip, no shader-side
//! V-flip. Vulkan shares D3D's top-left texel origin; iryoku's OpenGL integration guide flip
//! is OpenGL-only and does not apply here.

/// `AreaTex`'s width in texels.
pub const AREA_TEX_W: u32 = 160;
/// `AreaTex`'s height in texels.
pub const AREA_TEX_H: u32 = 560;
/// `SearchTex`'s width in texels.
pub const SEARCH_TEX_W: u32 = 64;
/// `SearchTex`'s height in texels.
pub const SEARCH_TEX_H: u32 = 16;

/// The raw `AreaTex` payload (`AREA_TEX_W * AREA_TEX_H * 2` bytes, R8G8, row-major,
/// no Y-flip). Uploaded via [`crate::upload_texture_2d_raw`] at boot.
pub static AREA_TEX_BYTES: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/smaa/AreaTex.bin"));
/// The raw `SearchTex` payload (`SEARCH_TEX_W * SEARCH_TEX_H` bytes, R8, row-major, no
/// Y-flip). Uploaded via [`crate::upload_texture_2d_raw`] at boot.
pub static SEARCH_TEX_BYTES: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/smaa/SearchTex.bin"));

/// SHA-256 pin of [`AREA_TEX_BYTES`] (the W3 integrity gate) — computed from the committed
/// `assets/smaa/AreaTex.bin` and asserted equal in the unit tests below, so a corrupted or
/// re-extracted-wrong binary fails the build's test suite instead of shipping silently.
pub const AREA_TEX_SHA256: &str =
    "35065cef2a02cabcad711d6bf430239ae64e27d71c4e4fa06f29cce2c992f0d2";
/// SHA-256 pin of [`SEARCH_TEX_BYTES`] — see [`AREA_TEX_SHA256`].
pub const SEARCH_TEX_SHA256: &str =
    "3694eae5e9d44b8ebb4415a13f8c7b94dc08a2fc86658434d771c4610fe5744d";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_tex_has_the_documented_length_and_leading_anchor() {
        assert_eq!(AREA_TEX_BYTES.len(), 179_200, "160 * 560 * 2 (R8G8)");
        assert_eq!(&AREA_TEX_BYTES[0..12], &[0u8; 12], "the canonical header's leading bytes");
    }

    #[test]
    fn search_tex_has_the_documented_length_and_byte_offset_anchors() {
        assert_eq!(SEARCH_TEX_BYTES.len(), 1_024, "64 * 16 (R8)");
        assert_eq!(
            &SEARCH_TEX_BYTES[0..8],
            &[0xFE, 0xFE, 0x00, 0x7F, 0x7F, 0x00, 0x00, 0xFE],
            "the canonical header's leading bytes"
        );
        assert_eq!(&SEARCH_TEX_BYTES[1016..1024], &[0u8; 8], "the canonical header's trailing bytes");
        assert!(
            SEARCH_TEX_BYTES.iter().all(|b| matches!(b, 0x00 | 0x7F | 0xFE)),
            "SearchTex is a packed 3-level (0/0.5/1.0) lookup — every byte must be one of \
             {{0x00, 0x7F, 0xFE}}"
        );
    }

    #[test]
    fn area_tex_sha256_matches_the_pin() {
        assert_eq!(sha256_hex(AREA_TEX_BYTES), AREA_TEX_SHA256);
    }

    #[test]
    fn search_tex_sha256_matches_the_pin() {
        assert_eq!(sha256_hex(SEARCH_TEX_BYTES), SEARCH_TEX_SHA256);
    }

    // ---- A tiny, test-only, dependency-free SHA-256 (FIPS 180-4) --------------------------
    // No `sha2` crate exists in this workspace and this digest is used ONLY by the two
    // regression pins above (never on the hot path, never in a `pub` surface) — a small
    // from-scratch implementation is the Principle-5-compliant choice over a new dependency.

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    fn sha256_hex(data: &[u8]) -> String {
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];

        // Pad: 0x80, zeros, then the 64-bit BIG-ENDIAN bit length, to a multiple of 64 bytes.
        let bit_len = (data.len() as u64) * 8;
        let mut msg = data.to_vec();
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in msg.chunks_exact(64) {
            let mut w = [0u32; 64];
            for (i, word) in w.iter_mut().enumerate().take(16) {
                let b = i * 4;
                *word = u32::from_be_bytes([chunk[b], chunk[b + 1], chunk[b + 2], chunk[b + 3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let temp1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = s0.wrapping_add(maj);

                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }

            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        h.iter().map(|word| format!("{word:08x}")).collect()
    }

    #[test]
    fn sha256_self_test_matches_the_known_empty_string_digest() {
        // FIPS 180-4 test vector: SHA-256("") — verifies the from-scratch implementation
        // itself before trusting it to gate the LUT bytes above.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
