//! Render wave PX0 (H3 Scope 3 / 3b; `px0/cut.md` PX0-5) — **the pre-registered chain model**:
//! the `lit` format decision's expected outcome, computed on the host BEFORE the device runs and
//! pinned into the measurement record.
//!
//! Host only, not ignored, pure arithmetic over `lit_format` (the frozen shared model):
//! - [`probe_inputs_separate_classes`] — every pair of non-equivalent classes of each (format,
//!   path) differs on at least one input of that path's set, and the equivalences hold as
//!   properties of the model (`Z16 ≡ Z` everywhere, `N16 ≡ N` on RGBA16F), so "more than one class
//!   matches the device" cannot happen by construction (OPEN-ITEMS #2, `critique_verify_r5` I1);
//! - [`expected_table_is_pinned`] — the per-arm × per-class table under AgX (ACES Hill beside it,
//!   Punchy reported) equals the record's `px0-expected` block, so a later edit of either side is
//!   red;
//! - [`placement_invariance`] — scaling an arm by `2^j` leaves its per-op errors bit-identical over
//!   the normal-range placements (`px0/critique.md` O3: below the smallest normal it is not);
//! - [`flat_stall_equals_s_over_2a`] — K3's A2-flat stall;
//! - [`n_family_passes_and_z_family_fails_a1s1_k64`] — the model separates the families on the
//!   arm the decision is tightest on;
//! - [`the_format_decision_curve_is_the_shipping_default`] — D-P4's tripwire (verify r5 O1);
//! - [`the_record_names_neutral_as_its_decided_curve`] — the tripwire's record parse reads the real
//!   record (render wave PXt);
//! - [`the_decided_curve_keeps_the_decided_format`] — row 6: the record's decided curve passes the
//!   content rule on RGBA16F (render wave PXt, ruling 116).
//!
//! [`write_expected_block`] (`generator:`) rewrites the record's block from this tree's model.

mod lit_format;

use std::path::PathBuf;

use boyko_shaderdsl::pixfmt::{Channel, Member, Path, PathClass, Target, classes_for, decode};

use lit_format::{
    Arm, Curve, Family, class_inputs, decode_image, exact, exact_display, expected_block, metric,
    predict, predict_class, simulate,
};

/// The record this model pre-registers into.
const RECORD: &str = "docs/measurements/2026-10-16-px0-lit-format-decision.md";
const BEGIN: &str = "<!-- px0-expected:begin -->";
const END: &str = "<!-- px0-expected:end -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn record_text() -> String {
    let p = repo_root().join(RECORD);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())).replace("\r\n", "\n")
}

/// The pinned block: the text strictly between the markers, trimmed of the surrounding newlines.
fn pinned_block(record: &str) -> &str {
    let a = record.find(BEGIN).unwrap_or_else(|| panic!("{RECORD} has no `{BEGIN}`"));
    let b = record.find(END).unwrap_or_else(|| panic!("{RECORD} has no `{END}`"));
    record[a + BEGIN.len()..b].trim_matches('\n')
}

/// **Every pair of non-equivalent classes is separated by every path's input set**, and the
/// equivalences the classes merge are properties of the model.
#[test]
fn probe_inputs_separate_classes() {
    let mut failures = Vec::new();
    for target in [Target::B10G11R11, Target::Rgba16f] {
        for path in Path::ALL {
            let inputs = class_inputs(target, path);
            assert!(!inputs.is_empty(), "{target:?}/{path:?}: an empty input set separates nothing");
            let classes = classes_for(target, path);
            for (i, a) in classes.iter().enumerate() {
                for b in &classes[i + 1..] {
                    let separated = inputs.iter().any(|x| predict_class(target, path, *a, x) != predict_class(target, path, *b, x));
                    if !separated {
                        failures.push(format!(
                            "({}, {}, {} vs {}): no input separates the two classes — enrich the set, or merge them with a proof",
                            target.spelling(),
                            path.spelling(),
                            a.name(),
                            b.name()
                        ));
                    }
                }
            }
            // (b) The merged members ARE equivalent on every input of the set.
            for x in &inputs {
                assert_eq!(
                    predict(target, path, Member::Z16, x),
                    predict(target, path, Member::Z, x),
                    "{target:?}/{path:?}: Z16 and Z disagree on {x:?}"
                );
                if target == Target::Rgba16f {
                    assert_eq!(
                        predict(target, path, Member::N16, x),
                        predict(target, path, Member::N, x),
                        "RGBA16F/{path:?}: N16 and N disagree on {x:?}"
                    );
                }
            }
            // Every input targets a finite value at or below the channel's largest (open
            // question 4): a device's overflow rule never enters classification.
            let chans = target.channels();
            for x in &inputs {
                for (v, ch) in x.value.iter().zip(chans) {
                    assert!(v.is_finite() && *v <= ch.max_finite(), "{x:?}");
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// **The pre-registration pin.** The table this tree's model computes equals the record's block.
#[test]
fn expected_table_is_pinned() {
    let record = record_text();
    let pinned = pinned_block(&record);
    let model = expected_block();
    let model = model.trim_matches('\n');
    if pinned != model {
        let first = pinned.lines().zip(model.lines()).position(|(a, b)| a != b);
        let (p, m) = match first {
            Some(i) => (pinned.lines().nth(i).unwrap_or(""), model.lines().nth(i).unwrap_or("")),
            None => ("<length differs>", "<length differs>"),
        };
        panic!(
            "{RECORD}'s px0-expected block differs from the model (first differing line {first:?}):\n  record: {p}\n  model:  {m}\nThe block is the pre-registration: it is regenerated only by a deliberate model change, never after the device run."
        );
    }
    assert!(model.contains("Punchy"), "the reported Punchy arm is part of the pre-registration (rev-6.2 H3 row)");
}

/// **The placement argument** (H3 3b) over the normal range: an arm scaled by `2^j` simulates to
/// exactly `2^j` times the unscaled image, member by member. Restricted to `j ∈ [−5, 4]` on the
/// A1-S1 arms over black: below that the first layers of the fire arm's blue fall under `2^-14`,
/// where power-of-two scaling is not exact (`px0/critique.md` O3).
#[test]
fn placement_invariance() {
    for color in [[1.0f32, 1.0, 1.0], [1.0, 0.45, 0.12]] {
        for k in [16u32, 64] {
            let base = Arm { family: Family::A1S1, k, color, scale: 3.2 / k as f32, background: 0.0, path: Path::BlendAdd };
            for member in [Member::N, Member::Z, Member::N16, Member::Npre] {
                let one = decode_image(&simulate(&base, Target::B10G11R11, member), Target::B10G11R11);
                for j in -5..=4 {
                    let s = 2f32.powi(j);
                    let scaled = Arm { scale: base.scale * s, ..base };
                    let img = decode_image(&simulate(&scaled, Target::B10G11R11, member), Target::B10G11R11);
                    for (px, (a, b)) in one.iter().zip(&img).enumerate() {
                        for c in 0..3 {
                            assert_eq!(a[c] * f64::from(s), b[c], "{member:?} k{k} {color:?} j={j} px {px} channel {c}");
                        }
                    }
                }
            }
        }
    }
}

/// **K3's stall.** A flat (equal-colour) over stack under `N` stops where the increment falls under
/// half a step — `s / (2 · a_max)` short of the colour — within one step `s`, on every pixel.
#[test]
fn flat_stall_equals_s_over_2a() {
    let c = 0.2f32;
    let a_bar = 0.1f32;
    let arm = Arm { family: Family::A2Flat, k: 256, color: [c; 3], scale: a_bar, background: 0.0, path: Path::BlendOver };
    let img = simulate(&arm, Target::B10G11R11, Member::N);
    let a_max = f64::from(a_bar) * 1.5;
    for (ch_i, ch) in [(0usize, Channel::Uf11), (2, Channel::Uf10)] {
        // `s` is the step of the binade the stall lands in ([0.125, 0.25) for c = 0.2).
        let s = 2f64.powi(-3 - ch.mantissa_bits() as i32);
        let want = s / (2.0 * a_max);
        for (px, p) in img.iter().enumerate() {
            let gap = f64::from(c) - f64::from(decode(ch, p[ch_i]));
            // Within one step either way: the stall is set by the largest alpha the remaining
            // layers actually draw, which approaches `a_max` from below.
            assert!(
                (gap - want).abs() <= s + 1e-9,
                "{ch:?} px {px}: stalled {gap:.6} short of {c}, the model says s/(2·a_max) = {want:.6} within s = {s:.6}"
            );
        }
    }
}

/// **The families separate where the decision is tightest**: A1-S1 at k = 64 passes under `N` and
/// fails under `Z` through AgX, on both colours.
#[test]
fn n_family_passes_and_z_family_fails_a1s1_k64() {
    for color in [[1.0f32, 1.0, 1.0], [1.0, 0.45, 0.12]] {
        let arm = Arm { family: Family::A1S1, k: 64, color, scale: 0.05, background: 0.0, path: Path::BlendAdd };
        let ex = exact(&arm, Target::B10G11R11);
        let disp = exact_display(Curve::Agx, &ex);
        let n = metric(Curve::Agx, &decode_image(&simulate(&arm, Target::B10G11R11, Member::N), Target::B10G11R11), &disp);
        let z = metric(Curve::Agx, &decode_image(&simulate(&arm, Target::B10G11R11, Member::Z), Target::B10G11R11), &disp);
        assert!(n.passes(), "{}: N fails ({n:?})", arm.label());
        assert!(!z.passes(), "{}: Z passes ({z:?}) — the model no longer separates the families", arm.label());
    }
}

/// **D-P4 — the decision's curve is the shipping default.** While `post.rs` carries no AgX word,
/// the default must be `Tonemapper::Aces` (the PX1a / PX1b transitional default, H3 row 6
/// exempt); once an AgX word exists (PXt), the default must be the curve the record decided under
/// (`decided_curve`), so K9's revert of the default reds in the commit that makes it.
#[test]
fn the_format_decision_curve_is_the_shipping_default() {
    let post = std::fs::read_to_string(repo_root().join("crates/boyko_shaderdsl/src/post.rs")).expect("post.rs is readable");
    let has_agx = post.to_ascii_lowercase().contains("agx");
    let default = format!("{:?}", boyko_render::Tonemapper::default()).to_ascii_lowercase();
    if !has_agx {
        assert_eq!(default, "aces", "post.rs has no AgX word, so the default must be the transitional ACES Hill");
        return;
    }
    let record = record_text();
    let decided = decided_curve(&record).unwrap_or_else(|| panic!("{RECORD} names no `decided_curve`"));
    assert_eq!(
        default, decided,
        "the shipping default curve changed after PX0: H3 row 6 re-runs the content arms and the pinned model under the new curve before the change merges"
    );
}

/// The curve the record decided under: the value of its `decided_curve = "<curve>"` line, which the
/// record writes as a back-ticked Markdown code span (`` `decided_curve = "agx"` ``) — so the
/// back-ticks are stripped with the whitespace.
fn decided_curve(record: &str) -> Option<&str> {
    record
        .lines()
        .find_map(|l| l.trim().trim_matches('`').strip_prefix("decided_curve = \""))
        .and_then(|v| v.strip_suffix('"'))
}

/// Render wave PXt (`pxt/critique.md` C1; ruling 116 amended the record's curve to Khronos PBR
/// Neutral, its §10): the tripwire's parse finds the decided curve on the REAL record, so its live
/// branch (once `post.rs` carries an AgX word) compares the default with the record's curve instead
/// of panicking that the record names none.
#[test]
fn the_record_names_neutral_as_its_decided_curve() {
    assert_eq!(
        decided_curve(&record_text()),
        Some("neutral"),
        "{RECORD}: the tripwire cannot read the record's `decided_curve` line"
    );
}

/// **Row 6 — the decided curve keeps the decided format** (render wave PXt, ruling 116; the
/// record's §10). The record's `decided_curve` names a curve this model evaluates, and every
/// decisive content arm passes §2's rule under it on RGBA16F — the format PX0 decided — under EVERY
/// modelled class of the arm's path: the device classified RGBA16F's store as `Z` and its blends as
/// `Unmodelled:1.000`, so no single class stands for the blends. B10G11R11's numbers under its
/// device class (`Z` on every path) are printed beside them for the record, never asserted.
/// `--nocapture` prints the table the record's §10 quotes.
#[test]
fn the_decided_curve_keeps_the_decided_format() {
    let record = record_text();
    let decided = decided_curve(&record).unwrap_or_else(|| panic!("{RECORD} names no `decided_curve`"));
    let curve = Curve::from_decided(decided)
        .unwrap_or_else(|| panic!("{RECORD}: `decided_curve = \"{decided}\"` names no curve of the content model"));
    let n = curve.name();
    println!(
        "| arm | RGBA16F `Z`: {n} max\\|Δ\\| (channel, placement) | {n} chroma | RGBA16F, every modelled class: worst max\\|Δ\\| / chroma | verdict ({n}, RGBA16F) | B10G11R11 `Z`: {n} max\\|Δ\\| / chroma | B10G11R11 `Z` (record only) |"
    );
    println!("|---|---|---|---|---|---|---|");
    let ch = ["r", "g", "b"];
    let (mut fails, mut b10_fails, mut arms) = (Vec::new(), 0usize, 0usize);
    let (mut worst_delta, mut worst_chroma) = (0.0f64, 0.0f64);
    for arm in lit_format::arms().into_iter().filter(|a| a.family.decisive()) {
        arms += 1;
        let ex = exact(&arm, Target::Rgba16f);
        let disp = exact_display(curve, &ex);
        let mut z = None;
        let (mut arm_delta, mut arm_chroma, mut arm_passes) = (0.0f64, 0.0f64, true);
        for class in classes_for(Target::Rgba16f, arm.path) {
            let member = Member::of_class(*class).expect("invariant: a modelled class");
            let m = metric(curve, &decode_image(&simulate(&arm, Target::Rgba16f, member), Target::Rgba16f), &disp);
            arm_delta = arm_delta.max(m.max_delta);
            arm_chroma = arm_chroma.max(m.chroma);
            arm_passes &= m.passes();
            if *class == PathClass::Z {
                z = Some(m);
            }
        }
        let z = z.expect("invariant: every RGBA16F path has a `Z` class");
        worst_delta = worst_delta.max(arm_delta);
        worst_chroma = worst_chroma.max(arm_chroma);
        if !arm_passes {
            fails.push(arm.label());
        }
        let b10_ex = exact(&arm, Target::B10G11R11);
        let b10 = metric(
            curve,
            &decode_image(&simulate(&arm, Target::B10G11R11, Member::Z), Target::B10G11R11),
            &exact_display(curve, &b10_ex),
        );
        if !b10.passes() {
            b10_fails += 1;
        }
        println!(
            "| {} | {:.3} ({}, j={}) | {:.3} | {:.3} / {:.3} | {} | {:.3} / {:.3} | {} |",
            arm.label(),
            z.max_delta,
            ch[z.at.0],
            z.at.1,
            z.chroma,
            arm_delta,
            arm_chroma,
            if arm_passes { "pass" } else { "FAIL" },
            b10.max_delta,
            b10.chroma,
            if b10.passes() { "pass" } else { "FAIL" }
        );
    }
    println!(
        "\n{n}: RGBA16F {} of {arms} decisive arms fail under every modelled class (worst max|Δ| {worst_delta:.3}, worst chroma {worst_chroma:.3} LSB; threshold {} LSB); B10G11R11 under `Z`: {b10_fails} of {arms} fail (record only)",
        fails.len(),
        lit_format::THRESHOLD_LSB
    );
    assert_eq!(arms, 42, "the decisive arm set changed: the record's rule reads 42 decisive arms per format");
    assert!(
        fails.is_empty(),
        "{RECORD} row 6: the decided curve {n} fails the content rule on RGBA16F on {fails:?} — the late escape with AD-1's re-bless, not a keep"
    );
}

/// The classes the pre-registration covers are exactly the class table's (no row for a class the
/// device cannot classify as, none missing).
#[test]
fn the_table_covers_every_class_of_every_arm() {
    for arm in lit_format::arms() {
        for target in [Target::B10G11R11, Target::Rgba16f] {
            let classes = classes_for(target, arm.path);
            assert!(classes.contains(&PathClass::N) && classes.contains(&PathClass::Z));
        }
    }
}

/// **The generator** — rewrites the record's `px0-expected` block from this tree's model. Run it
/// only at the model commit (or for a deliberate model change before any device run).
#[test]
#[ignore = "generator: rewrites the px0-expected block of the PX0 lit-format record; the pre-registration, run only at the model commit"]
fn write_expected_block() {
    let p = repo_root().join(RECORD);
    let record = std::fs::read_to_string(&p).expect("the record exists");
    let crlf = record.contains("\r\n");
    let text = record.replace("\r\n", "\n");
    let a = text.find(BEGIN).expect("begin marker") + BEGIN.len();
    let b = text.find(END).expect("end marker");
    let mut out = String::new();
    out.push_str(&text[..a]);
    out.push('\n');
    out.push_str(expected_block().trim_matches('\n'));
    out.push('\n');
    out.push_str(&text[b..]);
    let out = if crlf { out.replace('\n', "\r\n") } else { out };
    std::fs::write(&p, out).expect("write the record");
}
