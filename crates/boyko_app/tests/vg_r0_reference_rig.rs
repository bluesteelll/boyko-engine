//! **VG-R0 rung R0a — the reference-rig gate.** Reads `docs/VG-R0-REFERENCE-RIG.toml` and asserts
//! the branch-appropriate parts of its one gate (the plan's §8 R0a).
//!
//! # Why this rung exists, and why the gate is mechanical rather than a paragraph
//!
//! R0a kills **K2** — "no Nanite reference is producible here" — cheapest, and the campaign's whole
//! falsifiability argument rests on it. The rung's purpose is that a NEGATIVE be **the machine's,
//! honestly bounded, rather than the author's**: Rev 1 let the record ship `achievable = false`
//! with the test asserting "that shape", which any author satisfies by typing `false`; Rev 2
//! re-walked an author-supplied `probed_paths` list, which is the same defect one level in (record
//! only an empty directory and the assertion is permanently true, while a real install elsewhere
//! fires nothing).
//!
//! So the search space is **DESCRIBED by the record and DEFINED here**, over a bounded, enumerable
//! set of documented authorities. Rev 4's "enumerate fixed volumes" was retracted in turn: a
//! recursive walk of two ~240 GB volumes inside a `cargo test` is unbounded runtime, permission
//! denials, reparse-point cycles, and false positives from any stray editor binary in an extracted
//! archive.
//!
//! # The two shapes
//!
//! `achievable = true` → (a) every POSITIVE field present and not `PENDING`; (b) the recorded GPU
//! name matches the one this engine reports at boot; (c) the recorded resolution equals
//! `[census].decision_resolution` read from the **thresholds** file rather than a constant this
//! rung authors.
//!
//! `achievable = false` → (a′) the NEGATIVE field set present and not `PENDING`, with `reason` a
//! member of `[k2_probe].reason_values`; (b′) the re-derivation passes **for whichever value was
//! recorded** — for `[k2_probe].machine_rederived_reason` the authorities must report NO engine,
//! and for every other value they must report that an engine IS present
//! (`[k2_probe].non_rederived_reasons_require_engine_present`), because those three causes
//! presuppose an install and a record claiming one while no engine is registered is
//! self-contradictory.
//!
//! (d) is asserted on **both** branches: the recorded thresholds digest equals the literal
//! `THRESHOLDS_SHA256` pinned in `crates/boyko_render/tests/vg_thresholds_freeze.rs` **and** the
//! file re-hashed here. Recording that literal rather than minting a second number is the point;
//! re-hashing is what makes "edit any threshold → (d) reds" fire.
//!
//! # ⚠️ The binary-presence clause, which is this rung's first EXECUTED finding
//!
//! An authority entry counts as a registered engine **only if the directory it names carries
//! `Engine/Binaries/Win64/<editor_binary_name>`**. Running the probe on this box is what forced it:
//! the HKCU builds key names `D:/Epic Games/UE_5.7`, that path does not exist, and under a naive
//! read the authorities would "report an engine" — reding the *true* record
//! `no_engine_registered` while the *false* `insufficient_disk` would have greened it. A gate whose
//! only green value is a false one is the defect family this campaign hunts.
//!
//! The clause narrows what *registered* means and does not widen what counts as absent: a real
//! registered install still carries its binary, so a fabricated `no_engine_registered` still reds —
//! [`a_registered_engine_is_still_detected`] demonstrates exactly that. It reclassifies only
//! entries pointing at nothing, which cannot produce a capture and are therefore not a rig.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The `PENDING` sentinel — the discipline `goldens/PINS.toml:15` defines: an unfilled field makes
/// the checker fail rather than pass.
const PENDING: &str = "PENDING";

const RIG_REL: &str = "../../docs/VG-R0-REFERENCE-RIG.toml";
const THRESHOLDS_REL: &str = "../../docs/VG-CAMPAIGN-THRESHOLDS.toml";
/// The freeze tripwire, read as TEXT so (d) can assert the record carries *its* literal.
const FREEZE_TEST_REL: &str = "../boyko_render/tests/vg_thresholds_freeze.rs";

/// The NEGATIVE field set — the plan's §8 R0a names exactly these four.
const NEGATIVE_FIELDS: [&str; 4] = ["reason", "search_method", "editor_binary_name", "probed_at"];

/// The POSITIVE field set. §8 R0a fixes its domain as "the record fields the Lands list
/// enumerates, all of them; that list is the set's single home" — this array is that list, and it
/// is the only enumeration of it in code.
const POSITIVE_FIELDS: [&str; 12] = [
    "ue_version",
    "install_path",
    "gpu_name",
    "driver_version",
    "capture_tool",
    "capture_tool_version",
    "render_resolution",
    "max_pixels_per_edge",
    "free_disk_gb_install_volume",
    "thresholds_sha256",
    "pass_correspondence_map",
    "per_pass_table",
];

/// Where the editor binary lives inside an engine install, relative to the directory an authority
/// names.
const EDITOR_BINARY_SUBPATH: [&str; 3] = ["Engine", "Binaries", "Win64"];

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let path = repo_path(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// TOML scanning. Deliberately a scanner and not a parser: these files are hand-authored campaign
// records with flat scalar keys, and the workspace carries no third-party dependencies.
// ---------------------------------------------------------------------------------------------

/// Strips a trailing `#` comment, respecting double-quoted strings so a `#` inside a value is not
/// mistaken for one. The sibling sweep learned this the hard way — a row read as a payload because
/// its comment was not stripped.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\\' if in_quotes => escaped = !escaped,
            b'"' if !escaped => in_quotes = !in_quotes,
            b'#' if !in_quotes => return &line[..i],
            _ => escaped = false,
        }
    }
    line
}

/// Unquotes a TOML scalar and undoes the two escapes these records use.
fn unquote(raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].replace("\\\\", "\\").replace("\\\"", "\"")
    } else {
        t.to_string()
    }
}

/// A top-level `key = value` from a flat record. Stops at the first table header, so a key inside a
/// table cannot be mistaken for a top-level one.
fn flat_scalar(toml: &str, key: &str) -> Option<String> {
    for line in toml.lines() {
        let line = strip_comment(line).trim();
        if line.starts_with('[') && !line.starts_with("[[") {
            break;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(unquote(v));
        }
    }
    None
}

/// A `key = value` inside `[table]`.
fn table_scalar(toml: &str, table: &str, key: &str) -> Option<String> {
    let header = format!("[{table}]");
    let mut inside = false;
    for line in toml.lines() {
        let line = strip_comment(line).trim();
        if line.starts_with('[') {
            inside = line == header;
            continue;
        }
        if inside
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(unquote(v));
        }
    }
    None
}

/// The members of a `key = ["a", "b"]` array inside `[table]`.
fn table_list(toml: &str, table: &str, key: &str) -> Vec<String> {
    let Some(raw) = table_scalar(toml, table, key) else {
        return Vec::new();
    };
    raw.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| unquote(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Whitespace-insensitive comparand for the resolution arrays gate (c) compares.
fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

// ---------------------------------------------------------------------------------------------
// The documented authorities.
// ---------------------------------------------------------------------------------------------

const LAUNCHER_MANIFEST: &str = r"C:\ProgramData\Epic\UnrealEngineLauncher\LauncherInstalled.dat";
const HKCU_BUILDS: &str = r"HKCU\Software\Epic Games\Unreal Engine\Builds";
const HKLM_INSTALLS: &str = r"HKLM\SOFTWARE\EpicGames\Unreal Engine";

/// What the bounded search saw. Every field is recorded into the rig file, so the record cannot
/// drift away from the machine that produced it ([`the_record_matches_a_live_probe`]).
struct Authorities {
    manifest_present: bool,
    hkcu_present: bool,
    hklm_present: bool,
    /// Install directories named by any authority.
    entries: Vec<String>,
    /// Those of `entries` that actually carry the editor binary.
    with_binary: Vec<String>,
}

impl Authorities {
    /// The predicate (b′) asserts against. See the module doc's binary-presence clause.
    fn reports_engine(&self) -> bool {
        !self.with_binary.is_empty()
    }
}

/// Does the directory an authority named carry a usable editor?
fn carries_editor_binary(dir: &str, editor_binary_name: &str) -> bool {
    let mut p = PathBuf::from(dir);
    for seg in EDITOR_BINARY_SUBPATH {
        p.push(seg);
    }
    p.push(editor_binary_name);
    p.is_file()
}

/// Runs `reg.exe query`. Returns `(key_present, value_data)`; a missing key exits non-zero, which
/// is how absence is distinguished from an empty key.
fn reg_query(key: &str, recursive: bool, only_value_named: Option<&str>) -> (bool, Vec<String>) {
    let mut cmd = Command::new("reg.exe");
    cmd.arg("query").arg(key);
    if recursive {
        cmd.arg("/s");
    }
    let Ok(out) = cmd.output() else {
        return (false, Vec::new());
    };
    if !out.status.success() {
        return (false, Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut values = Vec::new();
    for line in text.lines() {
        // `    <name>    REG_SZ    <data>` — split on the type marker rather than on whitespace,
        // because both the name and the data may contain spaces.
        for marker in ["REG_EXPAND_SZ", "REG_SZ"] {
            if let Some((name, data)) = line.split_once(marker) {
                let name = name.trim();
                let data = data.trim();
                if data.is_empty() {
                    continue;
                }
                if only_value_named.is_none_or(|want| name.eq_ignore_ascii_case(want)) {
                    values.push(data.to_string());
                }
                break;
            }
        }
    }
    (true, values)
}

/// Extracts every `"InstallLocation": "…"` from the launcher manifest.
fn manifest_install_locations(json: &str) -> Vec<String> {
    const KEY: &str = "\"InstallLocation\"";
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(KEY) {
        rest = &rest[i + KEY.len()..];
        let Some(colon) = rest.find(':') else { break };
        let after = &rest[colon + 1..];
        let Some(open) = after.find('"') else { break };
        let tail = &after[open + 1..];
        let Some(close) = tail.find('"') else { break };
        let value = tail[..close].replace("\\\\", "\\");
        if !value.is_empty() {
            out.push(value);
        }
        rest = &tail[close..];
    }
    out
}

fn probe_authorities(editor_binary_name: &str) -> Authorities {
    let manifest_path = Path::new(LAUNCHER_MANIFEST);
    let manifest_present = manifest_path.is_file();
    let mut entries = Vec::new();
    if manifest_present && let Ok(json) = std::fs::read_to_string(manifest_path) {
        entries.extend(manifest_install_locations(&json));
    }

    let (hkcu_present, hkcu_values) = reg_query(HKCU_BUILDS, false, None);
    entries.extend(hkcu_values);

    let (hklm_present, hklm_values) = reg_query(HKLM_INSTALLS, true, Some("InstalledDirectory"));
    entries.extend(hklm_values);

    let with_binary = entries
        .iter()
        .filter(|d| carries_editor_binary(d, editor_binary_name))
        .cloned()
        .collect();

    Authorities { manifest_present, hkcu_present, hklm_present, entries, with_binary }
}

// ---------------------------------------------------------------------------------------------
// The gate itself, as a pure function of its inputs — which is what lets every control below
// substitute ONE input and demonstrate the corresponding red.
// ---------------------------------------------------------------------------------------------

struct GateInputs<'a> {
    record: &'a str,
    /// What the authorities reported. Substituted by the (b′) controls.
    engine_present: bool,
    /// The thresholds file re-hashed at test time.
    digest_recomputed: &'a str,
    /// `THRESHOLDS_SHA256` as pinned in the freeze tripwire's source.
    digest_pinned: &'a str,
    reason_values: &'a [String],
    machine_rederived_reason: &'a str,
    non_rederived_require_engine: bool,
    decision_resolution: &'a str,
    /// `None` when no device was booted — only the positive branch needs it.
    gpu_name_at_boot: Option<&'a str>,
}

fn filled(record: &str, key: &str) -> Option<String> {
    match flat_scalar(record, key) {
        Some(v) if v != PENDING && !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// Every violation of R0a's one gate, most structural first. Empty means the rung is green.
fn violations(inp: &GateInputs) -> Vec<String> {
    let mut v = Vec::new();

    // ---- (d), asserted on BOTH branches ----
    match filled(inp.record, "thresholds_sha256") {
        None => v.push("(d): `thresholds_sha256` is absent or PENDING".to_string()),
        Some(recorded) => {
            if recorded != inp.digest_pinned {
                v.push(format!(
                    "(d): recorded thresholds_sha256 {recorded} is not the literal the freeze \
                     tripwire pins ({}) — R0a records THAT literal rather than minting a second",
                    inp.digest_pinned
                ));
            }
            if recorded != inp.digest_recomputed {
                v.push(format!(
                    "(d): recorded thresholds_sha256 {recorded} does not match the file re-hashed \
                     at test time ({})",
                    inp.digest_recomputed
                ));
            }
        }
    }

    let Some(achievable) = flat_scalar(inp.record, "achievable") else {
        v.push("`achievable` is absent — the record names neither branch".to_string());
        return v;
    };

    match achievable.as_str() {
        "true" => {
            // ---- (a) ----
            for f in POSITIVE_FIELDS {
                if filled(inp.record, f).is_none() {
                    v.push(format!("(a): positive field `{f}` is absent or PENDING"));
                }
            }
            // ---- (b) ----
            match (filled(inp.record, "gpu_name"), inp.gpu_name_at_boot) {
                (Some(recorded), Some(booted)) if recorded != booted => v.push(format!(
                    "(b): recorded gpu_name {recorded:?} != the name this engine reports at boot \
                     {booted:?}"
                )),
                (Some(_), None) => v.push(
                    "(b): no device was booted, so the recorded GPU name was never cross-checked"
                        .to_string(),
                ),
                _ => {}
            }
            // ---- (c) ----
            if let Some(recorded) = filled(inp.record, "render_resolution")
                && squash(&recorded) != squash(inp.decision_resolution)
            {
                v.push(format!(
                    "(c): recorded render_resolution {recorded} != [census].decision_resolution {}",
                    inp.decision_resolution
                ));
            }
        }
        "false" => {
            // ---- (a′) ----
            for f in NEGATIVE_FIELDS {
                if filled(inp.record, f).is_none() {
                    v.push(format!("(a'): negative field `{f}` is absent or PENDING"));
                }
            }
            let reason = flat_scalar(inp.record, "reason").unwrap_or_default();
            if !inp.reason_values.contains(&reason) {
                v.push(format!(
                    "(a'): reason {reason:?} is outside the frozen [k2_probe].reason_values \
                     {:?} — an unrecognised string is a typo or a cause nobody has thought \
                     through, and both must stop the rung rather than disarm it",
                    inp.reason_values
                ));
            } else if reason == inp.machine_rederived_reason {
                // ---- (b′), re-derivable arm ----
                if inp.engine_present {
                    v.push(format!(
                        "(b'): the record claims {reason:?} while the documented authorities \
                         report an engine IS registered"
                    ));
                }
            } else if inp.non_rederived_require_engine && !inp.engine_present {
                // ---- (b′), presupposition arm ----
                v.push(format!(
                    "(b'): {reason:?} presupposes an install, but the documented authorities \
                     report NO engine — the record is self-contradictory"
                ));
            }
        }
        other => v.push(format!("`achievable` is {other:?}, which is neither branch")),
    }

    v
}

// ---------------------------------------------------------------------------------------------
// Inputs assembled from the live tree.
// ---------------------------------------------------------------------------------------------

/// The `THRESHOLDS_SHA256` literal, read out of the freeze tripwire's SOURCE. R0a asserts the
/// record carries *that* number; extracting it textually is what makes "rather than minting a
/// second one" a check instead of a hope.
fn pinned_digest_literal() -> String {
    let src = read(FREEZE_TEST_REL);
    for line in src.lines() {
        let line = line.trim();
        if line.starts_with("const THRESHOLDS_SHA256")
            && let Some(open) = line.find('"')
            && let Some(close) = line[open + 1..].find('"')
        {
            return line[open + 1..open + 1 + close].to_string();
        }
    }
    panic!("invariant: the freeze tripwire must declare `const THRESHOLDS_SHA256 = \"…\"`");
}

/// `\r\n` → `\n`, because this repository has `core.autocrlf` behaviour active and a hash over raw
/// bytes would be a hash of the checkout configuration.
fn normalised_thresholds() -> Vec<u8> {
    let raw = std::fs::read(repo_path(THRESHOLDS_REL)).expect("invariant: thresholds file readable");
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' && i + 1 < raw.len() && raw[i + 1] == b'\n' {
            i += 1;
            continue;
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4), in-house because this workspace carries no third-party dependencies.
//
// ⚠️ This is deliberately a SECOND, INDEPENDENT implementation from the one in
// `crates/boyko_render/tests/vg_thresholds_freeze.rs`, and the independence is the point rather
// than an oversight: (d) and the freeze tripwire assert the same fact, so a single shared
// implementation would make one bug green BOTH gates — a common-mode failure in a tripwire. Each
// carries its own known-answer test, because a wrong hash is perfectly *stable* and would pass
// forever while hashing something reproducible by nobody.
// ---------------------------------------------------------------------------------------------

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

fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
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
        for (slot, val) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(val);
        }
    }

    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// Everything the live gate and every control needs, read once.
struct Fixture {
    record: String,
    reason_values: Vec<String>,
    machine_rederived_reason: String,
    non_rederived_require_engine: bool,
    decision_resolution: String,
    digest_recomputed: String,
    digest_pinned: String,
    authorities: Authorities,
}

fn fixture() -> Fixture {
    let record = read(RIG_REL);
    let thresholds = read(THRESHOLDS_REL);
    let editor = flat_scalar(&record, "editor_binary_name").unwrap_or_default();
    Fixture {
        reason_values: table_list(&thresholds, "k2_probe", "reason_values"),
        machine_rederived_reason: table_scalar(&thresholds, "k2_probe", "machine_rederived_reason")
            .expect("invariant: [k2_probe].machine_rederived_reason is frozen"),
        non_rederived_require_engine: table_scalar(
            &thresholds,
            "k2_probe",
            "non_rederived_reasons_require_engine_present",
        )
        .expect("invariant: [k2_probe].non_rederived_reasons_require_engine_present is frozen")
            == "true",
        decision_resolution: table_scalar(&thresholds, "census", "decision_resolution")
            .expect("invariant: [census].decision_resolution is frozen"),
        digest_recomputed: sha256_hex(&normalised_thresholds()),
        digest_pinned: pinned_digest_literal(),
        authorities: probe_authorities(&editor),
        record,
    }
}

impl Fixture {
    /// `gpu_name_at_boot` stays `None`: booting a device to cross-check a field only the positive
    /// branch reads would make a filesystem-only rung GPU-dependent for nothing.
    fn inputs<'a>(&'a self, record: &'a str, engine_present: bool) -> GateInputs<'a> {
        GateInputs {
            record,
            engine_present,
            digest_recomputed: &self.digest_recomputed,
            digest_pinned: &self.digest_pinned,
            reason_values: &self.reason_values,
            machine_rederived_reason: &self.machine_rederived_reason,
            non_rederived_require_engine: self.non_rederived_require_engine,
            decision_resolution: &self.decision_resolution,
            gpu_name_at_boot: None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// THE GATE.
// ---------------------------------------------------------------------------------------------

#[test]
fn r0a_gate_holds_on_the_live_record() {
    let fx = fixture();
    let engine = fx.authorities.reports_engine();
    let v = violations(&fx.inputs(&fx.record, engine));
    assert!(
        v.is_empty(),
        "RED: R0a's gate does not hold on docs/VG-R0-REFERENCE-RIG.toml:\n  - {}",
        v.join("\n  - ")
    );
}

/// The record DESCRIBES the search; this asserts it describes the search that actually ran, so the
/// evidence fields cannot drift away from the machine that produced them.
#[test]
fn the_record_matches_a_live_probe() {
    let fx = fixture();
    let a = &fx.authorities;
    let expect = [
        ("authority_launcher_manifest_present", a.manifest_present.to_string()),
        ("authority_hkcu_builds_key_present", a.hkcu_present.to_string()),
        ("authority_hklm_installs_key_present", a.hklm_present.to_string()),
        ("authority_entries_seen", a.entries.len().to_string()),
        ("authority_entries_with_editor_binary", a.with_binary.len().to_string()),
        ("authority_reports_engine", a.reports_engine().to_string()),
    ];
    for (key, measured) in expect {
        let recorded = flat_scalar(&fx.record, key)
            .unwrap_or_else(|| panic!("invariant: the record must carry `{key}`"));
        assert_eq!(
            recorded, measured,
            "RED: the record's `{key}` disagrees with a live probe. The record describes the \
             search; it may not describe a different one. entries={:?} with_binary={:?}",
            a.entries, a.with_binary
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RED MUTATIONS. Each substitutes exactly ONE input of the live state and requires the
// corresponding part to report — a mutation that is only argued does not count.
// ---------------------------------------------------------------------------------------------

/// Replaces a top-level scalar's value, anchored to the KEY LINE rather than to the value's text —
/// the `replacen` trap this campaign hit three times is a mutation that edits a *comment* mentioning
/// the name instead of the construct.
fn rewrite_key(record: &str, key: &str, new_value: &str) -> String {
    let mut out = String::with_capacity(record.len());
    let mut hit = 0usize;
    let mut past_tables = false;
    for line in record.lines() {
        let stripped = strip_comment(line).trim();
        if stripped.starts_with('[') && !stripped.starts_with("[[") {
            past_tables = true;
        }
        let is_target = !past_tables
            && stripped
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == key);
        if is_target {
            hit += 1;
            out.push_str(&format!("{key} = {new_value}"));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    assert_eq!(hit, 1, "invariant: the mutation must hit `{key}` exactly once, not {hit}");
    out
}

#[test]
fn blanking_a_negative_field_is_reported() {
    let fx = fixture();
    for field in NEGATIVE_FIELDS {
        let dirty = rewrite_key(&fx.record, field, "\"PENDING\"");
        let v = violations(&fx.inputs(&dirty, fx.authorities.reports_engine()));
        assert!(
            v.iter().any(|m| m.contains("(a')") && m.contains(field)),
            "RED: blanking `{field}` was NOT reported by (a'). got={v:?}"
        );
    }
}

#[test]
fn a_reason_outside_the_frozen_set_is_reported() {
    let fx = fixture();
    let dirty = rewrite_key(&fx.record, "reason", "\"no_engine_registerd\"");
    let v = violations(&fx.inputs(&dirty, fx.authorities.reports_engine()));
    assert!(
        v.iter().any(|m| m.contains("(a')") && m.contains("outside the frozen")),
        "RED: a `reason` outside [k2_probe].reason_values was NOT reported — a one-character typo \
         must stop the rung rather than disarm it. got={v:?}"
    );
}

/// (b′), presupposition arm: a cause that presupposes an install, recorded on a box where the
/// authorities report none, is self-contradictory.
#[test]
fn a_non_rederived_reason_with_no_engine_present_is_reported() {
    let fx = fixture();
    let dirty = rewrite_key(&fx.record, "reason", "\"insufficient_disk\"");
    let v = violations(&fx.inputs(&dirty, false));
    assert!(
        v.iter().any(|m| m.contains("(b')") && m.contains("self-contradictory")),
        "RED: `insufficient_disk` on a box reporting no engine was NOT reported. got={v:?}"
    );
}

/// (b′), re-derivable arm — the converse direction, and the one that makes the negative the
/// machine's rather than the author's.
#[test]
fn claiming_no_engine_while_one_is_registered_is_reported() {
    let fx = fixture();
    let v = violations(&fx.inputs(&fx.record, true));
    assert!(
        v.iter().any(|m| m.contains("(b')") && m.contains("IS registered")),
        "RED: a fabricated `no_engine_registered` was NOT refuted by the authorities. got={v:?}"
    );
}

/// (d)'s mutation — the P0's, and the one Rev 1 had no way to express. Editing a threshold moves
/// the file's digest while the record keeps the old one.
#[test]
fn an_edited_threshold_is_reported() {
    let fx = fixture();
    let base =
        String::from_utf8(normalised_thresholds()).expect("invariant: the thresholds file is UTF-8");

    // ⚠️ ANCHORED TO THE KEY LINE, newlines included. `d_est_min = 1.0` also appears inside a prose
    // comment forty lines above its key, so a bare `replace` would edit the COMMENT — the
    // `replacen` trap this campaign hit three times, where a control passed for the wrong reason.
    let anchor = "\nd_est_min = 1.0\n";
    assert_eq!(
        base.matches(anchor).count(),
        1,
        "invariant: the mutation must name the key line exactly once"
    );
    let edited = base.replace(anchor, "\nd_est_min = 0.9\n");
    let moved = sha256_hex(edited.as_bytes());
    assert_ne!(
        moved, fx.digest_recomputed,
        "invariant: the mutation must move the digest, or it demonstrates nothing"
    );
    let inp = GateInputs { digest_recomputed: &moved, ..fx.inputs(&fx.record, false) };
    let v = violations(&inp);
    assert!(
        v.iter().any(|m| m.contains("(d)") && m.contains("re-hashed")),
        "RED: an edited threshold was NOT reported by (d). got={v:?}"
    );
}

/// The other half of (d): the record may not mint its own number.
#[test]
fn a_digest_that_is_not_the_pinned_literal_is_reported() {
    let fx = fixture();
    let minted = sha256_hex(b"a second number, minted here");
    let dirty = rewrite_key(&fx.record, "thresholds_sha256", &format!("\"{minted}\""));
    let v = violations(&fx.inputs(&dirty, false));
    assert!(
        v.iter().any(|m| m.contains("(d)") && m.contains("freeze tripwire pins")),
        "RED: a minted digest was NOT reported by (d). got={v:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// The binary-presence clause, in BOTH directions.
// ---------------------------------------------------------------------------------------------

/// The clause must not weaken detection: a directory that really carries the editor binary is
/// still reported as a registered engine, which is what keeps a fabricated `no_engine_registered`
/// refutable.
#[test]
fn a_registered_engine_is_still_detected() {
    let root = std::env::temp_dir().join("vg_r0a_probe_fixture");
    let mut bin_dir = root.join("UE_Fixture");
    for seg in EDITOR_BINARY_SUBPATH {
        bin_dir.push(seg);
    }
    std::fs::create_dir_all(&bin_dir).expect("invariant: the fixture directory is creatable");
    let engine_dir = root.join("UE_Fixture");
    let engine_dir_str = engine_dir.to_string_lossy().to_string();

    assert!(
        !carries_editor_binary(&engine_dir_str, "UnrealEditor.exe"),
        "a directory with no editor binary must not count as a registered engine — this is the \
         stale-entry case the live box exhibits"
    );

    let binary = bin_dir.join("UnrealEditor.exe");
    std::fs::write(&binary, b"").expect("invariant: the fixture binary is writable");
    assert!(
        carries_editor_binary(&engine_dir_str, "UnrealEditor.exe"),
        "RED: a directory that DOES carry the editor binary was not detected — the clause would \
         then let a fabricated `no_engine_registered` pass"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------------------------
// Known-answer test for the digest — see the SHA-256 block's note on why it is independent.
// ---------------------------------------------------------------------------------------------

#[test]
fn sha256_matches_the_fips_known_answers() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // 56 bytes — the padding boundary, where an off-by-one in the length block hides.
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}
