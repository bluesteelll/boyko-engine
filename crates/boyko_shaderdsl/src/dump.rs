//! Render wave PX0 (`px0/cut.md` §1.3 #3) — **the OBSD dump file**: the 32-byte header
//! `boyko_app`'s image dump writes (`boyko_app/src/image_dump.rs`), its parse, and the contract the
//! dumps born later must keep, so `post_eval` gains a binding kind by a writer, never by an edit.
//!
//! # The file
//!
//! Eight little-endian `u32`s — `"OBSD"`, `format`, `width`, `height`, `mip`, `layers`, `frame`,
//! `texel_bytes` — then exactly `width · height · layers · texel_bytes` bytes of texels, row-major,
//! layer after layer.
//!
//! # The contract for the dumps born later (W12 item 5)
//!
//! - **A buffer dump** (PX3's `exposure_state`, V10): `format = 0` ([`FORMAT_BUFFER`]),
//!   `width` = the byte length, `height = 1`, `layers = 1`, `texel_bytes = 1`.
//! - **A 3D image dump** (V2's `fog_integrated`, PX5's `post_lut`): `layers` = the depth.
//!
//! `no_std`, no allocation: a parse borrows the texels from the caller's bytes.

use core::fmt;

use crate::pixfmt::{FORMAT_BUFFER, texel_bytes};

/// The header's first word, `"OBSD"` read little-endian.
pub const DUMP_MAGIC: u32 = u32::from_le_bytes(*b"OBSD");
/// The header's size.
pub const HEADER_BYTES: usize = 32;

/// A dump's header, as the recorder stamped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DumpHeader {
    /// The VkFormat code (or [`FORMAT_BUFFER`]).
    pub format: u32,
    /// Width in texels (a buffer dump: its byte length).
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// The mip level the copy read.
    pub mip: u32,
    /// Array layers (a 3D image dump: its depth).
    pub layers: u32,
    /// The engine frame index the recording frame carried.
    pub frame: u32,
    /// Bytes per texel, as the recorder used them.
    pub texel_bytes: u32,
}

/// Why a dump does not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DumpError {
    /// Fewer bytes than the header.
    Short {
        /// The file's length.
        len: usize,
    },
    /// The first word is not `"OBSD"`.
    BadMagic(u32),
    /// A `format` this crate does not decode.
    UnknownFormat(u32),
    /// The header's texel size contradicts the format's.
    TexelBytes {
        /// The format.
        format: u32,
        /// What the header says.
        header: u32,
        /// What the format is.
        known: u32,
    },
    /// The texel bytes are not `width · height · layers · texel_bytes`.
    Length {
        /// What the header implies.
        want: u64,
        /// What the file holds after the header.
        got: u64,
    },
    /// A buffer dump whose shape breaks the buffer contract (`height = layers = texel_bytes = 1`).
    BufferShape,
}

impl fmt::Display for DumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            DumpError::Short { len } => write!(f, "{len} bytes is shorter than the {HEADER_BYTES}-byte header"),
            DumpError::BadMagic(m) => write!(f, "bad magic {m:#x}"),
            DumpError::UnknownFormat(v) => write!(f, "VkFormat {v} has no decoder"),
            DumpError::TexelBytes { format, header, known } => {
                write!(f, "VkFormat {format} is {known} B/texel, the header says {header}")
            }
            DumpError::Length { want, got } => {
                write!(f, "the header needs {want} texel bytes, the file holds {got}")
            }
            DumpError::BufferShape => {
                write!(f, "a buffer dump must have height = layers = texel_bytes = 1")
            }
        }
    }
}

impl DumpHeader {
    /// The texel count the header implies (`width · height · layers`).
    #[must_use]
    pub const fn texel_count(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.layers as u64
    }

    /// The 32 header bytes the recorder writes for this header (the fixtures' writer).
    #[must_use]
    pub fn to_bytes(&self) -> [u8; HEADER_BYTES] {
        let words = [
            DUMP_MAGIC,
            self.format,
            self.width,
            self.height,
            self.mip,
            self.layers,
            self.frame,
            self.texel_bytes,
        ];
        let mut out = [0u8; HEADER_BYTES];
        for (i, w) in words.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    /// The bytes of texel `(x, y)` of `layer`, or `None` outside the extent.
    #[must_use]
    pub fn texel<'a>(&self, texels: &'a [u8], x: u32, y: u32, layer: u32) -> Option<&'a [u8]> {
        if x >= self.width || y >= self.height || layer >= self.layers {
            return None;
        }
        let tb = self.texel_bytes as usize;
        let i = ((layer as usize * self.height as usize + y as usize) * self.width as usize + x as usize) * tb;
        texels.get(i..i + tb)
    }
}

/// Parses a dump's bytes into its header and its texel bytes.
///
/// # Errors
/// A short file, a bad magic, an unknown format, a texel size the format contradicts, a texel
/// length that is not `width · height · layers · texel_bytes`, or a buffer dump off its contract.
pub fn parse(bytes: &[u8]) -> Result<(DumpHeader, &[u8]), DumpError> {
    if bytes.len() < HEADER_BYTES {
        return Err(DumpError::Short { len: bytes.len() });
    }
    let w = |i: usize| u32::from_le_bytes([bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3]]);
    if w(0) != DUMP_MAGIC {
        return Err(DumpError::BadMagic(w(0)));
    }
    let h = DumpHeader {
        format: w(1),
        width: w(2),
        height: w(3),
        mip: w(4),
        layers: w(5),
        frame: w(6),
        texel_bytes: w(7),
    };
    let known = texel_bytes(h.format).ok_or(DumpError::UnknownFormat(h.format))?;
    if known != h.texel_bytes {
        return Err(DumpError::TexelBytes { format: h.format, header: h.texel_bytes, known });
    }
    if h.format == FORMAT_BUFFER && (h.height != 1 || h.layers != 1) {
        return Err(DumpError::BufferShape);
    }
    let texels = &bytes[HEADER_BYTES..];
    let want = h.texel_count() * u64::from(h.texel_bytes);
    if texels.len() as u64 != want {
        return Err(DumpError::Length { want, got: texels.len() as u64 });
    }
    Ok((h, texels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixfmt::{VK_FORMAT_B10G11R11_UFLOAT_PACK32, VK_FORMAT_R8G8B8A8_UNORM};

    fn file(h: DumpHeader, texels: &[u8]) -> Vec<u8> {
        let mut v = h.to_bytes().to_vec();
        v.extend_from_slice(texels);
        v
    }

    fn rgba8(w: u32, hgt: u32, layers: u32) -> DumpHeader {
        DumpHeader { format: VK_FORMAT_R8G8B8A8_UNORM, width: w, height: hgt, mip: 0, layers, frame: 7, texel_bytes: 4 }
    }

    #[test]
    fn a_well_formed_dump_parses_and_indexes_layers() {
        let h = rgba8(2, 1, 2);
        let tex: Vec<u8> = (0..16).collect();
        let bytes = file(h, &tex);
        let (got, t) = parse(&bytes).expect("well formed");
        assert_eq!(got, h);
        assert_eq!(h.texel(t, 1, 0, 0), Some(&tex[4..8]));
        assert_eq!(h.texel(t, 0, 0, 1), Some(&tex[8..12]));
        assert_eq!(h.texel(t, 2, 0, 0), None);
        assert_eq!(h.texel(t, 0, 0, 2), None);
    }

    #[test]
    fn each_malformation_is_its_named_error() {
        assert_eq!(parse(&[0u8; 16]), Err(DumpError::Short { len: 16 }));
        let mut bad = file(rgba8(1, 1, 1), &[0; 4]);
        bad[0] = b'X';
        assert!(matches!(parse(&bad), Err(DumpError::BadMagic(_))));
        let unknown = DumpHeader { format: 999, ..rgba8(1, 1, 1) };
        assert_eq!(parse(&file(unknown, &[0; 4])), Err(DumpError::UnknownFormat(999)));
        let lying = DumpHeader { texel_bytes: 8, ..rgba8(1, 1, 1) };
        assert!(matches!(parse(&file(lying, &[0; 8])), Err(DumpError::TexelBytes { .. })));
        // `layers` counts: a 2-layer header over one layer's bytes is short.
        assert_eq!(parse(&file(rgba8(2, 2, 2), &[0; 16])), Err(DumpError::Length { want: 32, got: 16 }));
        assert_eq!(parse(&file(rgba8(2, 2, 1), &[0; 12])), Err(DumpError::Length { want: 16, got: 12 }));
    }

    #[test]
    fn the_buffer_contract() {
        let buf = DumpHeader { format: FORMAT_BUFFER, width: 12, height: 1, mip: 0, layers: 1, frame: 0, texel_bytes: 1 };
        let bytes = file(buf, &[9; 12]);
        let (h, t) = parse(&bytes).expect("a buffer dump");
        assert_eq!((h.width, t.len()), (12, 12));
        let tall = DumpHeader { height: 2, width: 6, ..buf };
        assert_eq!(parse(&file(tall, &[0; 12])), Err(DumpError::BufferShape));
        let wide = DumpHeader { texel_bytes: 4, width: 3, ..buf };
        assert!(matches!(parse(&file(wide, &[0; 12])), Err(DumpError::TexelBytes { .. })));
    }

    #[test]
    fn a_b10g11r11_dump_parses() {
        let h = DumpHeader { format: VK_FORMAT_B10G11R11_UFLOAT_PACK32, ..rgba8(1, 1, 1) };
        assert!(parse(&file(h, &[0; 4])).is_ok());
    }
}
