//! `ug15 unit-controls`: V11's red controls and its two green controls on one commit, and the
//! control receipt that certifies the instrument key (IB design Rev 5 RC-11 (a)-(d), RC-13; Rev 6
//! RC-11 (e)/(f), RC-14, green control (d); ruling 15 W13/W14; orchestrator ruling 55 Q3/Q6).
//!
//! # Each control evaluates only the legs it is aimed at (critique W3 (ii))
//!
//! A control never runs `unit` end to end. It calls the leg's own function on the builds it needs
//! and compares that leg's per-unit verdict with its expectation, so an edit that by construction
//! breaks another leg — RC-11 (a)/(b)/(d) change the very manifests V11 (a) checks — cannot turn
//! into an "unexpected" verdict on a leg the control does not read.
//!
//! # Main tree and scratch trees
//!
//! G-b, RC-11 (c), RC-13 and RC-14 (a)/(b) run on this tree, in the lane's target dir, after the
//! reference builds (the touched runner and the probe) whose receipt RC-11 (e)/(f) and G-d compare
//! against. RC-11 (a)/(b)/(d)/(e)/(f), RC-14 (c) and G-d each need an edited (or a second) checkout:
//! a `git worktree add --detach` of HEAD under `--scratch/<id>/tree`, its own target dir and retain
//! dir beside it, and ITS OWN `ug15` built in it (`host::check_cwd` ties an executable to its tree).
//! That `ug15 unit-arm` writes a receipt the controller reads back; its key must equal the
//! controller's, or the control is UNEXPECTED (an edit reached the instrument). Trees run one at a
//! time; after each, the tree and the target dir are deleted by absolute path below `--scratch`
//! once the marker this command wrote is found, and `git worktree prune` runs only when every
//! prunable worktree lies below `--scratch` (it is repository-global; critique O4).
//!
//! # Under the profile's codegen-units (orchestrator ruling 57)
//!
//! Every build here goes through `retain::build_retained`, so every final unit is compiled at the
//! profile's effective codegen-units. Before that ruling, V11 (c2) was red at the baseline (a
//! codegen-units = 1 image against the shipped one), and RC-14 (a)'s expected (c2) red could not
//! tell its extra argument from that baseline. Now (c2) is green at the baseline (RF-5a, re-run),
//! so RC-14 (a)'s red is its argument's alone. RC-14 (b) reads freshness only, which codegen units do
//! not touch. Under RC-11 (e) the profile sets `codegen-units = 8`, so that tree's final lines carry
//! `-C codegen-units=8` twice (cargo's and ours) against the reference's single `=16`. (d3)'s closed
//! list does not normalise that token, so (d3) is red for that reason as well.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::closure::Graph;
use crate::host;
use crate::objbuild::{self, Ctx, Kind, Subject};
use crate::plain;
use crate::red::{Red, RedKind, Result};
use crate::retain::{self, RetainDir, Retained};
use crate::sha256;
use crate::unit::{self, ControlLine, ControlsReceipt, Receipt, UnitRun};

/// RC-11 (a)'s subject: the runner source built as a `[[bench]]` of a scratch parity package that
/// carries boyko-physics's dev line (the W8 mechanism). It exists only in that scratch tree.
pub const RC11A_BENCH: Subject =
    Subject { key: "rc11a_bench", package: "boyko-physics-parity", kind: Kind::Bench, name: "jolt_parity_pyramid_rc11a" };

/// `unit-controls`' arguments.
#[derive(Clone, Debug)]
pub struct ControlsArgs {
    /// The main tree's retain directory.
    pub retain: PathBuf,
    /// The scratch root: one `<id>/` per scratch control.
    pub scratch: PathBuf,
    /// Where the receipt goes when not the committed path (the window directory; `--only` needs it).
    pub receipt_out: Option<PathBuf>,
    /// `--only`: a subset of the control ids; the receipt is then partial.
    pub only: Option<Vec<String>>,
}

/// One control's outcome.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// Its receipt line.
    pub line: ControlLine,
    /// What it measured.
    pub details: Vec<String>,
}

fn outcome(id: &str, as_expected: bool, expected: &str, observed: String, details: Vec<String>) -> Outcome {
    Outcome { line: ControlLine { id: id.to_owned(), as_expected, expected: expected.to_owned(), observed }, details }
}

/// A tree-independent package key: `name@version` (a path package's id carries its tree's path).
#[must_use]
pub fn stable_pkg(id: &str) -> String {
    match id.rsplit_once('#') {
        Some((_, frag)) if frag.contains('@') => frag.to_owned(),
        Some((url, frag)) => format!("{}@{frag}", url.rsplit(['/', '\\']).next().unwrap_or(url)),
        None => id.to_owned(),
    }
}

/// The receipt's library rows of `tag` as [`retain::LibRow`]s (package ids made tree-independent),
/// so a scratch tree's tables compare with [`unit::diff_tables`].
fn rows(r: &Receipt, tag: &str) -> Vec<retain::LibRow> {
    r.libs_of(tag)
        .into_iter()
        .map(|l| retain::LibRow {
            crate_name: l.crate_name.clone(),
            package_id: stable_pkg(&l.package_id),
            kind: if l.kind == "proc-macro" { objbuild::ArtifactKind::ProcMacro } else { objbuild::ArtifactKind::Lib },
            host: l.host,
            path: PathBuf::from(l.path.rsplit(['\\', '/']).next().unwrap_or(&l.path)),
            sha256: l.sha256.clone(),
        })
        .collect()
}

/// The stable keys of every package that depends, transitively, on `on` (by name), in `graph`.
fn dependents_of(graph: &Graph, on: &str) -> BTreeSet<String> {
    let Some(target) = graph.nodes.values().find(|n| n.name == on).map(|n| n.id.clone()) else { return BTreeSet::new() };
    graph.nodes.keys().filter(|id| **id != target && graph.reachable(id).contains(&target)).map(|id| stable_pkg(id)).collect()
}

fn names(ids: &BTreeSet<String>) -> String {
    ids.iter().cloned().collect::<Vec<_>>().join(", ")
}

// ── The arms a scratch tree's `ug15 unit-arm` runs ──────────────────────────────────────────────

/// `ug15 unit-arm --arm <reference|runner|bench>`: the builds a scratch control needs, on THIS
/// tree, recorded in a receipt and nothing evaluated (the controller evaluates).
pub fn unit_arm(ctx: &Ctx, arm: &str, retain: &Path) -> Result<String> {
    let mut run = UnitRun::start(ctx, retain, &format!("unit-arm {arm}"))?;
    let lint = ["-W".to_owned(), "unused-crate-dependencies".to_owned()];
    let runner_s = objbuild::subject("parity_runner")?;
    let probe_s = objbuild::subject("physics_probe")?;
    match arm {
        "reference" => {
            run.touch_closure()?;
            run.build(runner_s, "runner", &[], &[], &lint)?;
            run.build(probe_s, "probe", &[], &[], &[])?;
        }
        "runner" => {
            run.touch_closure()?;
            run.build(runner_s, "runner", &[], &[], &lint)?;
        }
        "bench" => {
            run.build(RC11A_BENCH, "rc11a-runner", &[], &[], &[])?;
            run.build(probe_s, "probe", &[], &[], &[])?;
        }
        other => return Err(Red::new(RedKind::Usage, format!("unit-arm: no arm `{other}` (reference, runner, bench)"))),
    }
    Ok(run.receipt.to_text())
}

// ── Scratch trees ────────────────────────────────────────────────────────────────────────────────

/// One textual edit of a scratch tree: `old` must occur exactly `count` times in `file`.
#[derive(Clone, Copy, Debug)]
pub struct Edit {
    /// Relative to the tree root, forward slashes.
    pub file: &'static str,
    /// The text replaced (LF line ends).
    pub old: &'static str,
    /// Its replacement (LF line ends).
    pub new: &'static str,
    /// Exact occurrence count.
    pub count: usize,
}

/// What a scratch control's edit does to `Cargo.lock`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockRule {
    /// The edit changes the dependency graph: `cargo update --workspace --offline` re-derives it.
    Update,
    /// The lock must not change: `cargo metadata --locked` must accept the edited tree.
    MustNotChange,
}

/// One scratch control.
#[derive(Clone, Copy, Debug)]
pub struct ScratchControl {
    /// The control id.
    pub id: &'static str,
    /// Its edits.
    pub edits: &'static [Edit],
    /// The lock rule.
    pub lock: LockRule,
    /// The `unit-arm` it runs.
    pub arm: &'static str,
}

const PARITY_TOML: &str = "crates/boyko_physics_parity/Cargo.toml";
const PROBE_TOML: &str = "crates/boyko_physics_probe/Cargo.toml";
const PROBE_RS: &str = "crates/boyko_physics_probe/src/bin/physics_probe.rs";
const RUNNER_RS: &str = "crates/boyko_physics_parity/src/bin/jolt_parity_pyramid.rs";

/// The scratch controls, in run order.
pub const SCRATCH: [ScratchControl; 7] = [
    ScratchControl {
        id: "rc11a",
        edits: &[Edit {
            file: PARITY_TOML,
            old: "[lints]\nworkspace = true\n",
            new: "[lints]\nworkspace = true\n\n# RC-11 (a): boyko-physics's own dev line, the runner built as a [[bench]].\n[dev-dependencies]\nboyko-log = { path = \"../boyko_log\", features = [\"test-probe\"] }\n\n[[bench]]\nname = \"jolt_parity_pyramid_rc11a\"\npath = \"src/bin/jolt_parity_pyramid.rs\"\nharness = false\n",
            count: 1,
        }],
        lock: LockRule::Update,
        arm: "bench",
    },
    ScratchControl {
        id: "rc11b",
        edits: &[Edit {
            file: PARITY_TOML,
            old: "boyko-physics = { path = \"../boyko_physics\" }\n",
            new: "boyko-physics = { path = \"../boyko_physics\", features = [\"narrowphase-counts\"] }\n",
            count: 1,
        }],
        lock: LockRule::MustNotChange,
        arm: "reference",
    },
    ScratchControl {
        id: "rc11d",
        // The probe without the host line, and with its windowed branch compiled out (critique
        // W3 (i)): `mod windowed` names boyko_app, so the line alone would not build.
        edits: &[
            Edit { file: PROBE_TOML, old: "[target.'cfg(windows)'.dependencies]\nboyko-app = { path = \"../boyko_app\" }\n", new: "", count: 1 },
            Edit { file: PROBE_RS, old: "#[cfg(windows)]\n", new: "#[cfg(any())]\n", count: 2 },
            Edit { file: PROBE_RS, old: "#[cfg(not(windows))]\n", new: "#[cfg(not(any()))]\n", count: 1 },
        ],
        lock: LockRule::Update,
        arm: "reference",
    },
    ScratchControl {
        id: "rc11e",
        edits: &[Edit { file: "Cargo.toml", old: "[profile.release]\nlto = \"fat\"\n", new: "[profile.release]\nlto = \"fat\"\ncodegen-units = 8\n", count: 1 }],
        lock: LockRule::MustNotChange,
        arm: "reference",
    },
    ScratchControl {
        id: "rc11f",
        edits: &[Edit {
            file: "crates/boyko_input/Cargo.toml",
            old: "windows-sys.workspace = true\n",
            new: "windows-sys = { workspace = true, features = [\"Win32_System_Threading\"] }\n",
            count: 1,
        }],
        lock: LockRule::MustNotChange,
        arm: "reference",
    },
    ScratchControl {
        id: "rc14c",
        edits: &[Edit {
            file: RUNNER_RS,
            old: "use boyko_diag::lane::{LANE_COUNT, LANE_DISPATCHER, claim_lane};\n",
            new: "#[cfg(windows)]\nuse boyko_app as _;\nuse boyko_diag::lane::{LANE_COUNT, LANE_DISPATCHER, claim_lane};\n",
            count: 1,
        }],
        lock: LockRule::MustNotChange,
        arm: "runner",
    },
    ScratchControl { id: "g-d", edits: &[], lock: LockRule::MustNotChange, arm: "reference" },
];

/// The marker a scratch directory carries, so deletion only ever removes what this command made.
const MARKER: &str = ".ug15-scratch";

fn run_logged(cmd: &mut Command, log: &Path, what: &str) -> Result<()> {
    let f = std::fs::File::create(log).map_err(|e| Red::io(log, &e))?;
    let f2 = f.try_clone().map_err(|e| Red::io(log, &e))?;
    let status = cmd
        .stdout(Stdio::from(f))
        .stderr(Stdio::from(f2))
        .status()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("{what}: could not spawn: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Red::new(RedKind::BuildFailed, format!("{what} exited {:?}; see {}", status.code(), log.display())))
    }
}

fn apply_edit(tree: &Path, e: &Edit) -> Result<()> {
    let p = tree.join(e.file);
    let raw = std::fs::read(&p).map_err(|err| Red::io(&p, &err))?;
    let crlf = raw.windows(2).any(|w| w == b"\r\n");
    let text = String::from_utf8_lossy(&crate::key::normalize_eol(&raw)).into_owned();
    let n = text.matches(e.old).count();
    if n != e.count {
        return Err(Red::new(RedKind::Usage, format!("scratch edit of {}: the text to replace occurs {n} time(s), want {}", e.file, e.count)));
    }
    let edited = text.replace(e.old, e.new);
    let out = if crlf { edited.replace('\n', "\r\n") } else { edited };
    std::fs::write(&p, out).map_err(|err| Red::io(&p, &err))
}

/// Removes `dir` (a path below `scratch` whose parent carries this command's marker), retrying while
/// the linker's PDB server releases its files. Returns a note when something is left.
fn remove_scratch(scratch: &Path, dir: &Path) -> Option<String> {
    let marker = dir.parent().map(|p| p.join(MARKER));
    if !retain::is_within(dir, scratch) || !marker.as_ref().is_some_and(|m| m.is_file()) {
        return Some(format!("refused to delete {}: not below {} with a {MARKER} marker", dir.display(), scratch.display()));
    }
    for attempt in 0..6 {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return None,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) if attempt == 5 => return Some(format!("left {}: {e}", dir.display())),
            Err(_) => std::thread::sleep(std::time::Duration::from_secs(10)),
        }
    }
    None
}

/// `git worktree prune`, but only when every prunable worktree of the repository lies below
/// `scratch`: the command is repository-global, and another lane's missing tree is not ours to drop.
fn guarded_prune(root: &Path, scratch: &Path) -> Option<String> {
    let out = match Command::new("git").current_dir(root).args(["worktree", "list", "--porcelain"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Some("git worktree list failed, so git worktree prune was NOT run".to_owned()),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut current: Option<String> = None;
    let mut foreign = Vec::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current = Some(p.to_owned());
        } else if line.starts_with("prunable")
            && let Some(p) = &current
            && !retain::is_within(Path::new(p), scratch)
        {
            foreign.push(p.clone());
        }
    }
    if !foreign.is_empty() {
        return Some(format!("git worktree prune NOT run: prunable worktree(s) outside {}: {foreign:?}", scratch.display()));
    }
    let st = Command::new("git").current_dir(root).args(["worktree", "prune"]).status();
    if st.is_ok_and(|s| s.success()) { None } else { Some("git worktree prune failed".to_owned()) }
}

/// The paths of one scratch control.
struct ScratchPaths {
    dir: PathBuf,
    tree: PathBuf,
    target: PathBuf,
    retained: PathBuf,
}

/// A scratch tree's steps: check out HEAD, edit, apply the lock rule, build ITS OWN ug15, run its
/// arm, and read its receipt back.
fn scratch_steps(ctx: &Ctx, c: &ScratchControl, p: &ScratchPaths) -> Result<Receipt> {
    run_logged(
        Command::new("git").current_dir(&ctx.root).args(["worktree", "add", "--detach"]).arg(&p.tree).arg("HEAD"),
        &p.dir.join("worktree-add.log"),
        "git worktree add",
    )?;
    for e in c.edits {
        apply_edit(&p.tree, e)?;
    }
    let cargo = || {
        let mut cmd = Command::new(objbuild::cargo_program());
        cmd.current_dir(&p.tree).env("CARGO_TARGET_DIR", &p.target).env_remove("RUSTFLAGS").env_remove("CARGO_ENCODED_RUSTFLAGS");
        cmd
    };
    match c.lock {
        LockRule::Update => run_logged(cargo().args(["update", "--workspace", "--offline"]), &p.dir.join("lock.log"), "cargo update --workspace --offline")?,
        LockRule::MustNotChange => run_logged(
            cargo().args(["metadata", "--format-version", "1", "--locked", "--offline"]),
            &p.dir.join("lock.log"),
            "cargo metadata --locked (the edit must not move Cargo.lock)",
        )?,
    }
    run_logged(cargo().args(["build", "-p", "boyko-symcensus", "--bin", "ug15", "--locked"]), &p.dir.join("ug15-build.log"), "the scratch tree's ug15 build")?;
    let exe = p.target.join("debug").join(format!("ug15{}", std::env::consts::EXE_SUFFIX));
    let out = p.dir.join("receipt.txt");
    run_logged(
        Command::new(&exe)
            .current_dir(&p.tree)
            .env("CARGO_TARGET_DIR", &p.target)
            .args(["unit-arm", "--arm", c.arm, "--retain"])
            .arg(&p.retained)
            .arg("--out")
            .arg(&out),
        &p.dir.join("arm.log"),
        &format!("the scratch tree's ug15 unit-arm --arm {}", c.arm),
    )?;
    let text = std::fs::read_to_string(&out).map_err(|e| Red::io(&out, &e))?;
    Receipt::parse(&text)
}

/// Runs one scratch control's tree ([`scratch_steps`]), then keeps the edit's diff and removes the
/// tree and its target dir whatever the steps returned. Returns the steps' receipt (or their RED)
/// and the cleanup notes.
fn scratch_tree(ctx: &Ctx, scratch: &Path, c: &ScratchControl, key_hex: &str) -> (Result<Receipt>, Vec<String>) {
    let dir = scratch.join(c.id);
    if dir.exists() {
        return (Err(Red::new(RedKind::Usage, format!("{} exists; every scratch control starts from nothing", dir.display()))), Vec::new());
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (Err(Red::io(&dir, &e)), Vec::new());
    }
    let marker = dir.join(MARKER);
    if let Err(e) = std::fs::write(&marker, format!("ug15 unit-controls scratch {} for key {key_hex}\n", c.id)) {
        return (Err(Red::io(&marker, &e)), Vec::new());
    }
    let p = ScratchPaths { tree: dir.join("tree"), target: dir.join("target"), retained: dir.join("retained"), dir };
    (ctx.echo)(&format!("ug15 unit-controls: scratch {} at {}", c.id, p.dir.display()));
    let result = scratch_steps(ctx, c, &p);
    let mut notes = Vec::new();
    // The edit as a diff, lock included, kept beside the receipt for the reviewer (the tree goes).
    if let Ok(o) = Command::new("git").current_dir(&p.tree).args(["diff"]).output()
        && let Err(e) = std::fs::write(p.dir.join("edit.diff"), &o.stdout)
    {
        notes.push(format!("could not keep the edit's diff: {e}"));
    }
    for d in [&p.tree, &p.target] {
        if let Some(n) = remove_scratch(scratch, d) {
            notes.push(n);
        }
    }
    if let Some(n) = guarded_prune(&ctx.root, scratch) {
        notes.push(n);
    }
    (result, notes)
}

// ── The expectations ─────────────────────────────────────────────────────────────────────────────

fn id_set<'a>(v: impl Iterator<Item = &'a String>) -> BTreeSet<String> {
    v.cloned().collect()
}

/// RC-11 (a): path differs at boyko_log and every target unit depending on it; nothing else moves.
fn expect_rc11a(r: &Receipt, graph: &Graph) -> (bool, String, Vec<String>) {
    let (a, b) = (rows(r, "rc11a-runner"), rows(r, "probe"));
    let d = unit::diff_tables(&a, &b, |_| true);
    let present: BTreeSet<String> = a.iter().filter(|l| !l.host).map(|l| l.package_id.clone()).collect();
    let mut want: BTreeSet<String> = dependents_of(graph, "boyko-log").intersection(&present).cloned().collect();
    want.insert(stable_pkg(&graph.nodes.values().find(|n| n.name == "boyko-log").map(|n| n.id.clone()).unwrap_or_default()));
    let got = id_set(d.path_differs.iter().map(|x| &x.0));
    let host_moved: Vec<&retain::LibRow> = a.iter().filter(|l| l.host && got.contains(&l.package_id)).collect();
    let ok = got == want && d.sha_differs.is_empty() && d.only_a.is_empty() && d.only_b.is_empty() && host_moved.is_empty();
    (ok, format!("(b) red, path differs at {} unit(s): {}", got.len(), names(&got)), d.lines("(b)"))
}

/// RC-11 (b): path differs at boyko_physics alone (and at its dependents in the build, if any).
fn expect_rc11b(r: &Receipt, graph: &Graph) -> (bool, String, Vec<String>) {
    let (a, b) = (rows(r, "runner"), rows(r, "probe"));
    let d = unit::diff_tables(&a, &b, |_| true);
    let present: BTreeSet<String> = a.iter().filter(|l| !l.host).map(|l| l.package_id.clone()).collect();
    let mut want: BTreeSet<String> = dependents_of(graph, "boyko-physics").intersection(&present).cloned().collect();
    match graph.workspace_id("boyko-physics") {
        Ok(id) => {
            want.insert(stable_pkg(id));
        }
        Err(red) => return (false, format!("the controller's graph names no boyko-physics: {red}"), Vec::new()),
    }
    let got = id_set(d.path_differs.iter().map(|x| &x.0));
    let ok = got == want && d.sha_differs.is_empty() && d.only_a.is_empty() && d.only_b.is_empty();
    (ok, format!("(b) red, path differs at {}", names(&got)), d.lines("(b)"))
}

/// RC-11 (d): through (e)'s closure-restricted comparison, the PATH differs at windows_sys,
/// boyko_input, boyko_scene and boyko_physics, and both tables cover the closure (the probe still
/// builds: its windowed branch is compiled out with the host line). Any further red crate is a
/// second mechanism, recorded and named (Rev 5), not unexpected.
fn expect_rc11d(r: &Receipt) -> (bool, String, Vec<String>) {
    let closure: BTreeSet<String> = r.closure.iter().map(|c| stable_pkg(&c.package_id)).collect();
    let (a, b) = (rows(r, "runner"), rows(r, "probe"));
    let d = unit::diff_tables(&a, &b, |l| closure.contains(&l.package_id));
    let red = d.differing_ids();
    let red_crates: BTreeSet<String> = a.iter().chain(b.iter()).filter(|l| red.contains(&l.package_id)).map(|l| l.crate_name.clone()).collect();
    let path_crates: BTreeSet<String> = d.path_differs.iter().map(|x| x.1.clone()).collect();
    let want: BTreeSet<String> = ["windows_sys", "boyko_input", "boyko_scene", "boyko_physics"].iter().map(|s| (*s).to_owned()).collect();
    let covered = |t: &[retain::LibRow]| -> Vec<String> {
        r.closure.iter().filter(|c| !t.iter().any(|l| l.package_id == stable_pkg(&c.package_id))).map(|c| c.crate_name.clone()).collect()
    };
    let (miss_a, miss_b) = (covered(&a), covered(&b));
    let mut lines = d.lines("(e)");
    let extra: Vec<&String> = red_crates.difference(&want).collect();
    if !extra.is_empty() {
        lines.push(format!("(e) a second mechanism, recorded (Rev 5): also red at {extra:?}"));
    }
    if !miss_a.is_empty() || !miss_b.is_empty() {
        lines.push(format!("(e) floor: the runner misses {miss_a:?}, the probe misses {miss_b:?}"));
    }
    let ok = want.is_subset(&path_crates) && miss_a.is_empty() && miss_b.is_empty();
    (ok, format!("(e) red at {} (path at {})", names(&red_crates), names(&path_crates)), lines)
}

/// RC-11 (e): (d1) red at every target closure unit and unchanged at every host unit; (d2) red at
/// every target workspace line, the proc-macro's unchanged; (d3) red; (d4) green (W14).
fn expect_rc11e(rep: &unit::CompareReport, pre: &Receipt) -> (bool, String, Vec<String>) {
    let host_crates: BTreeSet<String> = pre.libs_of("runner").iter().filter(|l| l.host).map(|l| l.crate_name.clone()).collect();
    let d1_target_same: Vec<&unit::Cmp> = rep.d1.iter().filter(|c| !c.host && c.same).collect();
    let d1_host_moved: Vec<&unit::Cmp> = rep.d1.iter().filter(|c| c.host && !c.same).collect();
    let d2_target_same: Vec<&unit::Cmp> = rep.d2.iter().filter(|c| !host_crates.contains(&c.item) && c.same).collect();
    let d2_host_moved: Vec<&unit::Cmp> = rep.d2.iter().filter(|c| host_crates.contains(&c.item) && !c.same).collect();
    let ok = rep.refusals.is_empty()
        && !rep.d1.is_empty()
        && d1_target_same.is_empty()
        && d1_host_moved.is_empty()
        && rep.d1.iter().any(|c| c.host)
        && d2_target_same.is_empty()
        && d2_host_moved.is_empty()
        && rep.d2.iter().any(|c| host_crates.contains(&c.item))
        && !rep.leg_green(3)
        && rep.leg_green(4);
    let observed = format!(
        "(d1) {} target red / {} host unchanged; (d2) {} target red / {} host unchanged; (d3) {}; (d4) {}",
        rep.d1.iter().filter(|c| !c.host && !c.same).count(),
        rep.d1.iter().filter(|c| c.host && c.same).count(),
        rep.d2.iter().filter(|c| !host_crates.contains(&c.item) && !c.same).count(),
        rep.d2.iter().filter(|c| host_crates.contains(&c.item) && c.same).count(),
        if rep.leg_green(3) { "green" } else { "red" },
        if rep.leg_green(4) { "green" } else { "red" }
    );
    let mut lines: Vec<String> = rep.text().lines().map(str::to_owned).collect();
    for c in d1_target_same.iter().chain(d2_target_same.iter()) {
        lines.push(format!("UNEXPECTED: {} stayed equal", c.item));
    }
    for c in d1_host_moved.iter().chain(d2_host_moved.iter()) {
        lines.push(format!("UNEXPECTED: host unit {} moved", c.item));
    }
    (ok, observed, lines)
}

/// RC-11 (f): (d1) red at windows_sys, boyko_input, boyko_scene, boyko_physics and nowhere else;
/// (d2) red at those three workspace lines; (d3) red at the `--extern` names of boyko_physics and
/// boyko_app only; (d4) green.
fn expect_rc11f(rep: &unit::CompareReport) -> (bool, String, Vec<String>) {
    let d1_red: BTreeSet<String> = rep.d1.iter().filter(|c| !c.same).map(|c| c.item.clone()).collect();
    let d2_red: BTreeSet<String> = rep.d2.iter().filter(|c| !c.same).map(|c| c.item.clone()).collect();
    let want1: BTreeSet<String> = ["windows_sys", "boyko_input", "boyko_scene", "boyko_physics"].iter().map(|s| (*s).to_owned()).collect();
    let want2: BTreeSet<String> = ["boyko_input", "boyko_scene", "boyko_physics"].iter().map(|s| (*s).to_owned()).collect();
    let d3_words_ok = rep.d3.iter().all(|c| {
        c.same
            || (!c.words.is_empty()
                && c.words.iter().all(|w| {
                    let w = w.trim_matches('\'');
                    w.starts_with("boyko_physics=") || w.starts_with("boyko_app=")
                }))
    });
    let ok = rep.refusals.is_empty() && d1_red == want1 && d2_red == want2 && !rep.leg_green(3) && d3_words_ok && rep.leg_green(4);
    let observed = format!(
        "(d1) red at {}; (d2) red at {}; (d3) {}{}; (d4) {}",
        names(&d1_red),
        names(&d2_red),
        if rep.leg_green(3) { "green" } else { "red" },
        if d3_words_ok { " at the boyko_physics/boyko_app --extern names only" } else { " at other words too" },
        if rep.leg_green(4) { "green" } else { "red" }
    );
    (ok, observed, rep.text().lines().map(str::to_owned).collect())
}

// ── The run ──────────────────────────────────────────────────────────────────────────────────────

/// The control ids, in run order: the main tree's, then the scratch trees'.
pub const ORDER: [&str; 12] = ["g-b", "rc11c", "rc13", "rc14a", "rc14b", "rc11a", "rc11b", "rc11d", "rc11e", "rc11f", "rc14c", "g-d"];

fn tree_is_clean(root: &Path) -> Result<()> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("git status: {e}")))?;
    if out.status.success() && out.stdout.is_empty() {
        Ok(())
    } else {
        Err(Red::new(RedKind::Usage, "unit-controls runs on a clean tree only: the scratch trees are checkouts of HEAD, and the receipt certifies HEAD's instrument"))
    }
}

/// `ug15 unit-controls`: every control (or `--only`'s), the control receipt written, and `Ok` iff
/// every control run gave its expected verdict. Returns the receipt and where it was written.
pub fn unit_controls(ctx: &Ctx, args: &ControlsArgs) -> Result<(ControlsReceipt, PathBuf, bool)> {
    tree_is_clean(&ctx.root)?;
    if args.only.is_some() && args.receipt_out.is_none() {
        return Err(Red::new(RedKind::Usage, "--only writes a partial receipt, which never certifies: give it --receipt-out outside both trees"));
    }
    if let Some(p) = &args.receipt_out
        && (!p.is_absolute() || retain::is_within(p, &ctx.root) || retain::is_within(p, &ctx.target))
    {
        return Err(Red::new(RedKind::Usage, format!("--receipt-out {} must be absolute and outside the tree and the target dir", p.display())));
    }
    let wanted: Vec<&str> = match &args.only {
        Some(list) => {
            for id in list {
                if !ORDER.contains(&id.as_str()) {
                    return Err(Red::new(RedKind::Usage, format!("--only: no control `{id}` ({ORDER:?})")));
                }
            }
            ORDER.iter().copied().filter(|id| list.iter().any(|l| l == id)).collect()
        }
        None => ORDER.to_vec(),
    };
    // The scratch root obeys the retain directory's rules: absolute, outside both trees, empty.
    let scratch = RetainDir::open(&args.scratch, &ctx.root, &ctx.target)?.path().to_path_buf();
    let mut run = UnitRun::start(ctx, &args.retain, "unit-controls reference")?;
    let key_hex = run.key.hex.clone();
    let mut receipt = ControlsReceipt {
        key: key_hex.clone(),
        key_lines: run.key.receipt().lines().map(str::to_owned).collect(),
        tree: host::git_provenance(&ctx.root),
        partial: args.only.is_some(),
        ..ControlsReceipt::default()
    };
    let lint = ["-W".to_owned(), "unused-crate-dependencies".to_owned()];
    let runner_s = objbuild::subject("parity_runner")?;
    let probe_s = objbuild::subject("physics_probe")?;

    // The reference: the touched runner and the probe (the receipt RC-11 (e)/(f) and G-d read).
    run.touch_closure()?;
    let reference = run.build(runner_s, "runner", &[], &[], &lint)?;
    run.build(probe_s, "probe", &[], &[], &[])?;
    let reference_receipt = run.receipt.clone();
    let ref_path = run.retain.path().join("reference-receipt.txt");
    std::fs::write(&ref_path, reference_receipt.to_text()).map_err(|e| Red::io(&ref_path, &e))?;

    for id in &wanted {
        // A main-tree control that cannot produce its verdict (a build RED) is UNEXPECTED and
        // recorded. The run goes on, so the receipt of a STOP still holds every other verdict.
        let main = |id: &str, r: Result<Outcome>| {
            r.unwrap_or_else(|red| outcome(id, false, "the control's builds complete", format!("the control could not run: {red}"), Vec::new()))
        };
        let o = match *id {
            "g-b" => main("g-b", control_gb(&mut run, &reference, runner_s, &lint)),
            "rc11c" => main("rc11c", control_rc11c(&mut run, &reference, probe_s)),
            "rc13" => main("rc13", control_rc13(&mut run, runner_s, &lint)),
            "rc14a" => main("rc14a", control_rc14a(&mut run, runner_s, &lint)),
            "rc14b" => main("rc14b", control_rc14b(&mut run, runner_s, &lint)),
            sid => {
                let c = SCRATCH.iter().find(|c| c.id == sid).expect("invariant: ORDER's scratch ids are SCRATCH's");
                for e in c.edits {
                    receipt.edits.push((sid.to_owned(), format!("{}: replace {:?} ({}x) with {:?}", e.file, e.old, e.count, e.new)));
                }
                if c.edits.is_empty() {
                    receipt.edits.push((sid.to_owned(), "no edit: a second checkout of HEAD at another path depth".to_owned()));
                }
                control_scratch(ctx, &scratch, c, &key_hex, &run.graph, &reference_receipt)
            }
        };
        (ctx.echo)(&format!(
            "ug15 unit-controls: {} {}: {}",
            o.line.id,
            if o.line.as_expected { "as expected" } else { "UNEXPECTED" },
            o.line.observed
        ));
        receipt.details.extend(o.details.iter().map(|d| (o.line.id.clone(), d.clone())));
        receipt.controls.push(o.line);
    }
    let path = match &args.receipt_out {
        Some(p) => p.clone(),
        None => ctx.root.join(unit::committed_controls_path(run.key.k8())),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Red::io(parent, &e))?;
    }
    std::fs::write(&path, receipt.to_text()).map_err(|e| Red::io(&path, &e))?;
    let all = receipt.controls.iter().all(|c| c.as_expected);
    Ok((receipt, path, all))
}

fn control_gb(run: &mut UnitRun<'_>, reference: &Retained, runner_s: Subject, lint: &[String]) -> Result<Outcome> {
    let again = run.build(runner_s, "rc11g-runner", &[], &[], lint)?;
    let d = unit::diff_tables(&reference.libs, &again.libs, |_| true);
    let miss = unit::floor_missing(&again.libs, &run.closure);
    let ok = d.is_equal() && miss.is_empty();
    Ok(outcome("g-b", ok, "(b) green: two runner builds, nothing changed", format!("(b) {}: {} unit(s) identical; floor missing {miss:?}", if d.is_equal() { "green" } else { "red" }, d.same.len()), d.lines("(b)")))
}

fn control_rc11c(run: &mut UnitRun<'_>, reference: &Retained, probe_s: Subject) -> Result<Outcome> {
    let env = [("BOYKO_PROFILE".to_owned(), "shipping".to_owned())];
    let shipping = run.build(probe_s, "rc11c-probe", &[], &env, &[])?;
    let d = unit::diff_tables(&reference.libs, &shipping.libs, |_| true);
    let diag = run.graph.nodes.values().find(|n| n.name == "boyko-diag").map(|n| n.id.clone()).unwrap_or_default();
    let dependents = dependents_of(&run.graph, "boyko-diag");
    let sha: BTreeSet<String> = d.sha_differs.iter().map(|x| x.0.clone()).collect();
    let diag_moved = sha.contains(&diag);
    let stray: Vec<&String> = sha.iter().filter(|id| **id != diag && !dependents.contains(&stable_pkg(id))).collect();
    let ok = diag_moved && stray.is_empty() && d.path_differs.is_empty() && d.only_a.is_empty() && d.only_b.is_empty();
    let dep_recorded: Vec<String> = sha.iter().filter(|id| **id != diag).map(|id| stable_pkg(id)).collect();
    let mut lines = d.lines("(b)");
    lines.push(format!("(b) dependents of boyko_diag whose sha256 moved (recorded, not required): {dep_recorded:?}"));
    Ok(outcome(
        "rc11c",
        ok,
        "(b) red at boyko_diag: same path, different sha256; every other unit's path unchanged, every non-dependent's sha256 too",
        format!(
            "boyko_diag sha256 {}; {} path change(s); {} stray sha256 change(s); {} dependent sha256 change(s)",
            if diag_moved { "moved" } else { "UNMOVED" },
            d.path_differs.len(),
            stray.len(),
            dep_recorded.len()
        ),
        lines,
    ))
}

fn control_rc13(run: &mut UnitRun<'_>, runner_s: Subject, lint: &[String]) -> Result<Outcome> {
    let first = run.build(runner_s, "rc13a-runner", &[], &[], lint)?;
    run.build(runner_s, "rc13b-runner", &["narrowphase-counts".to_owned()], &[], lint)?;
    let guard = first.built.check_intact();
    let pair_guard = matches!(&guard, Err(r) if r.kind == RedKind::PairGuard);
    let img = sha256::file_hex(&first.image)?;
    let obj = sha256::file_hex(&first.object)?;
    let held = img == first.image_sha256 && obj == first.object_sha256;
    let ok = pair_guard && held;
    Ok(outcome(
        "rc13",
        ok,
        "the first build's check_intact RED [pair guard] after the second; its retained copies still hash to its own image and object sha256",
        format!(
            "check_intact {}; retained copies {}",
            match &guard {
                Ok(()) => "GREEN (the stem collision did not happen)".to_owned(),
                Err(r) => r.kind.label().to_owned(),
            },
            if held { "intact" } else { "CHANGED" }
        ),
        vec![format!("{guard:?}"), format!("retained image {img} recorded {}; object {obj} recorded {}", first.image_sha256, first.object_sha256)],
    ))
}

fn control_rc14a(run: &mut UnitRun<'_>, runner_s: Subject, lint: &[String]) -> Result<Outcome> {
    let mut extra = lint.to_vec();
    extra.push("-C".to_owned());
    extra.push("llvm-args=-align-all-functions=8".to_owned());
    let built = run.build(runner_s, "rc14a-runner", &[], &[], &extra)?;
    let p = plain::plain_build(run.ctx, runner_s, &[], &run.retain, &built)?;
    let c1 = plain::check_c1(&p, &built)?;
    let c2 = plain::check_c2(&built.image, &p.image)?;
    let ok = c1.green && !c2.green;
    let mut lines = c1.lines.clone();
    lines.extend(c2.lines.clone());
    Ok(outcome(
        "rc14a",
        ok,
        "(c1) green, (c2) red: an extra codegen argument reaches the final image only",
        format!("(c1) {}, (c2) {}", if c1.green { "green" } else { "red" }, if c2.green { "green" } else { "red" }),
        lines,
    ))
}

fn control_rc14b(run: &mut UnitRun<'_>, runner_s: Subject, lint: &[String]) -> Result<Outcome> {
    let built = run.build(runner_s, "rc14b-runner", &[], &[], lint)?;
    let input = run.ctx.root.join("crates").join("boyko_input").join("src").join("lib.rs");
    objbuild::touch(&input)?;
    let p = plain::plain_build(run.ctx, runner_s, &[], &run.retain, &built)?;
    let c1 = plain::check_c1(&p, &built)?;
    let got = plain::non_fresh(&p);
    let runner_id = run.graph.workspace_id("boyko-physics-parity")?.to_owned();
    let input_id = run.graph.workspace_id("boyko-input")?.to_owned();
    let in_build: BTreeSet<String> = p.artifacts.iter().map(|a| a.package_id.clone()).collect();
    let mut want: BTreeSet<String> = run.graph.dependents_within(&runner_id, &input_id).intersection(&in_build).cloned().collect();
    want.insert(input_id);
    want.remove(&runner_id);
    let ok = !c1.green && got == want && !p.final_fresh;
    let mut lines = c1.lines.clone();
    let missing: Vec<String> = want.difference(&got).map(|i| stable_pkg(i)).collect();
    let extra: Vec<String> = got.difference(&want).map(|i| stable_pkg(i)).collect();
    lines.push(format!("non-fresh {} unit(s); expected {}; missing {missing:?}; extra {extra:?}; final unit fresh {}", got.len(), want.len(), p.final_fresh));
    Ok(outcome(
        "rc14b",
        ok,
        "(c1) red; the non-fresh set EQUALS boyko_input, its dependents in the runner's build, and the final unit",
        format!("(c1) {}; non-fresh {} = expected {} ({}); final unit rebuilt {}", if c1.green { "green" } else { "red" }, got.len(), want.len(), if got == want { "equal" } else { "DIFFERENT" }, !p.final_fresh),
        lines,
    ))
}

fn control_scratch(ctx: &Ctx, scratch: &Path, c: &ScratchControl, key_hex: &str, graph: &Graph, reference: &Receipt) -> Outcome {
    let expected = match c.id {
        "rc11a" => "(b) red: path differs at boyko_log and every target unit depending on it; host units and all others identical",
        "rc11b" => "(b) red at boyko_physics (path) only",
        "rc11d" => "(e) red at windows_sys, boyko_input, boyko_scene, boyko_physics (paths); a further red crate is a second mechanism",
        "rc11e" => "(d1) red at every target closure unit, host units unchanged; (d2) red at every target workspace line, the proc-macro's unchanged; (d3) red; (d4) green",
        "rc11f" => "(d1) red at windows_sys, boyko_input, boyko_scene, boyko_physics only; (d2) red at those three workspace lines; (d3) red at the --extern names of boyko_physics and boyko_app; (d4) green; Cargo.lock unchanged",
        "rc14c" => "(f) red: no boyko_app unused-crate diagnostic",
        _ => "(d1)-(d4) all green",
    };
    let (r, notes) = match scratch_tree(ctx, scratch, c, key_hex) {
        (Ok(r), notes) => (r, notes),
        (Err(red), notes) => return outcome(c.id, false, expected, format!("the scratch tree did not produce a receipt: {red}"), notes),
    };
    if r.key != key_hex {
        return outcome(c.id, false, expected, format!("the scratch ug15's key {} is not the controller's: the edit reached the instrument", r.key), notes);
    }
    let (ok, observed, mut lines) = match c.id {
        "rc11a" => expect_rc11a(&r, graph),
        "rc11b" => expect_rc11b(&r, graph),
        "rc11d" => expect_rc11d(&r),
        "rc14c" => {
            let diags: Vec<String> = r.diags.iter().filter(|(t, _)| t == "runner").map(|(_, l)| l.clone()).collect();
            let f = unit::host_unused_line(&diags);
            (f.is_none(), format!("(f) {}", if f.is_none() { "red: no boyko_app diagnostic" } else { "GREEN" }), f.into_iter().collect())
        }
        id => {
            let rep = unit::unit_compare(reference, &r);
            match id {
                "rc11e" => expect_rc11e(&rep, reference),
                "rc11f" => expect_rc11f(&rep),
                _ => (rep.green(), format!("(d1)-(d4) {}", if rep.green() { "green" } else { "RED" }), rep.text().lines().map(str::to_owned).collect()),
            }
        }
    };
    lines.extend(notes);
    let mut obs = observed;
    let _ = write!(obs, " [scratch receipt {}]", scratch.join(c.id).join("receipt.txt").display());
    outcome(c.id, ok, expected, obs, lines)
}
