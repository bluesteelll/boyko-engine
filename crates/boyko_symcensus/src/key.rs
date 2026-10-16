//! The instrument key K: which `ug15` a control receipt certifies (IB design Rev 6, "Control
//! receipts certify an instrument, not a date"; ruling 15 W13; IB P0r critique W4).
//!
//! K is sha256 over, in order:
//! * `crates/boyko_symcensus/Cargo.toml`, then every file under `crates/boyko_symcensus/src/` in
//!   sorted forward-slash path order, each framed as `<relpath>\n<len>\n<bytes>` with `\r\n`
//!   normalised to `\n` (a raw hash would measure the checkout's line endings, not the source);
//! * `rustc -vV` of the compiler cargo invokes, and `cargo -V` of the very cargo `ug15` spawns,
//!   framed the same way (`\r\n` → `\n`, trimmed).
//!
//! # Bound to the binary that runs (W4)
//!
//! The source half is computed twice: from the bytes compiled into THIS executable
//! ([`COMPILED_SOURCES`], one `include_bytes!` per file), and from the tree on disk. They must be
//! equal, file set and bytes, or the run is RED [`Stale`](RedKind::Stale): an executable built
//! before an edit would otherwise compute K over the new sources while exercising the old code,
//! and a receipt it wrote would certify code no control ran. A file added under `src/` and not
//! listed here is on disk only, so it is RED by the same comparison, never silently uncovered.
//!
//! # Which rustc (critique open question 3)
//!
//! The one cargo invokes. K reads `$RUSTC` when set, else `rustc` resolved from the tree root with
//! the inherited `PATH` and `RUSTUP_TOOLCHAIN` — cargo's own resolution when neither `RUSTC` nor a
//! config `build.rustc` is set. That is a claim, so it is checked: every IB build reads the
//! compiler path from its final unit's `Running` line and requires that program's `-vV` to equal
//! K's ([`check_invoked_rustc`]); a wrapper or a different toolchain is RED [`Mismatch`](RedKind::Mismatch).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::red::{Red, RedKind, Result};
use crate::sha256::{self, Sha256};

/// The crate's directory relative to the workspace root, forward slashes.
pub const CRATE_DIR: &str = "crates/boyko_symcensus";

/// `(path relative to the crate directory, bytes)` of every file the key covers, as compiled
/// into this executable. Sorted: `Cargo.toml` first, then `src/` in forward-slash order.
pub const COMPILED_SOURCES: &[(&str, &[u8])] = &[
    ("Cargo.toml", include_bytes!("../Cargo.toml")),
    ("src/bin/ug15.rs", include_bytes!("bin/ug15.rs")),
    ("src/candidates.rs", include_bytes!("candidates.rs")),
    ("src/closure.rs", include_bytes!("closure.rs")),
    ("src/contain.rs", include_bytes!("contain.rs")),
    ("src/controls.rs", include_bytes!("controls.rs")),
    ("src/host.rs", include_bytes!("host.rs")),
    ("src/json.rs", include_bytes!("json.rs")),
    ("src/key.rs", include_bytes!("key.rs")),
    ("src/leg7b.rs", include_bytes!("leg7b.rs")),
    ("src/lib.rs", include_bytes!("lib.rs")),
    ("src/llvm.rs", include_bytes!("llvm.rs")),
    ("src/maps.rs", include_bytes!("maps.rs")),
    ("src/normalize.rs", include_bytes!("normalize.rs")),
    ("src/objbuild.rs", include_bytes!("objbuild.rs")),
    ("src/objview.rs", include_bytes!("objview.rs")),
    ("src/pe.rs", include_bytes!("pe.rs")),
    ("src/pins.rs", include_bytes!("pins.rs")),
    ("src/plain.rs", include_bytes!("plain.rs")),
    ("src/probes.rs", include_bytes!("probes.rs")),
    ("src/red.rs", include_bytes!("red.rs")),
    ("src/retain.rs", include_bytes!("retain.rs")),
    ("src/seam.rs", include_bytes!("seam.rs")),
    ("src/sha256.rs", include_bytes!("sha256.rs")),
    ("src/snapshot.rs", include_bytes!("snapshot.rs")),
    ("src/source.rs", include_bytes!("source.rs")),
    ("src/tools.rs", include_bytes!("tools.rs")),
    ("src/unit.rs", include_bytes!("unit.rs")),
];

/// One file's contribution, for the receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFile {
    /// `crates/boyko_symcensus/<rel>`.
    pub path: String,
    /// Length of the normalised bytes.
    pub len: usize,
    /// sha256 of the normalised bytes.
    pub sha256: String,
}

/// The instrument key and every component of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// sha256 over the source half and the toolchain half.
    pub hex: String,
    /// sha256 over the source half alone.
    pub source_hex: String,
    /// The source half, file by file.
    pub files: Vec<KeyFile>,
    /// `rustc -vV`, normalised and trimmed.
    pub rustc_vv: String,
    /// `cargo -V`, normalised and trimmed.
    pub cargo_v: String,
}

impl Key {
    /// The first eight hex digits: the control receipt's file-name key.
    #[must_use]
    pub fn k8(&self) -> &str {
        &self.hex[..8]
    }

    /// The receipt block naming every component.
    #[must_use]
    pub fn receipt(&self) -> String {
        let mut s = format!("key          {}\nkey source   {}\n", self.hex, self.source_hex);
        for f in &self.files {
            s.push_str(&format!("key file     {} {} {}\n", f.path, f.len, f.sha256));
        }
        for l in self.rustc_vv.lines() {
            s.push_str(&format!("key rustc    {l}\n"));
        }
        s.push_str(&format!("key cargo    {}\n", self.cargo_v));
        s
    }
}

/// `\r\n` → `\n`.
#[must_use]
pub fn normalize_eol(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn frame(h: &mut Sha256, name: &str, normalised: &[u8]) {
    h.update(name.as_bytes());
    h.update(b"\n");
    h.update(normalised.len().to_string().as_bytes());
    h.update(b"\n");
    h.update(normalised);
}

/// The source half over `files` (`(path relative to the crate dir, raw bytes)`), which must be
/// in key order; returns its digest and the per-file rows.
#[must_use]
pub fn source_digest(files: &[(String, Vec<u8>)]) -> (String, Vec<KeyFile>) {
    let mut h = Sha256::new();
    let mut rows = Vec::with_capacity(files.len());
    for (rel, raw) in files {
        let n = normalize_eol(raw);
        let path = format!("{CRATE_DIR}/{rel}");
        frame(&mut h, &path, &n);
        rows.push(KeyFile { path, len: n.len(), sha256: sha256::bytes_hex(&n) });
    }
    (h.finish_hex(), rows)
}

/// K over a source half and a toolchain half.
#[must_use]
pub fn key_hex(files: &[(String, Vec<u8>)], rustc_vv: &str, cargo_v: &str) -> String {
    let mut h = Sha256::new();
    for (rel, raw) in files {
        frame(&mut h, &format!("{CRATE_DIR}/{rel}"), &normalize_eol(raw));
    }
    frame(&mut h, "rustc -vV", normalize_text(rustc_vv).as_bytes());
    frame(&mut h, "cargo -V", normalize_text(cargo_v).as_bytes());
    h.finish_hex()
}

fn normalize_text(s: &str) -> String {
    s.replace("\r\n", "\n").trim().to_owned()
}

/// Sorts `(rel, bytes)` into key order: `Cargo.toml` first, then forward-slash path order.
pub fn sort_key_order(files: &mut [(String, Vec<u8>)]) {
    files.sort_by(|(a, _), (b, _)| (a != "Cargo.toml", a.as_str()).cmp(&(b != "Cargo.toml", b.as_str())));
}

/// The source half as compiled into this executable.
#[must_use]
pub fn compiled() -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = COMPILED_SOURCES.iter().map(|(p, b)| ((*p).to_owned(), b.to_vec())).collect();
    sort_key_order(&mut v);
    v
}

/// The source half as the tree at `crate_dir` holds it: `Cargo.toml` and every file under `src/`.
pub fn on_disk(crate_dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let manifest = crate_dir.join("Cargo.toml");
    let mut v = vec![("Cargo.toml".to_owned(), std::fs::read(&manifest).map_err(|e| Red::io(&manifest, &e))?)];
    let mut stack: Vec<PathBuf> = vec![crate_dir.join("src")];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir).map_err(|e| Red::io(&dir, &e))?;
        for entry in rd {
            let entry = entry.map_err(|e| Red::io(&dir, &e))?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p
                    .strip_prefix(crate_dir)
                    .map_err(|_| Red::new(RedKind::Io, format!("{} is not under {}", p.display(), crate_dir.display())))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&p).map_err(|e| Red::io(&p, &e))?;
                v.push((rel, bytes));
            }
        }
    }
    sort_key_order(&mut v);
    Ok(v)
}

/// RED [`Stale`](RedKind::Stale) unless `compiled` and `disk` hold the same files with the same
/// normalised bytes.
pub fn check_fresh(compiled: &[(String, Vec<u8>)], disk: &[(String, Vec<u8>)]) -> Result<()> {
    let mut stale = Vec::new();
    for (rel, bytes) in disk {
        match compiled.iter().find(|(r, _)| r == rel) {
            None => stale.push(format!("{rel}: on disk, not compiled into this ug15 (list it in key::COMPILED_SOURCES)")),
            Some((_, c)) if normalize_eol(c) != normalize_eol(bytes) => stale.push(format!("{rel}: edited since this ug15 was built")),
            Some(_) => {}
        }
    }
    for (rel, _) in compiled {
        if !disk.iter().any(|(r, _)| r == rel) {
            stale.push(format!("{rel}: compiled into this ug15, absent from the tree"));
        }
    }
    if stale.is_empty() {
        Ok(())
    } else {
        Err(Red::new(
            RedKind::Stale,
            format!(
                "this ug15 is not the instrument the tree holds, so the key it would compute names code it does not \
                 run: {}. Rebuild it: `cargo build -p boyko-symcensus --bin ug15 --locked`.",
                stale.join("; ")
            ),
        ))
    }
}

/// The compiler cargo invokes: `$RUSTC`, else `rustc`.
#[must_use]
pub fn rustc_program() -> std::ffi::OsString {
    std::env::var_os("RUSTC").filter(|v| !v.is_empty()).unwrap_or_else(|| "rustc".into())
}

fn run_in(root: &Path, program: &std::ffi::OsStr, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn `{}`: {e}", program.to_string_lossy())))?;
    if !out.status.success() {
        return Err(Red::new(
            RedKind::ToolFailed,
            format!("`{} {}` exited {:?}", program.to_string_lossy(), args.join(" "), out.status.code()),
        ));
    }
    Ok(normalize_text(&String::from_utf8_lossy(&out.stdout)))
}

/// K for the tree at `root`, computed by this executable: RED [`Stale`](RedKind::Stale) unless the
/// compiled sources are the tree's; RED if a wrapper stands between cargo and rustc.
pub fn instrument_key(root: &Path, cargo: &std::ffi::OsStr) -> Result<Key> {
    for var in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
        if std::env::var_os(var).is_some_and(|v| !v.is_empty()) {
            return Err(Red::new(RedKind::Mismatch, format!("{var} is set: the key would name a compiler that is not the one producing the artifacts")));
        }
    }
    let compiled = compiled();
    let disk = on_disk(&root.join(CRATE_DIR))?;
    check_fresh(&compiled, &disk)?;
    let rustc_vv = run_in(root, &rustc_program(), &["-vV"])?;
    let cargo_v = run_in(root, cargo, &["-V"])?;
    let (source_hex, files) = source_digest(&compiled);
    let hex = key_hex(&compiled, &rustc_vv, &cargo_v);
    Ok(Key { hex, source_hex, files, rustc_vv, cargo_v })
}

/// The compiler program of a cargo `Running` line: its first word, single quotes honoured.
#[must_use]
pub fn running_program(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("Running `")?;
    if let Some(q) = rest.strip_prefix('\'') {
        q.find('\'').map(|end| q[..end].to_owned())
    } else {
        Some(rest.split(' ').next()?.trim_end_matches('`').to_owned())
    }
}

/// RED unless the program of `final_running` (a final unit's `Running` line) prints `-vV` equal to
/// `key.rustc_vv`: the compiler cargo invoked is the one K names.
pub fn check_invoked_rustc(root: &Path, final_running: &str, key: &Key) -> Result<()> {
    let program = running_program(final_running)
        .ok_or_else(|| Red::new(RedKind::Malformed, format!("no compiler program in the final unit's line: {final_running}")))?;
    let got = run_in(root, std::ffi::OsStr::new(&program), &["-vV"])?;
    if got == key.rustc_vv {
        Ok(())
    } else {
        Err(Red::new(
            RedKind::Mismatch,
            format!("cargo invoked `{program}`, whose `-vV` is not the key's rustc:\n{got}\n-- against --\n{}", key.rustc_vv),
        ))
    }
}
