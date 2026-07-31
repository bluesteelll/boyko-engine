//! **VG-R0 rung R0c — the host-side density reducer.**
//!
//! Turns one `vb_id` readback into the statistics a census row carries. The GPU half of R0c (the
//! ring's `TRANSFER_SRC` usage, the `Option`-threaded armed readback) hands this function a buffer
//! of `R32G32_UINT` texels; everything below is pure CPU and is exhaustively testable without a
//! device, which is why it is its own module.
//!
//! # What a row is
//!
//! A **census row** is one reading, at one `(camera path, ladder rung)` pair, of every statistic
//! R0d(b) enumerates *that is readable at that pair*. `D_est` is not among them — it divides
//! `visible_tris` at the TOP rung by `covered_pixels` at the DECISION rung, two different rungs —
//! and neither is the convergence check, which is a relation *between* rungs. Both are derived from
//! rows and reported per path. This module produces exactly the per-pair members.
//!
//! # The one design decision worth stating
//!
//! Distinct visible triangles are counted by **sorting the `(instance, primitive)` keys and
//! counting runs**, not by a hash set — `HashMap`/`HashSet` are disallowed in this workspace, and
//! the sort is the better instrument anyway: a run's *length* is exactly that triangle's covered
//! pixel count, so the histogram falls out of the same pass rather than needing a second structure.

/// A pixel the mesh raster leg never covered — the SDF leg's own hit, or the sky background. Host
/// mirror of the shader-side `VB_ID_SENTINEL` in `vb_pack.hlsli`.
pub const VB_ID_SENTINEL: u32 = 0xFFFF_FFFF;

/// The per-pair statistics of one census row.
///
/// Every field here is readable at a single `(path, rung)` pair. Anything indexed by more than one
/// rung, or by none, is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CensusRow {
    /// Texels the mesh raster leg covered — `D_est`'s denominator at the decision rung.
    pub covered_pixels: u64,
    /// Distinct `(instance_id, primitive_id)` pairs that won at least one texel — `D_est`'s
    /// numerator at the top rung.
    pub visible_tris: u64,
    /// `histogram[b]` = triangles whose covered-pixel count `c` satisfies `2^b <= c < 2^(b+1)`.
    ///
    /// ⚠️ **Left-censored at one pixel by construction**: a triangle that wins no texel is not
    /// visible and does not appear at all, so bucket 0 is the lowest occupiable bucket and the
    /// distribution cannot represent sub-pixel triangles. In the micro-polygon regime the census
    /// exists to serve, every newly visible triangle enters at bucket 0 and pushes the mode the
    /// wrong way — which is precisely why R0d reports the cross-rung shift instead of gating it.
    pub histogram: Vec<u64>,
    /// The most populated bucket, or `None` on an empty frame. Ties take the LOWER bucket, stated
    /// because a silent tie-break would be an unrecorded decision in a decision-bearing statistic.
    pub modal_bucket: Option<u32>,
}

impl CensusRow {
    /// `visible_tris / covered_pixels` — a `[k1].report_only` statistic. It **saturates at 1.0 by
    /// construction** (one winner per texel), which is why it adjudicates nothing and is reported
    /// rather than gated.
    pub fn visible_tri_per_covered_pixel(&self) -> f64 {
        if self.covered_pixels == 0 {
            return 0.0;
        }
        self.visible_tris as f64 / self.covered_pixels as f64
    }

    /// `submitted / covered_pixels` — the other `[k1].report_only` statistic, a cull-efficiency
    /// reading. `submitted` counts culled and off-screen geometry, so it is an upper bound that
    /// bounds nothing tightly; it comes from the draw path, not from the readback.
    pub fn submitted_per_covered_pixel(&self, submitted_tris: u64) -> f64 {
        if self.covered_pixels == 0 {
            return 0.0;
        }
        submitted_tris as f64 / self.covered_pixels as f64
    }

    /// Whether this row clears the non-degeneracy floors R0c(c′) and R0d(c) assert.
    ///
    /// Both floors exist because `D_est` and the convergence check are **divisions**: on a
    /// sentinel-only readback `visible_tris = 0`, the convergence check reads `0 <= 0` (converged)
    /// and `D_est = 0`, so an empty frame satisfied K1's fire condition in an earlier revision.
    /// A frame that cannot be adjudicated must be refused, not divided by.
    pub fn is_non_degenerate(&self, min_covered_pixels: u64, min_visible_tris: u64) -> bool {
        self.covered_pixels >= min_covered_pixels && self.visible_tris >= min_visible_tris
    }
}

/// Reduces one `vb_id` readback into a [`CensusRow`].
///
/// `texels` is the raw `R32G32_UINT` buffer in row-major order: `.0` is `instance_id`, `.1` is the
/// raw primitive id. Texels carrying [`VB_ID_SENTINEL`] in `.0` are not mesh-covered and are
/// excluded from every statistic — the census's denominator is **mesh-covered pixels**, not all
/// pixels.
pub fn reduce(texels: &[[u32; 2]]) -> CensusRow {
    // One key per covered texel: `(instance << 32) | primitive`. Packing into a `u64` makes the
    // sort a single scalar compare and keeps the working set half the size of a tuple sort.
    let mut keys: Vec<u64> = Vec::with_capacity(texels.len());
    for t in texels {
        if t[0] != VB_ID_SENTINEL {
            keys.push(((t[0] as u64) << 32) | t[1] as u64);
        }
    }
    let covered_pixels = keys.len() as u64;
    if covered_pixels == 0 {
        return CensusRow::default();
    }

    keys.sort_unstable();

    // A run of equal keys is one triangle; the run's LENGTH is its covered-pixel count, so the
    // distinct count and the histogram come out of the same pass.
    let mut histogram: Vec<u64> = Vec::new();
    let mut visible_tris = 0u64;
    let mut run_start = 0usize;
    for i in 1..=keys.len() {
        if i == keys.len() || keys[i] != keys[run_start] {
            visible_tris += 1;
            let run = (i - run_start) as u64;
            let bucket = run.ilog2() as usize;
            if histogram.len() <= bucket {
                histogram.resize(bucket + 1, 0);
            }
            histogram[bucket] += 1;
            run_start = i;
        }
    }

    // Ties take the LOWER bucket: `>` rather than `>=` keeps the first maximum.
    let modal_bucket = histogram
        .iter()
        .enumerate()
        .fold(None::<(usize, u64)>, |best, (b, &n)| match best {
            Some((_, bn)) if n <= bn => best,
            _ if n == 0 => best,
            _ => Some((b, n)),
        })
        .map(|(b, _)| b as u32);

    CensusRow { covered_pixels, visible_tris, histogram, modal_bucket }
}

/// The SHA-256 round constants (first 32 bits of the fractional parts of the cube roots of the
/// first 64 primes).
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// A STREAMING SHA-256 — the digest R0d(a) compares across processes.
///
/// Streaming rather than one-shot because `[census].readback_retention` is `"stream_and_hash"`, and
/// that is an operational requirement, not a style preference: the top ladder rung's readback is
/// 3840 × 2160 × 8 B = 66.4 MB, and the workspace's other in-house SHA-256
/// (`smaa_luts.rs`, `#[cfg(test)]`) begins `data.to_vec()` — a full second copy, so a one-shot hash
/// of a top-rung readback would peak at ~133 MB against the disk-and-memory pressure the plan's
/// environment record already flags. This one holds 64 bytes.
#[derive(Debug, Clone)]
pub struct Sha256 {
    h: [u32; 8],
    /// Bytes accepted but not yet compressed — always fewer than 64.
    tail: [u8; 64],
    tail_len: usize,
    /// Total bytes accepted, for the big-endian length suffix.
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// A fresh hasher seeded with the FIPS 180-4 initial state.
    pub fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            tail: [0u8; 64],
            tail_len: 0,
            total: 0,
        }
    }

    /// Accepts the next chunk. Chunk boundaries do not affect the digest.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.tail_len > 0 {
            let want = (64 - self.tail_len).min(data.len());
            self.tail[self.tail_len..self.tail_len + want].copy_from_slice(&data[..want]);
            self.tail_len += want;
            data = &data[want..];
            if self.tail_len < 64 {
                // The chunk did not complete the buffered block — leave it buffered. Falling
                // through would reach the `remainder()` store below and OVERWRITE `tail_len` with
                // 0, silently dropping the partial block.
                debug_assert!(data.is_empty(), "invariant: a short fill consumed the whole chunk");
                return;
            }
            let block = self.tail;
            self.compress(&block);
            self.tail_len = 0;
        }
        let mut chunks = data.chunks_exact(64);
        for block in &mut chunks {
            let mut b = [0u8; 64];
            b.copy_from_slice(block);
            self.compress(&b);
        }
        let rest = chunks.remainder();
        self.tail[..rest.len()].copy_from_slice(rest);
        self.tail_len = rest.len();
    }

    /// Pads, compresses the final block(s) and returns the lowercase hex digest.
    pub fn finish_hex(mut self) -> String {
        let bit_len = self.total.wrapping_mul(8);
        // Pad: 0x80, zeros to a 56-mod-64 boundary, then the 64-bit BIG-ENDIAN bit length. At most
        // two further blocks, so the padding is applied through `update`'s own buffering.
        let mut pad = [0u8; 72];
        pad[0] = 0x80;
        let zeros = (56usize + 64 - ((self.tail_len + 1) % 64)) % 64;
        let end = 1 + zeros;
        pad[end..end + 8].copy_from_slice(&bit_len.to_be_bytes());
        // `update` re-adds these to `total`, which is already consumed above — harmless, the value
        // is never read again.
        self.update(&pad[..end + 8]);
        debug_assert_eq!(self.tail_len, 0, "invariant: padding lands on a block boundary");

        let mut out = String::with_capacity(64);
        for word in self.h {
            for byte in word.to_be_bytes() {
                out.push(char::from_digit((byte >> 4) as u32, 16).expect("invariant: nibble < 16"));
                out.push(char::from_digit((byte & 0xf) as u32, 16).expect("invariant: nibble < 16"));
            }
        }
        out
    }

    /// One 64-byte block through the compression function.
    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = self.h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` texels all belonging to one triangle.
    fn tri(instance: u32, prim: u32, n: usize) -> Vec<[u32; 2]> {
        vec![[instance, prim]; n]
    }

    #[test]
    fn a_sentinel_only_readback_is_degenerate_rather_than_zero_density() {
        let row = reduce(&vec![[VB_ID_SENTINEL, 0]; 4096]);
        assert_eq!(row.covered_pixels, 0);
        assert_eq!(row.visible_tris, 0);
        assert_eq!(row.modal_bucket, None, "an empty frame has no mode to report");
        assert!(
            !row.is_non_degenerate(1024, 1024),
            "the whole point of the floors: an empty frame must be REFUSED, not divided by"
        );
        // And the report-only ratios must not divide by zero.
        assert_eq!(row.visible_tri_per_covered_pixel(), 0.0);
        assert_eq!(row.submitted_per_covered_pixel(5_000), 0.0);
    }

    #[test]
    fn sentinel_texels_are_excluded_from_the_denominator() {
        let mut t = tri(1, 7, 100);
        t.extend(vec![[VB_ID_SENTINEL, 0]; 900]);
        let row = reduce(&t);
        assert_eq!(row.covered_pixels, 100, "the denominator is MESH-covered pixels, not all pixels");
        assert_eq!(row.visible_tris, 1);
    }

    #[test]
    fn distinct_triangles_are_counted_across_instances_and_primitives() {
        let mut t = tri(0, 0, 3);
        t.extend(tri(0, 1, 3)); // same instance, different primitive
        t.extend(tri(1, 0, 3)); // different instance, SAME primitive id
        let row = reduce(&t);
        assert_eq!(
            row.visible_tris, 3,
            "the key is the PAIR — a primitive id is only unique within its instance"
        );
        assert_eq!(row.covered_pixels, 9);
    }

    /// The property R0c gate (b) rests on: with an analytically known screen-space triangle size,
    /// the modal bucket IS the analytic bucket.
    #[test]
    fn the_modal_bucket_is_the_analytic_bucket() {
        for px in [1u32, 2, 3, 4, 7, 8, 32, 1024] {
            let mut t = Vec::new();
            for p in 0..64u32 {
                t.extend(tri(0, p, px as usize));
            }
            let row = reduce(&t);
            assert_eq!(
                row.modal_bucket,
                Some(px.ilog2()),
                "{px} px/triangle must land in bucket floor(log2({px}))"
            );
        }
    }

    /// R0c(b)'s named red mutation, as arithmetic: subdividing the fixture 4x quarters each
    /// triangle's area, so the mode must move by exactly TWO buckets. A control that only asserts
    /// "the number changed" is the defect this campaign keeps finding — the required DIRECTION and
    /// MAGNITUDE is what makes it a gate.
    #[test]
    fn a_four_fold_subdivision_moves_the_mode_by_exactly_two_buckets() {
        let coarse: Vec<[u32; 2]> = (0..64u32).flat_map(|p| tri(0, p, 64)).collect();
        // Same total coverage, four times the triangles, a quarter of the area each.
        let fine: Vec<[u32; 2]> = (0..256u32).flat_map(|p| tri(0, p, 16)).collect();

        let c = reduce(&coarse);
        let f = reduce(&fine);
        assert_eq!(c.covered_pixels, f.covered_pixels, "the fixture covers the same area");
        assert_eq!(
            c.modal_bucket.unwrap() - f.modal_bucket.unwrap(),
            2,
            "4x subdivision must move the mode DOWN by two buckets, not merely move it"
        );
    }

    #[test]
    fn the_histogram_is_left_censored_at_one_pixel() {
        // A triangle winning zero texels is not in the readback at all, so bucket 0 is the floor.
        let row = reduce(&(0..10u32).flat_map(|p| tri(0, p, 1)).collect::<Vec<_>>());
        assert_eq!(row.modal_bucket, Some(0));
        assert_eq!(row.histogram[0], 10);
        assert_eq!(row.histogram.len(), 1, "nothing can occupy a bucket below 0");
    }

    #[test]
    fn the_raw_ratio_saturates_at_one_which_is_why_it_adjudicates_nothing() {
        // Every triangle winning exactly one texel is the densest readable case.
        let row = reduce(&(0..500u32).flat_map(|p| tri(0, p, 1)).collect::<Vec<_>>());
        assert_eq!(
            row.visible_tri_per_covered_pixel(),
            1.0,
            "one winner per texel caps this statistic at 1.0 by construction"
        );
        // The cull-efficiency reading is NOT capped — it counts submitted geometry.
        assert!(row.submitted_per_covered_pixel(5_000) > 1.0);
    }

    #[test]
    fn a_tie_takes_the_lower_bucket() {
        let mut t: Vec<[u32; 2]> = (0..4u32).flat_map(|p| tri(0, p, 2)).collect(); // bucket 1
        t.extend((10..14u32).flat_map(|p| tri(0, p, 8))); // bucket 3, same count
        let row = reduce(&t);
        assert_eq!(row.histogram[1], 4);
        assert_eq!(row.histogram[3], 4);
        assert_eq!(row.modal_bucket, Some(1), "a tie must resolve the way the doc says it does");
    }

    #[test]
    fn the_non_degeneracy_floors_bind_both_terms() {
        // Above the pixel floor, below the triangle floor: one huge triangle.
        let row = reduce(&tri(0, 0, 4096));
        assert!(row.covered_pixels >= 1024);
        assert!(!row.is_non_degenerate(1024, 1024), "the triangle floor must bind on its own");
    }

    fn hash(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        h.finish_hex()
    }

    #[test]
    fn sha256_matches_the_fips_known_answers() {
        // The published digests — this hasher is validated against the standard, not against a
        // value it produced itself.
        assert_eq!(
            hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 56 bytes: the length suffix lands in a SECOND block, the padding branch that a one-block
        // test never reaches.
        assert_eq!(
            hash(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn the_digest_is_independent_of_chunk_boundaries() {
        // The whole point of streaming: R0d(a) compares digests across PROCESSES, so a digest that
        // depended on how the readback happened to be sliced would compare noise.
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let want = hash(&data);
        // Every split that exercises a distinct branch: shorter than a block (buffers), exactly a
        // block, spanning a block boundary, and a short fill that must NOT clear the tail.
        for chunk in [1usize, 7, 63, 64, 65, 100, 128, 999] {
            let mut h = Sha256::new();
            for part in data.chunks(chunk) {
                h.update(part);
            }
            assert_eq!(h.finish_hex(), want, "chunking by {chunk} changed the digest");
        }
    }

    #[test]
    fn every_tail_length_pads_onto_a_block_boundary() {
        // The pad arithmetic is `(56 - (tail_len + 1)) mod 64`, and `tail_len == 63` is the case
        // that wraps into a second block. Exercising all 64 residues costs nothing and is the only
        // way this is a check rather than a spot sample.
        for n in 0..200usize {
            let data = vec![0xa5u8; n];
            let mut streamed = Sha256::new();
            streamed.update(&data);
            let mut byte_at_a_time = Sha256::new();
            for b in &data {
                byte_at_a_time.update(core::slice::from_ref(b));
            }
            assert_eq!(
                streamed.finish_hex(),
                byte_at_a_time.finish_hex(),
                "length {n} disagrees between one-shot and byte-at-a-time"
            );
        }
    }
}
