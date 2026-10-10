//! Streaming SHA-256 (FIPS 180-4) for receipts: object and image digests, normalised-body digests.
//!
//! The same algorithm as `boyko_render::vg_census::Sha256`, duplicated rather than shared: a
//! dependency on `boyko_render` would put the render graph under a dev tool whose whole subject is
//! what the build links. Checked against `sha256sum` on a real object at B3 (probe receipt).

use std::io::Read;
use std::path::Path;

use crate::red::{Red, Result};

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H0: [u32; 8] =
    [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

/// A streaming hasher; chunk boundaries do not affect the digest.
#[derive(Clone, Debug)]
pub struct Sha256 {
    h: [u32; 8],
    tail: [u8; 64],
    tail_len: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// A fresh hasher.
    #[must_use]
    pub const fn new() -> Self {
        Self { h: H0, tail: [0u8; 64], tail_len: 0, total: 0 }
    }

    /// Accepts the next chunk.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.tail_len > 0 {
            let want = (64 - self.tail_len).min(data.len());
            self.tail[self.tail_len..self.tail_len + want].copy_from_slice(&data[..want]);
            self.tail_len += want;
            data = &data[want..];
            if self.tail_len < 64 {
                return;
            }
            let block = self.tail;
            self.compress(&block);
            self.tail_len = 0;
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            self.compress(block);
        }
        self.tail[..rest.len()].copy_from_slice(rest);
        self.tail_len = rest.len();
    }

    /// Pads, finishes and returns the lowercase hex digest.
    #[must_use]
    pub fn finish_hex(mut self) -> String {
        let bit_len = self.total.wrapping_mul(8);
        let mut pad = [0u8; 72];
        pad[0] = 0x80;
        let zeros = (56usize + 64 - ((self.tail_len + 1) % 64)) % 64;
        let end = 1 + zeros;
        pad[end..end + 8].copy_from_slice(&bit_len.to_be_bytes());
        self.update(&pad[..end + 8]);
        debug_assert_eq!(self.tail_len, 0, "invariant: padding lands on a block boundary");
        let mut out = String::with_capacity(64);
        for word in self.h {
            for byte in word.to_be_bytes() {
                out.push(char::from_digit(u32::from(byte >> 4), 16).expect("invariant: nibble < 16"));
                out.push(char::from_digit(u32::from(byte & 0xf), 16).expect("invariant: nibble < 16"));
            }
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = self.h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in self.h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
}

/// The hex digest of `bytes`.
#[must_use]
pub fn bytes_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finish_hex()
}

/// The hex digest of the file at `path`, read in 1 MiB chunks.
pub fn file_hex(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|e| Red::io(path, &e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| Red::io(path, &e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finish_hex())
}
