//! [`PngError`] — the domain error type for PNG decode failures.
//!
//! Mirrors `boyko_ecs::ecs::core::asset::error::AssetError`'s convention: a
//! `#[non_exhaustive]` enum with `Display` + `std::error::Error` impls, never
//! `anyhow` (this is library code, not a bin/main).

/// Failures that can arise while decoding a PNG byte stream.
///
/// `#[non_exhaustive]`: new variants may be added in minor versions without
/// breaking downstream exhaustive matches.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngError {
    /// The first 8 bytes are not the PNG signature
    /// (`89 50 4E 47 0D 0A 1A 0A`), or the stream is shorter than 8 bytes.
    Signature,
    /// `IHDR`'s color type is not one of the supported types (0 grayscale,
    /// 2 truecolor, 4 grayscale+alpha, 6 truecolor+alpha). Color type 3
    /// (palette) is deliberately rejected — see the crate-level scope note.
    UnsupportedColorType(u8),
    /// `IHDR`'s bit depth is not legal for the given color type, or is a
    /// sub-byte depth (1/2/4) — only 8 and 16 are supported.
    UnsupportedBitDepth {
        /// The color type the bit depth was paired with.
        color_type: u8,
        /// The rejected bit depth.
        bit_depth: u8,
    },
    /// `IHDR`'s interlace method is Adam7 (method 1). Only method 0
    /// (no interlace) is supported — see the crate-level scope note.
    UnsupportedInterlace,
    /// A chunk's declared length runs past the end of the buffer, a chunk
    /// type is malformed, or a mandatory chunk (`IHDR`, `IDAT`) is missing
    /// or out of order. Carries a short static reason.
    BadChunk(&'static str),
    /// The byte stream ended before a length-prefixed field/chunk/block
    /// could be fully read.
    Truncated,
    /// A palette (`PLTE`, color type 3) PNG was rejected — out of the
    /// supported-scope color types (see the crate-level scope note).
    PaletteUnsupported,
    /// The DEFLATE/zlib stream is structurally invalid (bad block type,
    /// over-subscribed Huffman code, invalid back-reference distance, bad
    /// zlib header, unsupported preset dictionary, ...). Carries a short
    /// static reason.
    InflateError(&'static str),
    /// `width * height * channels * (bit_depth / 8)` (or an intermediate
    /// per-row computation) would overflow `usize`, or exceeds the crate's
    /// sanity ceiling — a decompression-bomb / malformed-header guard.
    DimensionOverflow,
}

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PngError::Signature => write!(f, "not a PNG file (bad or truncated signature)"),
            PngError::UnsupportedColorType(ct) => {
                write!(f, "unsupported PNG color type {ct} (only 0/2/4/6 are supported)")
            }
            PngError::UnsupportedBitDepth { color_type, bit_depth } => write!(
                f,
                "unsupported bit depth {bit_depth} for color type {color_type} (only 8/16 are supported)"
            ),
            PngError::UnsupportedInterlace => {
                write!(f, "Adam7 interlacing is not supported (interlace method 1)")
            }
            PngError::BadChunk(reason) => write!(f, "malformed PNG chunk: {reason}"),
            PngError::Truncated => write!(f, "truncated PNG byte stream"),
            PngError::PaletteUnsupported => {
                write!(f, "palette PNGs (color type 3) are not supported")
            }
            PngError::InflateError(reason) => write!(f, "DEFLATE/zlib stream error: {reason}"),
            PngError::DimensionOverflow => {
                write!(f, "image dimensions overflow or exceed the decode size ceiling")
            }
        }
    }
}

impl std::error::Error for PngError {}
