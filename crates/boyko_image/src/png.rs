//! PNG container: signature/chunk framing, `IHDR` validation, per-scanline
//! unfiltering, and RGBA pixel expansion. Delegates decompression to
//! [`crate::inflate`].
//!
//! # Ancillary chunks
//!
//! `gAMA`, `sRGB`, `cHRM`, `iCCP`, text chunks, etc. are parsed only far
//! enough to skip them (length-prefixed, so no chunk-specific knowledge is
//! needed). Color-space interpretation is deliberately left to the CALLER
//! (the material slot that consumes a decoded texture decides gamma/color
//! space); this decoder hands back raw sample bytes only.

use boyko_log::codes::W2601;

use crate::error::PngError;
use crate::inflate;

/// The 8-byte PNG signature every conforming file starts with (PNG spec
/// §5.2): a mix of a high-bit byte (detects 7-bit transport truncation), the
/// ASCII "PNG", a CRLF/LF pair (detects line-ending translation), and a
/// Ctrl-Z (detects DOS text-mode truncation at EOF).
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Decompression-bomb / malformed-header guard: reject an `IHDR` whose
/// declared dimensions would decode to more than this many bytes of RGBA
/// pixel data. 256 MiB comfortably covers any real PBR texture (e.g. a
/// 8192x8192 RGBA16 albedo map is 1 GiB... texture sets this large are
/// authored as compressed GPU formats, not raw PNG, so this ceiling is
/// deliberately generous for legitimate assets while still rejecting a
/// tiny file that claims an absurd resolution).
const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;

/// A fully-decoded PNG image: dimensions plus tightly-packed, row-major RGBA
/// pixel data.
///
/// # Channel / bit-depth expansion
///
/// `channels` is always `4` (RGBA) — grayscale is replicated into R/G/B and
/// an opaque alpha is synthesized when the source has none (color types 0,
/// 2); truecolor+alpha (type 6) passes through with zero extra copies.
///
/// `bit_depth` mirrors the source: `8` means one byte per RGBA channel (4
/// bytes/pixel); `16` means two bytes per channel (8 bytes/pixel), and those
/// two-byte samples are **big-endian**, exactly as PNG itself encodes them
/// (RFC — PNG spec §7.2) — this decoder does NOT byte-swap to a
/// little-endian/native representation. Callers that need native-endian
/// `u16` samples convert with `u16::from_be_bytes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Always `4` (RGBA) after expansion — see the struct-level doc.
    pub channels: u8,
    /// `8` or `16` bits per RGBA channel — see the struct-level doc.
    pub bit_depth: u8,
    /// Row-major, tightly-packed RGBA pixel data
    /// (`width * height * channels * (bit_depth / 8)` bytes).
    pub pixels: Vec<u8>,
}

/// Decodes a PNG byte stream into RGBA pixel data.
///
/// Supported: color types 0 (grayscale), 2 (truecolor), 4 (grayscale+alpha),
/// 6 (truecolor+alpha); bit depths 8 and 16; no interlacing. Rejected with a
/// specific [`PngError`]: color type 3 (palette), Adam7 interlacing
/// (interlace method 1), sub-byte bit depths (1/2/4), and any other
/// structurally malformed input.
///
/// A per-chunk CRC-32 mismatch is logged and does not fail the decode (PNG
/// files with a stale/incorrect CRC but otherwise-valid pixel data still
/// decode); a malformed/truncated DEFLATE stream, chunk framing, or `IHDR`
/// is always a hard error.
pub fn decode_png(bytes: &[u8]) -> Result<DecodedImage, PngError> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(PngError::Signature);
    }

    let mut ihdr: Option<IhdrInfo> = None;
    let mut idat_chunks: Vec<&[u8]> = Vec::new();
    let mut seen_iend = false;

    let mut pos = PNG_SIGNATURE.len();
    while pos < bytes.len() {
        let chunk = read_chunk(bytes, &mut pos)?;
        match &chunk.kind {
            b"IHDR" => {
                if ihdr.is_some() {
                    return Err(PngError::BadChunk("duplicate IHDR chunk"));
                }
                ihdr = Some(parse_ihdr(chunk.data)?);
            }
            b"IDAT" => {
                if ihdr.is_none() {
                    return Err(PngError::BadChunk("IDAT chunk before IHDR"));
                }
                idat_chunks.push(chunk.data);
            }
            b"IEND" => {
                seen_iend = true;
                break;
            }
            // Ancillary chunks (gAMA, sRGB, tEXt, ...) and PLTE on a
            // non-palette color type carry no information this decoder
            // needs — skip.
            _ => {}
        }
    }

    let info = ihdr.ok_or(PngError::BadChunk("missing IHDR chunk"))?;
    if !seen_iend {
        return Err(PngError::BadChunk("missing IEND chunk"));
    }
    if idat_chunks.is_empty() {
        return Err(PngError::BadChunk("missing IDAT chunk"));
    }

    let channels = channels_for_color_type(info.color_type);
    let bytes_per_sample = (info.bit_depth / 8) as usize;
    let pixel_stride = channels as usize * bytes_per_sample;

    let row_bytes = (info.width as usize)
        .checked_mul(pixel_stride)
        .ok_or(PngError::DimensionOverflow)?;
    let scanline_stride = row_bytes.checked_add(1).ok_or(PngError::DimensionOverflow)?;
    let expected_len = scanline_stride
        .checked_mul(info.height as usize)
        .ok_or(PngError::DimensionOverflow)?;

    let final_size = (info.width as usize)
        .checked_mul(info.height as usize)
        .and_then(|px| px.checked_mul(4 * bytes_per_sample))
        .ok_or(PngError::DimensionOverflow)?;
    if final_size > MAX_DECODED_BYTES {
        return Err(PngError::DimensionOverflow);
    }

    // Concatenate all IDAT chunk payloads into one buffer BEFORE inflating —
    // chunk boundaries are non-semantic (PNG spec §5.6: "the boundaries
    // between chunks are arbitrary and impose no semantic layer").
    // Allocated once, exactly sized (Principle 1).
    let idat_total: usize = idat_chunks.iter().map(|c| c.len()).sum();
    let mut idat = Vec::with_capacity(idat_total);
    for chunk_data in &idat_chunks {
        idat.extend_from_slice(chunk_data);
    }

    let raw = inflate::zlib_decompress(&idat, expected_len)?;
    if raw.len() != expected_len {
        // PNG's filtered scanline layout is fully determined by IHDR; a
        // stream that inflates to a different length is corrupt regardless
        // of whether the DEFLATE block structure itself was well-formed.
        return Err(PngError::Truncated);
    }

    let unfiltered = unfilter(
        &raw,
        info.width as usize,
        info.height as usize,
        pixel_stride,
        scanline_stride,
    )?;
    let pixels = expand_to_rgba(unfiltered, info.width, info.height, info.color_type, bytes_per_sample);

    debug_assert_eq!(pixels.len(), final_size, "invariant: expand_to_rgba produces exactly width*height*4*bytes_per_sample bytes");

    Ok(DecodedImage { width: info.width, height: info.height, channels: 4, bit_depth: info.bit_depth, pixels })
}

// ---------------------------------------------------------------------------
// Chunk framing + CRC-32
// ---------------------------------------------------------------------------

struct RawChunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
}

/// Reads one `length | type | data | CRC` chunk starting at `*pos` (PNG spec
/// §5.3) and advances `*pos` past it. A CRC mismatch is logged, not a hard
/// error (robustness against a stale/incorrect trailer on otherwise-valid
/// data); a chunk whose declared length runs past the buffer end is.
fn read_chunk<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<RawChunk<'a>, PngError> {
    let start = *pos;
    if start + 8 > bytes.len() {
        return Err(PngError::Truncated);
    }
    let len = u32::from_be_bytes([bytes[start], bytes[start + 1], bytes[start + 2], bytes[start + 3]]) as usize;
    let kind = [bytes[start + 4], bytes[start + 5], bytes[start + 6], bytes[start + 7]];

    let data_start = start + 8;
    let data_end = data_start.checked_add(len).ok_or(PngError::Truncated)?;
    let crc_end = data_end.checked_add(4).ok_or(PngError::Truncated)?;
    if crc_end > bytes.len() {
        return Err(PngError::Truncated);
    }

    let data = &bytes[data_start..data_end];
    let stored_crc = u32::from_be_bytes([
        bytes[data_end],
        bytes[data_end + 1],
        bytes[data_end + 2],
        bytes[data_end + 3],
    ]);
    let computed_crc = crc32_chunk(&kind, data);
    if stored_crc != computed_crc {
        report_chunk_crc_mismatch(&kind, computed_crc, stored_crc);
    }

    *pos = crc_end;
    Ok(RawChunk { kind, data })
}

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

/// Reports `boyko-W2601`: a chunk whose stored CRC-32 disagrees with the one computed over its
/// bytes, which this decoder deliberately continues past.
///
/// `#[cold]` + `#[inline(never)]` because [`read_chunk`] runs once per chunk of every PNG the
/// engine loads and a corrupt chunk is the rare case: the whole emission, including rendering the
/// chunk's four type bytes, stays out of the reader's straight-line code.
///
/// `RatePolicy::Every`, and it is not an oversight. Each occurrence names a *different chunk*, a
/// file with two bad chunks is a different report from a file with one, and the site runs at load
/// time and not per frame — so there is nothing here for a latch to protect.
#[cold]
#[inline(never)]
fn report_chunk_crc_mismatch(kind: &[u8; 4], computed: u32, stored: u32) {
    boyko_log::warn!(
        boyko_log::Image,
        W2601.number(),
        "chunk '{}' CRC-32 mismatch (expected {:#010x}, got {:#010x}); continuing",
        boyko_log::dsp!(String::from_utf8_lossy(kind), 8),
        computed,
        stored
    );
}

/// PNG spec Annex D's CRC-32 table (reflected, polynomial `0xEDB88320`),
/// computed once at compile time.
const CRC32_TABLE: [u32; 256] = crc32_table();

/// Computes a chunk's CRC-32 over its type bytes followed by its data (PNG
/// spec §5.3: the length field and the CRC field itself are excluded).
fn crc32_chunk(kind: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in kind.iter().chain(data.iter()) {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// IHDR
// ---------------------------------------------------------------------------

struct IhdrInfo {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
}

/// Parses and fully validates an `IHDR` chunk's 13-byte payload (PNG spec
/// §11.2.2), rejecting anything outside this decoder's supported scope
/// (color type 3/palette, Adam7 interlacing, sub-byte bit depths) right
/// here — before any `IDAT` data is even inspected.
fn parse_ihdr(data: &[u8]) -> Result<IhdrInfo, PngError> {
    if data.len() != 13 {
        return Err(PngError::BadChunk("IHDR chunk must be exactly 13 bytes"));
    }

    let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let bit_depth = data[8];
    let color_type = data[9];
    let compression_method = data[10];
    let filter_method = data[11];
    let interlace_method = data[12];

    if width == 0 || height == 0 {
        return Err(PngError::BadChunk("IHDR width/height must be nonzero"));
    }
    if compression_method != 0 {
        return Err(PngError::BadChunk("IHDR compression method must be 0 (deflate)"));
    }
    if filter_method != 0 {
        return Err(PngError::BadChunk("IHDR filter method must be 0 (adaptive)"));
    }
    match interlace_method {
        0 => {}
        1 => return Err(PngError::UnsupportedInterlace),
        _ => return Err(PngError::BadChunk("IHDR interlace method must be 0 or 1")),
    }

    if color_type == 3 {
        return Err(PngError::PaletteUnsupported);
    }
    if !matches!(color_type, 0 | 2 | 4 | 6) {
        return Err(PngError::UnsupportedColorType(color_type));
    }
    if !matches!(bit_depth, 8 | 16) {
        return Err(PngError::UnsupportedBitDepth { color_type, bit_depth });
    }

    Ok(IhdrInfo { width, height, bit_depth, color_type })
}

/// Sample count per pixel for a supported (non-palette) PNG color type (PNG
/// spec §11.2.2 table).
fn channels_for_color_type(color_type: u8) -> u8 {
    match color_type {
        0 => 1, // grayscale
        2 => 3, // truecolor (RGB)
        4 => 2, // grayscale + alpha
        6 => 4, // truecolor + alpha (RGBA)
        _ => unreachable!("invariant: parse_ihdr already rejected any other color type"),
    }
}

// ---------------------------------------------------------------------------
// Scanline unfiltering
// ---------------------------------------------------------------------------

/// PNG spec §9.4's Paeth predictor. The `<=` (not `<`) comparisons are
/// load-bearing: on a tie the spec mandates `a` over `b`, and `b` over `c` —
/// a naive strict `<` would silently pick a different, spec-incorrect byte
/// on those ties (verified by the `paeth_tie_break_prefers_b_over_c` test,
/// where `a=1,b=10,c=4` ties `pb==pc` and the correct answer is `b`, not
/// `c`).
fn paeth_predictor(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Reverses PNG's per-scanline adaptive filtering (spec §9), producing the
/// raw (unfiltered) sample bytes for every row. `pixel_stride` is the
/// filter's `bpp` (bytes per pixel — the left/upper-left neighbor distance);
/// since only 8/16-bit depths are supported, this is always a whole number
/// of bytes (no sub-byte packing to account for).
///
/// Allocates the output buffer once, sized exactly `row_bytes * height`
/// (Principle 1); every read is a plain bounds-checked index into either the
/// already-unfiltered prefix of the current row or the previous row — no
/// `unsafe` is needed for this (the plan explicitly allows shipping the
/// unfilter path fully safe for v1).
fn unfilter(
    raw: &[u8],
    width: usize,
    height: usize,
    pixel_stride: usize,
    scanline_stride: usize,
) -> Result<Vec<u8>, PngError> {
    let row_bytes = scanline_stride - 1;
    debug_assert_eq!(row_bytes, width * pixel_stride, "invariant: row_bytes matches width*pixel_stride");
    let mut out = vec![0u8; row_bytes * height];

    for row in 0..height {
        let raw_row_start = row * scanline_stride;
        let filter_type = raw[raw_row_start];
        let filt = &raw[raw_row_start + 1..raw_row_start + 1 + row_bytes];
        let out_row_start = row * row_bytes;

        for x in 0..row_bytes {
            let a = if x >= pixel_stride { out[out_row_start + x - pixel_stride] } else { 0 };
            let b = if row > 0 { out[out_row_start - row_bytes + x] } else { 0 };
            let c = if row > 0 && x >= pixel_stride {
                out[out_row_start - row_bytes + x - pixel_stride]
            } else {
                0
            };
            let f = filt[x];
            let recon = match filter_type {
                0 => f,
                1 => f.wrapping_add(a),
                2 => f.wrapping_add(b),
                3 => f.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => f.wrapping_add(paeth_predictor(a as i32, b as i32, c as i32) as u8),
                _ => return Err(PngError::BadChunk("invalid scanline filter type (must be 0-4)")),
            };
            out[out_row_start + x] = recon;
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// RGBA expansion
// ---------------------------------------------------------------------------

/// Expands unfiltered source samples to tightly-packed RGBA, allocating the
/// final pixel buffer once (Principle 1). Color type 6 (already RGBA) is
/// returned by MOVE with zero copying — the unfiltered buffer already has
/// the exact target layout.
fn expand_to_rgba(unfiltered: Vec<u8>, width: u32, height: u32, color_type: u8, bytes_per_sample: usize) -> Vec<u8> {
    let px_count = width as usize * height as usize;
    let s = bytes_per_sample;

    match color_type {
        6 => unfiltered,
        2 => {
            let src_stride = 3 * s;
            let dst_stride = 4 * s;
            let mut out = vec![0u8; px_count * dst_stride];
            for i in 0..px_count {
                let src = i * src_stride;
                let dst = i * dst_stride;
                out[dst..dst + src_stride].copy_from_slice(&unfiltered[src..src + src_stride]);
                out[dst + src_stride..dst + dst_stride].fill(0xFF);
            }
            out
        }
        0 => {
            let dst_stride = 4 * s;
            let mut out = vec![0u8; px_count * dst_stride];
            for i in 0..px_count {
                let src = i * s;
                let dst = i * dst_stride;
                let gray = &unfiltered[src..src + s];
                out[dst..dst + s].copy_from_slice(gray);
                out[dst + s..dst + 2 * s].copy_from_slice(gray);
                out[dst + 2 * s..dst + 3 * s].copy_from_slice(gray);
                out[dst + 3 * s..dst + 4 * s].fill(0xFF);
            }
            out
        }
        4 => {
            let src_stride = 2 * s;
            let dst_stride = 4 * s;
            let mut out = vec![0u8; px_count * dst_stride];
            for i in 0..px_count {
                let src = i * src_stride;
                let dst = i * dst_stride;
                let gray = &unfiltered[src..src + s];
                let alpha = &unfiltered[src + s..src + 2 * s];
                out[dst..dst + s].copy_from_slice(gray);
                out[dst + s..dst + 2 * s].copy_from_slice(gray);
                out[dst + 2 * s..dst + 3 * s].copy_from_slice(gray);
                out[dst + 3 * s..dst + 4 * s].copy_from_slice(alpha);
            }
            out
        }
        _ => unreachable!("invariant: parse_ihdr already rejected any other color type"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // Test-only PNG/DEFLATE construction helpers. These deliberately do NOT
    // reuse the production encoder-shaped internals (there isn't one) —
    // they hand-assemble bytes per the RFC/PNG spec text independently, so a
    // decoder bug isn't masked by testing it against itself.
    // -----------------------------------------------------------------

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + data.len() + 4);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc32_chunk(kind, data).to_be_bytes());
        out
    }

    fn ihdr_chunk(width: u32, height: u32, bit_depth: u8, color_type: u8, interlace: u8) -> Vec<u8> {
        let mut d = Vec::with_capacity(13);
        d.extend_from_slice(&width.to_be_bytes());
        d.extend_from_slice(&height.to_be_bytes());
        d.push(bit_depth);
        d.push(color_type);
        d.push(0); // compression method
        d.push(0); // filter method
        d.push(interlace);
        chunk(b"IHDR", &d)
    }

    /// Wraps an already-complete raw DEFLATE bitstream in RFC 1950 zlib framing, with the
    /// **correct** Adler-32 of `decoded` (the bytes the payload inflates to) as the trailer.
    ///
    /// # The trailer used to be a dummy, and rung L8a had to stop that
    ///
    /// This helper wrote `[0, 0, 0, 0]` and its doc said so proudly: "a mismatch is a non-fatal
    /// warning by design, so tests need not duplicate the checksum algorithm". The consequence
    /// was that **every fixture-decoding test in this module silently exercised the corrupt-stream
    /// path**, which was invisible while the warning was an `eprintln!` nobody counted. The moment
    /// `boyko-W2602` became a counted record, those twenty-odd tests started injecting records
    /// into any concurrent observer's window — measured as a flaky `left: 7, right: 1` with the
    /// stat tuple showing every one of them delivered on the `Image` target.
    ///
    /// A fixture whose checksum is deliberately wrong is a fixture that tests something other than
    /// what its name says. One test still builds a mismatching stream on purpose
    /// (`adler32_mismatch_warns_but_still_decodes`); the rest are now clean.
    fn wrap_zlib(deflate_payload: &[u8], decoded: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01]; // CMF=8(deflate)/CINFO=7, FLG s.t. header%31==0, FDICT=0
        out.extend_from_slice(deflate_payload);
        out.extend_from_slice(&crate::inflate::adler32(decoded).to_be_bytes());
        out
    }

    /// [`wrap_zlib`] with a deliberately wrong trailer — the one fixture shape that must keep it.
    fn wrap_zlib_bad_adler(deflate_payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        out.extend_from_slice(deflate_payload);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    /// Builds a one-block STORED-DEFLATE zlib stream around `raw` (must be
    /// `<= 65535` bytes — true of every fixture in this file).
    fn zlib_stored(raw: &[u8]) -> Vec<u8> {
        assert!(raw.len() <= u16::MAX as usize, "test fixture too large for a single stored block");
        let mut deflate = Vec::with_capacity(5 + raw.len());
        deflate.push(0b0000_0001); // BFINAL=1, BTYPE=00 (stored)
        let len = raw.len() as u16;
        deflate.extend_from_slice(&len.to_le_bytes());
        deflate.extend_from_slice(&(!len).to_le_bytes());
        deflate.extend_from_slice(raw);
        wrap_zlib(&deflate, raw)
    }

    fn assemble_png(width: u32, height: u32, bit_depth: u8, color_type: u8, zlib_bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PNG_SIGNATURE);
        out.extend_from_slice(&ihdr_chunk(width, height, bit_depth, color_type, 0));
        out.extend_from_slice(&chunk(b"IDAT", zlib_bytes));
        out.extend_from_slice(&chunk(b"IEND", &[]));
        out
    }

    /// Convenience: a PNG whose IDAT is a single stored (uncompressed) block.
    fn build_stored_png(width: u32, height: u32, bit_depth: u8, color_type: u8, raw_scanlines: &[u8]) -> Vec<u8> {
        assemble_png(width, height, bit_depth, color_type, &zlib_stored(raw_scanlines))
    }

    /// Reverses the low `n` bits of `v` — converts an RFC 1951 §3.2.2
    /// canonical (MSB-first) Huffman code into the LSB-first bit-stream
    /// order the bit reader actually consumes.
    fn reverse_bits(v: u32, n: u32) -> u32 {
        v.reverse_bits() >> (32 - n)
    }

    /// Minimal LSB-first bit writer for hand-building DEFLATE bitstreams.
    struct TestBitWriter {
        bytes: Vec<u8>,
        acc: u32,
        nbits: u32,
    }

    impl TestBitWriter {
        fn new() -> Self {
            Self { bytes: Vec::new(), acc: 0, nbits: 0 }
        }

        fn put_bits(&mut self, value: u32, n: u32) {
            self.acc |= (value & ((1u32.checked_shl(n).unwrap_or(0)).wrapping_sub(1))) << self.nbits;
            self.nbits += n;
            while self.nbits >= 8 {
                self.bytes.push((self.acc & 0xFF) as u8);
                self.acc >>= 8;
                self.nbits -= 8;
            }
        }

        /// Appends a Huffman code given in canonical MSB-first form.
        fn put_huffman_msb(&mut self, code: u32, n: u32) {
            let reversed = reverse_bits(code, n);
            self.put_bits(reversed, n);
        }

        fn finish(mut self) -> Vec<u8> {
            if self.nbits > 0 {
                self.bytes.push((self.acc & 0xFF) as u8);
            }
            self.bytes
        }
    }

    /// Independent (from `inflate::HuffmanTable`) canonical-code computation
    /// per RFC 1951 §3.2.2, used only to derive bit patterns for hand-built
    /// fixtures — NOT shared with the production decoder.
    fn canonical_codes(lengths: &[u8]) -> Vec<(u32, u8)> {
        let max_len = *lengths.iter().max().unwrap_or(&0) as usize;
        let mut bl_count = vec![0u32; max_len + 1];
        for &l in lengths {
            if l != 0 {
                bl_count[l as usize] += 1;
            }
        }
        let mut next_code = vec![0u32; max_len + 1];
        let mut code = 0u32;
        for len in 1..=max_len {
            code = (code + bl_count[len - 1]) << 1;
            next_code[len] = code;
        }
        let mut out = vec![(0u32, 0u8); lengths.len()];
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            out[sym] = (next_code[len as usize], len);
            next_code[len as usize] += 1;
        }
        out
    }

    const CODE_LENGTH_ORDER: [usize; 19] =
        [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

    /// Builds a fixed-Huffman (BTYPE=01) DEFLATE block emitting every byte
    /// of `literals` as a literal, followed by end-of-block.
    fn deflate_fixed_literals(literals: &[u8]) -> Vec<u8> {
        let mut w = TestBitWriter::new();
        w.put_bits(1, 1); // BFINAL
        w.put_bits(0b01, 2); // BTYPE = fixed Huffman
        for &byte in literals {
            let code = fixed_literal_code(byte as u16);
            w.put_huffman_msb(code.0, code.1);
        }
        let eob = fixed_literal_code(256);
        w.put_huffman_msb(eob.0, eob.1);
        w.finish()
    }

    /// RFC 1951 §3.2.6's fixed literal/length code, `(code, bits)`.
    fn fixed_literal_code(sym: u16) -> (u32, u32) {
        match sym {
            0..=143 => (0b0011_0000 + sym as u32, 8),
            144..=255 => (0b1_1001_0000 + (sym - 144) as u32, 9),
            256..=279 => (sym as u32 - 256, 7),
            280..=287 => (0b1100_0000 + (sym - 280) as u32, 8),
            _ => panic!("invalid literal/length symbol"),
        }
    }

    #[test]
    fn rejects_bad_signature() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes[0] = 0x00;
        assert_eq!(decode_png(&bytes), Err(PngError::Signature));
    }

    #[test]
    fn rejects_truncated_before_signature() {
        assert_eq!(decode_png(&[0x89, 0x50, 0x4E]), Err(PngError::Signature));
    }

    #[test]
    fn rejects_palette_color_type() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&ihdr_chunk(1, 1, 8, 3, 0));
        bytes.extend_from_slice(&chunk(b"IEND", &[]));
        assert_eq!(decode_png(&bytes), Err(PngError::PaletteUnsupported));
    }

    #[test]
    fn rejects_adam7_interlace() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&ihdr_chunk(1, 1, 8, 2, 1));
        bytes.extend_from_slice(&chunk(b"IEND", &[]));
        assert_eq!(decode_png(&bytes), Err(PngError::UnsupportedInterlace));
    }

    #[test]
    fn rejects_sub_byte_bit_depth() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&ihdr_chunk(1, 1, 4, 0, 0));
        bytes.extend_from_slice(&chunk(b"IEND", &[]));
        assert_eq!(
            decode_png(&bytes),
            Err(PngError::UnsupportedBitDepth { color_type: 0, bit_depth: 4 })
        );
    }

    #[test]
    fn rejects_truncated_idat() {
        let full = build_stored_png(1, 1, 8, 0, &[0, 200]);
        let truncated = &full[..full.len() - 6]; // cut into IDAT's payload
        assert!(decode_png(truncated).is_err());
    }

    #[test]
    fn garbage_chunk_crc_warns_but_still_decodes() {
        let mut bytes = build_stored_png(1, 1, 8, 0, &[0, 200]);
        // Flip a byte in the last chunk's CRC field (IEND's, at the very end).
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let img = decode_png(&bytes).expect("a corrupt trailing CRC must not fail the decode");
        assert_eq!(img.pixels, vec![200, 200, 200, 255]);
    }

    #[test]
    fn adler32_mismatch_warns_but_still_decodes() {
        // Rung L8a: this test now builds its own mismatching stream instead of relying on every
        // OTHER fixture in the module being wrong. The old comment here read "every other test in
        // this module implicitly exercises the warns-but-decodes contract" — which was true, and
        // was the defect: twenty tests were quietly driving the corrupt path, and once the warning
        // became a counted `boyko-W2602` record they became a source of interference for anything
        // that measured delivery. See `wrap_zlib`'s doc for the measurement.
        let mut deflate = vec![0b0000_0001u8];
        deflate.extend_from_slice(&2u16.to_le_bytes());
        deflate.extend_from_slice(&(!2u16).to_le_bytes());
        deflate.extend_from_slice(&[0, 42]);
        let bytes = assemble_png(1, 1, 8, 0, &wrap_zlib_bad_adler(&deflate));
        let img = decode_png(&bytes).expect("Adler-32 mismatch must not fail the decode");
        assert_eq!(img.pixels, vec![42, 42, 42, 255]);
    }

    #[test]
    fn stored_block_grayscale_8bit() {
        let bytes = build_stored_png(1, 1, 8, 0, &[0, 200]);
        let img = decode_png(&bytes).expect("stored-block grayscale PNG should decode");
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 1);
        assert_eq!(img.channels, 4);
        assert_eq!(img.bit_depth, 8);
        assert_eq!(img.pixels, vec![200, 200, 200, 255]);
    }

    #[test]
    fn grayscale_16bit_preserves_big_endian_byte_order() {
        // A single pixel, gray sample 0x1234 (BE bytes 0x12,0x34).
        let bytes = build_stored_png(1, 1, 16, 0, &[0, 0x12, 0x34]);
        let img = decode_png(&bytes).expect("16-bit grayscale PNG should decode");
        assert_eq!(img.bit_depth, 16);
        assert_eq!(img.pixels, vec![0x12, 0x34, 0x12, 0x34, 0x12, 0x34, 0xFF, 0xFF]);
    }

    #[test]
    fn truecolor_sub_filter() {
        // 1 row, 2 RGB pixels: pixel0=(10,20,30) filtered directly (no left
        // neighbor -> Filt==Recon); pixel1=(15,25,35) filtered as
        // pixel1 - pixel0 = (5,5,5).
        let raw = [1u8, 10, 20, 30, 5, 5, 5]; // filter=Sub, then 6 filtered bytes
        let bytes = build_stored_png(2, 1, 8, 2, &raw);
        let img = decode_png(&bytes).expect("Sub-filtered truecolor PNG should decode");
        assert_eq!(img.pixels, vec![10, 20, 30, 255, 15, 25, 35, 255]);
    }

    #[test]
    fn grayscale_up_filter() {
        // row0: gray=50 (filter None). row1: filter=Up, Filt=70-50=20.
        let raw = [0u8, 50, 2, 20];
        let bytes = build_stored_png(1, 2, 8, 0, &raw);
        let img = decode_png(&bytes).expect("Up-filtered grayscale PNG should decode");
        assert_eq!(img.pixels, vec![50, 50, 50, 255, 70, 70, 70, 255]);
    }

    #[test]
    fn grayscale_alpha_average_filter() {
        // 1 row, 2 gray+alpha pixels, filter=Average(3), first row (b=0).
        // pixel0=(gray=20,alpha=200): a=0,b=0 -> Filt==Recon.
        // pixel1=(gray=30,alpha=210): Filt(gray)=30-floor((20+0)/2)=20;
        //                             Filt(alpha)=210-floor((200+0)/2)=110.
        let raw = [3u8, 20, 200, 20, 110];
        let bytes = build_stored_png(2, 1, 8, 4, &raw);
        let img = decode_png(&bytes).expect("Average-filtered gray+alpha PNG should decode");
        assert_eq!(img.pixels, vec![20, 20, 20, 200, 30, 30, 30, 210]);
    }

    #[test]
    fn paeth_predictor_tie_break_prefers_a_on_pa_pb_tie() {
        // a == b == 5: pa == pb by construction; the `<=` rule picks `a`
        // (functionally identical to `b` here, but exercises the branch).
        assert_eq!(paeth_predictor(5, 5, 3), 5);
    }

    #[test]
    fn paeth_predictor_tie_break_prefers_b_over_c() {
        // a=1, b=10, c=4: p=7, pa=6, pb=3, pc=3 -- pb==pc with pa the
        // largest, so the FIRST branch is skipped and the tie between b/c
        // must resolve to `b` (10), not `c` (4), per the spec's `<=` rule.
        assert_eq!(paeth_predictor(1, 10, 4), 10);
    }

    #[test]
    fn paeth_filtered_truecolor_uses_the_load_bearing_tie_break() {
        // row0 = [4, 10] (filter None). row1 = Paeth-filtered:
        //   col0: a=0,b=4,c=0 -> predictor=4 (picks b); Filt=1-4=253 (mod 256)
        //         -> Recon=253+4=257 mod256=1.
        //   col1: a=Recon(row1,col0)=1, b=Recon(row0,col1)=10,
        //         c=Recon(row0,col0)=4 -> predictor=10 (the b/c tie-break);
        //         Filt=0 -> Recon=10.
        let raw = [0u8, 4, 10, 4u8, 253, 0];
        let bytes = build_stored_png(2, 2, 8, 0, &raw);
        let img = decode_png(&bytes).expect("Paeth-filtered grayscale PNG should decode");
        assert_eq!(
            img.pixels,
            vec![
                4, 4, 4, 255, // row0 col0
                10, 10, 10, 255, // row0 col1
                1, 1, 1, 255, // row1 col0
                10, 10, 10, 255, // row1 col1 (the tie-break result)
            ]
        );
    }

    #[test]
    fn fixed_huffman_literal_only_grayscale() {
        let deflate = deflate_fixed_literals(&[0, 77]); // filter=None, gray=77
        let bytes = assemble_png(1, 1, 8, 0, &wrap_zlib(&deflate, &[0, 77]));
        let img = decode_png(&bytes).expect("fixed-Huffman grayscale PNG should decode");
        assert_eq!(img.pixels, vec![77, 77, 77, 255]);
    }

    #[test]
    fn dynamic_huffman_realistic_grayscale_row() {
        // A "real-ish" 4x1 8-bit grayscale row: filter byte 0 (None), pixels
        // [10, 20, 10, 30]. Five distinct literal/length symbols appear:
        // 0, 10, 20, 30, and end-of-block (256).
        let raw_scanline: [u8; 5] = [0, 10, 20, 10, 30];

        // Kraft-complete lengths (RFC 1951 §3.2.2): 3 symbols at length 2
        // (0, 10, 256), 2 symbols at length 3 (20, 30). HLIT covers indices
        // 0..=256 (the highest symbol used).
        let mut lit_lengths = vec![0u8; 257];
        lit_lengths[0] = 2;
        lit_lengths[10] = 2;
        lit_lengths[20] = 3;
        lit_lengths[30] = 3;
        lit_lengths[256] = 2;

        // No back-references: the single (minimum HDIST=1) distance code is
        // left unused (length 0).
        let dist_lengths = [0u8; 1];

        // The code-length alphabet needs entries only for the distinct
        // length VALUES used above: 0 (implicit), 2, 3. Kraft-complete:
        // symbol 0 -> length 1; symbols 2, 3 -> length 2 each.
        let mut cl_lengths = [0u8; 19];
        cl_lengths[0] = 1;
        cl_lengths[2] = 2;
        cl_lengths[3] = 2;

        let mut w = TestBitWriter::new();
        w.put_bits(1, 1); // BFINAL
        w.put_bits(0b10, 2); // BTYPE = dynamic Huffman
        w.put_bits((lit_lengths.len() - 257) as u32, 5); // HLIT
        w.put_bits((dist_lengths.len() - 1) as u32, 5); // HDIST

        // Transmit code-length-alphabet lengths through the last symbol we
        // need (symbol 2, permutation index 15) -> 16 entries.
        let hclen_count = 16;
        w.put_bits((hclen_count - 4) as u32, 4); // HCLEN
        for &sym in CODE_LENGTH_ORDER.iter().take(hclen_count) {
            w.put_bits(cl_lengths[sym] as u32, 3);
        }

        let cl_codes = canonical_codes(&cl_lengths);
        let write_cl_symbol = |w: &mut TestBitWriter, sym: usize| {
            let (code, len) = cl_codes[sym];
            w.put_huffman_msb(code, len as u32);
        };
        // All 257 + 1 = 258 literal+dist lengths, transmitted as direct 0-15
        // code-length symbols (no 16/17/18 run-length codes needed for a
        // fixture this small).
        for &len in lit_lengths.iter().chain(dist_lengths.iter()) {
            write_cl_symbol(&mut w, len as usize);
        }

        let lit_codes = canonical_codes(&lit_lengths);
        for &byte in &raw_scanline {
            let (code, len) = lit_codes[byte as usize];
            w.put_huffman_msb(code, len as u32);
        }
        let (eob_code, eob_len) = lit_codes[256];
        w.put_huffman_msb(eob_code, eob_len as u32);

        let deflate_bytes = w.finish();
        let bytes = assemble_png(4, 1, 8, 0, &wrap_zlib(&deflate_bytes, &raw_scanline));
        let img = decode_png(&bytes).expect("dynamic-Huffman grayscale PNG should decode");
        assert_eq!(
            img.pixels,
            vec![
                10, 10, 10, 255, //
                20, 20, 20, 255, //
                10, 10, 10, 255, //
                30, 30, 30, 255, //
            ]
        );
    }
}

#[cfg(test)]
mod l8a_w2601 {
    use super::*;
    use boyko_log::probe::{arm, watch, watched};

    /// A well-formed zero-length `IEND` chunk whose stored CRC is deliberately wrong.
    /// (The real one is `0xAE42_6082`.)
    const IEND_BAD_CRC: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0, 0, 0, 0];

    #[test]
    fn w2601_reports_every_bad_chunk_because_each_names_a_different_one() {
        // Driven through `read_chunk`, the production parser, not through the reporter: the
        // claim under test is that a corrupt chunk still DECODES (`Ok`, "continuing") while
        // being reported, and only the real parser can show both at once.
        arm::<boyko_log::Image>();

        watch(b'W', W2601.number());
        for _ in 0..3 {
            let mut pos = 0usize;
            let chunk =
                read_chunk(&IEND_BAD_CRC, &mut pos).expect("a bad CRC must not fail the read");
            assert_eq!(&chunk.kind, b"IEND");
        }
        assert_eq!(
            watched(),
            3,
            "`Every`: three corrupt chunks are three reports, because each names a different one"
        );
    }

    #[test]
    fn a_good_crc_reports_nothing() {
        // The positive control. Without it, a `read_chunk` that reported unconditionally would
        // pass the test above, and every PNG the engine loads would warn on every chunk.
        arm::<boyko_log::Image>();

        let mut good = IEND_BAD_CRC;
        let real = crc32_chunk(b"IEND", &[]);
        good[8..12].copy_from_slice(&real.to_be_bytes());

        watch(b'W', W2601.number());
        let mut pos = 0usize;
        let _ = read_chunk(&good, &mut pos).expect("a valid chunk reads");
        assert_eq!(watched(), 0, "a matching CRC is silent");
    }
}
