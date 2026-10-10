//! Dynamic parity scenes (lane DYN-SCENES, triage r1 F1): the event hook runs OUTSIDE the timed
//! pair, in our runner and in the Rapier dyn harness.
//!
//! ## Why a source census
//!
//! Every harness applies a step's events (kicks, launches) between the previous step's readout and
//! the next step's `Instant::now()`, so the timed pair holds the engine's step and nothing else
//! (`benches/jolt_parity_pyramid/dyn_scenes.rs`'s module doc; the record,
//! `docs/physics/perf-campaign/levers/scenes/01-DESIGN.md`). Where the hook sits changes no bit of
//! the simulation. Tester r1's mutant O3 moved `before_step` after `let t0`; it passed all 12
//! structural checks of `gate/ours_gates.sh` and kept the pose pin. The move changes only the
//! `wall_ns` column, which window 9c times and no structural gate reads. S-SHOOT's 60 spawns, with
//! their 1,300-body clearance scans, and S-KICK's energy evaluations would then enter the timed
//! mean without a trace. Jolt's patched source is gated by `gate/jolt_gates.sh` G6; this file
//! gates the other two harnesses. The property belongs to the source, so the gate reads the source,
//! as `tests/ke16_app1_lane_count_census.rs` does for its own class.
//!
//! ## What is checked
//!
//! The scan blanks comments and the contents of string and char literals, then takes the body of
//! the step loop (from the header's `{` to its matching `}`) and removes whitespace. In that body:
//!
//! 1. each of the five anchors occurs exactly once: the pre-step hook, `let t0 = Instant::now();`,
//!    the step call, `let wall = t0.elapsed();` and the post-step hook;
//! 2. the timed pair is exactly three consecutive code lines: `let t0`, the step call and
//!    `let wall`, each alone on its line. Nothing else can enter the pair, whether it is a hook or
//!    not;
//! 3. the pre-step hook comes before `let t0`, and the post-step hook comes after `let wall`.
//!
//! The Rapier source is read from the repository's copy, `rapier/main.rs.txt`.
//! `gate/rapier_gates.py` G1 ties that copy byte for byte to the source the dyn exes were built from
//! (`dyn/bin/SOURCES.sha256`). If the runner moves (lane IB, record PC-DYN-1), the paths below move
//! with it. A path that does not resolve fails here; it is never skipped.
//!
//! [`the_predicate_rejects_each_reordering_and_ignores_prose`] runs the predicate over mutants that
//! it derives from the real sources. A scan that stopped matching anything therefore cannot report
//! a clean tree.

use std::fs;
use std::path::PathBuf;

/// One harness's step loop and the five statements the gate orders in it, as written (whitespace
/// is removed before matching).
struct Harness {
    name: &'static str,
    /// The source, relative to this crate's manifest directory.
    path: &'static str,
    /// The step loop's header line, ending in its `{`.
    header: &'static str,
    /// The pre-step event hook (applies the step's events).
    pre: &'static str,
    /// The timed pair's opening statement.
    start: &'static str,
    /// The engine step, the only statement the pair may hold.
    call: &'static str,
    /// The timed pair's closing statement.
    stop: &'static str,
    /// The post-step hook (reads the state after the step).
    post: &'static str,
}

const OURS: Harness = Harness {
    name: "ours (benches/jolt_parity_pyramid.rs)",
    path: "benches/jolt_parity_pyramid.rs",
    header: "for step in 0..args.steps {",
    pre: "d.before_step(step, &mut rig);",
    start: "let t0 = Instant::now();",
    call: "rig.physics.run(&mut rig.world);",
    stop: "let wall = t0.elapsed();",
    post: "d.after_step(step, &rig);",
};

const RAPIER: Harness = Harness {
    name: "Rapier dyn (scenes/rapier/main.rs.txt)",
    path: "../../docs/physics/perf-campaign/levers/scenes/rapier/main.rs.txt",
    header: "for i in 0..args.steps {",
    pre: "d.before_step(i, w);",
    start: "let t0 = Instant::now();",
    call: "step(w);",
    stop: "let wall = t0.elapsed();",
    post: "d.after_step(i, w);",
};

const HARNESSES: [&Harness; 2] = [&OURS, &RAPIER];

fn read(h: &Harness) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(h.path);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: {} must be readable at {} ({e})", h.name, path.display()))
}

fn squeeze(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `text` with every comment, and the contents of every string and char literal, replaced by
/// spaces. Newlines are kept, so every byte keeps its line. Raw strings (`r"…"`, `r#"…"#`) count as
/// literals too. A `'` that does not open a char literal is a lifetime and stays as it is.
fn code_only(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to.min(b.len())] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        let next = b.get(i + 1).copied();
        match b[i] {
            b'/' if next == Some(b'/') => {
                let end = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |n| i + n);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if next == Some(b'*') => {
                let (mut depth, mut j) = (0usize, i);
                while j < b.len() {
                    if b[j..].starts_with(b"/*") {
                        depth += 1;
                        j += 2;
                    } else if b[j..].starts_with(b"*/") {
                        depth -= 1;
                        j += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
                blank(&mut out, i, j);
                i = j;
            }
            b'r' if (i == 0 || !is_ident(b[i - 1]) || (b[i - 1] == b'b' && (i < 2 || !is_ident(b[i - 2]))))
                && matches!(next, Some(b'"' | b'#')) =>
            {
                let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
                let open = i + 1 + hashes;
                if b.get(open) != Some(&b'"') {
                    // `r#ident`, a raw identifier.
                    i += 1;
                    continue;
                }
                let mut close = vec![b'"'];
                close.extend(std::iter::repeat_n(b'#', hashes));
                let end = b[open + 1..]
                    .windows(close.len())
                    .position(|w| w == close.as_slice())
                    .map_or(b.len(), |n| open + 1 + n);
                blank(&mut out, open + 1, end);
                i = end + close.len();
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                blank(&mut out, i + 1, j);
                i = j + 1;
            }
            b'\'' => {
                if next == Some(b'\\') {
                    // The escaped character itself may be a `'` (`'\''`): search after it.
                    let from = (i + 3).min(b.len());
                    let end = b[from..].iter().position(|&c| c == b'\'').map_or(b.len(), |n| from + n);
                    blank(&mut out, i + 1, end);
                    i = end + 1;
                } else if let Some(c) = text[i + 1..].chars().next()
                    && b.get(i + 1 + c.len_utf8()) == Some(&b'\'')
                {
                    blank(&mut out, i + 1, i + 1 + c.len_utf8());
                    i += 2 + c.len_utf8();
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("invariant: only whole UTF-8 sequences between ASCII delimiters are blanked")
}

/// The code lines of the step loop's body, as (1-based line number, whitespace-free text), blank
/// lines dropped. The header must occur exactly once.
fn loop_lines(code: &str, header: &str) -> Result<Vec<(usize, String)>, String> {
    let want = squeeze(header);
    let mut starts = Vec::new();
    let mut offset = 0;
    for (n, line) in code.split('\n').enumerate() {
        if squeeze(line) == want {
            starts.push((n + 1, offset + line.rfind('{').expect("invariant: the header ends in `{`")));
        }
        offset += line.len() + 1;
    }
    let &[(header_line, open)] = starts.as_slice() else {
        return Err(format!("the loop header `{header}` occurs {} times (want exactly 1)", starts.len()));
    };
    let mut depth = 0usize;
    let mut close = None;
    for (k, c) in code.bytes().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(k);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close.ok_or_else(|| format!("the loop at line {header_line} has no closing brace"))?;
    Ok(code[open + 1..close]
        .split('\n')
        .enumerate()
        .map(|(k, l)| (header_line + k, squeeze(l)))
        .filter(|(_, l)| !l.is_empty())
        .collect())
}

/// Checks `h`'s three properties on `text`; the error names the first one that fails.
fn check(h: &Harness, text: &str) -> Result<(), String> {
    let lines = loop_lines(&code_only(text), h.header)?;
    let find = |anchor: &str| -> Result<usize, String> {
        let a = squeeze(anchor);
        let n: usize = lines.iter().map(|(_, l)| l.matches(a.as_str()).count()).sum();
        match lines.iter().position(|(_, l)| l.contains(a.as_str())) {
            Some(k) if n == 1 => Ok(k),
            _ => Err(format!("`{anchor}` occurs {n} times in the step loop (want exactly 1)")),
        }
    };
    let (pre, start, call, stop, post) = (find(h.pre)?, find(h.start)?, find(h.call)?, find(h.stop)?, find(h.post)?);
    let alone = |k: usize, anchor: &str| lines[k].1 == squeeze(anchor);
    if !(call == start + 1 && stop == call + 1 && alone(start, h.start) && alone(call, h.call) && alone(stop, h.stop)) {
        return Err(format!(
            "the timed pair is not exactly `{}` / `{}` / `{}` on three consecutive code lines (lines {}, {}, {})",
            h.start, h.call, h.stop, lines[start].0, lines[call].0, lines[stop].0
        ));
    }
    if pre >= start {
        return Err(format!(
            "the pre-step hook (line {}) is not before the timed pair (line {})",
            lines[pre].0, lines[start].0
        ));
    }
    if post <= stop {
        return Err(format!(
            "the post-step hook (line {}) is not after the timed pair (line {})",
            lines[post].0, lines[stop].0
        ));
    }
    Ok(())
}

#[test]
fn the_event_hooks_run_outside_the_timed_pair() {
    for h in HARNESSES {
        if let Err(why) = check(h, &read(h)) {
            panic!("{}: {why}", h.name);
        }
    }
}

/// The index of the line of `text` that is `anchor` once trimmed (the first after `after`).
fn line_of(text: &[&str], anchor: &str, after: usize) -> usize {
    (after..text.len())
        .find(|&k| text[k].trim() == anchor)
        .unwrap_or_else(|| panic!("invariant: the real source has the line `{anchor}`"))
}

/// Mutants derived from the real source: (what, text, whether the gate must pass it).
fn mutants(h: &Harness, text: &str) -> Vec<(&'static str, String, bool)> {
    let real: Vec<&str> = text.split('\n').collect();
    let header = (0..real.len())
        .find(|&k| squeeze(real[k]) == squeeze(h.header))
        .expect("invariant: the real source has the loop header");
    let (pre, start, stop, post) = (
        line_of(&real, h.pre, header),
        line_of(&real, h.start, header),
        line_of(&real, h.stop, header),
        line_of(&real, h.post, header),
    );
    let edit = |f: &dyn Fn(&mut Vec<String>)| {
        let mut v: Vec<String> = real.iter().map(|s| (*s).to_owned()).collect();
        f(&mut v);
        v.join("\n")
    };
    vec![
        ("the real source", text.to_owned(), true),
        (
            "`let t0` hoisted above the pre-step hook (tester r1's O3)",
            edit(&|v| {
                let l = v.remove(start);
                v.insert(header + 1, l);
            }),
            false,
        ),
        (
            "the pre-step hook moved into the timed pair",
            edit(&|v| {
                let l = v.remove(pre);
                v.insert(start, l);
            }),
            false,
        ),
        (
            "the post-step hook moved into the timed pair",
            edit(&|v| {
                let l = v.remove(post);
                v.insert(stop, l);
            }),
            false,
        ),
        (
            "another statement inside the timed pair",
            edit(&|v| v.insert(start + 1, "        std::hint::black_box(0);".to_owned())),
            false,
        ),
        (
            "a statement on the step call's own line",
            edit(&|v| v[start + 1].push_str(" std::hint::black_box(0);")),
            false,
        ),
        (
            "a second timer start after the post-step hook",
            edit(&|v| v.insert(post + 1, format!("        {}", h.start))),
            false,
        ),
        (
            "the hooks named in comments inside the timed pair (prose, not code)",
            edit(&|v| {
                v.insert(stop, format!("        // {} /* {} */", h.pre, h.post));
                v.insert(start + 1, format!("        /* {} */", h.pre));
            }),
            true,
        ),
        (
            "braces and the anchors inside string, raw-string and char literals",
            edit(&|v| {
                v.insert(header + 1, format!("        let _s = (\"}}{{ {} \", r#\"}} \"{}\" \"#, '}}', '{{');", h.start, h.pre));
            }),
            true,
        ),
    ]
}

#[test]
fn the_predicate_rejects_each_reordering_and_ignores_prose() {
    for h in HARNESSES {
        let text = read(h);
        for (what, mutant, must_pass) in mutants(h, &text) {
            let got = check(h, &mutant);
            assert_eq!(got.is_ok(), must_pass, "{}: {what}: the predicate returned {got:?}", h.name);
        }
    }
}
