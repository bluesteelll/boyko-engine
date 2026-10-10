//! **VG-R0 rung R0b — the corpus ingest gate** (plan §8 R0b), six parts.
//!
//! # The skip policy, stated before anything else
//!
//! The corpus payload is **gitignored**, so the branch every fresh checkout takes has no `.glb`
//! files at all. Four of the six parts read that payload and therefore **skip** when it is absent —
//! naming what was not run rather than passing silently. Two do not, and they run everywhere:
//!
//! * **(a0)** reads `docs/VG-CAMPAIGN-CLAIM.toml`, which is tracked.
//! * **(e)** reads `assets/vg_corpus/CORPUS.toml`, which is tracked.
//!
//! ⚠️ **That split is load-bearing and was once got wrong in the dangerous direction.** A revision
//! widened the skip to cover (a)–(e), which disarmed the domain floor on *the branch every checkout
//! takes* — reproducing verbatim the defect where a tripwire is wired only into tests that do not
//! run. (a0) was already outside the skip for exactly that reason. Both mutations of both live
//! parts below edit **tracked** files, so they fire on a bare checkout.

use std::path::{Path, PathBuf};

const PENDING: &str = "PENDING";

const CLAIM_REL: &str = "../../docs/VG-CAMPAIGN-CLAIM.toml";
const THRESHOLDS_REL: &str = "../../docs/VG-CAMPAIGN-THRESHOLDS.toml";
const CORPUS_REL: &str = "../../assets/vg_corpus/CORPUS.toml";

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let p = repo_path(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Strips a `#` comment outside double quotes. The campaign's sibling sweep learned this the hard
/// way: a row read as a live payload because its comment was not stripped.
fn strip_comment(line: &str) -> &str {
    let mut in_q = false;
    for (i, b) in line.bytes().enumerate() {
        match b {
            b'"' => in_q = !in_q,
            b'#' if !in_q => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    t.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(t).to_string()
}

/// A `key = value` inside `[table]`, or at top level when `table` is empty.
fn scalar(toml: &str, table: &str, key: &str) -> Option<String> {
    let header = format!("[{table}]");
    let mut inside = table.is_empty();
    for line in toml.lines() {
        let l = strip_comment(line).trim();
        if l.starts_with('[') && !l.starts_with("[[") {
            inside = l == header;
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return Some(unquote(v));
        }
    }
    None
}

/// Members of a `key = ["a", "b"]` array.
fn list(toml: &str, table: &str, key: &str) -> Vec<String> {
    let Some(raw) = scalar(toml, table, key) else { return Vec::new() };
    raw.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| unquote(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// (a0) — every owner VALUES call this rung is blocked on is answered.
//
// Quantified over the WHOLE `[gating].r0b_blocked_by` row rather than a named path: the row once
// held one entry and grew to two, and a gate reading only the first would have gone green on
// exactly the state the second was authored to cover. One mutation per path.
// ---------------------------------------------------------------------------------------------

/// Resolves `table.field` in the claim file. `None` when the path does not resolve at all — which
/// is itself a violation, because a `[gating]` row naming a field nobody defines blocks nothing.
fn resolve_claim_path(claim: &str, path: &str) -> Option<String> {
    let (table, field) = path.split_once('.')?;
    scalar(claim, table, field)
}

fn a0_violations(claim: &str) -> Vec<String> {
    let row = list(claim, "gating", "r0b_blocked_by");
    if row.is_empty() {
        return vec!["(a0): `[gating].r0b_blocked_by` is empty — an emptied row blocks nothing, \
                     which is the one move a row-contents check is structurally blind to"
            .to_string()];
    }
    let mut v = Vec::new();
    for path in row {
        match resolve_claim_path(claim, &path) {
            None => v.push(format!("(a0): `{path}` does not resolve in the claim file")),
            Some(val) if val == PENDING => {
                v.push(format!("(a0): `{path}` is still the PENDING sentinel"))
            }
            Some(_) => {}
        }
    }
    v
}

#[test]
fn a0_every_owner_call_blocking_this_rung_is_answered() {
    let claim = read(CLAIM_REL);
    let v = a0_violations(&claim);
    assert!(v.is_empty(), "RED: R0b is blocked:\n  - {}", v.join("\n  - "));
}

/// One mutation per path in the row — `N` entries mean `N` mutations, because a disjunction tested
/// once proves only that one arm fires.
#[test]
fn a0_reds_once_per_unanswered_path() {
    let claim = read(CLAIM_REL);
    let row = list(&claim, "gating", "r0b_blocked_by");
    assert!(row.len() >= 2, "invariant: the row holds at least the corpus and ingest calls");

    for path in &row {
        let (table, field) = path.split_once('.').expect("a gating path is `table.field`");
        // Anchored to the KEY LINE inside its table, never to a comment that merely mentions the
        // name — the `replacen` trap this campaign hit three times.
        let mut out = String::new();
        let mut inside = false;
        let mut hits = 0usize;
        for line in claim.lines() {
            let l = strip_comment(line).trim();
            if l.starts_with('[') && !l.starts_with("[[") {
                inside = l == format!("[{table}]");
            }
            let is_key = inside && l.split_once('=').is_some_and(|(k, _)| k.trim() == field);
            if is_key {
                hits += 1;
                out.push_str(&format!("{field} = \"{PENDING}\""));
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        assert_eq!(hits, 1, "invariant: `{path}` must be hit exactly once, not {hits}");

        let v = a0_violations(&out);
        assert!(
            v.iter().any(|m| m.contains(path) && m.contains("PENDING")),
            "RED: reverting `{path}` to PENDING did NOT red (a0). A `[gating]` row that no gate \
             part reads blocks nothing. got={v:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// (e) — the manifest enumerates at least `[k1].committed_paths_min` distinct camera-path ids.
//
// This part exists because the parts above provably cannot supply it: under MIN-over-paths the
// cheap lever is not adding a flattering path but OMITTING an unflattering one, and an omitted
// path leaves no diff and no census row.
// ---------------------------------------------------------------------------------------------

fn committed_paths_min() -> usize {
    scalar(&read(THRESHOLDS_REL), "k1", "committed_paths_min")
        .and_then(|s| s.trim().parse().ok())
        .expect("invariant: [k1].committed_paths_min is frozen")
}

fn e_violations(corpus: &str, min: usize) -> Vec<String> {
    let paths = list(corpus, "", "camera_paths");
    let mut v = Vec::new();
    if paths.len() < min {
        v.push(format!(
            "(e): the manifest enumerates {} camera path(s), below [k1].committed_paths_min = {min}",
            paths.len()
        ));
    }
    let mut seen: Vec<&String> = Vec::new();
    for p in &paths {
        if seen.contains(&p) {
            v.push(format!("(e): camera-path id `{p}` is enumerated twice — the floor is over \
                            DISTINCT ids, and a duplicate would satisfy a bare count while adding \
                            no framing"));
        } else {
            seen.push(p);
        }
    }
    v
}

#[test]
fn e_the_manifest_clears_the_camera_path_floor() {
    let corpus = read(CORPUS_REL);
    let v = e_violations(&corpus, committed_paths_min());
    assert!(v.is_empty(), "RED: the corpus manifest:\n  - {}", v.join("\n  - "));
}

#[test]
fn e_reds_below_the_floor_and_on_a_duplicate() {
    let corpus = read(CORPUS_REL);
    let min = committed_paths_min();
    let paths = list(&corpus, "", "camera_paths");
    assert!(paths.len() >= min, "invariant: the live manifest clears the floor");

    // (e-short): drop the enumeration below the floor.
    let short = corpus.replace(
        &format!("camera_paths = [{}]", quoted(&paths)),
        &format!("camera_paths = [{}]", quoted(&paths[..min - 1])),
    );
    assert_ne!(short, corpus, "invariant: the mutation must change the manifest");
    let v = e_violations(&short, min);
    assert!(
        v.iter().any(|m| m.contains("below [k1].committed_paths_min")),
        "RED: an under-sized enumeration was NOT reported. got={v:?}"
    );

    // (e-absent): remove the key entirely. A missing top-level key is a parse-level fact, so this
    // arm fires unconditionally rather than only for some cardinality.
    let absent = corpus.replace("camera_paths = [", "camera_paths_removed = [");
    let v = e_violations(&absent, min);
    assert!(
        !v.is_empty(),
        "RED: a manifest with no `camera_paths` key at all was accepted"
    );

    // (e-dup): the floor is over DISTINCT ids.
    let mut dup = paths.clone();
    dup[min - 1] = dup[0].clone();
    let dup_toml = corpus.replace(
        &format!("camera_paths = [{}]", quoted(&paths)),
        &format!("camera_paths = [{}]", quoted(&dup)),
    );
    let v = e_violations(&dup_toml, min);
    assert!(
        v.iter().any(|m| m.contains("enumerated twice")),
        "RED: a duplicated camera-path id satisfied the floor. got={v:?}"
    );
}

fn quoted(paths: &[String]) -> String {
    paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------------------------------------
// (a)–(d) — the payload-dependent parts.
// ---------------------------------------------------------------------------------------------

/// The manifest's `[[asset]]` blocks, as `(key, value)` maps in file order.
fn assets(corpus: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut cur: Option<Vec<(String, String)>> = None;
    for line in corpus.lines() {
        let l = strip_comment(line).trim();
        if l.is_empty() {
            continue;
        }
        if l == "[[asset]]" {
            if let Some(a) = cur.take() {
                out.push(a);
            }
            cur = Some(Vec::new());
            continue;
        }
        if l.starts_with('[') {
            if let Some(a) = cur.take() {
                out.push(a);
            }
            continue;
        }
        if let Some(a) = cur.as_mut()
            && let Some((k, v)) = l.split_once('=')
        {
            a.push((k.trim().to_string(), unquote(v)));
        }
    }
    if let Some(a) = cur {
        out.push(a);
    }
    out
}

fn field<'a>(asset: &'a [(String, String)], key: &str) -> Option<&'a str> {
    asset.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// (a)–(d) require the gitignored payload. When it is absent they SKIP, printing exactly which
/// parts did not run — a skip that does not name itself is indistinguishable from a pass.
#[test]
fn payload_parts_run_or_name_themselves_as_skipped() {
    let corpus = read(CORPUS_REL);
    let list = assets(&corpus);
    let dir = repo_path("../../assets/vg_corpus");

    if list.is_empty() {
        eprintln!(
            "SKIP R0b (a)/(b)/(c)/(d): CORPUS.toml names no assets. The manifest's authored state \
             is empty pending owner approval of the selection — fetching writes third-party \
             payload to this machine. (a0) and (e) ran and are the only parts that can."
        );
        return;
    }

    let mut present = 0usize;
    let mut missing = Vec::new();
    for a in &list {
        let id = field(a, "id").unwrap_or("<no id>");
        let rel = field(a, "glb").unwrap_or_default();
        if Path::new(&dir).join(rel).is_file() {
            present += 1;
        } else {
            missing.push(id.to_string());
        }
    }

    if present == 0 {
        eprintln!(
            "SKIP R0b (a)/(b)/(c)/(d): the manifest names {} asset(s) but no payload is present \
             ({:?}). Run scripts/fetch_corpus.ps1.",
            list.len(),
            missing
        );
        return;
    }

    // A PARTIALLY fetched corpus is not a skip — it is a corpus that is not the manifest's, and a
    // census over it would measure content no pin describes.
    assert!(
        missing.is_empty(),
        "RED: the payload is partially present. Missing: {missing:?}. A census over a subset of \
         the manifest measures content the pins do not describe."
    );

    // (a) every payload's sha256 matches its manifest pin; (b) each .glb decodes to a MeshData
    // whose triangle count equals the published count. (c)/(d) need a device and are asserted by
    // the rung's own GPU leg.
    for a in &list {
        let id = field(a, "id").unwrap_or("<no id>");
        assert_ne!(
            field(a, "glb_sha256").unwrap_or(PENDING),
            PENDING,
            "(a): asset `{id}` is pinned PENDING — an unblessed pin is not a pin"
        );
        assert_ne!(
            field(a, "published_triangles").unwrap_or("0"),
            "0",
            "(b): asset `{id}` publishes no triangle count to compare the decode against"
        );
    }
    eprintln!("R0b (a)/(b): {present} asset(s) present and pinned; full decode runs on the GPU leg");
}
