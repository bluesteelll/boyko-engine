//! Render wave E0 — the DEVICE precision probe of the five shared eDSL primitives
//! (`Cf::{floor, frac, log2, exp2, pow}`) against "E0's bound", the Vulkan SPIR-V environment's
//! precision table (`boyko_shaderdsl::prims_probe`, D-E3).
//!
//! The test-only kernel `shaders/e0_prims_probe.comp.hlsl` (generated whole by `emit_e0_probe`,
//! byte-gated by `e0_prims_edsl_sync.rs`) evaluates each op on 64 k inputs; this file compares the
//! device words with the host `f64` reference by the acceptance rule D-E2 (cut `e0-pxe/cut.md`,
//! rounding mode per critique W5):
//!
//! | class | rule |
//! |---|---|
//! | finite normal input, finite normal result | `floor`: equal by value; `frac`: either bracketing f32 of the exact result (the rounding mode is implementation-defined); `log2` / `exp2` / `pow`: within the Vulkan bound of the exact `f64` result |
//! | subnormal input or subnormal result | the bound on the input as given, OR the result on the flushed (zero) input / output |
//! | NaN / `±Inf` / `−0` input, a `±Inf` result, `log2(x ≤ 0)`, `pow(x < 0, y)`, `pow(0, y)` | REPORTED, never gated: no shader here declares `SignedZeroInfNanPreserve`, so NotNaN / NotInf / NSZ are assumed, and `pow(0, y)` inherits `log2(0) = −∞` |
//!
//! Two tests:
//!
//! - `e0_bounds_comparator_flags_out_of_bound_results` (NOT ignored) — the comparator itself: the
//!   host's own results pass on the whole probe grid; an injected 4-ULP `log2` error outside
//!   `[0.5, 2]`, a truncating `frac` on a negative and a `2⁻²⁰` `log2` error inside `[0.5, 2]` are
//!   each flagged BY OP AND INPUT; a round-toward-zero `frac` passes. So the device gate can fail.
//! - `e0_prims_match_host_within_vulkan_bounds` (`gpu`-ignored) — the device run: dispatch, read
//!   back, judge every row, print the per-op table (worst `|Δ|/bound`, worst ULP distance, the
//!   flush outcome, the report-only rows). A finite normal row over the bound is a driver out of
//!   spec: STOP and report the row; the bound constants are never widened.
//!
//! Run (GL-E0): `BOYKO_DISABLE_VALIDATION=1 cargo test -p boyko_rhi_vulkan --test e0_prims_device
//! --locked -- --ignored --test-threads=1`.

use core::ptr::NonNull;

use boyko_rhi::{
    BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_shaderdsl::cf::{Cf, EvalCf};
use boyko_shaderdsl::prims_probe::{
    E0_PROBE_ELEMENTS, E0_PROBE_LOCAL_SIZE, E0_PROBE_OUT_BASE, E0_PROBE_STRIDE, E0Op, e0_eval,
    frac_bracket, ulp_distance, vk_exp2_bound, vk_log2_bound, vk_pow_bound,
};

/// The committed probe kernel (`include_bytes!` is `align(1)`; the words are re-assembled below).
static E0_PROBE_SPV: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/e0_prims_probe.comp.spv"));

/// The kernel's SPIR-V word stream (little-endian, as `dxc` writes it).
fn spv_words() -> Vec<u32> {
    assert!(E0_PROBE_SPV.len().is_multiple_of(4), "a SPIR-V module is whole words");
    E0_PROBE_SPV.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

// ---- The input grid ---------------------------------------------------------------------------

/// SplitMix64 — a deterministic, dependency-free sample source.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The 64 k `(x, y)` probe inputs, four quarters of 16 k: the `floor`/`frac` edge classes and
/// random bit patterns (every class, both signs, specials); `log2`'s log-spaced `[2⁻¹⁴⁹, 2¹²⁸)`;
/// `exp2`'s `[-150, 129)`; `pow`'s `(0, 2²⁰] × [-8, 8]` plus `pow(0, y)` rows. Every element runs
/// all five ops; each op's rule decides which rows it gates.
fn probe_inputs() -> Vec<(f32, f32)> {
    let quarter = (E0_PROBE_ELEMENTS / 4) as usize;
    let mut v: Vec<(f32, f32)> = Vec::with_capacity(E0_PROBE_ELEMENTS as usize);

    let specials = [
        0.0f32,
        -0.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(1),
        -f32::from_bits(1),
        8_388_608.0,
        -8_388_608.0,
        8_388_607.5,
        -8_388_607.5,
        -0.25,
        -0.3,
        -f32::from_bits(0.25f32.to_bits() + 1),
        -(2.0f32.powi(-24)),
        -(2.0f32.powi(-25)),
        -(2.0f32.powi(-31)),
    ];
    v.extend(specials.iter().map(|&x| (x, 1.0)));
    let lo = 2.0f32.powi(-24).to_bits();
    for k in 0..2048u32 {
        v.push((-f32::from_bits(lo - k * (lo / 2048)), 1.0));
    }
    let mut rng = Rng(0xE0_DE71CE);
    while v.len() < quarter {
        v.push((f32::from_bits(rng.next_u64() as u32), 1.0));
    }

    for k in 0..quarter {
        let x = (-149.0 + 277.0 * k as f64 / quarter as f64).exp2() as f32;
        v.push((x, 1.0));
    }
    for k in 0..quarter {
        v.push(((-150.0 + 279.0 * k as f64 / quarter as f64) as f32, 1.0));
    }

    let side = 128usize;
    for i in 0..side {
        let x = (-149.0 + 169.0 * (i + 1) as f64 / side as f64).exp2() as f32;
        for j in 0..side {
            let y = (-8.0 + 16.0 * j as f64 / (side - 1) as f64) as f32;
            v.push((x, y));
        }
    }
    // pow(0, y) rows (report-only, W5b) replace the first few pow-quarter rows.
    let pow_base = 3 * quarter;
    for (k, y) in [2.0f32, 0.5, 8.0, -1.0].iter().enumerate() {
        v[pow_base + k] = (0.0, *y);
    }
    assert_eq!(v.len(), E0_PROBE_ELEMENTS as usize, "the grid fills the probe exactly");
    v
}

// ---- The comparator (D-E2) --------------------------------------------------------------------

/// The verdict on one `(op, x, y, device)` row.
#[derive(Debug, Clone, PartialEq)]
enum Verdict {
    /// Within the bound on the input as given; `ratio` = `|Δ| / bound` (0 for an exact op that
    /// matched), `ulp` = the ULP distance to the f32-rounded reference.
    Pass { ratio: f64, ulp: u64 },
    /// Accepted as the result on a FLUSHED (zero) input or output.
    PassFlushed,
    /// Never gated (the class is unspecified by the environment); the reason names it.
    Report(&'static str),
    /// Out of bound — the message names the op, the input, the device value and the bound.
    Fail(String),
}

/// Gates `device` against the absolute `bound` around the exact `reference`.
fn within(op: E0Op, x: f32, y: f32, device: f32, reference: f64, bound: f64) -> Verdict {
    let err = (f64::from(device) - reference).abs();
    if device.is_finite() && err <= bound {
        Verdict::Pass { ratio: err / bound, ulp: ulp_distance(device, reference as f32) }
    } else {
        Verdict::Fail(format!(
            "{}(x = {x:e} [{:#010x}], y = {y:e}) = {device:e} [{:#010x}], exact {reference:e}, \
             |err| {err:e} > bound {bound:e}",
            op.name(),
            x.to_bits(),
            device.to_bits()
        ))
    }
}

/// Fails with the op, the input and the accepted set.
fn reject(op: E0Op, x: f32, device: f32, accepted: &str) -> Verdict {
    Verdict::Fail(format!(
        "{}(x = {x:e} [{:#010x}]) = {device:e} [{:#010x}], accepted {accepted}",
        op.name(),
        x.to_bits(),
        device.to_bits()
    ))
}

/// The D-E2 acceptance rule for one row.
fn judge(op: E0Op, x: f32, y: f32, device: f32) -> Verdict {
    if !x.is_finite() || (x == 0.0 && x.is_sign_negative()) {
        return Verdict::Report("NaN / Inf / -0 input (NotNaN, NotInf, NSZ assumed)");
    }
    match op {
        E0Op::Floor => {
            let exact = <EvalCf as Cf>::floor(x);
            // Integer results are representable: any rounding mode gives `exact`. Zero results
            // compare by value (NSZ).
            if device == exact {
                Verdict::Pass { ratio: 0.0, ulp: 0 }
            } else if x.is_subnormal() && device == 0.0 {
                Verdict::PassFlushed
            } else {
                reject(op, x, device, &format!("{exact:e}"))
            }
        }
        E0Op::Frac => {
            let (lo, hi) = frac_bracket(x);
            if device == lo || device == hi {
                Verdict::Pass { ratio: 0.0, ulp: ulp_distance(device, lo).min(ulp_distance(device, hi)) }
            } else if x.is_subnormal() && device == 0.0 {
                Verdict::PassFlushed
            } else {
                reject(op, x, device, &format!("{{{lo:e}, {hi:e}}}"))
            }
        }
        E0Op::Log2 => {
            if x <= 0.0 {
                return Verdict::Report("log2 of a non-positive input (-Inf / NaN, NotInf assumed)");
            }
            let reference = f64::from(x).log2();
            let v = within(op, x, y, device, reference, vk_log2_bound(x, reference));
            if matches!(v, Verdict::Fail(_)) && x.is_subnormal() && device == f32::NEG_INFINITY {
                Verdict::PassFlushed
            } else {
                v
            }
        }
        E0Op::Exp2 => {
            let reference = f64::from(x).exp2();
            if reference >= f64::from(f32::MAX) {
                return Verdict::Report("exp2 overflows f32 (NotInf assumed)");
            }
            let v = within(op, x, y, device, reference, vk_exp2_bound(x, reference));
            if matches!(v, Verdict::Fail(_)) && reference < f64::from(f32::MIN_POSITIVE) && device == 0.0 {
                Verdict::PassFlushed
            } else {
                v
            }
        }
        E0Op::Pow => {
            if !y.is_finite() {
                return Verdict::Report("NaN / Inf exponent");
            }
            if x < 0.0 {
                return Verdict::Report("pow(x < 0, y) is undefined (GLSL.std.450 Pow)");
            }
            if x == 0.0 {
                return Verdict::Report("pow(0, y) inherits log2(0) = -Inf (NotInf assumed)");
            }
            let reference = f64::from(x).powf(f64::from(y));
            if reference >= f64::from(f32::MAX) {
                return Verdict::Report("pow overflows f32 (NotInf assumed)");
            }
            let bound = vk_pow_bound(x, y, f64::from(x).log2(), reference);
            let v = within(op, x, y, device, reference, bound);
            if matches!(v, Verdict::Fail(_)) {
                if reference < f64::from(f32::MIN_POSITIVE) && device == 0.0 {
                    return Verdict::PassFlushed;
                }
                if x.is_subnormal() {
                    return Verdict::Report("subnormal base (pow of the flushed 0 is unspecified)");
                }
            }
            v
        }
    }
}

/// The per-op aggregate the device run prints.
#[derive(Default)]
struct OpStats {
    gated: usize,
    flushed: usize,
    reported: usize,
    worst_ratio: f64,
    worst_ulp: u64,
    fails: Vec<String>,
}

/// Judges every row of `outputs` (indexed `[element][op]`) and returns the per-op stats.
fn judge_all(inputs: &[(f32, f32)], outputs: &[[f32; 5]]) -> [OpStats; 5] {
    let mut stats: [OpStats; 5] = Default::default();
    let mut reports_printed = 0usize;
    for (i, (&(x, y), out)) in inputs.iter().zip(outputs).enumerate() {
        for (k, op) in E0Op::ALL.into_iter().enumerate() {
            let s = &mut stats[k];
            match judge(op, x, y, out[k]) {
                Verdict::Pass { ratio, ulp } => {
                    s.gated += 1;
                    s.worst_ratio = s.worst_ratio.max(ratio);
                    s.worst_ulp = s.worst_ulp.max(ulp);
                }
                Verdict::PassFlushed => s.flushed += 1,
                Verdict::Report(why) => {
                    s.reported += 1;
                    if reports_printed < 24 {
                        reports_printed += 1;
                        eprintln!(
                            "  report-only  {}(x = {x:e}, y = {y:e}) = {:e} [{:#010x}]  ({why})",
                            op.name(),
                            out[k],
                            out[k].to_bits()
                        );
                    }
                }
                Verdict::Fail(msg) => {
                    s.gated += 1;
                    if s.fails.len() < 8 {
                        s.fails.push(format!("element {i}: {msg}"));
                    }
                }
            }
        }
    }
    stats
}

/// The host's own results for every element (the Eval probe bodies).
fn host_outputs(inputs: &[(f32, f32)]) -> Vec<[f32; 5]> {
    inputs
        .iter()
        .map(|&(x, y)| E0Op::ALL.map(|op| e0_eval(op, x, y)))
        .collect()
}

#[test]
fn e0_bounds_comparator_flags_out_of_bound_results() {
    // 1. The host's own results pass on the whole grid (no row of a correct implementation is
    //    rejected), and every op is actually gated on it (the grid is not vacuous for any op).
    let inputs = probe_inputs();
    let outputs = host_outputs(&inputs);
    let stats = judge_all(&inputs, &outputs);
    for (op, s) in E0Op::ALL.iter().zip(&stats) {
        assert!(s.fails.is_empty(), "the host's own {} results were rejected:\n{}", op.name(), s.fails.join("\n"));
        assert!(s.gated >= 4096, "{}: only {} gated rows — the grid is vacuous for it", op.name(), s.gated);
    }

    // 2. Each injected out-of-bound result is flagged, naming the op and the input.
    let three = 3.0f32;
    let injected = [
        // A 4-ULP log2 error OUTSIDE [0.5, 2] (bound: 3 ULP).
        (E0Op::Log2, 8.0f32, 1.0f32, f32::from_bits(three.to_bits() + 4), "log2(x = 8e0"),
        // `frac` written as `x - trunc(x)` on a negative input.
        (E0Op::Frac, -0.25, 1.0, -0.25, "frac(x = -2.5e-1"),
        // A 2⁻²⁰ log2 error INSIDE [0.5, 2] (bound: absolute 2⁻²¹).
        (E0Op::Log2, 1.5, 1.0, (1.5f64.log2() + 2.0f64.powi(-20)) as f32, "log2(x = 1.5e0"),
        // `floor` written as a truncation.
        (E0Op::Floor, -0.25, 1.0, -0.0, "floor(x = -2.5e-1"),
        // `pow` with its operands swapped.
        (E0Op::Pow, 2.0, 10.0, 100.0, "pow(x = 2e0"),
    ];
    for (op, x, y, device, names) in injected {
        match judge(op, x, y, device) {
            Verdict::Fail(msg) => assert!(
                msg.starts_with(names),
                "the {} rejection must name the op and the input (`{names}…`), got: {msg}",
                op.name()
            ),
            other => panic!("an out-of-bound {} result was accepted: {:?} for x = {x}, device {device:e}", op.name(), other),
        }
    }

    // 3. The rounding-mode admission (W5): a round-toward-zero `frac` of an inexact negative is
    //    the lower bracketing f32 and passes; the round-to-nearest value passes too.
    // x = -(0.25 + 2⁻²⁵): `1 + x = 0.75 - 2⁻²⁵` needs the 2⁻²⁵ bit, which [0.5, 1) does not have.
    let x = -f32::from_bits(0.25f32.to_bits() + 1);
    let (lo, hi) = frac_bracket(x);
    assert!(lo < hi, "frac(-(0.25 + 2^-25)) is inexact, so its bracket is two values");
    assert!(matches!(judge(E0Op::Frac, x, 1.0, lo), Verdict::Pass { .. }), "an RTZ frac must pass");
    assert!(matches!(judge(E0Op::Frac, x, 1.0, hi), Verdict::Pass { .. }), "an RTE frac must pass");
    let (lo1, hi1) = frac_bracket(-(2.0f32.powi(-31)));
    assert_eq!((lo1, hi1), (f32::from_bits(1.0f32.to_bits() - 1), 1.0), "the tiny-negative frac bracket");

    // 4. The report-only classes are never gated.
    assert!(matches!(judge(E0Op::Pow, 0.0, 2.0, 123.0), Verdict::Report(_)), "pow(0, y) is report-only");
    assert!(matches!(judge(E0Op::Log2, f32::NAN, 1.0, 0.0), Verdict::Report(_)), "a NaN input is report-only");
}

// ---- The device run ---------------------------------------------------------------------------

/// Writes the inputs into the probe buffer's persistent host-coherent mapping.
fn write_inputs(base: NonNull<u8>, inputs: &[(f32, f32)]) {
    let words = base.as_ptr().cast::<u32>();
    for (i, &(x, y)) in inputs.iter().enumerate() {
        let at = i * E0_PROBE_STRIDE as usize;
        // SAFETY: the buffer is `E0_PROBE_ELEMENTS * E0_PROBE_STRIDE * 4` bytes inside the
        // persistent host-coherent mapping and `i < E0_PROBE_ELEMENTS`, so words `at` and `at + 1`
        // are in bounds; no submission is in flight yet, so the host is the only accessor; the
        // pointer comes from the live mapping (its provenance covers the whole buffer), and
        // `write_unaligned` tolerates the sub-allocated offset's alignment.
        unsafe {
            words.add(at).write_unaligned(x.to_bits());
            words.add(at + 1).write_unaligned(y.to_bits());
        }
    }
}

/// Reads the five result words of every element back from the mapping.
fn read_outputs(base: NonNull<u8>, n: usize) -> Vec<[f32; 5]> {
    let words = base.as_ptr().cast::<u32>();
    (0..n)
        .map(|i| {
            let at = i * E0_PROBE_STRIDE as usize + E0_PROBE_OUT_BASE as usize;
            core::array::from_fn(|k| {
                // SAFETY: `at + k < (i + 1) * E0_PROBE_STRIDE` (the five results sit inside the
                // element's stride, const-asserted in `prims_probe`) and `i < n =
                // E0_PROBE_ELEMENTS`, so the word is inside the mapping; the fence wait before this
                // call completed the only submission, so the GPU writes are finished and visible
                // through the coherent mapping; the pointer's provenance is the whole mapping.
                f32::from_bits(unsafe { words.add(at + k).read_unaligned() })
            })
        })
        .collect()
}

#[test]
#[ignore = "gpu: needs a Vulkan device — E0 intrinsic precision probe; run --ignored --test-threads=1"]
fn e0_prims_match_host_within_vulkan_bounds() {
    let ctx = match VulkanContext::boot(InstanceConfig { enable_validation: true, ..InstanceConfig::default() }) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("SKIP e0_prims_match_host_within_vulkan_bounds: no Vulkan device ({e:?})");
            return;
        }
    };
    println!("Vulkan device: {}", ctx.device_name());
    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    let inputs = probe_inputs();
    let n = inputs.len();
    let bytes = (n as u64) * u64::from(E0_PROBE_STRIDE) * 4;
    let buffer = device
        .create_buffer(&BufferDesc { size: bytes, usage: BufferUsage::STORAGE, location: MemoryLocation::HostVisibleCoherent })
        .expect("probe storage buffer");
    let mapped = device.buffer_mapped_ptr(&buffer).expect("a host-visible buffer is mapped");
    write_inputs(mapped, &inputs);

    let module = device.create_shader_module(&spv_words()).expect("e0_prims_probe shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
            spec_constants: &[],
        })
        .expect("e0_prims_probe compute pipeline");
    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    encoder.begin().expect("begin");
    encoder.bind_compute_pipeline(&pipeline);
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &(n as u32).to_ne_bytes());
    encoder.dispatch((n as u32).div_ceil(E0_PROBE_LOCAL_SIZE), 1, 1);
    encoder.end().expect("end");
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let outputs = read_outputs(mapped, n);
    let stats = judge_all(&inputs, &outputs);
    println!("op     gated  flushed  reported  worst |err|/bound  worst ULP");
    for (op, s) in E0Op::ALL.iter().zip(&stats) {
        println!(
            "{:<6} {:>6} {:>8} {:>9} {:>18.4} {:>10}",
            op.name(),
            s.gated,
            s.flushed,
            s.reported,
            s.worst_ratio,
            s.worst_ulp
        );
    }

    if ctx.validation_enabled() {
        let state = ctx.debug_state().expect("validation enabled => a debug-messenger state is present");
        assert_eq!(state.total(), 0, "validation reported {} message(s) during the probe", state.total());
    } else {
        eprintln!("NOTE: validation disabled - messenger oracle skipped");
    }

    // SAFETY: every resource below was created on `device` and is destroyed exactly once; the only
    // submission completed (fence-waited above), so none is in use by the GPU.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_buffer(buffer);
    }
    drop(ctx);

    let fails: Vec<String> = E0Op::ALL
        .iter()
        .zip(&stats)
        .filter(|(_, s)| !s.fails.is_empty())
        .map(|(op, s)| format!("{}:\n  {}", op.name(), s.fails.join("\n  ")))
        .collect();
    assert!(
        fails.is_empty(),
        "DEVICE OUT OF THE VULKAN BOUND on finite normal rows — a driver out of spec. STOP and report \
         these rows; the bound constants are never widened:\n{}",
        fails.join("\n")
    );
}
