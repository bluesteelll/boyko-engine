//! VB-SV0 DP6b (`docs/VB-SV0-DP6-DESIGN.md`, P1-5): the **two-sided `dxc -P` preprocessor gate**
//! for `vb_geo.comp.hlsl`'s new `-D VB_SV0_TERM=1` variant.
//!
//! # What this proves that the byte gate does not
//!
//! `vb_raster_geo_classify_spv_sync.rs` asserts that the committed `vb_geo.comp.spv` /
//! `vb_geo_mv.comp.spv` / `vb_geo_sv0.comp.spv` are the re-DXC of the committed source. That is a
//! statement about the compiler's OUTPUT. It cannot distinguish "the flag-OFF compile sees the same
//! program text" from "the flag-OFF compile sees a different program text that happens to optimise
//! to the same module" — and the second is exactly the state a careless edit to a `-D` variant
//! produces: a statement drifts one line outside its `#ifdef`, the optimiser folds it away today,
//! and the day some unrelated rung perturbs the surrounding code the base variant silently changes
//! behaviour. DP6b's whole safety claim is **additivity**: with `VB_SV0_TERM` undefined the file
//! preprocesses to the pre-DP6b program. This gate is the direct measurement of that claim.
//!
//! Two sides, both required, because either alone is satisfiable by a defect:
//!
//! 1. **ADDITIVE** — the no-define preprocessor output is identical to the PRE-DP6b file's. A gate
//!    with only this side is passed by a variant that adds nothing at all.
//! 2. **NON-EMPTY** — the `-D VB_SV0_TERM=1` output DIFFERS from the no-define one. A gate with only
//!    this side is passed by a variant whose flag also perturbs the base path.
//!
//! # The pre-DP6b side is RECOMPUTED, never committed — and what IS committed instead
//!
//! The design (P1-5) is explicit: *"the pre-DP6b hash is RECOMPUTED via `git show <DP6b^>:…`, not
//! committed — a committed literal is a datum nobody re-derives and the first 'fix' is to re-bless
//! it"*. That is followed here: **no hash, no length, no digest of the pre-DP6b text appears in this
//! file.** The whole reference side is materialised by `git show` at test time.
//!
//! What a `git show` still needs is a REVISION, and [`PRE_DP6B_REV`] is it. A revision is not the
//! same kind of datum as a hash: it names a fixed point in history whose CONTENT `git` re-derives,
//! so it cannot go stale in the direction that matters (the reference text drifting while the pin
//! keeps saying "green"). It can only go stale in the direction that reds — someone edits
//! `vb_geo.comp.hlsl` between that revision and HEAD outside a guard — which is precisely the event
//! this gate exists to report.
//!
//! # Normalisation, MEASURED rather than assumed
//!
//! `dxc -P` interleaves `#line <n> "<path>"` bookkeeping with the program text. Neither field can
//! survive this comparison:
//!
//! * the PATH differs because the pre-DP6b side is materialised into a temp file, and
//! * the NUMBERS differ by construction — DP6b's edit is additive, so every line after the first
//!   insertion is renumbered. A gate that compared them would red on a pure comment addition.
//!
//! Blank lines are dropped for a third, measured reason: **`dxc -P` does not preserve them across an
//! elided region.** With the DP6b guard block present, the single blank line between `} pc;` and
//! `[numthreads(64, 1, 1)]` is swallowed by the `#line` jump that replaces the guarded span — so the
//! two texts differ by exactly one empty line, on a source whose every added line is inside a guard.
//! That is a property of the preprocessor's line accounting, not of the program, and no source
//! formatting removes it (the jump is emitted whenever the elided run is long). Verified by running
//! it: with `#line` stripped the two sides differ by one blank line; with blank lines stripped too
//! they are character-identical over 571 lines.
//!
//! What survives normalisation is every token DXC will actually compile, so red mutation (1) from
//! the design — *move a statement outside `#ifdef VB_SV0_TERM`* — is caught. That is demonstrated
//! here rather than asserted: [`the_preprocess_gate_reds_on_an_unguarded_edit_and_not_on_a_guarded_one`]
//! runs both mutations and pins their opposite verdicts.
//!
//! # The reference is OLD SOURCE compiled against TODAY'S headers, deliberately
//!
//! `git show` retrieves only `vb_geo.comp.hlsl`; its `#include`s resolve against the CURRENT
//! `shaders/` directory. That is the right construction and not a shortcut. The claim under test is
//! *"the flag-OFF program is unchanged BY THIS RUNG"*, and holding the headers fixed at HEAD is what
//! isolates that: a `vb_pack.hlsli` edit landing in some later commit moves BOTH sides identically
//! and cancels, so it cannot masquerade as a DP6b defect. Retrieving the whole historical include
//! set would instead measure "has anything under `shaders/` changed since `PRE_DP6B_REV`", which
//! reds on every unrelated rung and would be re-blessed into uselessness within a week.
//!
//! The cost is stated: a header edit that changes the flag-OFF program is invisible HERE. It is not
//! invisible — `vb_raster_geo_classify_spv_sync.rs` re-DXCs `vb_geo.comp.spv` and
//! `vb_geo_mv.comp.spv` from today's tree against the committed bytes, which is exactly that
//! statement.
//!
//! # SKIP vs RED — the two unavailabilities are NOT the same, and one is a hard red
//!
//! * **`dxc` absent, or no `git` / not a work tree** — HOST unavailability. Named skip, the
//!   `cluster_cull_spv_sync.rs` idiom. Nothing about the pin is in question.
//! * **[`PRE_DP6B_REV`] unreachable while `git` works in a work tree** — PIN unavailability. **Hard
//!   RED**, never a skip. See [`PreSource`] for why collapsing the two made an earlier form of this
//!   file go silently green on its own central claim after a squash-merge, and [`PRE_DP6B_REV`] for
//!   the re-anchor procedure that repair reaches for.
//!
//! ⚠️ **A skip here is close to invisible under the house invocation**: `cargo test` CAPTURES a
//! passing test's output, so the `eprintln!` only appears with `--nocapture`. The reliable tell is
//! the ABSENCE of the `vb_geo_preprocess_sync: pre-DP6b(...)` report line, which the gate prints on
//! every real run. A skip is an absence of evidence and must never be read as a green.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` live and where
/// every `#include` must resolve from. Mirrors `vb_raster_geo_classify_spv_sync.rs`.
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// The shader whose additivity this file gates.
const VB_GEO_HLSL: &str = "vb_geo.comp.hlsl";

/// The `-D` axis DP6b adds. ONE flag: the source derives `VB_SV0` from it rather than making the
/// caller spell both, so the command line and the header contract cannot drift apart.
const SV0_DEFINE: &str = "VB_SV0_TERM=1";

/// The revision whose `vb_geo.comp.hlsl` is the PRE-DP6b reference — i.e. `DP6b^`.
///
/// This is the ONLY committed literal on the reference side, and it is a revision rather than a
/// hash on purpose (see the module doc). `git show <this>:<path>` re-derives the text on every run,
/// so the reference cannot rot into agreement with a drifted source.
///
/// # Re-pointing: FORBIDDEN for drift, MANDATORY for history rewrite
///
/// These two look identical from the failure message and are opposite in what they mean, so the
/// distinction is written down rather than left to judgement:
///
/// * **DRIFT** — the gate reds with a `First difference at normalized line N` report naming two
///   texts. The reference resolved fine; the SOURCE moved outside a guard. **Do not touch this
///   const.** Move the statement back inside its `#ifdef VB_SV0_TERM`. Re-pointing here would
///   bless exactly the defect the gate exists to catch, and it is the campaign's recorded
///   first-reach-for repair.
/// * **HISTORY REWRITE** — the gate reds with `PIN UNREACHABLE`. `git` works and this IS a work
///   tree, but the revision is gone: this branch was squash-merged, rebased, or the object was
///   pruned. The reference no longer exists, so the const MUST be re-pointed — leaving it is how
///   the gate would go permanently unevaluable.
///
/// **Re-anchor procedure** (history rewrite only), and it is checkable rather than trusted:
///
/// 1. Find the post-rewrite commit whose `crates/boyko_rhi_vulkan/shaders/vb_geo.comp.hlsl` is the
///    SAME pre-DP6b content — after a squash-merge that is the merge commit's first parent, i.e.
///    the last commit on the target branch before DP6b's squash landed.
/// 2. Set this const to it.
/// 3. **Run the gate.** The verification is the gate itself: a correctly re-anchored revision
///    yields the same normalized text the old one did, so
///    [`vb_geo_sv0_variant_is_additive_under_the_flag_and_non_empty_with_it`] passes. A revision
///    picked one commit too late (one that already contains the DP6b span) fails the ADDITIVE side
///    immediately, because its `-P` carries the guard block's text. **A re-anchor that reds is a
///    wrong re-anchor, never a reason to relax the comparison.**
///
/// The invariant to hold in mind: `git show <new rev>:<shader>` must preprocess to the identical
/// normalized text `git show <old rev>:<shader>` did. Nothing else about the revision matters.
const PRE_DP6B_REV: &str = "c1caa422";

/// Locates the `dxc` executable: first the pinned Vulkan-SDK path (the repo's offline recipe), then
/// `$VULKAN_SDK/Bin`, then `PATH`. Returns `None` if none resolve — the `cluster_cull_spv_sync.rs`
/// idiom verbatim.
fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// The outcome of resolving the pre-DP6b reference — **three states, not two**, because the two
/// failure modes have opposite correct verdicts.
///
/// Collapsing them into one `Option::None` is what made an earlier form of this file go SILENTLY
/// GREEN on the very claim it exists to make: a squash-merge prunes [`PRE_DP6B_REV`], the reference
/// stops resolving, the test returns early, and `libtest` reports a PASS forever. The `eprintln!`
/// does not rescue it — `cargo test` CAPTURES output for PASSING tests, so under the house
/// invocation nobody sees the line at all. A skip that reports as a pass and prints nothing is
/// indistinguishable from evidence.
enum PreSource {
    /// The pre-DP6b shader text.
    Text(String),
    /// HOST unavailability: no `git`, or this checkout is not a work tree (a vendored source drop,
    /// a `cargo package` tarball). Nothing about the pin is in question — a named SKIP is honest.
    NoGit,
    /// PIN unavailability: `git` works and this IS a work tree, but the revision does not resolve.
    /// That is a statement about THIS repository's history, not about the host, and it means the
    /// gate can no longer be evaluated at all. **RED**, carrying git's own diagnosis.
    RevUnreachable(String),
}

/// Resolves `<rev>:crates/boyko_rhi_vulkan/shaders/<name>` into a [`PreSource`].
///
/// `-C` is threaded at the crate directory rather than at a guessed repo root: `git` walks up to the
/// enclosing work tree itself, so this keeps working from a `git worktree` checkout, where a
/// hand-computed `../../..` would name the wrong tree.
///
/// The work-tree probe runs FIRST and separately from the `show`. Deriving "not a work tree" from a
/// failed `show` is exactly the conflation this type exists to prevent: `git show` fails for both
/// reasons and its exit status does not distinguish them.
fn git_show_shader(rev: &str, name: &str) -> PreSource {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let probe = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output();
    let inside_work_tree = match probe {
        Ok(out) => out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true",
        Err(_) => false, // the binary is not on PATH
    };
    if !inside_work_tree {
        return PreSource::NoGit;
    }

    let spec = format!("{rev}:crates/boyko_rhi_vulkan/shaders/{name}");
    let out = match Command::new("git").arg("-C").arg(manifest).args(["show", &spec]).output() {
        Ok(out) => out,
        Err(e) => return PreSource::RevUnreachable(format!("`git show {spec}` failed to run: {e}")),
    };
    if !out.status.success() {
        return PreSource::RevUnreachable(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    match String::from_utf8(out.stdout) {
        Ok(text) => PreSource::Text(text),
        Err(e) => PreSource::RevUnreachable(format!("`git show {spec}` produced non-UTF-8: {e}")),
    }
}

/// Runs the frozen recipe in PREPROCESS-ONLY mode (`-P`) over `src_path`, with `shaders_dir` on the
/// include path, and returns the preprocessed text.
///
/// The flags are `vb_geo.comp.hlsl`'s own header recipe plus `-P`, so the text this returns is the
/// text the real compile consumes — not a differently-configured preprocessor run. (`-spirv` and
/// `-fspv-target-env` are carried for that reason even though preprocessing does not consume them:
/// they set predefined macros, and a gate that ran with a different macro set would be measuring a
/// program the engine never builds.)
///
/// Panics on a non-zero exit: `dxc` was located, and a source that fails to PREPROCESS is a real
/// defect, not a host difference to skip over.
fn preprocess(dxc: &Path, shaders_dir: &Path, src_path: &Path, defines: &[&str], out_tag: &str) -> String {
    let out_path = std::env::temp_dir().join(format!("{out_tag}.p.hlsl"));
    let mut cmd = Command::new(dxc);
    cmd.args(["-P", "-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", "-I"]);
    cmd.arg(shaders_dir);
    for d in defines {
        cmd.args(["-D", d]);
    }
    cmd.arg(src_path).arg("-Fi").arg(&out_path);
    let out = cmd.output().expect("invariant: dxc was located and must run");
    assert!(
        out.status.success(),
        "dxc -P failed on {} {defines:?}: {}",
        src_path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&out_path).expect("invariant: dxc -P wrote its -Fi output");
    let _ = std::fs::remove_file(&out_path); // best-effort tidy
    text
}

/// Strips `dxc -P`'s line bookkeeping and empty lines, leaving exactly the token text the compile
/// consumes. See the module doc for why each of the two is removed — both reasons are measured, not
/// stylistic, and [`the_normaliser_drops_only_line_bookkeeping_and_blank_lines`] pins the selector.
fn normalize_preprocessed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.starts_with("#line") || trimmed.trim().is_empty() {
            continue;
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

/// Materialises `contents` into the process temp directory ([`std::env::temp_dir`]) as `name`, so
/// both sides of every comparison are preprocessed the same way — from a temp path, resolving
/// `#include`s through `-I <shaders_dir>`, and never writing into the shader directory itself.
///
/// Symmetry is the point: an asymmetric setup (one side compiled in place, the other through `-I`)
/// leaves "one side used `-I`" available as an explanation for a difference, and the gate would then
/// not be measuring the program.
fn materialize(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, contents).expect("invariant: the temp dir is writable");
    path
}

/// A compact 64-bit FNV-1a fingerprint, used ONLY to make the report lines human-readable. Never a
/// gate primitive — every assertion below compares the full normalized texts.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce4_84222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The first differing line of two normalized texts, as `(index, left, right)` — the failure
/// message's whole content. A byte offset would name a position no reader can locate in a 20 KB
/// preprocessor dump; a line pair names the statement that moved.
fn first_line_difference<'a>(left: &'a str, right: &'a str) -> Option<(usize, &'a str, &'a str)> {
    let mut l = left.lines();
    let mut r = right.lines();
    let mut i = 0usize;
    loop {
        match (l.next(), r.next()) {
            (None, None) => return None,
            (a, b) if a != b => return Some((i, a.unwrap_or("<end of text>"), b.unwrap_or("<end of text>"))),
            _ => i += 1,
        }
    }
}

/// **The gate.** Both sides of DP6b's additivity claim, in one test because a report that named only
/// one of them would be read as the whole property.
#[test]
fn vb_geo_sv0_variant_is_additive_under_the_flag_and_non_empty_with_it() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_geo_preprocess_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no $VULKAN_SDK/Bin, \
             not on PATH) — SKIPPING the two-sided -P gate on this host. A skip is NOT a pass: \
             nothing was proven about VB_SV0_TERM's additivity."
        );
        return;
    };
    let pre_src = match git_show_shader(PRE_DP6B_REV, VB_GEO_HLSL) {
        PreSource::Text(text) => text,
        PreSource::NoGit => {
            eprintln!(
                "vb_geo_preprocess_sync: no `git`, or this checkout is not a work tree — SKIPPING \
                 the two-sided -P gate. A skip is NOT a pass: the pre-DP6b reference could not be \
                 materialised, so additivity was not measured on this host. (This line is CAPTURED \
                 by `cargo test` unless `--nocapture` is passed; the skip's only other trace is \
                 that no `vb_geo_preprocess_sync: pre-DP6b(...)` report line was printed.)"
            );
            return;
        }
        PreSource::RevUnreachable(why) => panic!(
            "PIN UNREACHABLE: `git` works and this IS a work tree, but `{PRE_DP6B_REV}` does not \
             resolve, so DP6b's additivity claim can no longer be evaluated at all.\n  git said: \
             {why}\n\
             This is NOT a host problem and must NOT be skipped past. The usual cause is a history \
             rewrite — this branch was squash-merged or rebased, and the abbreviated revision was \
             pruned. Follow `PRE_DP6B_REV`'s documented RE-ANCHOR PROCEDURE: re-point the const at \
             the post-rewrite commit carrying the SAME pre-DP6b `{VB_GEO_HLSL}` (after a \
             squash-merge, the merge's first parent), then re-run this test — a correct re-anchor \
             yields the identical normalized text and passes; one taken a commit too late already \
             contains the guarded span and fails the ADDITIVE side. Re-pointing is MANDATORY here \
             and FORBIDDEN when the gate reds with a `First difference` report instead."
        ),
    };

    let dir = shaders_dir();
    let cur_src = std::fs::read_to_string(dir.join(VB_GEO_HLSL))
        .expect("invariant: vb_geo.comp.hlsl is the committed shader source");

    let pre_path = materialize("vb_geo_preprocess_sync_pre.hlsl", &pre_src);
    let cur_path = materialize("vb_geo_preprocess_sync_cur.hlsl", &cur_src);

    let pre = normalize_preprocessed(&preprocess(&dxc, &dir, &pre_path, &[], "vb_geo_pp_pre"));
    let base = normalize_preprocessed(&preprocess(&dxc, &dir, &cur_path, &[], "vb_geo_pp_base"));
    let sv0 =
        normalize_preprocessed(&preprocess(&dxc, &dir, &cur_path, &[SV0_DEFINE], "vb_geo_pp_sv0"));

    let _ = std::fs::remove_file(&pre_path);
    let _ = std::fs::remove_file(&cur_path);

    eprintln!(
        "vb_geo_preprocess_sync: pre-DP6b({PRE_DP6B_REV}) {} lines fnv1a_64={:#018x} | base {} \
         lines fnv1a_64={:#018x} | -D {SV0_DEFINE} {} lines fnv1a_64={:#018x}",
        pre.lines().count(),
        fnv1a_64(pre.as_bytes()),
        base.lines().count(),
        fnv1a_64(base.as_bytes()),
        sv0.lines().count(),
        fnv1a_64(sv0.as_bytes()),
    );

    // --- Side 1: ADDITIVE ---------------------------------------------------------------------
    if let Some((i, l, r)) = first_line_difference(&pre, &base) {
        panic!(
            "RED: with `VB_SV0_TERM` UNDEFINED, `{VB_GEO_HLSL}` no longer preprocesses to its \
             pre-DP6b program. First difference at normalized line {i}:\n  \
             {PRE_DP6B_REV}: {l}\n  HEAD      : {r}\n\
             DP6b's entire safety claim is that the SV0 span is additive under the flag — that is \
             what keeps `vb_geo.comp.spv` and `vb_geo_mv.comp.spv` byte-frozen and every VB golden \
             unmoved. A statement has drifted OUTSIDE an `#ifdef VB_SV0_TERM` guard (design red \
             mutation (1)). Move it back inside; do NOT re-point `PRE_DP6B_REV`."
        );
    }

    // --- Side 2: NON-EMPTY --------------------------------------------------------------------
    assert!(
        base != sv0,
        "RED: `-D {SV0_DEFINE}` preprocesses `{VB_GEO_HLSL}` to a text IDENTICAL to the no-define \
         compile. The variant is then empty, and side 1 above is vacuously green — it would report \
         a green for a flag that guards nothing. Either the guard spells a macro name the command \
         line does not set, or the span was removed."
    );
}

/// **The sensitivity control, run as two OPPOSITE mutations** — which is what makes the gate's green
/// mean something, and it is the shape the design's red-mutation list asks for.
///
/// A one-sided control ("some edit reds") would not distinguish this gate from one that reds on
/// EVERY edit, and a gate that reds on every edit is useless for a rung whose whole content is an
/// edit. The property under test is directional:
///
/// * an edit OUTSIDE every guard MUST move the base text (that is red mutation (1));
/// * an edit INSIDE the guard MUST NOT move the base text, and MUST move the `-D` text.
///
/// Both mutations are applied to scratch copies; the committed source is never touched.
///
/// RED here is a finding about the INSTRUMENT, not about the shader — do not retune a mutation to
/// force a green.
#[test]
fn the_preprocess_gate_reds_on_an_unguarded_edit_and_not_on_a_guarded_one() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_geo_preprocess_sync: dxc not found — SKIPPING the sensitivity control. A skip is \
             NOT a pass: the two-sided gate's teeth were not demonstrated on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let src = std::fs::read_to_string(dir.join(VB_GEO_HLSL))
        .expect("invariant: vb_geo.comp.hlsl is the committed shader source");

    // An UNGUARDED statement: the roughness floor, in `main()` above every `#ifdef`.
    const UNGUARDED: &str = "clamp(m.mrr.y, 0.045, 1.0)";
    // A GUARDED statement: the march-origin lift, inside `#ifdef VB_SV0_TERM`.
    const GUARDED: &str = "static const float SHADOW_NORMAL_BIAS = 0.02;";
    assert!(
        src.contains(UNGUARDED),
        "invariant: {UNGUARDED:?} must appear verbatim in {VB_GEO_HLSL} for the unguarded mutation \
         to be meaningful — if the expression changed, update this control"
    );
    assert!(
        src.contains(GUARDED),
        "invariant: {GUARDED:?} must appear verbatim in {VB_GEO_HLSL} (inside the \
         `#ifdef VB_SV0_TERM` tuning block) for the guarded mutation to be meaningful"
    );

    let cur_path = materialize("vb_geo_pp_ctl_cur.hlsl", &src);
    let unguarded_path = materialize(
        "vb_geo_pp_ctl_unguarded.hlsl",
        &src.replacen(UNGUARDED, "clamp(m.mrr.y, 0.046, 1.0)", 1),
    );
    let guarded_path = materialize(
        "vb_geo_pp_ctl_guarded.hlsl",
        &src.replacen(GUARDED, "static const float SHADOW_NORMAL_BIAS = 0.03;", 1),
    );

    let base = normalize_preprocessed(&preprocess(&dxc, &dir, &cur_path, &[], "vb_geo_pp_ctl_b"));
    let base_unguarded =
        normalize_preprocessed(&preprocess(&dxc, &dir, &unguarded_path, &[], "vb_geo_pp_ctl_ub"));
    let base_guarded =
        normalize_preprocessed(&preprocess(&dxc, &dir, &guarded_path, &[], "vb_geo_pp_ctl_gb"));
    let sv0 = normalize_preprocessed(&preprocess(&dxc, &dir, &cur_path, &[SV0_DEFINE], "vb_geo_pp_ctl_s"));
    let sv0_guarded = normalize_preprocessed(&preprocess(
        &dxc,
        &dir,
        &guarded_path,
        &[SV0_DEFINE],
        "vb_geo_pp_ctl_gs",
    ));

    for p in [&cur_path, &unguarded_path, &guarded_path] {
        let _ = std::fs::remove_file(p);
    }

    eprintln!(
        "vb_geo_preprocess_sync sensitivity control: base fnv1a_64={:#018x}; unguarded-mutant base \
         fnv1a_64={:#018x}; guarded-mutant base fnv1a_64={:#018x}; -D {SV0_DEFINE} \
         fnv1a_64={:#018x}; guarded-mutant -D fnv1a_64={:#018x}",
        fnv1a_64(base.as_bytes()),
        fnv1a_64(base_unguarded.as_bytes()),
        fnv1a_64(base_guarded.as_bytes()),
        fnv1a_64(sv0.as_bytes()),
        fnv1a_64(sv0_guarded.as_bytes()),
    );

    assert!(
        base != base_unguarded,
        "RED: editing an UNGUARDED statement ({UNGUARDED}) left the no-define preprocessor output \
         unchanged. The gate above is then BLIND to design red mutation (1) — a statement escaping \
         its `#ifdef VB_SV0_TERM` would pass. This is a finding about the instrument."
    );
    assert_eq!(
        base, base_guarded,
        "RED: editing a statement INSIDE `#ifdef VB_SV0_TERM` ({GUARDED}) moved the NO-DEFINE \
         preprocessor output. The guard is then not a guard, and the gate would red on every \
         legitimate SV0-side edit — the opposite failure, and just as disqualifying."
    );
    assert!(
        sv0 != sv0_guarded,
        "RED: editing {GUARDED} did not move the `-D {SV0_DEFINE}` output either, so the mutation \
         reached NEITHER text and proves nothing about the guarded direction. The token is \
         probably no longer inside the guarded span."
    );
}

/// FIXTURE CONTROL for [`normalize_preprocessed`], run unconditionally — pure string handling, no
/// toolchain, so it cannot SKIP.
///
/// It is not decorative. A normaliser that returned the EMPTY string would make BOTH sides of the
/// gate above compare equal, so side 1 would pass vacuously and side 2's inequality would fail
/// LOUDLY today — but a future edit that only kept side 1 would then be permanently green. These
/// fixtures pin exactly which two line classes are dropped and that nothing else is.
#[test]
fn the_normaliser_drops_only_line_bookkeeping_and_blank_lines() {
    assert_eq!(
        normalize_preprocessed("#line 1 \"a.hlsl\"\nfloat x = 1.0;\n"),
        "float x = 1.0;\n",
        "a `#line` directive is preprocessor bookkeeping, not program text"
    );
    assert_eq!(
        normalize_preprocessed("float x = 1.0;\n\n   \t \nfloat y = 2.0;\n"),
        "float x = 1.0;\nfloat y = 2.0;\n",
        "empty and whitespace-only lines are dropped — `dxc -P` does not preserve them across an \
         elided region, which is measured, not assumed (see the module doc)"
    );
    assert_eq!(
        normalize_preprocessed("float line_count = 1.0;\n  // #line inside a comment\n"),
        "float line_count = 1.0;\n  // #line inside a comment\n",
        "only a line STARTING with `#line` is bookkeeping — an identifier containing `line`, or a \
         comment quoting the directive, is program text and must survive"
    );
    assert_eq!(
        normalize_preprocessed("float a = 1.0;\r\nfloat b = 2.0;\r\n"),
        "float a = 1.0;\nfloat b = 2.0;\n",
        "CRLF is normalized, so a checkout's line-ending convention cannot decide the verdict"
    );
    assert!(
        !normalize_preprocessed("#line 7 \"x\"\nfloat z = 3.0;\n").is_empty(),
        "the normaliser must not eat program text — an empty result would make the gate above \
         vacuously green on its ADDITIVE side"
    );
}
