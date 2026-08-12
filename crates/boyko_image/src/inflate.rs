//! RFC 1950 (zlib) + RFC 1951 (DEFLATE) decompressor.
//!
//! This is the performance-critical half of the decoder: PNG scanline data is
//! always zlib/DEFLATE-compressed, so every byte of pixel data passes through
//! [`zlib_decompress`]. The design follows the two ideas the RFC leaves to the
//! implementor to get right:
//!
//! 1. A **bulk bit reader** that refills a 64-bit accumulator from whole bytes
//!    instead of pulling one bit at a time (RFC 1951 §3.1.1: "packed starting
//!    with the least-significant bit").
//! 2. A **canonical-Huffman fast lookup table** (root table + subtables for
//!    codes longer than the root) instead of a bit-by-bit tree walk, per
//!    RFC 1951 §3.2.2's canonical code construction.
//!
//! # A note on extra-bit order
//!
//! RFC 1951 §3.1.1 draws a hard line between two kinds of data in the
//! bitstream: **Huffman codes**, which are conceptually packed MSB-first (the
//! bits of the *canonical code value* are transmitted most-significant bit
//! first), and **"data elements other than Huffman codes"** — which includes
//! `BTYPE`, `HLIT`/`HDIST`/`HCLEN`, the code-length code lengths, the stored
//! block's `LEN`/`NLEN`, and critically the length/distance **extra bits** —
//! which are "packed starting with the least-significant bit of the data
//! element". Every reference/production DEFLATE implementation (zlib, puff.c,
//! miniz) reads extra bits LSB-first through the same bit reader as every
//! other non-Huffman field. This module does the same; a decoder that read
//! extra bits MSB-first would be unable to decode output from any real-world
//! DEFLATE encoder.

use boyko_log::codes::W2602;

use crate::error::PngError;

/// Root-table width for the canonical-Huffman fast decode table. Codes up to
/// this length decode with a single table lookup; longer codes (up to the
/// DEFLATE maximum of 15 bits) fall through to a subtable indexed by the
/// remaining bits. 9 bits keeps the root table at 512 entries (2 KiB at 4
/// bytes/entry) while covering the overwhelming majority of literal/length
/// codes in real Huffman-coded data.
const ROOT_BITS: u32 = 9;
/// The DEFLATE spec caps every Huffman code (literal/length, distance, and
/// the code-length alphabet) at 15 bits (RFC 1951 §3.2.7).
const MAX_CODE_BITS: u32 = 15;

/// RFC 1951 §3.2.5 length-code base values, indexed by `code - 257`.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// RFC 1951 §3.2.5 length-code extra-bit counts, indexed by `code - 257`.
const LENGTH_EXTRA_BITS: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// RFC 1951 §3.2.5 distance-code base values, indexed by the distance code.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// RFC 1951 §3.2.5 distance-code extra-bit counts, indexed by the distance code.
const DIST_EXTRA_BITS: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// RFC 1951 §3.2.7: the code-length alphabet's 19 symbols are transmitted in
/// this fixed, non-numeric permutation so that commonly-zero trailing
/// lengths (for rarely-used symbols) can be truncated via `HCLEN`.
const CODE_LENGTH_ORDER: [usize; 19] =
    [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

// ---------------------------------------------------------------------------
// Bit reader
// ---------------------------------------------------------------------------

/// LSB-first bit reader with a bulk 64-bit refill accumulator.
///
/// `acc`'s bit 0 always holds the next bit to be consumed; refilling ORs in
/// whole bytes above the current fill level rather than reading bit-by-bit,
/// which is the main throughput lever for the Huffman decode loop below (one
/// branch-heavy memory access per byte instead of per bit).
struct BitReader<'a> {
    data: &'a [u8],
    /// Index of the next byte in `data` not yet folded into `acc`.
    pos: usize,
    /// Bit accumulator; only the low `nbits` bits are valid.
    acc: u64,
    /// Number of valid bits currently held in `acc` (0..=64).
    nbits: u32,
}

impl<'a> BitReader<'a> {
    #[inline]
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, acc: 0, nbits: 0 }
    }

    /// Pulls whole bytes from `data` into `acc` until either the input is
    /// exhausted or fewer than 8 more bits would fit. `nbits` never exceeds
    /// 64: the loop only adds a byte while `nbits <= 56`, so the shift below
    /// (`<< nbits`, nbits <= 56) always lands within the 64-bit accumulator.
    #[inline]
    fn refill(&mut self) {
        while self.nbits <= 56 && self.pos < self.data.len() {
            self.acc |= (self.data[self.pos] as u64) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
    }

    /// Returns the low `n` bits of the accumulator without consuming them.
    /// Caller must ensure at least `n` bits are available (via `refill`).
    #[inline]
    fn peek_bits(&self, n: u32) -> u32 {
        debug_assert!(n <= 32, "invariant: peek_bits is only used for <=32-bit windows");
        (self.acc & ((1u64 << n) - 1)) as u32
    }

    #[inline]
    fn consume_bits(&mut self, n: u32) {
        debug_assert!(n <= self.nbits, "invariant: cannot consume more bits than buffered");
        self.acc >>= n;
        self.nbits -= n;
    }

    /// Reads and consumes an `n`-bit (`n <= 32`) LSB-first data element (RFC
    /// 1951 §3.1.1's "other than Huffman codes" packing — used for `BTYPE`,
    /// `HLIT`/`HDIST`/`HCLEN`, code-length code lengths, and length/distance
    /// extra bits alike).
    #[inline]
    fn get_bits(&mut self, n: u32) -> Result<u32, PngError> {
        if n == 0 {
            return Ok(0);
        }
        self.refill();
        if self.nbits < n {
            return Err(PngError::Truncated);
        }
        let v = self.peek_bits(n);
        self.consume_bits(n);
        Ok(v)
    }

    /// Discards the partial byte so the next read starts at a byte boundary
    /// (RFC 1951 §3.2.4, used before a stored block). Any WHOLE bytes still
    /// sitting in the accumulator were already pulled from `data` ahead of
    /// the logical read cursor, so `pos` is rewound to point at them again —
    /// `read_raw_bytes` then reads straight from `data`, not from `acc`.
    #[inline]
    fn align_to_byte(&mut self) {
        let frac = self.nbits % 8;
        self.acc >>= frac;
        self.nbits -= frac;
        let buffered_bytes = (self.nbits / 8) as usize;
        self.pos -= buffered_bytes;
        self.acc = 0;
        self.nbits = 0;
    }

    /// Returns a zero-copy slice of `n` raw bytes at the current (byte-aligned)
    /// position. Caller must have called [`Self::align_to_byte`] first.
    #[inline]
    fn read_raw_bytes(&mut self, n: usize) -> Result<&'a [u8], PngError> {
        debug_assert_eq!(self.nbits, 0, "invariant: read_raw_bytes requires byte alignment");
        let end = self.pos.checked_add(n).ok_or(PngError::Truncated)?;
        if end > self.data.len() {
            return Err(PngError::Truncated);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
}

// ---------------------------------------------------------------------------
// Canonical-Huffman fast-table decoder
// ---------------------------------------------------------------------------

/// One decode-table slot: either a resolved symbol, a redirect to a
/// subtable, or unused. 4 bytes so the 512-entry root table is a compact
/// 2 KiB (fits comfortably in L1d).
#[derive(Clone, Copy)]
struct Entry {
    kind: EntryKind,
    /// Code length in bits for `Symbol`; unused otherwise.
    bits: u8,
    /// Decoded symbol value for `Symbol`; unused otherwise (a `Redirect`'s
    /// subtable base is a fixed `prefix << sub_bits` formula, not stored).
    symbol: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    /// This bit pattern is not a valid prefix of any code in the table.
    /// Reachable only with corrupt/malicious input.
    Invalid,
    /// A resolved literal/length, distance, or code-length symbol.
    Symbol,
    /// Root-table-only: the low `ROOT_BITS` bits match multiple codes longer
    /// than `ROOT_BITS`; decode continues into the subtable.
    Redirect,
}

const EMPTY_ENTRY: Entry = Entry { kind: EntryKind::Invalid, bits: 0, symbol: 0 };

/// A canonical-Huffman decode table built from RFC 1951 §3.2.2 code lengths.
///
/// Reused across DEFLATE blocks (one instance for the literal/length
/// alphabet, one for distances, and a small scratch one for the code-length
/// alphabet) to avoid a heap allocation per dynamic-Huffman block header —
/// only the growable `sub` `Vec` may reallocate, and only when a block's
/// codes actually reach past the root width.
struct HuffmanTable {
    root: [Entry; 1 << ROOT_BITS],
    sub: Vec<Entry>,
    /// Bit width of the subtable index (0 when no code exceeds `ROOT_BITS`).
    sub_bits: u32,
}

impl HuffmanTable {
    fn new() -> Self {
        Self { root: [EMPTY_ENTRY; 1 << ROOT_BITS], sub: Vec::new(), sub_bits: 0 }
    }

    /// (Re)builds the table for `lengths[sym]` = code length of `sym` (0 =
    /// unused). Implements RFC 1951 §3.2.2's canonical code assignment, then
    /// scatters each resolved code across every root/subtable slot whose
    /// low bits match it (the "don't care" bits above the code's own length
    /// are filled too, so a single table lookup at `ROOT_BITS`+`sub_bits`
    /// bits of lookahead always resolves — no per-bit branching to grow the
    /// window).
    fn build(&mut self, lengths: &[u8]) -> Result<(), PngError> {
        self.root.fill(EMPTY_ENTRY);

        let mut bl_count = [0u32; (MAX_CODE_BITS + 1) as usize];
        let mut max_len = 0u32;
        let mut used = 0u32;
        for &len in lengths {
            if len != 0 {
                bl_count[len as usize] += 1;
                max_len = max_len.max(len as u32);
                used += 1;
            }
        }

        if used == 0 {
            // An empty table (e.g. an unused distance alphabet) never decodes
            // anything; leave every entry `Invalid`.
            self.sub_bits = 0;
            self.sub.clear();
            return Ok(());
        }

        // Kraft inequality: sum(2^(max_len - len)) must not exceed 2^max_len.
        // Equality is a "complete" code (every codepoint assigned); LESS is
        // an "incomplete but valid" code — RFC 1951 §3.2.7 explicitly allows
        // this for a lone symbol ("one distance code... one unused code"),
        // and the fixed distance table (§3.2.6: 30 of 32 5-bit codepoints
        // used) is the standing incomplete-but-valid example baked into
        // every DEFLATE stream. An incomplete code is still unambiguous —
        // the fill loop below simply leaves the unused codepoints `Invalid`,
        // which `decode()` correctly rejects only if one is ever actually
        // read. MORE than 2^max_len is over-subscribed: two symbols would
        // claim the same bit pattern, an unambiguous corrupt-stream signal.
        let mut kraft = 0u32;
        for len in 1..=max_len {
            kraft += bl_count[len as usize] << (max_len - len);
        }
        if kraft > 1u32 << max_len {
            return Err(PngError::InflateError("Huffman code lengths are over-subscribed"));
        }

        // next_code[len] = the canonical MSB-first code value for the first
        // symbol of that length (RFC 1951 §3.2.2 algorithm).
        let mut next_code = [0u32; (MAX_CODE_BITS + 1) as usize];
        let mut code = 0u32;
        for len in 1..=max_len as usize {
            code = (code + bl_count[len - 1]) << 1;
            next_code[len] = code;
        }

        self.sub_bits = max_len.saturating_sub(ROOT_BITS);
        if self.sub_bits > 0 {
            let sub_len = (1usize << ROOT_BITS) << self.sub_bits;
            self.sub.clear();
            self.sub.resize(sub_len, EMPTY_ENTRY);
        } else {
            self.sub.clear();
        }

        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let len = len as u32;
            let code_val = next_code[len as usize];
            next_code[len as usize] += 1;
            // The bitstream is read LSB-first, but the canonical code above
            // is an MSB-first value (RFC 1951 §3.1.1) — reversing its `len`
            // bits gives the pattern that will actually appear, in order, at
            // the front of the bit reader's window when this code is next.
            let reversed = reverse_bits(code_val, len);
            let entry = Entry { kind: EntryKind::Symbol, bits: len as u8, symbol: sym as u16 };

            if len <= ROOT_BITS {
                let step = 1u32 << len;
                let mut idx = reversed;
                while idx < (1 << ROOT_BITS) {
                    self.root[idx as usize] = entry;
                    idx += step;
                }
            } else {
                let prefix = reversed & ((1 << ROOT_BITS) - 1);
                self.root[prefix as usize] = Entry { kind: EntryKind::Redirect, bits: 0, symbol: 0 };
                let extra_bits = len - ROOT_BITS;
                let base = (prefix as usize) << self.sub_bits;
                let step = 1u32 << extra_bits;
                let mut extra = reversed >> ROOT_BITS;
                while extra < (1 << self.sub_bits) {
                    self.sub[base + extra as usize] = entry;
                    extra += step;
                }
            }
        }

        Ok(())
    }

    /// Decodes one symbol from `br`, consuming exactly its code length in
    /// bits. Returns `InflateError` if the bit pattern does not correspond
    /// to any valid code (corrupt stream) or the stream runs out mid-code.
    #[inline]
    fn decode(&self, br: &mut BitReader) -> Result<u16, PngError> {
        br.refill();
        let window = br.peek_bits(ROOT_BITS);
        let root_entry = self.root[window as usize];
        match root_entry.kind {
            EntryKind::Symbol => {
                if br.nbits < root_entry.bits as u32 {
                    return Err(PngError::Truncated);
                }
                br.consume_bits(root_entry.bits as u32);
                Ok(root_entry.symbol)
            }
            EntryKind::Redirect => {
                // `ROOT_BITS + sub_bits <= 15` (DEFLATE's own code-length cap),
                // well within `peek_bits`'s 32-bit window.
                let full_window = br.peek_bits(ROOT_BITS + self.sub_bits);
                let extra = full_window >> ROOT_BITS;
                let base = (window as usize) << self.sub_bits;
                let sub_entry = self.sub[base + extra as usize];
                if sub_entry.kind != EntryKind::Symbol {
                    return Err(PngError::InflateError("invalid Huffman code (unassigned bit pattern)"));
                }
                if br.nbits < sub_entry.bits as u32 {
                    return Err(PngError::Truncated);
                }
                br.consume_bits(sub_entry.bits as u32);
                Ok(sub_entry.symbol)
            }
            EntryKind::Invalid => {
                Err(PngError::InflateError("invalid Huffman code (unassigned bit pattern)"))
            }
        }
    }
}

/// Reverses the low `n` bits of `v` (`n <= 16`).
#[inline]
fn reverse_bits(v: u32, n: u32) -> u32 {
    debug_assert!(n <= 16, "invariant: DEFLATE code lengths never exceed 15 bits");
    v.reverse_bits() >> (32 - n)
}

// ---------------------------------------------------------------------------
// DEFLATE block decoding
// ---------------------------------------------------------------------------

/// Builds the RFC 1951 §3.2.6 fixed (non-dynamic) Huffman tables. These are
/// the same for every fixed-Huffman block in every DEFLATE stream ever
/// produced, so the caller may cache the result across blocks if desired;
/// here we simply rebuild (cheap: 288+32 entries, no I/O).
fn build_fixed_tables(lit: &mut HuffmanTable, dist: &mut HuffmanTable) -> Result<(), PngError> {
    let mut lit_lengths = [0u8; 288];
    for (sym, len) in lit_lengths.iter_mut().enumerate() {
        *len = match sym {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8, // 280..=287
        };
    }
    lit.build(&lit_lengths)?;

    let dist_lengths = [5u8; 30];
    dist.build(&dist_lengths)
}

/// Reads a dynamic-Huffman block header (RFC 1951 §3.2.7) and builds `lit`
/// and `dist` from the transmitted code lengths.
fn read_dynamic_tables(
    br: &mut BitReader,
    lit: &mut HuffmanTable,
    dist: &mut HuffmanTable,
    cl_scratch: &mut HuffmanTable,
) -> Result<(), PngError> {
    let hlit = br.get_bits(5)? as usize + 257;
    let hdist = br.get_bits(5)? as usize + 1;
    let hclen = br.get_bits(4)? as usize + 4;

    let mut cl_lengths = [0u8; 19];
    for &sym in CODE_LENGTH_ORDER.iter().take(hclen) {
        cl_lengths[sym] = br.get_bits(3)? as u8;
    }
    cl_scratch.build(&cl_lengths)?;

    let total = hlit + hdist;
    let mut all_lengths = vec![0u8; total];
    let mut i = 0;
    let mut prev = 0u8;
    while i < total {
        let sym = cl_scratch.decode(br)?;
        match sym {
            0..=15 => {
                all_lengths[i] = sym as u8;
                prev = sym as u8;
                i += 1;
            }
            16 => {
                let repeat = br.get_bits(2)? + 3;
                if i == 0 {
                    return Err(PngError::InflateError("code-16 repeat with no previous length"));
                }
                if i + repeat as usize > total {
                    return Err(PngError::InflateError("code-16 repeat overruns the length table"));
                }
                for _ in 0..repeat {
                    all_lengths[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let repeat = br.get_bits(3)? + 3;
                if i + repeat as usize > total {
                    return Err(PngError::InflateError("code-17 repeat overruns the length table"));
                }
                i += repeat as usize;
                prev = 0;
            }
            18 => {
                let repeat = br.get_bits(7)? + 11;
                if i + repeat as usize > total {
                    return Err(PngError::InflateError("code-18 repeat overruns the length table"));
                }
                i += repeat as usize;
                prev = 0;
            }
            _ => return Err(PngError::InflateError("invalid code-length alphabet symbol")),
        }
    }

    lit.build(&all_lengths[..hlit])?;
    dist.build(&all_lengths[hlit..])
}

/// Decodes one stored (uncompressed) block (RFC 1951 §3.2.4) directly into
/// `out`, honoring `max_len` as a decompression-bomb guard.
fn inflate_stored(br: &mut BitReader, out: &mut Vec<u8>, max_len: usize) -> Result<(), PngError> {
    br.align_to_byte();
    let header = br.read_raw_bytes(4)?;
    let len = u16::from_le_bytes([header[0], header[1]]);
    let nlen = u16::from_le_bytes([header[2], header[3]]);
    if len != !nlen {
        return Err(PngError::InflateError("stored block LEN/NLEN mismatch"));
    }
    let payload = br.read_raw_bytes(len as usize)?;
    if out.len() + payload.len() > max_len {
        return Err(PngError::DimensionOverflow);
    }
    out.extend_from_slice(payload);
    Ok(())
}

/// Copies `length` bytes starting `distance` bytes back from the current end
/// of `out` (an LZ77 back-reference, RFC 1951 §3.2.3). Matches may overlap
/// the write cursor (e.g. `distance == 1` is a run-length fill), so this
/// copies byte-forward rather than via a single self-overlapping slice copy —
/// each read observes bytes already appended earlier in this same call.
#[inline]
fn lz77_copy(out: &mut Vec<u8>, distance: usize, length: usize, max_len: usize) -> Result<(), PngError> {
    if distance == 0 || distance > out.len() {
        return Err(PngError::InflateError("back-reference distance points before the output start"));
    }
    if out.len() + length > max_len {
        return Err(PngError::DimensionOverflow);
    }
    out.reserve(length);
    let start = out.len() - distance;
    for i in 0..length {
        // SAFETY: `start + i < out.len()` at the moment of this read: `start
        // = original_len - distance` with `distance >= 1`, and after `i`
        // pushes `out.len() == original_len + i`, so `start + i < out.len()`
        // holds for every `i` in `0..length`. `out.reserve(length)` above
        // guarantees the push below never reallocates mid-loop (the pointer
        // read here stays valid for the whole loop).
        let byte = out[start + i];
        out.push(byte);
    }
    Ok(())
}

/// Decodes one compressed (fixed- or dynamic-Huffman) block's literal/length
/// symbol stream into `out`, until the end-of-block symbol (256) or an
/// error.
fn inflate_block(
    br: &mut BitReader,
    lit: &HuffmanTable,
    dist: &HuffmanTable,
    out: &mut Vec<u8>,
    max_len: usize,
) -> Result<(), PngError> {
    loop {
        let sym = lit.decode(br)?;
        match sym {
            0..=255 => {
                if out.len() >= max_len {
                    return Err(PngError::DimensionOverflow);
                }
                out.push(sym as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let idx = (sym - 257) as usize;
                let extra = br.get_bits(LENGTH_EXTRA_BITS[idx] as u32)?;
                let length = LENGTH_BASE[idx] as usize + extra as usize;

                let dist_sym = dist.decode(br)?;
                if dist_sym as usize >= DIST_BASE.len() {
                    return Err(PngError::InflateError("invalid distance code"));
                }
                let dist_idx = dist_sym as usize;
                let dist_extra = br.get_bits(DIST_EXTRA_BITS[dist_idx] as u32)?;
                let distance = DIST_BASE[dist_idx] as usize + dist_extra as usize;

                lz77_copy(out, distance, length, max_len)?;
            }
            _ => return Err(PngError::InflateError("invalid literal/length symbol")),
        }
    }
}

/// Decompresses a raw RFC 1951 DEFLATE stream (no zlib framing) into `out`,
/// which is grown via ordinary `Vec` pushes but must have been pre-reserved
/// by the caller to `max_len` so the common (well-formed) case never
/// reallocates (Principle 1 / 5: allocate the output once).
///
/// `max_len` bounds every write — a decompression-bomb guard shared by
/// stored, literal, and back-reference paths.
pub(crate) fn inflate_into(data: &[u8], max_len: usize, out: &mut Vec<u8>) -> Result<(), PngError> {
    let mut br = BitReader::new(data);
    let mut lit_table = HuffmanTable::new();
    let mut dist_table = HuffmanTable::new();
    let mut cl_table = HuffmanTable::new();

    loop {
        let bfinal = br.get_bits(1)?;
        let btype = br.get_bits(2)?;
        match btype {
            0 => inflate_stored(&mut br, out, max_len)?,
            1 => {
                build_fixed_tables(&mut lit_table, &mut dist_table)?;
                inflate_block(&mut br, &lit_table, &dist_table, out, max_len)?;
            }
            2 => {
                read_dynamic_tables(&mut br, &mut lit_table, &mut dist_table, &mut cl_table)?;
                inflate_block(&mut br, &lit_table, &dist_table, out, max_len)?;
            }
            _ => return Err(PngError::InflateError("invalid DEFLATE block type (3 is reserved)")),
        }
        if bfinal == 1 {
            return Ok(());
        }
    }
}

// ---------------------------------------------------------------------------
// Adler-32 (RFC 1950 zlib trailer)
// ---------------------------------------------------------------------------

const ADLER_MOD: u32 = 65521;

/// RFC 1950 §9's Adler-32 checksum, computed over the decompressed bytes.
/// Reports `boyko-W2602`: the zlib trailer's Adler-32 does not match the inflated bytes, and
/// this decoder keeps them anyway.
///
/// `#[cold]` + `#[inline(never)]`: [`zlib_decompress`] is the entry point for every PNG the
/// engine loads, and the mismatch is the rare arm.
///
/// **Not an `Err`, and that is the pre-existing behaviour this rung preserves.** The checksum
/// covers the whole stream, so a mismatch says "some byte in this image is wrong" without saying
/// which — and refusing the image outright would turn a possibly-cosmetic corruption into a
/// missing texture. What changes here is only that the report now carries a code.
#[cold]
#[inline(never)]
fn report_adler_mismatch(expected: u32, actual: u32) {
    boyko_log::warn!(
        boyko_log::Image,
        W2602.number(),
        "zlib Adler-32 mismatch (expected {:#010x}, got {:#010x}); decoded pixel data may still \
         be usable, continuing",
        expected,
        actual
    );
}

pub(crate) fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    // NMAX = 5552 is the largest chunk size for which `b` cannot overflow a
    // u32 before a modulo reduction is due (the standard Adler-32
    // batching trick — avoids a `% ADLER_MOD` on every single byte).
    const NMAX: usize = 5552;
    for chunk in data.chunks(NMAX) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b << 16) | a
}

// ---------------------------------------------------------------------------
// RFC 1950 zlib wrapper
// ---------------------------------------------------------------------------

/// Decompresses a zlib-wrapped (RFC 1950) DEFLATE stream: verifies the
/// 2-byte header (compression method must be `8` = DEFLATE; a preset
/// dictionary is rejected — this decoder never supplies one), inflates the
/// payload, and checks the trailing Adler-32.
///
/// `expected_len` is the exact decompressed size (known up front from a
/// PNG's `IHDR`, since PNG's filtered scanline layout is fully determined by
/// width/height/bit-depth/color-type); the output `Vec` is reserved to it
/// once, and inflate is bounded by it as a decompression-bomb guard.
///
/// An Adler-32 mismatch is logged and NOT a hard failure (robustness against
/// encoders that got the trailer wrong but produced otherwise-valid pixel
/// data) — a truncated/malformed DEFLATE stream is still a hard error via
/// [`inflate_into`].
pub(crate) fn zlib_decompress(data: &[u8], expected_len: usize) -> Result<Vec<u8>, PngError> {
    if data.len() < 6 {
        // 2-byte header + 4-byte Adler-32 trailer is the smallest possible
        // zlib stream (an empty DEFLATE payload is itself impossible, but we
        // don't need to special-case that: `inflate_into` below will report
        // `Truncated` on the empty remainder).
        return Err(PngError::Truncated);
    }

    let cmf = data[0];
    let flg = data[1];
    let cm = cmf & 0x0F;
    if cm != 8 {
        return Err(PngError::InflateError("zlib compression method is not DEFLATE (CM != 8)"));
    }
    if (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
        return Err(PngError::InflateError("zlib header checksum (FCHECK) failed"));
    }
    let fdict = (flg >> 5) & 1;
    if fdict != 0 {
        return Err(PngError::InflateError("zlib preset dictionaries (FDICT) are not supported"));
    }

    let payload = &data[2..data.len() - 4];
    let mut out = Vec::with_capacity(expected_len);
    inflate_into(payload, expected_len, &mut out)?;

    let trailer = &data[data.len() - 4..];
    let expected_adler = u32::from_be_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let actual_adler = adler32(&out);
    if expected_adler != actual_adler {
        report_adler_mismatch(expected_adler, actual_adler);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-built RFC 1951 stored block: BFINAL=1, BTYPE=00, then
    /// byte-aligned LEN/NLEN/data. The literal payload is "hi" (2 bytes).
    #[test]
    fn stored_block_round_trips() {
        // First byte: bit0=BFINAL(1), bits1-2=BTYPE(00), remaining bits are
        // padding to the next byte boundary (all zero).
        let mut data = vec![0b0000_0001u8];
        data.extend_from_slice(&2u16.to_le_bytes()); // LEN = 2
        data.extend_from_slice(&(!2u16).to_le_bytes()); // NLEN = !LEN
        data.extend_from_slice(b"hi");

        let mut out = Vec::new();
        inflate_into(&data, 1024, &mut out).expect("stored block should decode");
        assert_eq!(out, b"hi");
    }

    #[test]
    fn fixed_huffman_literal_only() {
        // Fixed-Huffman block containing exactly the literal 'A' (0x41 = 65,
        // which per RFC 1951 §3.2.6 falls in the 0-143 range: 8-bit code,
        // value = 0x30 + 65 = 0x71, MSB-first) followed by the end-of-block
        // symbol 256 (7-bit code 0000000).
        //
        // Bits, MSB-first per symbol, in transmission order:
        //   BFINAL=1, BTYPE=01(fixed) -> as LSB-first data elements: bit0=1,
        //   bits1-2 = 01 read LSB-first from the stream (i.e. `01` value 1
        //   with bit1=1,bit2=0).
        // Building this by hand bit-by-bit is error-prone; instead assert
        // round-trip behavior using the encoder-free approach: construct the
        // bitstream with a tiny helper bit writer scoped to this test.
        let mut w = TestBitWriter::new();
        w.put_bits(1, 1); // BFINAL
        w.put_bits(0b01, 2); // BTYPE = fixed Huffman
        // Literal 'A' = 65 -> fixed code (8 bits): value = 48 + 65 = 113 = 0b01110001, MSB-first.
        w.put_huffman_msb(0b0111_0001, 8);
        // End-of-block symbol 256 -> fixed code (7 bits): value = 0b0000000.
        w.put_huffman_msb(0b0000000, 7);
        let data = w.finish();

        let mut out = Vec::new();
        inflate_into(&data, 1024, &mut out).expect("fixed-Huffman literal should decode");
        assert_eq!(out, b"A");
    }

    #[test]
    fn fixed_huffman_back_reference() {
        // Encodes "aaaa" as: literal 'a', then a length-3 back-reference at
        // distance 1 (fills 3 more 'a's), then end-of-block.
        let mut w = TestBitWriter::new();
        w.put_bits(1, 1); // BFINAL
        w.put_bits(0b01, 2); // BTYPE = fixed

        // Literal 'a' = 97 -> in 0..=143 -> 8-bit code = 48 + 97 = 145 = 0b10010001.
        w.put_huffman_msb(0b1001_0001, 8);

        // Length 3 -> length code 257 (extra bits = 0). Fixed code for
        // symbol 257: symbols 256-279 get 7-bit codes starting at 0b0000000
        // for 256, so 257 = 0b0000001.
        w.put_huffman_msb(0b0000001, 7);
        // Distance 1 -> distance code 0 (extra bits = 0), fixed 5-bit code
        // 0b00000.
        w.put_huffman_msb(0b00000, 5);

        // End-of-block symbol 256 -> 0b0000000 (7 bits).
        w.put_huffman_msb(0b0000000, 7);
        let data = w.finish();

        let mut out = Vec::new();
        inflate_into(&data, 1024, &mut out).expect("back-reference should decode");
        assert_eq!(out, b"aaaa");
    }

    #[test]
    fn rejects_bad_block_type() {
        let mut w = TestBitWriter::new();
        w.put_bits(1, 1); // BFINAL
        w.put_bits(0b11, 2); // BTYPE = 3 (reserved/invalid)
        let data = w.finish();

        let mut out = Vec::new();
        let err = inflate_into(&data, 1024, &mut out).unwrap_err();
        assert!(matches!(err, PngError::InflateError(_)));
    }

    #[test]
    fn adler32_matches_known_vector() {
        // "Wikipedia" -> Adler-32 = 0x11E60398 (widely-cited reference vector).
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    /// Minimal LSB-first bit writer used ONLY by these unit tests to hand-
    /// build DEFLATE bitstreams without depending on an external encoder.
    struct TestBitWriter {
        bytes: Vec<u8>,
        acc: u32,
        nbits: u32,
    }

    impl TestBitWriter {
        fn new() -> Self {
            Self { bytes: Vec::new(), acc: 0, nbits: 0 }
        }

        /// Appends `n` bits of `value` (already in LSB-first "data element"
        /// order — i.e. `value`'s bit 0 is transmitted first).
        fn put_bits(&mut self, value: u32, n: u32) {
            self.acc |= (value & ((1 << n) - 1)) << self.nbits;
            self.nbits += n;
            while self.nbits >= 8 {
                self.bytes.push((self.acc & 0xFF) as u8);
                self.acc >>= 8;
                self.nbits -= 8;
            }
        }

        /// Appends a Huffman code given in its canonical MSB-first form
        /// (`code`'s top used bit is transmitted first) by reversing it into
        /// the LSB-first "data element" order the bitstream actually uses.
        fn put_huffman_msb(&mut self, code: u32, n: u32) {
            let reversed = super::reverse_bits(code, n);
            self.put_bits(reversed, n);
        }

        fn finish(mut self) -> Vec<u8> {
            if self.nbits > 0 {
                self.bytes.push((self.acc & 0xFF) as u8);
            }
            self.bytes
        }
    }
}

#[cfg(test)]
mod l8a_w2602 {
    use super::*;
    use boyko_log::probe::{arm, watch, watched};

    /// A minimal zlib stream: `78 01` header, one BFINAL stored block carrying the single
    /// byte `0x41`, then a four-byte Adler-32 the caller supplies.
    fn zlib_one_byte(adler: u32) -> Vec<u8> {
        let mut v = vec![0x78, 0x01, 0x01, 0x01, 0x00, 0xFE, 0xFF, 0x41];
        v.extend_from_slice(&adler.to_be_bytes());
        v
    }

    #[test]
    fn w2602_reports_the_mismatch_and_still_returns_the_bytes() {
        // Both halves matter and only the real `zlib_decompress` shows both: the checksum covers
        // the WHOLE stream, so a mismatch says "some byte is wrong" without saying which --
        // refusing the image outright would turn a possibly-cosmetic corruption into a missing
        // texture. So the bytes come back AND the condition is reported.
        arm::<boyko_log::Image>();

        watch(b'W', W2602.number());
        let out = zlib_decompress(&zlib_one_byte(0x0000_0000), 1)
            .expect("an Adler mismatch must not fail the decode");
        assert_eq!(out, vec![0x41], "the decoded bytes are kept");
        assert_eq!(watched(), 1, "the mismatch is reported");
    }

    #[test]
    fn a_matching_adler_reports_nothing() {
        // The positive control.
        arm::<boyko_log::Image>();

        watch(b'W', W2602.number());
        let out =
            zlib_decompress(&zlib_one_byte(adler32(&[0x41])), 1).expect("a clean stream decodes");
        assert_eq!(out, vec![0x41]);
        assert_eq!(watched(), 0, "a matching Adler is silent");
    }
}
