//! Render wave PX0 (H3 Scope 3 / 3b; `px0/cut.md` D-P1..D-P4, §1.3 #2 and #6) — **the
//! pre-registered `lit` format model**: the content arms, their seeded per-pixel inputs, the
//! class simulators (through `boyko_shaderdsl::pixfmt`), the `f64` composite, the display metric,
//! and the classification input sets.
//!
//! # This module is FROZEN at the model commit
//!
//! The decision is judged against a table this module computed BEFORE the device ran. The record
//! cites the model commit, the probe commit and the decision commit and prints both
//! `git merge-base --is-ancestor <model> <probe>` and
//! `git diff --exit-code <model> <decision> -- crates/boyko_rhi_vulkan/tests/lit_format
//! crates/boyko_rhi_vulkan/tests/b10g11r11_chain_model.rs` (`px0/critique.md` W2) — the second is
//! red on any later edit of this directory, whatever else moved with it. The probe's own code (the
//! upload packing, the device dispatch, the classifier, the decision rows) lives in
//! `b10g11r11_rounding_probe.rs`, outside the frozen set, and only CALLS this module.
//!
//! # Inputs are generated on the host and uploaded (D-P2)
//!
//! Every per-pixel, per-layer value is drawn here from a seeded SplitMix64 and uploaded; the device
//! kernels read, write, blend or store and compute nothing but A3's `lit · t + s`. Host and device
//! therefore see identical sequences by construction.
//!
//! # The metric (H3 3b, pre-registered here)
//!
//! Per arm and channel `c`, `Δ_c(j)` is the region mean of `255 · [D(2^j · lit_device) −
//! D(2^j · lit_exact)]_c` with `D` = AgX (`agx_f64`) followed by the OETF, unquantised;
//! `chroma(j) = max_{c,c'} |Δ_c(j) − Δ_c'(j)|`. The arm's numbers are `max_{j,c} |Δ_c(j)|` and
//! `max_j chroma(j)`, `j ∈ [−8, 4]` — each maximised over the placement, the conservative reading.
//! An arm passes iff both are `≤ 2` LSB.

#![allow(dead_code)] // the chain model and the probe each use a subset

#[path = "../../../boyko_shaderdsl/tests/post_f64/mod.rs"]
pub mod post_f64;
// Render wave PXt (DS-H6): `post_f64` includes `agx_f64.rs` by path (the AgX leaf's host reference),
// so this model reaches the same module through it — one load of the file per crate.
pub use post_f64::agx_f64;

use boyko_shaderdsl::pixfmt::{Channel, Member, Path, PathClass, Rounding, Target, decode, encode};

use agx_f64::AgxLook;

// ---- Constants ------------------------------------------------------------------------------------

/// The region every content arm runs over: 64 × 64 = 4096 independent stacks per arm (the region
/// mean's standard error is ≈ 1/64 of the per-pixel spread).
pub const REGION_W: u32 = 64;
/// The region's height.
pub const REGION_H: u32 = 64;
/// Pixels per region.
pub const REGION_PX: usize = (REGION_W * REGION_H) as usize;
/// The placement range of the metric.
pub const PLACEMENT_J: core::ops::RangeInclusive<i32> = -8..=4;
/// The pass threshold, in display LSB.
pub const THRESHOLD_LSB: f64 = 2.0;
/// The background values the arms run over (black and 0.18 grey), uploaded as nearest-even codes.
pub const BACKGROUNDS: [f32; 2] = [0.0, 0.18];

// ---- SplitMix64 ----------------------------------------------------------------------------------

/// SplitMix64 (Steele, Lea, Flood 2014) — the content generator's only randomness.
#[derive(Clone, Copy, Debug)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform `f32` in `[lo, hi)` built from 24 random bits — exact in f32.
    pub fn uniform(&mut self, lo: f32, hi: f32) -> f32 {
        let u = (self.next_u64() >> 40) as f32 / 16_777_216.0;
        lo + (hi - lo) * u
    }
}

/// The per-pixel stream: seeded from the arm's seed and the pixel index, so any one pixel's
/// sequence is reproducible alone.
pub fn pixel_rng(arm_seed: u64, px: usize) -> SplitMix64 {
    let mut s = SplitMix64(arm_seed ^ (px as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
    s.next_u64();
    s
}

// ---- The arms -------------------------------------------------------------------------------------

/// The content arm families of H3 3b (+ A3-blend, Row 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Additive, `c · m · u`, `u ∼ U[0.5, 1.5]` — decisive.
    A1S1,
    /// Additive, `u ∼ U[0, 2]` (a sprite's falloff) — decisive.
    A1S2,
    /// Premultiplied over, alpha and colour jittered independently — decisive.
    A2,
    /// Premultiplied over, colour fixed, alpha jittered — K3's stall, reported.
    A2Flat,
    /// Storage RMW `lit ← lit · t + s` — decisive.
    A3,
    /// A3's `t`, `s` through the premultiplied-over blend, `src = (s, 1 − t)` — Row 7's arm.
    A3Blend,
    /// Equal layers (both blends), reported.
    UEqual,
    /// Near-uniform layers (±5 %, both blends), reported.
    UNear,
}

impl Family {
    /// Whether the family enters the decision.
    pub const fn decisive(self) -> bool {
        matches!(self, Family::A1S1 | Family::A1S2 | Family::A2 | Family::A3)
    }

    /// The short name.
    pub const fn name(self) -> &'static str {
        match self {
            Family::A1S1 => "A1-S1",
            Family::A1S2 => "A1-S2",
            Family::A2 => "A2",
            Family::A2Flat => "A2-flat",
            Family::A3 => "A3",
            Family::A3Blend => "A3-blend",
            Family::UEqual => "U-eq",
            Family::UNear => "U-near",
        }
    }
}

/// One content arm: a family, a layer count, its colour / alpha parameters, a background, and the
/// path its layers go through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arm {
    /// The family.
    pub family: Family,
    /// Layers (passes for A3).
    pub k: u32,
    /// The colour `c` (A1) or `c̄` (A2 / U), per channel.
    pub color: [f32; 3],
    /// `m` (A1: the stack sums to 3.2) or `ā` (A2 / U over); 0 where unused.
    pub scale: f32,
    /// The background `lit` value the stack starts on (uploaded as its nearest-even code).
    pub background: f32,
    /// The path the arm's layers go through.
    pub path: Path,
}

const GREY: [f32; 3] = [1.0, 1.0, 1.0];
const FIRE: [f32; 3] = [1.0, 0.45, 0.12];

impl Arm {
    /// A stable label, e.g. `A1-S1 k64 fire bg0.18`.
    pub fn label(&self) -> String {
        let col = if self.color == FIRE {
            "fire".to_owned()
        } else if self.color == GREY {
            "grey".to_owned()
        } else {
            format!("grey{}", self.color[0])
        };
        let p = match self.family {
            Family::A2 | Family::A2Flat => format!(" a{}", self.scale),
            Family::UEqual | Family::UNear => format!(" {}", self.path.spelling()),
            _ => String::new(),
        };
        format!("{} k{} {col}{p} bg{}", self.family.name(), self.k, self.background)
    }

    /// The arm's seed — a hash of its label, so reordering the arm list moves no arm's content.
    pub fn seed(&self) -> u64 {
        self.label().bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3))
    }

    /// One layer of pixel `px`: the source colour (premultiplied for the over path) and the
    /// alpha the blend reads (0 for the additive path; for A3 the pass's `t` travels as
    /// `alpha = t` and its `s` as the source).
    pub fn layers(&self, px: usize) -> Vec<Layer> {
        let mut rng = pixel_rng(self.seed(), px);
        (0..self.k)
            .map(|_| match self.family {
                Family::A1S1 | Family::A1S2 => {
                    let (lo, hi) = if self.family == Family::A1S1 { (0.5, 1.5) } else { (0.0, 2.0) };
                    let u = rng.uniform(lo, hi);
                    Layer { src: self.color.map(|c| c * self.scale * u), alpha: 0.0 }
                }
                Family::A2 => {
                    let a = self.scale * rng.uniform(0.5, 1.5);
                    let uc = rng.uniform(0.5, 1.5);
                    Layer { src: self.color.map(|c| c * uc * a), alpha: a }
                }
                Family::A2Flat => {
                    let a = self.scale * rng.uniform(0.5, 1.5);
                    Layer { src: self.color.map(|c| c * a), alpha: a }
                }
                Family::A3 => {
                    let t = rng.uniform(0.8, 1.0);
                    let s = rng.uniform(0.0, 0.05);
                    Layer { src: [s; 3], alpha: t }
                }
                Family::A3Blend => {
                    let t = rng.uniform(0.8, 1.0);
                    let s = rng.uniform(0.0, 0.05);
                    Layer { src: [s; 3], alpha: 1.0 - t }
                }
                Family::UEqual | Family::UNear => {
                    let j = if self.family == Family::UNear { rng.uniform(0.95, 1.05) } else { 1.0 };
                    match self.path {
                        Path::BlendOver => {
                            let a = self.scale;
                            Layer { src: self.color.map(|c| c * j * a), alpha: a }
                        }
                        _ => Layer { src: self.color.map(|c| c * self.scale * j), alpha: 0.0 },
                    }
                }
            })
            .collect()
    }
}

/// One layer's uploaded values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    /// The source colour the blend adds (premultiplied for the over path), or A3's `s`.
    pub src: [f32; 3],
    /// The blend's source alpha (over path), or A3's `t`; 0 for the additive path.
    pub alpha: f32,
}

/// Every arm of the pre-registration, in table order.
pub fn arms() -> Vec<Arm> {
    let mut v = Vec::new();
    for bg in BACKGROUNDS {
        for family in [Family::A1S1, Family::A1S2] {
            for k in [16u32, 64] {
                for color in [GREY, FIRE] {
                    v.push(Arm { family, k, color, scale: 3.2 / k as f32, background: bg, path: Path::BlendAdd });
                }
            }
        }
        for k in [16u32, 64] {
            for a in [0.1f32, 0.3] {
                for c in [0.05f32, 0.2, 0.5] {
                    v.push(Arm { family: Family::A2, k, color: [c; 3], scale: a, background: bg, path: Path::BlendOver });
                }
            }
        }
        v.push(Arm { family: Family::A3, k: 4, color: GREY, scale: 0.0, background: bg, path: Path::Store });
        v.push(Arm { family: Family::A3Blend, k: 4, color: GREY, scale: 0.0, background: bg, path: Path::BlendOver });
        for a in [0.1f32, 0.3] {
            for c in [0.05f32, 0.2, 0.5] {
                v.push(Arm { family: Family::A2Flat, k: 64, color: [c; 3], scale: a, background: bg, path: Path::BlendOver });
            }
        }
    }
    for family in [Family::UEqual, Family::UNear] {
        v.push(Arm { family, k: 64, color: GREY, scale: 0.05, background: 0.0, path: Path::BlendAdd });
        v.push(Arm { family, k: 64, color: [0.2; 3], scale: 0.1, background: 0.0, path: Path::BlendOver });
    }
    v
}

// ---- Simulation and the exact composite ------------------------------------------------------------

/// The background code of channel `ch` — uploaded as packed bits, identical for every class.
pub fn background_code(ch: Channel, bg: f32) -> u32 {
    encode(ch, bg, Rounding::NearestEven)
}

/// The device image a correct `member` produces for `arm` on `target`: per pixel, the three codes.
pub fn simulate(arm: &Arm, target: Target, member: Member) -> Vec<[u32; 3]> {
    let chans = target.channels();
    (0..REGION_PX)
        .map(|px| {
            let layers = arm.layers(px);
            core::array::from_fn(|c| {
                let ch = chans[c];
                let mut d = background_code(ch, arm.background);
                for l in &layers {
                    d = match (arm.family, arm.path) {
                        // The storage RMW: the kernel computes `lit · t + s` in f32 and stores it.
                        (Family::A3, _) => member.convert(ch, decode(ch, d) * l.alpha + l.src[c]),
                        (_, Path::BlendAdd) => member.blend_add(ch, d, l.src[c]),
                        (_, Path::BlendOver) => member.blend_over(ch, d, l.src[c], l.alpha),
                        (_, p) => panic!("invariant: no arm writes through {p:?} without a blend"),
                    };
                }
                d
            })
        })
        .collect()
}

/// The `f64` composite of the same uploaded values, from the decoded background.
pub fn exact(arm: &Arm, target: Target) -> Vec<[f64; 3]> {
    let chans = target.channels();
    (0..REGION_PX)
        .map(|px| {
            let layers = arm.layers(px);
            core::array::from_fn(|c| {
                let mut x = f64::from(decode(chans[c], background_code(chans[c], arm.background)));
                for l in &layers {
                    let s = f64::from(l.src[c]);
                    let a = f64::from(l.alpha);
                    x = match (arm.family, arm.path) {
                        (Family::A3, _) => x * a + s,
                        (_, Path::BlendAdd) => x + s,
                        (_, Path::BlendOver) => s + x * (1.0 - a),
                        (_, p) => panic!("invariant: no arm writes through {p:?} without a blend"),
                    };
                }
                x
            })
        })
        .collect()
}

/// Decodes a simulated / dumped image to linear values.
pub fn decode_image(img: &[[u32; 3]], target: Target) -> Vec<[f64; 3]> {
    let chans = target.channels();
    img.iter().map(|p| core::array::from_fn(|c| f64::from(decode(chans[c], p[c])))).collect()
}

// ---- The display metric ----------------------------------------------------------------------------

/// The curve the metric reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// AgX — the curve PX0 decided under (the pre-registered table's verdict column).
    Agx,
    /// AgX with Filament's Punchy look — reported, never decisive.
    AgxPunchy,
    /// Hill ACES (`post_f64::aces_f64`) — reported beside AgX (K9 reads it).
    AcesHill,
    /// Khronos PBR Neutral (`post_f64::neutral_f64`) — the shipping default since render wave PXt's
    /// ruling 116; the record's row 6 re-runs the content arms under it.
    Neutral,
}

impl Curve {
    /// The display value of a linear colour: the curve, then the OETF, ×255, unquantised.
    pub fn display(self, c: [f64; 3]) -> [f64; 3] {
        let exact = |v| post_f64::Tracked { v, b: 0.0 };
        let t = match self {
            Curve::Agx => agx_f64::agx_f64(c, AgxLook::Base),
            Curve::AgxPunchy => agx_f64::agx_f64(c, AgxLook::Punchy),
            Curve::AcesHill => post_f64::aces_f64(c.map(exact)).map(|t| t.v),
            Curve::Neutral => post_f64::neutral_f64(c.map(exact)).map(|t| t.v),
        };
        t.map(|v| 255.0 * agx_f64::oetf_f64(v))
    }

    /// The name the table prints.
    pub const fn name(self) -> &'static str {
        match self {
            Curve::Agx => "AgX",
            Curve::AgxPunchy => "AgX-Punchy",
            Curve::AcesHill => "ACES-Hill",
            Curve::Neutral => "Neutral",
        }
    }

    /// The curve a record's `decided_curve` value names (`Tonemapper`'s `Debug` name, lower case),
    /// or `None` for a name this model has no curve for.
    pub fn from_decided(name: &str) -> Option<Curve> {
        match name {
            "agx" => Some(Curve::Agx),
            "aces" => Some(Curve::AcesHill),
            "neutral" => Some(Curve::Neutral),
            _ => None,
        }
    }
}

/// One arm's metric under one curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmMetric {
    /// `max_{j,c} |Δ_c(j)|`, display LSB.
    pub max_delta: f64,
    /// The channel and placement of [`Self::max_delta`].
    pub at: (usize, i32),
    /// `max_j chroma(j)`, display LSB.
    pub chroma: f64,
    /// The placement of [`Self::chroma`].
    pub chroma_j: i32,
}

impl ArmMetric {
    /// Whether the arm passes the 2-LSB rule.
    pub fn passes(&self) -> bool {
        self.max_delta <= THRESHOLD_LSB && self.chroma <= THRESHOLD_LSB
    }
}

/// The display of `exact` at every placement — computed once per arm, shared by every class.
pub fn exact_display(curve: Curve, exact: &[[f64; 3]]) -> Vec<Vec<[f64; 3]>> {
    PLACEMENT_J
        .map(|j| {
            let s = 2f64.powi(j);
            exact.iter().map(|p| curve.display(p.map(|v| v * s))).collect()
        })
        .collect()
}

/// The metric of a device (or simulated) image against the exact composite's display.
pub fn metric(curve: Curve, device: &[[f64; 3]], exact_disp: &[Vec<[f64; 3]>]) -> ArmMetric {
    let mut m = ArmMetric { max_delta: 0.0, at: (0, 0), chroma: 0.0, chroma_j: 0 };
    for (ji, j) in PLACEMENT_J.enumerate() {
        let s = 2f64.powi(j);
        let mut sum = [0.0f64; 3];
        for (p, e) in device.iter().zip(&exact_disp[ji]) {
            let d = curve.display(p.map(|v| v * s));
            for c in 0..3 {
                sum[c] += d[c] - e[c];
            }
        }
        let delta = sum.map(|v| v / device.len() as f64);
        for (c, v) in delta.iter().enumerate() {
            if v.abs() > m.max_delta {
                m.max_delta = v.abs();
                m.at = (c, j);
            }
        }
        let chroma = (delta[0] - delta[1]).abs().max((delta[1] - delta[2]).abs()).max((delta[0] - delta[2]).abs());
        if chroma > m.chroma {
            m.chroma = chroma;
            m.chroma_j = j;
        }
    }
    m
}

// ---- The classification input sets (H3 Scope 3; px0/cut.md §1.3 #2) ------------------------------

/// One classification input: the value to write (single-write paths), or the `(dst, src, alpha)`
/// whose exact blend result is that value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClassInput {
    /// The target value — the exact result of the operation.
    pub value: [f32; 3],
    /// The destination codes a blend starts from (uploaded as packed bits).
    pub dst: [u32; 3],
    /// The blend source.
    pub src: [f32; 3],
    /// The blend's source alpha (0 for single writes and the additive blend).
    pub alpha: f32,
}

/// The single-write values of one channel. Per binade `2^e`, `e ∈ binades`, up to `intervals`
/// intervals between consecutive representable points (every interval when the binade has no
/// more; otherwise a spread whose lower codes alternate parity), and per interval four values: the
/// exact midpoint, the midpoint `± ½` binary16 ULP (the inputs a double rounding moves), and the
/// three-quarter point. Together they separate every pair of non-equivalent classes
/// (`probe_inputs_separate_classes`): the three-quarter point splits `N`/`N16` from `Z`, the
/// midpoint `+ ½` binary16 ULP splits `N` from `N16`, and the odd-lower-code midpoints split the
/// ties.
///
/// The interval whose upper end is the overflow point (the top binade's last one) is excluded:
/// every input stays at or below the channel's largest finite value, so a device's overflow rule
/// (clamp or Inf) never enters classification (`px0/critique.md` open question 4).
pub fn channel_values(ch: Channel, binades: &[i32], intervals: usize) -> Vec<f32> {
    let m = ch.mantissa_bits() as i32;
    let n_int = 1usize << m;
    let max = ch.max_finite();
    let mut v = Vec::new();
    for &e in binades {
        let base = 2f32.powi(e);
        let ulp = 2f32.powi(e - m);
        let half16 = 2f32.powi(e - 11);
        let ks: Vec<usize> = if n_int <= intervals {
            (0..n_int).collect()
        } else {
            let step = n_int / intervals;
            (0..intervals).map(|i| i * step + (i % 2)).collect()
        };
        for k in ks {
            let lo = base + k as f32 * ulp;
            if lo + ulp > max {
                continue;
            }
            let mid = lo + 0.5 * ulp;
            v.extend([mid, mid + half16, mid - half16, lo + 0.75 * ulp]);
        }
    }
    v
}

/// Every binade of the classified range `[2^-14, 2^15]`.
pub fn all_binades() -> Vec<i32> {
    (-14..=15).collect()
}

/// The clear path's binades — a subset, because a clear is one `vkCmdClearColorImage` per value
/// (one array layer each, ≤ 2048 layers): the edges, the binades around 1, and a spread.
pub fn clear_binades() -> Vec<i32> {
    vec![-14, -10, -6, -2, -1, 0, 1, 2, 6, 10, 14, 15]
}

/// The per-channel value lists zipped into triples (the shorter lists cycled).
fn zip3(lists: [Vec<f32>; 3]) -> Vec<[f32; 3]> {
    let n = lists.iter().map(Vec::len).max().unwrap_or(0);
    (0..n).map(|i| core::array::from_fn(|c| lists[c][i % lists[c].len()])).collect()
}

/// The classification input set of `(target, path)`.
///
/// Single writes (store / attachment / clear) write the value. The additive blend starts from
/// `dst = trunc(value / 4)` (a code) with `src = value − dst`; the over blend from
/// `dst = trunc(value)` with `alpha = ½` and `src = value − dst · ½`. Every operand and every
/// intermediate is exact in f32 (dyadic alpha, `px0/critique.md` O4), so a blend unit's FMA or
/// factor precision cannot move an input off the point it targets.
pub fn class_inputs(target: Target, path: Path) -> Vec<ClassInput> {
    let chans = target.channels();
    let (binades, intervals) = match path {
        Path::Clear => (clear_binades(), 8),
        _ => (all_binades(), 64),
    };
    let values = zip3(chans.map(|ch| channel_values(ch, &binades, intervals)));
    values
        .into_iter()
        .map(|value| match path {
            Path::Store | Path::Attachment | Path::Clear => {
                ClassInput { value, dst: [0; 3], src: value, alpha: 0.0 }
            }
            Path::BlendAdd => {
                let dst: [u32; 3] = core::array::from_fn(|c| encode(chans[c], value[c] / 4.0, Rounding::TowardZero));
                let src = core::array::from_fn(|c| value[c] - decode(chans[c], dst[c]));
                ClassInput { value, dst, src, alpha: 0.0 }
            }
            Path::BlendOver => {
                let dst: [u32; 3] = core::array::from_fn(|c| encode(chans[c], value[c], Rounding::TowardZero));
                let src = core::array::from_fn(|c| value[c] - decode(chans[c], dst[c]) * 0.5);
                ClassInput { value, dst, src, alpha: 0.5 }
            }
        })
        .collect()
}

/// What a correct `member` writes for `input` on `path` (the three codes).
pub fn predict(target: Target, path: Path, member: Member, input: &ClassInput) -> [u32; 3] {
    let chans = target.channels();
    core::array::from_fn(|c| match path {
        Path::Store | Path::Attachment | Path::Clear => member.convert(chans[c], input.value[c]),
        Path::BlendAdd => member.blend_add(chans[c], input.dst[c], input.src[c]),
        Path::BlendOver => member.blend_over(chans[c], input.dst[c], input.src[c], input.alpha),
    })
}

/// The class's prediction (its representative member's), `None` for `Unmodelled`.
pub fn predict_class(target: Target, path: Path, class: PathClass, input: &ClassInput) -> Option<[u32; 3]> {
    Member::of_class(class).map(|m| predict(target, path, m, input))
}

// ---- The pre-registered table -----------------------------------------------------------------------

/// One row of the expected table: an arm, a target, a class, its metric under AgX and ACES Hill.
#[derive(Clone, Debug)]
pub struct Row {
    /// The arm.
    pub arm: Arm,
    /// The target format.
    pub target: Target,
    /// The class the arm was simulated under.
    pub class: PathClass,
    /// Under AgX.
    pub agx: ArmMetric,
    /// Under Hill ACES.
    pub aces: ArmMetric,
}

impl Row {
    /// The row's table line.
    pub fn line(&self) -> String {
        let ch = ["r", "g", "b"];
        let verdict = if !self.arm.family.decisive() && self.arm.family != Family::A3Blend {
            "reported"
        } else if self.agx.passes() {
            "pass"
        } else {
            "FAIL"
        };
        format!(
            "| {} | {} | {} | {:.2} ({}, j={}) | {:.2} | {:.2} | {:.2} | {verdict} |",
            self.arm.label(),
            self.target.spelling(),
            self.class.name(),
            self.agx.max_delta,
            ch[self.agx.at.0],
            self.agx.at.1,
            self.agx.chroma,
            self.aces.max_delta,
            self.aces.chroma
        )
    }
}

/// The table's header lines.
pub const TABLE_HEADER: &str = "| arm | target | class | AgX max\\|Δ\\| (channel, placement) | AgX chroma | ACES-Hill max\\|Δ\\| | ACES-Hill chroma | verdict (AgX) |\n|---|---|---|---|---|---|---|---|";

/// Every row: every arm × both targets × every class of the arm's path on that target.
pub fn table_rows() -> Vec<Row> {
    let arm_list = arms();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(4);
    let chunk = arm_list.len().div_ceil(threads);
    let mut out: Vec<(usize, Row)> = std::thread::scope(|s| {
        let handles: Vec<_> = arm_list
            .chunks(chunk)
            .enumerate()
            .map(|(ci, arms)| {
                s.spawn(move || {
                    let mut rows = Vec::new();
                    for (ai, arm) in arms.iter().enumerate() {
                        for (ti, target) in [Target::B10G11R11, Target::Rgba16f].into_iter().enumerate() {
                            let ex = exact(arm, target);
                            let agx_ex = exact_display(Curve::Agx, &ex);
                            let aces_ex = exact_display(Curve::AcesHill, &ex);
                            for (ki, class) in boyko_shaderdsl::pixfmt::classes_for(target, arm.path).iter().enumerate() {
                                let member = Member::of_class(*class).expect("invariant: a modelled class");
                                let dev = decode_image(&simulate(arm, target, member), target);
                                let order = ((ci * chunk + ai) * 2 + ti) * 8 + ki;
                                rows.push((
                                    order,
                                    Row {
                                        arm: *arm,
                                        target,
                                        class: *class,
                                        agx: metric(Curve::Agx, &dev, &agx_ex),
                                        aces: metric(Curve::AcesHill, &dev, &aces_ex),
                                    },
                                ));
                            }
                        }
                    }
                    rows
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("invariant: a table worker does not panic")).collect()
    });
    out.sort_by_key(|(o, _)| *o);
    out.into_iter().map(|(_, r)| r).collect()
}

/// The tightest decisive arm on B10G11R11 under `N`: the decisive arm whose AgX metric comes
/// closest to the threshold (the Punchy row's content).
pub fn tightest_decisive(rows: &[Row]) -> Option<Arm> {
    rows.iter()
        .filter(|r| r.arm.family.decisive() && r.target == Target::B10G11R11 && r.class == PathClass::N)
        .max_by(|a, b| {
            let ka = a.agx.max_delta.max(a.agx.chroma);
            let kb = b.agx.max_delta.max(b.agx.chroma);
            ka.total_cmp(&kb)
        })
        .map(|r| r.arm)
}

/// The reported Punchy lines: the tightest decisive arm under every class of its path on
/// B10G11R11, through AgX + Punchy.
pub fn punchy_lines(arm: &Arm) -> Vec<String> {
    let target = Target::B10G11R11;
    let ex = exact(arm, target);
    let disp = exact_display(Curve::AgxPunchy, &ex);
    boyko_shaderdsl::pixfmt::classes_for(target, arm.path)
        .iter()
        .map(|class| {
            let member = Member::of_class(*class).expect("invariant: a modelled class");
            let dev = decode_image(&simulate(arm, target, member), target);
            let m = metric(Curve::AgxPunchy, &dev, &disp);
            format!(
                "| {} | {} | {} | {:.2} | {:.2} | reported (never decisive) |",
                arm.label(),
                target.spelling(),
                class.name(),
                m.max_delta,
                m.chroma
            )
        })
        .collect()
}

/// The header of the Punchy lines.
pub const PUNCHY_HEADER: &str = "| arm | target | class | AgX-Punchy max\\|Δ\\| | AgX-Punchy chroma | status |\n|---|---|---|---|---|---|";

/// The whole pinned block (between the record's `px0-expected` markers, exclusive).
pub fn expected_block() -> String {
    let rows = table_rows();
    let mut s = String::new();
    s.push_str(TABLE_HEADER);
    s.push('\n');
    for r in &rows {
        s.push_str(&r.line());
        s.push('\n');
    }
    s.push('\n');
    if let Some(arm) = tightest_decisive(&rows) {
        s.push_str(&format!("Punchy (rev-6.2 H3 row) on the tightest decisive arm, `{}`:\n\n", arm.label()));
        s.push_str(PUNCHY_HEADER);
        s.push('\n');
        for l in punchy_lines(&arm) {
            s.push_str(&l);
            s.push('\n');
        }
    }
    s
}
