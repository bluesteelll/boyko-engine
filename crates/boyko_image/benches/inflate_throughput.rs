//! Criterion throughput bench for the canonical-Huffman fast-table decode
//! path (`boyko_image::decode_png`'s hot loop).
//!
//! Drives the PUBLIC API only (`decode_png`) against a synthetically
//! generated fixed-Huffman-compressed PNG — fixed Huffman still exercises
//! the same root+subtable fast-lookup decode loop as dynamic Huffman (only
//! the code-length assignment differs), so this measures the
//! perf-critical piece: canonical-Huffman symbol decode + LZ77 copy +
//! per-scanline unfilter, end to end.
//!
//! This crate has zero third-party deps outside `dev-dependencies`, so there
//! is no in-crate "naive bit-by-bit tree walk" baseline to A/B against
//! without shipping a second, otherwise-unused decoder purely for this
//! bench — throughput here is reported as an absolute (GiB/s), not a
//! ratio.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

use boyko_image::decode_png;

/// Reverses the low `n` bits of `v` (RFC 1951 §3.1.1: Huffman codes are
/// conceptually MSB-first, but the bitstream itself is packed LSB-first).
fn reverse_bits(v: u32, n: u32) -> u32 {
    v.reverse_bits() >> (32 - n)
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

/// Minimal LSB-first bit writer, mirroring `png.rs`'s test-only one.
struct BitWriter {
    bytes: Vec<u8>,
    acc: u32,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self { bytes: Vec::new(), acc: 0, nbits: 0 }
    }

    fn put_bits(&mut self, value: u32, n: u32) {
        let mask = (1u32.checked_shl(n).unwrap_or(0)).wrapping_sub(1);
        self.acc |= (value & mask) << self.nbits;
        self.nbits += n;
        while self.nbits >= 8 {
            self.bytes.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }

    fn put_huffman_msb(&mut self, code: u32, n: u32) {
        self.put_bits(reverse_bits(code, n), n);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.bytes.push((self.acc & 0xFF) as u8);
        }
        self.bytes
    }
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
const CRC32_TABLE: [u32; 256] = crc32_table();

/// A mismatched CRC/Adler-32 is only a non-fatal WARNING in the decoder
/// (logged via `eprintln!`) — but that makes it load-bearing for a clean
/// bench: an incorrect checksum would hit that `eprintln!` path on every
/// single timed iteration, and the resulting I/O would dominate (and
/// badly skew) the measured throughput. Fixtures here always carry correct
/// checksums so the timed loop measures decode, not stderr writes.
fn crc32_chunk(kind: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in kind.iter().chain(data.iter()) {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    const NMAX: usize = 5552;
    for group in data.chunks(NMAX) {
        for &byte in group {
            a += byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + data.len() + 4);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32_chunk(kind, data).to_be_bytes());
    out
}

/// Builds a `size x size` 8-bit grayscale PNG whose IDAT is a single
/// fixed-Huffman (BTYPE=01) block of literal bytes (a simple deterministic
/// pseudo-random pattern — no back-references, so every byte round-trips
/// through the Huffman decode loop rather than the LZ77 copy path).
fn build_fixed_huffman_png(size: u32) -> Vec<u8> {
    let row_bytes = size as usize;
    let mut raw = Vec::with_capacity((row_bytes + 1) * size as usize);
    let mut lcg_state: u32 = 0x1234_5678;
    for _row in 0..size {
        raw.push(0); // filter type: None
        for _ in 0..row_bytes {
            // A tiny xorshift-style LCG — deterministic, dependency-free
            // "varied enough" pixel data for a representative bench.
            lcg_state ^= lcg_state << 13;
            lcg_state ^= lcg_state >> 17;
            lcg_state ^= lcg_state << 5;
            raw.push((lcg_state & 0xFF) as u8);
        }
    }

    let mut w = BitWriter::new();
    w.put_bits(1, 1); // BFINAL
    w.put_bits(0b01, 2); // BTYPE = fixed Huffman
    for &byte in &raw {
        let (code, len) = fixed_literal_code(byte as u16);
        w.put_huffman_msb(code, len);
    }
    let (eob_code, eob_len) = fixed_literal_code(256);
    w.put_huffman_msb(eob_code, eob_len);
    let deflate = w.finish();

    let mut zlib = vec![0x78, 0x01];
    zlib.extend_from_slice(&deflate);
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // bit_depth=8, color_type=0(gray), compression/filter/interlace=0

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", &zlib));
    png.extend_from_slice(&chunk(b"IEND", &[]));
    png
}

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_png_fixed_huffman");
    for &size in &[64u32, 256, 1024] {
        let png_bytes = build_fixed_huffman_png(size);
        let pixel_bytes = (size as u64) * (size as u64) * 4; // decoded RGBA size
        group.throughput(Throughput::Bytes(pixel_bytes));
        group.bench_function(format!("{size}x{size}"), |b| {
            b.iter(|| decode_png(std::hint::black_box(&png_bytes)).expect("bench fixture must decode"));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_decode);
criterion_main!(benches);
