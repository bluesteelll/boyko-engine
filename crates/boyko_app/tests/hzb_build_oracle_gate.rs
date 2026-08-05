//! VG R3 piece 1, step P1-7 — **gate G3: the HZB build shader equals the host oracle,
//! bit-exactly, with no engine involved.**
//!
//! The pyramid's only float operation is `min` — an exact selection, no rounding, no ULP
//! question — so "the shader agrees with `boyko_render::hzb::build_pyramid`" is a DECIDABLE
//! demand rather than an ambitious one. This file decides it, at `to_bits()` fidelity, at every
//! texel of every level of seven source extents.
//!
//! # Why the file lives in `boyko_app`
//!
//! The oracle is `boyko_render::hzb`; the shader and its `.spv` are `boyko_rhi_vulkan`. The
//! dependency runs `boyko_render → boyko_rhi_vulkan`, never the reverse, so a test inside
//! `boyko_rhi_vulkan/tests/` cannot name the oracle. `boyko_app` is one of only two crates that
//! name both (`crates/boyko_app/Cargo.toml:17-21`), which is what forces the bit-exact comparison
//! to run here (plan §4).
//!
//! # NO ENGINE IS INVOLVED, and that is the point (plan §5)
//!
//! This gate creates its own depth image, its own pattern, its own per-mip views, its own
//! descriptor-set layout, its own pipeline and its own readback. It does NOT touch `HzbTargets`,
//! `GBufferScene`, the framegraph or the runner. The ONLY thing it shares with the engine is the
//! compiled `hzb_build.comp.spv`, which is the thing under test: a wiring bug the engine and the
//! gate had in common would otherwise cancel out and the gate would certify it.
//!
//! For the same reason the BARRIERS here are hand-written rather than framegraph-derived, and they
//! are deliberately coarser than the engine's will be (whole-image ranges). G3 is testing the
//! SHADER, not the barrier derivation, and hand-writing them is what lets this gate land before the
//! framegraph work does.
//!
//! # Non-vacuity is ASSERTED, not hoped for
//!
//! Byte-agreement with an oracle proves nothing if the shader never ran and the destination
//! happened to hold the right thing. So every mip of the pyramid is POISONED with `-1.0` before the
//! dispatch, and the readback asserts that no texel still holds it. `-1.0` cannot be a legitimate
//! output: the reduce is a `min` over a pattern in `(0, 1]` (or `±0.0` in the signed-zero case) and
//! every written texel has at least one live child, so no written texel is `+INFINITY` either.
//!
//! The poison buffer and the readback buffer are ONE buffer, which sharpens that: "the dispatch
//! never wrote" and "the readback copy never happened" both surface as the same `-1.0` assertion
//! instead of one of them being invisible.
//!
//! # Run
//!
//! `cargo test -p boyko-app --test hzb_build_oracle_gate -- --ignored --nocapture
//! --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1`.

use core::ptr::NonNull;

use boyko_rhi::{
    BarrierAccess, BarrierDesc, BarrierStage, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc,
    BindGroupLayoutEntry, BufferBarrier, BufferDesc, BufferImageCopy, BufferUsage,
    ComputePipelineDesc, DescriptorKind, Format, ImageAspect, ImageBarrierDesc, ImageLayout,
    ImageSubresourceRange, ImageUsage, MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue,
    SamplerDesc, ShaderStage, TextureDesc, TextureDimension, TextureViewDesc,
};
use boyko_rhi_vulkan::compute::{
    HZB_BUILD_PUSH_BYTES, HZB_BUILD_TILE, HZB_LEVELS_PER_PASS, MAX_HZB_PASSES, hzb_build_spirv,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

use boyko_render::hzb::{HzbLayout, build_pyramid};

// ==============================================================================================
// The push block — mirrored FIELD FOR FIELD from `hzb_build.comp.hlsl`'s `HzbBuildPush`
// ==============================================================================================

/// The 72-byte `hzb_build` push constant, in the shader's own member order.
///
/// The six destination extents are six SEPARATE fields rather than an array, exactly as the HLSL
/// spells them, so a reader can diff this declaration one-for-one against
/// `crates/boyko_rhi_vulkan/shaders/hzb_build.comp.hlsl` instead of counting subscripts.
#[repr(C)]
#[derive(Clone, Copy)]
struct HzbBuildPush {
    /// `S` — the SOURCE depth extent. ⚠️ NOT level 0's extent; see [`pass_push`].
    src_extent: [u32; 2],
    /// `E(d-1)`, the level this pass reduces FROM. Read only by a reduce pass.
    fine_extent: [u32; 2],
    /// `E(d)` — and on the base pass also `P = prev_pow2(S)` per axis.
    out_extent0: [u32; 2],
    /// `E(d+1)`.
    out_extent1: [u32; 2],
    /// `E(d+2)`.
    out_extent2: [u32; 2],
    /// `E(d+3)`.
    out_extent3: [u32; 2],
    /// `E(d+4)`.
    out_extent4: [u32; 2],
    /// `E(d+5)`.
    out_extent5: [u32; 2],
    /// `d` — the pass's first output level, and the variant discriminator (`0` ⇔ BASE).
    base_level: u32,
    /// How many levels THIS pass writes, in `1 ..= HZB_LEVELS_PER_PASS`.
    level_count: u32,
}

// The layout the shader's `OpMemberDecorate ... Offset` sequence declares, pinned host-side:
// eight `uint2` at 0,8,…,56 then two `uint` at 64 and 68.
const _: () = assert!(size_of::<HzbBuildPush>() == 72);
const _: () = assert!(size_of::<HzbBuildPush>() == HZB_BUILD_PUSH_BYTES as usize);

impl HzbBuildPush {
    /// Serializes the block to its 72 wire bytes, little-endian, field by field.
    ///
    /// Hand-serialized rather than transmuted: the byte offsets are the contract with the HLSL,
    /// and writing them out is what makes them reviewable.
    fn to_bytes(self) -> [u8; HZB_BUILD_PUSH_BYTES as usize] {
        let words: [u32; HZB_BUILD_PUSH_BYTES as usize / 4] = [
            self.src_extent[0],
            self.src_extent[1],
            self.fine_extent[0],
            self.fine_extent[1],
            self.out_extent0[0],
            self.out_extent0[1],
            self.out_extent1[0],
            self.out_extent1[1],
            self.out_extent2[0],
            self.out_extent2[1],
            self.out_extent3[0],
            self.out_extent3[1],
            self.out_extent4[0],
            self.out_extent4[1],
            self.out_extent5[0],
            self.out_extent5[1],
            self.base_level,
            self.level_count,
        ];
        let mut bytes = [0u8; HZB_BUILD_PUSH_BYTES as usize];
        for (i, w) in words.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        bytes
    }
}

/// Builds pass `p`'s push block from the layout — the host half of the contract.
///
/// ⚠️ `src_extent` is the SOURCE extent `[W, H]` on EVERY pass, not `level_extent(0)`. Level 0 is
/// `prev_pow2` of each source axis, so at 1920×1080 the source is 1920×1080 while level 0 is
/// 1024×1024, and the base map `first(t) = ⌈t·S/P⌉` reads BOTH. The two coincide only when
/// `S == P` (the `8 × 16` row of the sweep) and at extents small enough to hide it, which is
/// exactly why the sweep contains rows where they differ.
fn pass_push(layout: &HzbLayout, d: u32, n: u32) -> HzbBuildPush {
    let out = |k: u32| layout.level_extent(if k < n { d + k } else { d });
    HzbBuildPush {
        src_extent: [layout.x().source(), layout.y().source()],
        // `E(d-1)` on a reduce pass; on the BASE pass mip `d-1` does not exist and the shader's
        // `pc.base_level == 0` arm never reads this, so level 0's own extent is bound as a
        // well-defined placeholder rather than a zero that would divide.
        fine_extent: layout.level_extent(d.saturating_sub(1)),
        out_extent0: out(0),
        out_extent1: out(1),
        out_extent2: out(2),
        out_extent3: out(3),
        out_extent4: out(4),
        out_extent5: out(5),
        base_level: d,
        level_count: n,
    }
}

// ==============================================================================================
// The pattern
// ==============================================================================================

/// `2^23`. The noise takes `k / NOISE_DEN` for `k` in `1 ..= 2^23`: every value is inside
/// `(0, 1]`, is EXACTLY representable in f32 (a 23-bit numerator over a power of two), and there
/// are 8.4 million of them — so two different footprints almost never share a minimum by accident,
/// which is what makes a wrong-footprint bug show up as a mismatch rather than as a coincidence.
const NOISE_DEN: f32 = 8_388_608.0;

/// The value the planted tie block holds. Any value inside the noise range works; `0.5` is chosen
/// so it is recognisable in a failure dump.
const TIE_VALUE: f32 = 0.5;

/// `2^-24` — strictly below every noise value (whose floor is `2^-23`) and still inside `(0, 1]`,
/// so the planted global minimum is UNIQUE and propagates to the top of every level.
const GLOBAL_MIN: f32 = 1.0 / 16_777_216.0;

/// The poison every pyramid texel holds before the dispatch. It cannot be a legitimate output —
/// see the module header.
const POISON: f32 = -1.0;

/// A deterministic 32-bit mix of `(x, y, W, H)` — no RNG, no clock, and keyed by the EXTENT so two
/// different extents never share a pattern (a stale or wrong-extent upload cannot pass).
///
/// The finalizer is Murmur3's `fmix32`, used for its avalanche and nothing else.
fn mix(x: u32, y: u32, w: u32, h: u32) -> u32 {
    let mut v = x
        .wrapping_mul(0x9E37_79B1)
        ^ y.wrapping_mul(0x85EB_CA77)
        ^ w.wrapping_mul(0xC2B2_AE3D)
        ^ h.wrapping_mul(0x27D4_EB2F);
    v ^= v >> 16;
    v = v.wrapping_mul(0x85EB_CA6B);
    v ^= v >> 13;
    v = v.wrapping_mul(0xC2B2_AE35);
    v ^= v >> 16;
    v
}

/// One noise depth in `(0, 1]`, exactly representable in f32.
fn noise_depth(x: u32, y: u32, w: u32, h: u32) -> f32 {
    let k = (mix(x, y, w, h) & 0x007F_FFFF) + 1;
    k as f32 / NOISE_DEN
}

/// THE SWEEP PATTERN: reverse-Z depths in `(0, 1]`, plus two deliberate plants.
///
/// * **A TIE BLOCK.** A 2×2 (or 2×1, where the level is too short) block of LEVEL-0 texels is
///   forced to one value by filling its exact source preimage — computed through the oracle's own
///   `first_source`, so the plant lands on whole texels at every extent rather than on whichever
///   pixels a fixed rectangle happens to cover. The level-1 fold above it is then a genuine tie,
///   which is what exercises the shader's balanced-tree association against the oracle's left fold
///   (`hzb_build.comp.hlsl`, `hzb_fold_lds`). Equal-valued ties cannot DISTINGUISH the two shapes
///   at `to_bits()` — only `±0.0` can, and that is what
///   [`hzb_build_signed_zero_tie_order_to_bits`] is for — but they do exercise the path.
/// * **THE GLOBAL MINIMUM**, unique and in the INTERIOR rather than at a corner, so it must travel
///   the whole chain to the 1×1 top. It is placed at a level-0 texel strictly LEFT of the tie
///   block (`first_source` is strictly increasing), so the two plants cannot overlap. The one
///   exception is the `1 × 1` extent, where the single pixel IS the corner and there is nothing to
///   tie.
fn sweep_depth(layout: &HzbLayout) -> Vec<f32> {
    let (ax, ay) = (layout.x(), layout.y());
    let (w, h) = (ax.source(), ay.source());
    let mut depth = vec![0.0f32; layout.source_len()];
    for y in 0..h {
        let row = y as usize * w as usize;
        for x in 0..w {
            depth[row + x as usize] = noise_depth(x, y, w, h);
        }
    }

    let [w0, h0] = layout.level_extent(0);
    if w0 >= 4 {
        // Anchored at an EVEN texel index on each axis so the block is exactly one level-1
        // texel's footprint (`w0` is a power of two ≥ 4, so `w0 / 2` is even).
        let bw = 2;
        let bh = if h0 >= 4 { 2 } else { 1 };
        let tx0 = w0 / 2;
        let ty0 = if h0 >= 4 { h0 / 2 } else { 0 };
        let (x_lo, x_hi) = (ax.first_source(tx0), ax.first_source(tx0 + bw));
        let (y_lo, y_hi) = (ay.first_source(ty0), ay.first_source(ty0 + bh));
        for y in y_lo..y_hi {
            let row = y as usize * w as usize;
            for x in x_lo..x_hi {
                depth[row + x as usize] = TIE_VALUE;
            }
        }
    }

    let gx = ax.first_source(w0 / 4);
    let gy = ay.first_source(h0 / 2);
    depth[gy as usize * w as usize + gx as usize] = GLOBAL_MIN;
    depth
}

/// THE SIGNED-ZERO PATTERN, `8 × 8` only: two 2×2 level-0 blocks carrying `+0.0` and `-0.0` in
/// OPPOSITE orders.
///
/// `S == P == 8`, so level 0 is the identity on the source and level 1 folds a literal 2×2 of
/// source pixels. Block `(1,1)` gets `+0.0` at its FIRST operand and `-0.0` at its last; block
/// `(2,2)` gets the reverse. Both fold shapes under test compute "the earliest minimal element in
/// program order", so the correct answers are `+0.0` and `-0.0` respectively — two DIFFERENT bit
/// patterns from the same multiset of inputs, which is the only way the association is observable
/// at all (`+0.0 == -0.0` compares TRUE, so only `to_bits()` can see it; that is why every
/// comparison in this file is on bits).
fn signed_zero_depth(layout: &HzbLayout) -> Vec<f32> {
    let (w, h) = (layout.x().source(), layout.y().source());
    assert_eq!([w, h], [8, 8], "invariant: the signed-zero fixture is the 8x8 extent");
    let mut depth = vec![0.0f32; layout.source_len()];
    for y in 0..h {
        let row = y as usize * w as usize;
        for x in 0..w {
            depth[row + x as usize] = noise_depth(x, y, w, h);
        }
    }
    // Spelled by BIT PATTERN so the sign of each zero is unambiguous in the source.
    let pos_zero = f32::from_bits(0x0000_0000);
    let neg_zero = f32::from_bits(0x8000_0000);
    depth[2 * 8 + 2] = pos_zero; // level-1 texel (1,1), operand 0
    depth[3 * 8 + 3] = neg_zero; // level-1 texel (1,1), operand 3
    depth[4 * 8 + 4] = neg_zero; // level-1 texel (2,2), operand 0
    depth[5 * 8 + 5] = pos_zero; // level-1 texel (2,2), operand 3
    depth
}

// ==============================================================================================
// The HAND-COMPUTED expectation table
// ==============================================================================================

/// One row of the hand-computed table every extent is checked against.
///
/// Every number here was worked out with a pencil from `prev_pow2` / `max(1, base >> k)` /
/// `ceil(levels / HZB_LEVELS_PER_PASS)` / `ceil(E(d) / HZB_BUILD_TILE)` and is compared against
/// what the oracle and the dispatch arithmetic produce. Re-deriving them from the same formulas
/// would assert only that the code equals itself.
struct HandRow {
    /// The source extent `[W, H]`.
    extent: [u32; 2],
    /// `prev_pow2` per axis — level 0's extent.
    base: [u32; 2],
    /// `msb(max(base)) + 1`.
    levels: u32,
    /// The flat `f32` count of the whole chain.
    pyramid_len: usize,
    /// `levels.div_ceil(HZB_LEVELS_PER_PASS)`.
    pass_count: usize,
    /// Per pass: `[d, n, groups_x, groups_y]`. Slots past `pass_count` are unused zeros.
    passes: [[u32; 4]; MAX_HZB_PASSES],
    /// What this extent is in the sweep FOR.
    why: &'static str,
}

/// THE SWEEP. Seven extents, each named for the structure it fires.
const SWEEP: [HandRow; 7] = [
    HandRow {
        extent: [7, 3],
        base: [4, 2],
        levels: 3,
        // 4·2 + 2·1 + 1·1
        pyramid_len: 11,
        pass_count: 1,
        passes: [[0, 3, 1, 1], [0; 4], [0; 4]],
        why: "odd on both axes; prev_pow2 bites on both (P = 4x2); the first_source partition \
              {0,1}{2,3}{4,5}{6}",
    },
    HandRow {
        extent: [8, 16],
        base: [8, 16],
        levels: 5,
        // 128 + 32 + 8 + 2 + 1
        pyramid_len: 171,
        pass_count: 1,
        passes: [[0, 5, 1, 1], [0; 4], [0; 4]],
        why: "S == P on both axes, so the base map degenerates to the identity: a base-map bug \
              that only shows when S != P is ABSENT here and this row isolates the reduce",
    },
    HandRow {
        extent: [1, 1],
        base: [1, 1],
        levels: 1,
        pyramid_len: 1,
        pass_count: 1,
        passes: [[0, 1, 1, 1], [0; 4], [0; 4]],
        why: "levels == 1; every `k < level_count` guard above 0 is false; one lane, one texel",
    },
    HandRow {
        extent: [511, 1023],
        base: [256, 512],
        levels: 10,
        // 131072+32768+8192+2048+512+128+32+8+2+1
        pyramid_len: 174_763,
        pass_count: 2,
        // pass 1 reduces E(5) = [8,16] into E(6) = [4,8]
        passes: [[0, 6, 8, 16], [6, 4, 1, 1], [0; 4]],
        why: "P = 256x512; two passes; a clamped axis mid-chain (x bottoms out at level 8 while \
              y is still 2)",
    },
    HandRow {
        extent: [1920, 1080],
        base: [1024, 1024],
        levels: 11,
        // sum of 4^k, k = 0..10
        pyramid_len: 1_398_101,
        pass_count: 2,
        passes: [[0, 6, 32, 32], [6, 5, 1, 1], [0; 4]],
        why: "the real render extent; P = 1024x1024 while S = 1920x1080, so src_extent and \
              level_extent(0) differ on BOTH axes",
    },
    HandRow {
        extent: [1024, 64],
        base: [1024, 64],
        levels: 11,
        // 65536+16384+4096+1024+256+64+16+8+4+2+1
        pyramid_len: 87_391,
        pass_count: 2,
        passes: [[0, 6, 32, 2], [6, 5, 1, 1], [0; 4]],
        why: "an axis bottoming out EXACTLY at a pass boundary: E(6) = [16,1] is read from \
              fine = [32,2], so the reduce is a copy on Y at the first texel of the second pass",
    },
    HandRow {
        extent: [4096, 4096],
        base: [4096, 4096],
        levels: 13,
        // sum of 4^k, k = 0..12
        pyramid_len: 22_369_621,
        pass_count: 3,
        passes: [[0, 6, 128, 128], [6, 6, 2, 2], [12, 1, 1, 1]],
        why: "levels == 13 => THREE passes, and the third has level_count == 1, so gDst1..gDst5 \
              are bound to views it must not write. 4096 is Vulkan's guaranteed \
              maxImageDimension2D floor, which is why a 32-texel tile (not 64) makes this case \
              reachable on any conformant device",
    },
];

/// The signed-zero fixture's own row.
const ZERO_ROW: HandRow = HandRow {
    extent: [8, 8],
    base: [8, 8],
    levels: 4,
    // 64 + 16 + 4 + 1
    pyramid_len: 85,
    pass_count: 1,
    passes: [[0, 4, 1, 1], [0; 4], [0; 4]],
    why: "S == P == 8, so level 1 folds a literal 2x2 of SOURCE pixels and the planted zero signs \
          land on known fold operands",
};

// ==============================================================================================
// Device boot + raw-mapping helpers
// ==============================================================================================

/// Boots an offscreen context (validation off), or `None` with a SKIP log when no GPU/loader.
///
/// This is the ONLY reason the gate skips. Everything else — a failed allocation, a short
/// pyramid, a mismatching texel — fails the test.
fn boot_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP hzb_build_oracle_gate: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// Reads 32-bit word `i` out of a host-coherent mapping.
///
/// # Safety
///
/// `base` must point at a live host-coherent mapping of at least `(i + 1) * 4` bytes, and no GPU
/// work touching it may be in flight (the caller fence-waited). The read is UNALIGNED because a
/// sub-allocated buffer's mapping carries only the block's alignment guarantee.
unsafe fn read_word(base: NonNull<u8>, i: usize) -> u32 {
    // SAFETY: forwarded verbatim from this function's own contract — `base + i*4` is inside the
    // mapping, the bytes are stable, and `read_unaligned` imposes no alignment requirement.
    unsafe { base.as_ptr().cast::<u32>().add(i).read_unaligned() }
}

/// Fills `count` 32-bit words of a host-coherent mapping with `value`.
///
/// # Safety
///
/// `base` must point at a live host-coherent mapping of at least `count * 4` bytes, and no GPU
/// work touching it may be in flight.
unsafe fn fill_words(base: NonNull<u8>, count: usize, value: u32) {
    // SAFETY: forwarded verbatim from this function's own contract — every `base + i*4` for
    // `i < count` is inside the mapping, and `write_unaligned` imposes no alignment requirement.
    unsafe {
        let p = base.as_ptr().cast::<u32>();
        for i in 0..count {
            p.add(i).write_unaligned(value);
        }
    }
}

/// Writes the bit patterns of `values` into a host-coherent mapping.
///
/// # Safety
///
/// `base` must point at a live host-coherent mapping of at least `values.len() * 4` bytes, and no
/// GPU work touching it may be in flight.
unsafe fn write_f32s(base: NonNull<u8>, values: &[f32]) {
    // SAFETY: forwarded verbatim from this function's own contract — every written word is inside
    // the mapping, and `write_unaligned` imposes no alignment requirement.
    unsafe {
        let p = base.as_ptr().cast::<u32>();
        for (i, v) in values.iter().enumerate() {
            p.add(i).write_unaligned(v.to_bits());
        }
    }
}

// ==============================================================================================
// The gate body
// ==============================================================================================

/// Builds one extent's pyramid ON THE GPU from `pattern` and asserts it equals the host oracle at
/// every texel of every level, bit for bit.
///
/// Allocates and frees everything it needs per call, so the `4096 × 4096` row's ~300 MB never
/// coexists with the other six.
/// How a texel-level BIT mismatch is judged.
///
/// This exists because one MEASURED divergence is real and is not a shader defect — see
/// [`hzb_build_signed_zero_is_the_hardware_min_tie`]. Encoding it as a policy rather than deleting
/// the case keeps the narrow exception ASSERTED: anything outside it still fails.
#[derive(Clone, Copy)]
enum BitPolicy {
    /// Every texel must match on BITS. The sweep's policy and the NaN probe's.
    Exact,
    /// A mismatch is tolerated ONLY when BOTH sides are zeros (`+0.0` / `-0.0`) — numerically
    /// equal, differing in the sign bit alone — and at most `0` such texels may appear beyond the
    /// stated count. Anything else still fails, so this NARROWS `Exact` rather than suspending it.
    ZeroTiesExactly(usize),
}

fn run_case(
    ctx: &VulkanContext,
    row: &HandRow,
    pattern: fn(&HzbLayout) -> Vec<f32>,
    policy: BitPolicy,
) {
    let [w, h] = row.extent;
    let label = format!("{w}x{h}");
    let layout = HzbLayout::new(w, h)
        .unwrap_or_else(|e| panic!("[{label}] the extent is not a legal HZB source: {e}"));

    // ---- 0) THE HAND TABLE, checked before a single byte is allocated ------------------------
    assert_eq!(
        layout.level_extent(0),
        row.base,
        "[{label}] level 0 is not the hand-computed prev_pow2 ({})",
        row.why
    );
    assert_eq!(layout.levels(), row.levels, "[{label}] level count ({})", row.why);
    assert_eq!(layout.pyramid_len(), row.pyramid_len, "[{label}] pyramid length");
    let levels = layout.levels();
    let per_pass = HZB_LEVELS_PER_PASS;
    let pass_count = (levels as usize).div_ceil(per_pass as usize);
    assert_eq!(pass_count, row.pass_count, "[{label}] pass count ({})", row.why);
    assert!(
        pass_count <= MAX_HZB_PASSES,
        "[{label}] pass count {pass_count} exceeds the backend's MAX_HZB_PASSES capacity"
    );

    // ---- 1) the pattern and the ORACLE it is measured against ---------------------------------
    let depth = pattern(&layout);
    assert_eq!(depth.len(), layout.source_len(), "[{label}] the pattern is the wrong length");
    let mut oracle = vec![0.0f32; layout.pyramid_len()];
    build_pyramid(&layout, &depth, &mut oracle);

    // ---- 2) resources, all of them this gate's own --------------------------------------------
    let depth_bytes = (depth.len() * 4) as u64;
    let pyramid_words = layout.pyramid_len();
    let scratch_bytes = (pyramid_words * 4) as u64;

    let depth_staging = ctx
        .create_buffer(&BufferDesc {
            size: depth_bytes,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .unwrap_or_else(|e| panic!("[{label}] depth staging buffer ({depth_bytes} B): {e:?}"));
    // SAFETY: the buffer was just created host-coherent with `depth_bytes >= depth.len() * 4`
    // bytes and its persistent mapping is live; no submission has been made yet, so no GPU work
    // touches it.
    unsafe {
        write_f32s(
            ctx.buffer_mapped_ptr(&depth_staging)
                .unwrap_or_else(|| panic!("[{label}] depth staging is not host-mapped")),
            &depth,
        );
    }

    // ONE buffer for BOTH the poison source and the readback sink — see the module header for why
    // that sharpens the non-vacuity assertion rather than weakening it.
    let scratch = ctx
        .create_buffer(&BufferDesc {
            size: scratch_bytes,
            usage: BufferUsage::TRANSFER_SRC | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .unwrap_or_else(|e| panic!("[{label}] pyramid scratch buffer ({scratch_bytes} B): {e:?}"));
    let scratch_ptr = ctx
        .buffer_mapped_ptr(&scratch)
        .unwrap_or_else(|| panic!("[{label}] pyramid scratch is not host-mapped"));
    // SAFETY: the buffer was just created host-coherent with `pyramid_words * 4` bytes and its
    // persistent mapping is live; no submission has been made yet.
    unsafe { fill_words(scratch_ptr, pyramid_words, POISON.to_bits()) };

    // Three 1×1 depth probes read back after the dispatch, so a broken UPLOAD reports itself
    // instead of turning into a pyramid mismatch. Three (not one) so a wrong row pitch cannot
    // pass: they sit at distinct rows AND columns wherever the extent allows it.
    let probes = [[0u32, 0], [w / 2, h / 2], [w - 1, h - 1]];
    let probe = ctx
        .create_buffer(&BufferDesc {
            size: (probes.len() * 4) as u64,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .unwrap_or_else(|e| panic!("[{label}] depth probe buffer: {e:?}"));

    // The SOURCE depth: the real `D32_SFLOAT` the engine rasterizes into, not an `R32_SFLOAT`
    // stand-in — the DEPTH aspect is part of what this gate proves.
    //
    // ⚠️ `DEPTH_STENCIL_ATTACHMENT` is in the usage set because in THIS RHI that bit is what
    // selects a DEPTH-aspect view (`texture.rs`: `is_depth = usage & DEPTH_STENCIL_ATTACHMENT`).
    // Without it the backend would build a COLOR-aspect view of a depth format, which
    // `vkCreateImageView` rejects. It also matches the engine's own depth ring, which carries
    // `DEPTH_STENCIL_ATTACHMENT | SAMPLED`.
    let depth_tex = ctx
        .create_texture(&TextureDesc {
            width: w,
            height: h,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT
                | ImageUsage::SAMPLED
                | ImageUsage::TRANSFER_DST
                | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .unwrap_or_else(|e| panic!("[{label}] source depth image ({w}x{h} D32_SFLOAT): {e:?}"));

    // The pyramid: one `R32_SFLOAT` image with a REAL Vulkan mip chain. Vulkan's own mip rule
    // (`max(1, base >> k)`) IS the oracle's `level_extent(k)`, so the image's real extents and the
    // layout agree by construction rather than by a second derivation.
    let [base_w, base_h] = layout.level_extent(0);
    let pyramid = ctx
        .create_texture(&TextureDesc {
            width: base_w,
            height: base_h,
            depth: 1,
            format: Format::R32Sfloat,
            dimension: TextureDimension::D2,
            // TRANSFER_DST is this gate's own addition to the engine's set: it is how the poison
            // gets in without reaching for the crate-private `vkCmdClearColorImage` helper.
            usage: ImageUsage::STORAGE
                | ImageUsage::SAMPLED
                | ImageUsage::TRANSFER_SRC
                | ImageUsage::TRANSFER_DST,
            array_layers: 1,
            mip_levels: levels,
            view_format: None,
        })
        .unwrap_or_else(|e| {
            panic!("[{label}] pyramid image ({base_w}x{base_h}, {levels} mips): {e:?}")
        });

    // One SINGLE-MIP view per level — the shape a storage-image descriptor requires and the shape
    // no texture-owned view can produce.
    let mut level_views = Vec::with_capacity(levels as usize);
    for level in 0..levels {
        let view = ctx
            .create_texture_view(
                &pyramid,
                &TextureViewDesc { base_mip: level, mip_count: 1, ..TextureViewDesc::default() },
            )
            .unwrap_or_else(|e| panic!("[{label}] pyramid view for level {level}: {e:?}"));
        level_views.push(view);
    }

    let sampler = ctx
        .create_sampler(&SamplerDesc::default())
        .unwrap_or_else(|e| panic!("[{label}] depth sampler: {e:?}"));

    // The gate's OWN 8-binding set-0 layout, matching `hzb_build.comp.hlsl`'s table: SAMPLED
    // `gSrcDepth` @0, STORAGE `gFine` @1, STORAGE `gDst0`..`gDst5` @2..@7.
    let mut entries = [BindGroupLayoutEntry {
        binding: 0,
        count: 1,
        kind: DescriptorKind::SampledImage,
        stage: ShaderStage::COMPUTE,
    }; 8];
    for (i, e) in entries.iter_mut().enumerate().skip(1) {
        e.binding = i as u32;
        e.kind = DescriptorKind::StorageImage;
    }
    let set_layout = ctx
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &entries })
        .unwrap_or_else(|e| panic!("[{label}] hzb_build set layout: {e:?}"));

    let module = ctx
        .create_shader_module(hzb_build_spirv())
        .unwrap_or_else(|e| panic!("[{label}] hzb_build shader module: {e:?}"));
    let pipeline = ctx
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: HZB_BUILD_PUSH_BYTES,
            bind_group_layout: Some(&set_layout),
            spec_constants: &[],
        })
        .unwrap_or_else(|e| panic!("[{label}] hzb_build pipeline: {e:?}"));

    // One descriptor set per pass, padded exactly as `HzbTargets` pads: @1 is `level_views[d-1]`
    // (or level 0 on the base pass, which never reads it), and @2+k past this pass's `n` live
    // levels is `level_views[d]` — a real view of a real mip that the `k < pc.level_count` store
    // guard keeps unwritten.
    let mut sets = Vec::with_capacity(pass_count);
    for p in 0..pass_count {
        let d = p * per_pass as usize;
        let n = (levels as usize - d).min(per_pass as usize);
        let fine = &level_views[d.saturating_sub(1)];
        // The LEVEL each destination binding names — the padding rule spelled as indices so no
        // view type has to be named at the call site.
        let dst: [usize; HZB_LEVELS_PER_PASS as usize] =
            core::array::from_fn(|k| if k < n { d + k } else { d });
        let group = ctx
            .create_bind_group(&BindGroupDesc {
                layout: &set_layout,
                entries: &[
                    BindGroupEntry::SampledImage { texture: &depth_tex, sampler: &sampler },
                    BindGroupEntry::StorageImageView { view: fine },
                    BindGroupEntry::StorageImageView { view: &level_views[dst[0]] },
                    BindGroupEntry::StorageImageView { view: &level_views[dst[1]] },
                    BindGroupEntry::StorageImageView { view: &level_views[dst[2]] },
                    BindGroupEntry::StorageImageView { view: &level_views[dst[3]] },
                    BindGroupEntry::StorageImageView { view: &level_views[dst[4]] },
                    BindGroupEntry::StorageImageView { view: &level_views[dst[5]] },
                ],
            })
            .unwrap_or_else(|e| panic!("[{label}] hzb_build descriptor set for pass {p}: {e:?}"));
        sets.push(group);
    }

    // ---- 3) record ----------------------------------------------------------------------------
    let fence = ctx.create_fence(false).unwrap_or_else(|e| panic!("[{label}] fence: {e:?}"));
    let mut encoder = ctx
        .create_command_encoder()
        .unwrap_or_else(|e| panic!("[{label}] command encoder: {e:?}"));
    encoder.begin().unwrap_or_else(|e| panic!("[{label}] encoder begin: {e:?}"));

    let depth_range = ImageSubresourceRange::DEPTH;
    // A WHOLE-IMAGE range, deliberately coarser than the engine's framegraph-derived one: G3 is
    // testing the SHADER, not the barrier derivation, and hand-written barriers are what let this
    // gate land before the framegraph work does.
    let pyramid_range = ImageSubresourceRange {
        aspect: ImageAspect::COLOR,
        base_mip_level: 0,
        level_count: levels,
        base_array_layer: 0,
        layer_count: 1,
    };

    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth_tex,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::TRANSFER_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::TransferDstOptimal,
        range: depth_range,
    });
    encoder.copy_buffer_to_image(
        &depth_staging,
        &depth_tex,
        ImageLayout::TransferDstOptimal,
        &[BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::DEPTH,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
            image_offset_x: 0,
            image_offset_y: 0,
            image_offset_z: 0,
            image_extent_w: w,
            image_extent_h: h,
            image_extent_d: 1,
        }],
    );
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth_tex,
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::TRANSFER_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::TransferDstOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: depth_range,
    });

    // UNDEFINED → GENERAL over ALL `levels` mips, then the poison. GENERAL is a legal transfer
    // destination, so the poison needs no second layout.
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &pyramid,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::TRANSFER_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::General,
        range: pyramid_range,
    });
    let mip_regions: Vec<BufferImageCopy> = (0..levels)
        .map(|level| {
            let [lw, lh] = layout.level_extent(level);
            BufferImageCopy {
                buffer_offset: (layout.level_offset(level) * 4) as u64,
                buffer_row_length: 0,
                buffer_image_height: 0,
                aspect: ImageAspect::COLOR,
                mip_level: level,
                base_array_layer: 0,
                layer_count: 1,
                image_offset_x: 0,
                image_offset_y: 0,
                image_offset_z: 0,
                image_extent_w: lw,
                image_extent_h: lh,
                image_extent_d: 1,
            }
        })
        .collect();
    encoder.copy_buffer_to_image(&scratch, &pyramid, ImageLayout::General, &mip_regions);
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &pyramid,
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::TRANSFER_WRITE,
        // The first pass WRITES every destination mip; nothing reads the poison back, but the
        // read scope costs nothing and covers the reduce passes uniformly.
        dst_access: BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::General,
        range: pyramid_range,
    });

    encoder.bind_compute_pipeline(&pipeline);
    for (p, set) in sets.iter().enumerate() {
        let d = (p * per_pass as usize) as u32;
        let n = (levels - d).min(per_pass);
        let [ex, ey] = layout.level_extent(d);
        let groups = [ex.div_ceil(HZB_BUILD_TILE), ey.div_ceil(HZB_BUILD_TILE)];
        assert_eq!(
            [d, n, groups[0], groups[1]],
            row.passes[p],
            "[{label}] pass {p} does not match the hand-computed [d, n, groups_x, groups_y] ({})",
            row.why
        );

        if p > 0 {
            // Pass `p` reads mip `d-1`, which pass `p-1` wrote. A whole-image RAW barrier, coarser
            // than needed (see `pyramid_range`), plus SHADER_WRITE in the destination scope
            // because this pass also writes its own mips.
            encoder.image_barrier(&ImageBarrierDesc {
                texture: &pyramid,
                src_stage: BarrierStage::COMPUTE_SHADER,
                dst_stage: BarrierStage::COMPUTE_SHADER,
                src_access: BarrierAccess::SHADER_WRITE,
                dst_access: BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
                old_layout: ImageLayout::General,
                new_layout: ImageLayout::General,
                range: pyramid_range,
            });
        }

        encoder.bind_descriptor_set_compute(set, &pipeline);
        encoder.push_compute_constants(
            &pipeline,
            ShaderStage::COMPUTE,
            0,
            &pass_push(&layout, d, n).to_bytes(),
        );
        encoder.dispatch(groups[0], groups[1], 1);
    }

    encoder.image_barrier(&ImageBarrierDesc {
        texture: &pyramid,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::General,
        range: pyramid_range,
    });
    // The scratch buffer is read as the poison SOURCE above and written as the readback SINK
    // below; that WAR hazard is the buffer's own and no image barrier covers it.
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::TRANSFER,
        buffers: &[BufferBarrier {
            buffer: &scratch,
            src_access: BarrierAccess::TRANSFER_READ,
            dst_access: BarrierAccess::TRANSFER_WRITE,
        }],
    });
    encoder.copy_image_to_buffer(&pyramid, ImageLayout::General, &scratch, &mip_regions);

    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth_tex,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_READ,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::ShaderReadOnlyOptimal,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: depth_range,
    });
    let probe_regions: Vec<BufferImageCopy> = probes
        .iter()
        .enumerate()
        .map(|(i, [px, py])| BufferImageCopy {
            buffer_offset: (i * 4) as u64,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::DEPTH,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
            image_offset_x: *px as i32,
            image_offset_y: *py as i32,
            image_offset_z: 0,
            image_extent_w: 1,
            image_extent_h: 1,
            image_extent_d: 1,
        })
        .collect();
    encoder.copy_image_to_buffer(
        &depth_tex,
        ImageLayout::TransferSrcOptimal,
        &probe,
        &probe_regions,
    );

    encoder.end().unwrap_or_else(|e| panic!("[{label}] encoder end: {e:?}"));
    ctx.rhi_queue()
        .submit(&encoder, &fence)
        .unwrap_or_else(|e| panic!("[{label}] submit: {e:?}"));
    ctx.wait_fence(&fence, u64::MAX)
        .unwrap_or_else(|e| panic!("[{label}] wait_fence: {e:?}"));

    // ---- 4) the UPLOAD probe -------------------------------------------------------------------
    let probe_ptr = ctx
        .buffer_mapped_ptr(&probe)
        .unwrap_or_else(|| panic!("[{label}] depth probe is not host-mapped"));
    for (i, [px, py]) in probes.iter().enumerate() {
        // SAFETY: `probe` is host-coherent with `probes.len()` words, `i` is in range, and the
        // fence wait completed the copy that filled it, so the bytes are stable.
        let got = unsafe { read_word(probe_ptr, i) };
        let want = depth[*py as usize * w as usize + *px as usize].to_bits();
        assert_eq!(
            got, want,
            "[{label}] the SOURCE DEPTH read back at ({px}, {py}) is 0x{got:08x}, not the \
             uploaded 0x{want:08x} — the upload or its DEPTH aspect is wrong, so any pyramid \
             mismatch below would be a symptom rather than the cause"
        );
    }

    // ---- 5) the PYRAMID, texel by texel, on BITS ------------------------------------------------
    let poison_bits = POISON.to_bits();
    let mut zero_ties = 0usize;
    for level in 0..levels {
        let [lw, lh] = layout.level_extent(level);
        let off = layout.level_offset(level);
        let mut diff = 0usize;
        let mut first: Option<(u32, u32, u32, u32)> = None;
        for y in 0..lh {
            for x in 0..lw {
                let i = off + (y as usize * lw as usize + x as usize);
                // SAFETY: `scratch` is host-coherent with `pyramid_words` words and
                // `i < pyramid_words` (it is inside level `level`'s slice of the flat layout); the
                // fence wait completed the readback copy, so the bytes are stable.
                let gpu = unsafe { read_word(scratch_ptr, i) };
                assert_ne!(
                    gpu, poison_bits,
                    "[{label}] level {level} texel ({x}, {y}) still holds the {POISON} POISON — \
                     the shader never wrote it (or the readback never ran). {POISON} is not a \
                     legitimate output: the reduce is a min over a pattern in (0, 1] and every \
                     written texel has at least one live child"
                );
                let want = oracle[i].to_bits();
                if gpu != want {
                    // A ±0 TIE: both sides have zero magnitude and differ only in the sign bit,
                    // so the two are numerically EQUAL and no `<` comparison anywhere can tell
                    // them apart. Tolerated only under `ZeroTiesExactly`, and counted either way.
                    if (gpu | want) & 0x7fff_ffff == 0 {
                        zero_ties += 1;
                        if matches!(policy, BitPolicy::ZeroTiesExactly(_)) {
                            continue;
                        }
                    }
                    diff += 1;
                    if first.is_none() {
                        first = Some((x, y, gpu, want));
                    }
                }
            }
        }
        if let Some((x, y, gpu, want)) = first {
            panic!(
                "[{label}] level {level} ({lw}x{lh}) DIFFERS from the host oracle at ({x}, {y}): \
                 gpu_bits=0x{gpu:08x} oracle_bits=0x{want:08x} gpu={} oracle={} — {diff} of {} \
                 texels differ at this level. Extent rationale: {}",
                f32::from_bits(gpu),
                f32::from_bits(want),
                lw as usize * lh as usize,
                row.why
            );
        }
    }

    match policy {
        BitPolicy::Exact => {
            println!(
                "hzb_build G3 [{label}]: {levels} levels, {pass_count} pass(es), \
                 {pyramid_words} texels BIT-EXACT vs boyko_render::hzb::build_pyramid"
            );
        }
        // Pinned to an EXACT count, not a ceiling. A ceiling would stay green if the hardware
        // started tie-breaking the other way on some texels and not others; an exact count says
        // the divergence is the one that was measured and characterised, and nothing else.
        BitPolicy::ZeroTiesExactly(want_ties) => {
            assert_eq!(
                zero_ties, want_ties,
                "[{label}] {zero_ties} texels differ as a ±0 TIE, but exactly {want_ties} were \
                 measured when this gate was written. More means the divergence spread; FEWER \
                 means the hardware or the shader started preserving the source's tie order, \
                 which is a change worth knowing about even though it is an improvement. Either \
                 way, re-measure and re-pin rather than widening this number."
            );
            println!(
                "hzb_build G3 [{label}]: {levels} levels, {pass_count} pass(es), \
                 {pyramid_words} texels match vs boyko_render::hzb::build_pyramid, with exactly \
                 {zero_ties} ±0 TIE(s) — numerically equal, sign bit only (see the test's doc)"
            );
        }
    }

    // ---- 6) teardown, in reverse acquisition order ---------------------------------------------
    // SAFETY: every object below was created on `ctx` in this function and the last submission
    // completed (fence-waited above), so none is GPU-referenced. Each is consumed BY VALUE and so
    // destroyed exactly once. The order is the reverse of acquisition, and in particular every
    // descriptor set (which retains its views by raw handle) goes before the views, and every view
    // goes before the image it views — THE OWNERSHIP RULE, `VUID-vkDestroyImage-image-01000`.
    unsafe {
        ctx.destroy_command_encoder(encoder);
        ctx.destroy_fence(fence);
        for group in sets {
            ctx.destroy_bind_group(group);
        }
        ctx.destroy_compute_pipeline(pipeline);
        ctx.destroy_shader_module(module);
        ctx.destroy_bind_group_layout(set_layout);
        ctx.destroy_sampler(sampler);
        for view in level_views {
            ctx.destroy_texture_view(view);
        }
        ctx.destroy_texture(pyramid);
        ctx.destroy_texture(depth_tex);
        ctx.destroy_buffer(probe);
        ctx.destroy_buffer(scratch);
        ctx.destroy_buffer(depth_staging);
    }
}

// ==============================================================================================
// The tests
// ==============================================================================================

/// VG R3 P1-7 GATE G3: `hzb_build.comp.hlsl` equals `boyko_render::hzb::build_pyramid` to BITS,
/// over the seven-extent sweep, with no engine involved.
#[test]
#[ignore = "live dispatch gate (GPU + --nocapture --test-threads=1); the orchestrator runs it"]
fn hzb_build_gpu_eq_oracle_to_bits() {
    let Some(ctx) = boot_or_skip() else {
        return;
    };
    println!("hzb_build_oracle_gate on: {}", ctx.device_name());
    for row in &SWEEP {
        run_case(&ctx, row, sweep_depth, BitPolicy::Exact);
    }
}

/// VG R3 P1-7 GATE G3, the `±0.0` case — and ⚠️ **the one place the GPU does NOT agree with the
/// oracle on bits.** Kept SEPARATE so this reads as what it is rather than as a sweep regression.
///
/// # What was expected, and what was MEASURED
///
/// The oracle folds a 2×2 as a LEFT FOLD seeded with `+INFINITY`; the shader folds it as a
/// BALANCED TREE. The P1-3 review reasoned that they agree because both compute *the earliest
/// minimal element in program order* — `hzb_min(a, b)` returns `b` only on a strict `b < a`, so it
/// keeps `a` on a tie — and called this "the single most fragile equivalence in the step". The
/// reasoning is correct **about the source**. It is not correct about what runs.
///
/// Measured on an RTX 3060 Laptop, deterministic across three runs: level-1 texel `(1,1)`, whose
/// operands are `+0.0` (first) and `-0.0` (last), comes back `0x80000000` where both the oracle
/// and the shader's own source semantics say `0x00000000`. Texel `(2,2)`, planted with the zeros
/// in the OPPOSITE order, agrees. One texel of sixteen.
///
/// That asymmetry is the whole diagnosis: the divergence is not random and not an association
/// error. **The driver recognised `b < a ? b : a` and fused it into a hardware min**, whose
/// tie-break on `±0` returns the negative zero regardless of operand order. `(2,2)` should be
/// `-0.0` and is; `(1,1)` should be `+0.0` and is not. A compiler is ALLOWED to do this: `+0.0`
/// and `-0.0` compare equal, so the two semantics are indistinguishable to every `<` in the
/// program — only `to_bits()` can see it, and the shader does not request
/// `SignedZeroInfNanPreserve` (`hzb_build.comp.hlsl`, "What this module does NOT ask for"). This
/// is the first hard evidence for that section, which until now was a stated assumption.
///
/// # Why it is accepted rather than fixed
///
/// The two values are NUMERICALLY EQUAL. The pyramid is a conservative lower bound on depth and
/// its only consumer is `depth_near < occ`; `+0.0` and `-0.0` behave identically in that
/// comparison, and reverse-Z depth from a real rasteriser never reaches zero at all. Requesting
/// the execution mode would mean enabling a device feature, a new capability in the `.spv` census,
/// and a hardware constraint — all to fix a difference that cannot reach a pixel.
///
/// **The `OpExtInst == 0` pin is NOT defeated by this.** It proves DXC emitted no `NMin`; what
/// this measures is a DRIVER fusing the compare-and-select afterwards, which no artifact-level
/// check can see. The half of the NaN policy that has teeth — the explicit `isnan` branch — is a
/// different construct and is measured separately by
/// [`hzb_build_nan_collapses_to_negative_infinity`].
///
/// # What this test therefore asserts
///
/// Not "the bits agree". It asserts that **every** divergence in the whole chain is a ±0 tie —
/// numerically equal, sign bit only — and that there are EXACTLY as many as were measured. Any
/// other bit difference, anywhere, still fails. That is a narrowing of the sweep's claim, not a
/// suspension of it, and it goes red if the hardware's behaviour moves in either direction.
#[test]
#[ignore = "live dispatch gate (GPU + --nocapture --test-threads=1); the orchestrator runs it"]
fn hzb_build_signed_zero_is_the_hardware_min_tie() {
    let Some(ctx) = boot_or_skip() else {
        return;
    };
    println!("hzb_build_oracle_gate (signed zero) on: {}", ctx.device_name());

    // Non-vacuity of the fixture itself, on the HOST, before any GPU work: the two planted blocks
    // must reduce to opposite zero signs, or the bit comparison proves nothing about association.
    let layout = HzbLayout::new(ZERO_ROW.extent[0], ZERO_ROW.extent[1])
        .expect("invariant: 8x8 is a legal HZB extent");
    let depth = signed_zero_depth(&layout);
    let mut oracle = vec![0.0f32; layout.pyramid_len()];
    build_pyramid(&layout, &depth, &mut oracle);
    assert_eq!(
        layout.texel(&oracle, 1, 1, 1).to_bits(),
        0x0000_0000,
        "the oracle's level-1 texel (1,1) must be +0.0 (its FIRST operand is the +0.0), else the \
         GPU comparison cannot observe the fold association"
    );
    assert_eq!(
        layout.texel(&oracle, 1, 2, 2).to_bits(),
        0x8000_0000,
        "the oracle's level-1 texel (2,2) must be -0.0 (its FIRST operand is the -0.0), else the \
         GPU comparison cannot observe the fold association"
    );

    // MEASURED, not predicted: the count below was read off the failing run that discovered this,
    // then pinned. See the doc for why an exact count rather than a ceiling.
    run_case(&ctx, &ZERO_ROW, signed_zero_depth, BitPolicy::ZeroTiesExactly(ZERO_TIE_COUNT));
}

/// The number of ±0-tie texels the hardware min produces on [`ZERO_ROW`]'s fixture.
///
/// MEASURED — and then DERIVED, so a change in it can be reasoned about rather than merely
/// re-pinned. One planted block goes the "wrong" way and it propagates up the whole chain:
///
/// * **level 1 `(1,1)`** — operands `+0.0` (first) and `-0.0` (last); source semantics say `+0.0`,
///   the hardware min says `-0.0`. The sibling block at level 1 `(2,2)` has the zeros in the
///   opposite order, so its correct answer IS `-0.0` and it agrees.
/// * **level 2 `(0,0)`** — folds level-1 `(0,0)`, `(1,0)`, `(0,1)`, `(1,1)`; only `(1,1)` is a
///   zero, so this texel simply inherits the divergence. (Level 2 `(1,1)` covers the OTHER block
///   and agrees.)
/// * **level 3 `(0,0)`** — the 1×1 top folds both zeros; the oracle keeps the earliest minimal,
///   which is level-2 `(0,0)` = `+0.0`, and the hardware min again returns the negative zero.
///
/// Three, and no more: every other texel in the 85-texel chain is a strict min over positive
/// values, where the two semantics cannot differ.
const ZERO_TIE_COUNT: usize = 3;

/// `8 × 8` with a quiet NaN planted at ONE source pixel.
///
/// `S == P == 8`, so level 0 is the identity on the source and the NaN lands on exactly one
/// level-0 texel, whose footprint then propagates up every level of the chain.
fn nan_depth(layout: &HzbLayout) -> Vec<f32> {
    let (w, h) = (layout.x().source(), layout.y().source());
    assert_eq!([w, h], [8, 8], "invariant: the NaN fixture is the 8x8 extent");
    let mut depth = vec![0.0f32; layout.source_len()];
    for y in 0..h {
        let row = y as usize * w as usize;
        for x in 0..w {
            depth[row + x as usize] = noise_depth(x, y, w, h);
        }
    }
    // A QUIET NaN, spelled by bit pattern. Planted at (2,2) so it is not on any edge and its
    // footprint at every level is unambiguous.
    depth[2 * 8 + 2] = f32::from_bits(0x7fc0_0000);
    depth
}

/// The NaN fixture's hand table — [`ZERO_ROW`]'s shape, a different pattern.
const NAN_ROW: HandRow = HandRow {
    extent: [8, 8],
    base: [8, 8],
    levels: 4,
    pyramid_len: 85,
    pass_count: 1,
    passes: [[0, 4, 1, 1], [0; 4], [0; 4]],
    why: "S == P == 8 with a quiet NaN at one source pixel: the isnan guard must collapse every \
          footprint containing it to -INFINITY, and a driver that fused hzb_min into a hardware \
          min would instead return the OTHER operand",
};

/// VG R3 P1-7 GATE G3 — ⚠️ **the half of the NaN policy that has teeth, and the one thing the
/// artifact census cannot check.**
///
/// `hzb_build_spv_sync.rs` pins `OpExtInst == 0` and `OpExtInstImport == 0`, which proves DXC
/// emitted no `GLSL.std.450 NMin` — the instruction under which a NaN operand is silently
/// DISCARDED rather than propagated. That pin is real and it is not enough:
/// [`hzb_build_signed_zero_is_the_hardware_min_tie`] MEASURED the driver fusing
/// `b < a ? b : a` into a hardware min AFTER compilation, which no `.spv` census can see. If the
/// same fusion also swallowed the explicit `isnan` branch, the entire NaN policy would be gone and
/// every artifact-level check in the tree would still be green.
///
/// So it is measured directly. `conservative_min` (and `hzb_min`) answer `-INFINITY` on any NaN —
/// "unknown depth is infinitely far", the only reading that can never let piece 3's predicate
/// reject. With a NaN at source pixel `(2,2)`:
///
/// * the guard SURVIVED  ⇒ level 0 texel `(2,2)` is `0xFF800000`, and `-INFINITY` propagates to
///   the top of the chain, matching the oracle at every level;
/// * the guard was FUSED AWAY ⇒ `min(+INFINITY, NaN)` under `NMin` semantics returns the other
///   operand, so level 0 `(2,2)` comes back `0x7F800000` (`+INFINITY`) and this test reports the
///   exact bits.
///
/// `BitPolicy::Exact`: there is no tie here to tolerate. A real reverse-Z attachment cannot hold a
/// NaN (the rasteriser clamps to `[minDepth, maxDepth]`), so this is a CONTRACT being verified,
/// not a hot case — but it is the contract the whole conservative-reduce argument rests on.
#[test]
#[ignore = "live dispatch gate (GPU + --nocapture --test-threads=1); the orchestrator runs it"]
fn hzb_build_nan_collapses_to_negative_infinity() {
    let Some(ctx) = boot_or_skip() else {
        return;
    };
    println!("hzb_build_oracle_gate (NaN) on: {}", ctx.device_name());

    // Non-vacuity of the fixture, on the HOST, before any GPU work: the oracle must actually carry
    // `-INFINITY` from the planted texel all the way to the 1x1 top. If it did not, the GPU
    // comparison would be asserting nothing about the guard.
    let layout = HzbLayout::new(NAN_ROW.extent[0], NAN_ROW.extent[1])
        .expect("invariant: 8x8 is a legal HZB extent");
    let depth = nan_depth(&layout);
    let mut oracle = vec![0.0f32; layout.pyramid_len()];
    build_pyramid(&layout, &depth, &mut oracle);
    let neg_inf = f32::NEG_INFINITY.to_bits();
    assert_eq!(
        layout.texel(&oracle, 0, 2, 2).to_bits(),
        neg_inf,
        "the oracle's level-0 texel (2,2) must be -INFINITY — that IS conservative_min's NaN answer"
    );
    for level in 1..layout.levels() {
        let [lw, lh] = layout.level_extent(level);
        let (tx, ty) = (2 >> level, 2 >> level);
        assert!(tx < lw && ty < lh, "invariant: the NaN's containing texel exists at every level");
        assert_eq!(
            layout.texel(&oracle, level, tx, ty).to_bits(),
            neg_inf,
            "the oracle must propagate -INFINITY to level {level} texel ({tx}, {ty}); without \
             that propagation this gate would only be testing level 0"
        );
    }

    run_case(&ctx, &NAN_ROW, nan_depth, BitPolicy::Exact);
}
