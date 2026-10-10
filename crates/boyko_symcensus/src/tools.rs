//! Tool resolution: every tool the instrument runs is found, versioned and printed — or RED.
//!
//! **Absent is RED, never a skip** (DG6's rule, the one `profile_axis_census.rs` states for
//! `llvm-nm`). A gate that passes on every machine without its tool is a gate that passes.
//!
//! **Resolution order for the LLVM binutils: rustup's sysroot first, then `PATH`.** The reader of
//! an rlib's bitcode members must be the LLVM that wrote them, and rustc's own `llvm-tools`
//! component is that LLVM; a `PATH` copy of another major version can refuse the bitcode.
//! `BOYKO_UG15_LLVM_BIN` overrides both, and when it is set it is the ONLY place searched: an
//! override that silently fell back would make the tool-absent control (T) unable to go red.
//!
//! **The symbolizer has no fallback.** Rustup's msvc `llvm-tools` ships no `llvm-symbolizer`; the
//! MSVC Build Tools copy reads PDB (00 §11, O7). `BOYKO_UG15_SYMBOLIZER` names one file; unset, the
//! Visual Studio install roots are searched and the highest MSVC toolset version is taken.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::host::Host;
use crate::red::{Red, RedKind, Result};

/// The environment variable naming a directory that REPLACES LLVM binutil resolution.
pub const LLVM_BIN_ENV: &str = "BOYKO_UG15_LLVM_BIN";
/// The environment variable naming the one `llvm-symbolizer` executable to use.
pub const SYMBOLIZER_ENV: &str = "BOYKO_UG15_SYMBOLIZER";

/// The LLVM binutils the legs run, in the order `ug15 tools` prints them.
pub const LLVM_STEMS: [&str; 5] = ["llvm-nm", "llvm-objdump", "llvm-readobj", "llvm-size", "llvm-ar"];

/// A resolved tool.
#[derive(Clone, Debug)]
pub struct Tool {
    /// The tool's stem, e.g. `llvm-nm`.
    pub name: &'static str,
    /// The executable.
    pub path: PathBuf,
    /// The `LLVM version …` line of `--version`.
    pub version: String,
    /// Which rule found it: `override`, `rustup sysroot`, `PATH`, or `msvc build tools`.
    pub source: &'static str,
}

impl Tool {
    /// Runs the tool with `args` and returns its output; RED if it cannot be spawned.
    ///
    /// The exit status is NOT checked here: `llvm-nm` exits 0 on an image with no symbol table,
    /// so each wrapper checks status, stderr and emptiness itself, in that order.
    pub fn output<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        Command::new(&self.path)
            .args(args)
            .output()
            .map_err(|e| Red::new(RedKind::ToolFailed, format!("{} could not be run: {e}", self.path.display())))
    }

    /// One receipt line: name, version, source, path.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        format!("{:<16} {} [{}] {}", self.name, self.version, self.source, self.path.display())
    }
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) { format!("{stem}.exe") } else { stem.to_owned() }
}

/// Resolves the LLVM binutil `stem` (`llvm-nm`, …) for `host`.
pub fn llvm(stem: &'static str, host: &Host) -> Result<Tool> {
    let exe = exe_name(stem);
    if let Some(dir) = std::env::var_os(LLVM_BIN_ENV).filter(|v| !v.is_empty()) {
        let cand = PathBuf::from(&dir).join(&exe);
        if !cand.is_file() {
            return Err(Red::new(
                RedKind::ToolAbsent,
                format!(
                    "{stem}: {LLVM_BIN_ENV}={} is set and holds no {exe}. The override is the only \
                     place searched when set, so this is RED rather than a fallback.",
                    PathBuf::from(dir).display()
                ),
            ));
        }
        return versioned(stem, cand, "override");
    }
    let rustlib = host.sysroot.join("lib").join("rustlib").join(&host.triple).join("bin").join(&exe);
    if rustlib.is_file() {
        return versioned(stem, rustlib, "rustup sysroot");
    }
    if let Some(p) = on_path(&exe) {
        return versioned(stem, p, "PATH");
    }
    Err(Red::new(
        RedKind::ToolAbsent,
        format!(
            "{stem} is in neither {} nor on PATH. Install it with `rustup component add \
             llvm-tools`. Without it the leg cannot tell a missing symbol from an unread table.",
            rustlib.display()
        ),
    ))
}

/// Resolves `llvm-symbolizer`: the override if set, else the newest MSVC Build Tools copy.
pub fn symbolizer() -> Result<Tool> {
    if let Some(file) = std::env::var_os(SYMBOLIZER_ENV).filter(|v| !v.is_empty()) {
        let cand = PathBuf::from(file);
        if !cand.is_file() {
            return Err(Red::new(
                RedKind::ToolAbsent,
                format!(
                    "llvm-symbolizer: {SYMBOLIZER_ENV}={} names no file. No fallback: the \
                     sensitivity map's file map must come from the tool the receipt names.",
                    cand.display()
                ),
            ));
        }
        return versioned("llvm-symbolizer", cand, "override");
    }
    let found = msvc_symbolizers();
    let Some(best) = found.last() else {
        return Err(Red::new(
            RedKind::ToolAbsent,
            "llvm-symbolizer: no `VC/Tools/MSVC/<ver>/bin/Hostx64/x64/llvm-symbolizer.exe` under any \
             Visual Studio install root, and BOYKO_UG15_SYMBOLIZER is unset. Rustup's llvm-tools \
             has no symbolizer (00 §10, O7). RED, not a skip.",
        ));
    };
    versioned("llvm-symbolizer", best.1.clone(), "msvc build tools")
}

/// Every MSVC Build Tools `llvm-symbolizer.exe` (Hostx64/x64), sorted by toolset version ascending.
#[must_use]
pub fn msvc_symbolizers() -> Vec<(Vec<u32>, PathBuf)> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(v) = std::env::var_os(var) {
            roots.push(PathBuf::from(v).join("Microsoft Visual Studio"));
        }
    }
    for fixed in ["C:/Program Files (x86)/Microsoft Visual Studio", "C:/Program Files/Microsoft Visual Studio"] {
        let p = PathBuf::from(fixed);
        if !roots.iter().any(|r| same_dir(r, &p)) {
            roots.push(p);
        }
    }
    let mut out: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    for root in roots {
        for year in read_dirs(&root) {
            for edition in read_dirs(&year) {
                for ver in read_dirs(&edition.join("VC").join("Tools").join("MSVC")) {
                    let exe = ver.join("bin").join("Hostx64").join("x64").join("llvm-symbolizer.exe");
                    if exe.is_file() {
                        let key = ver
                            .file_name()
                            .map(|n| n.to_string_lossy().split('.').filter_map(|p| p.parse().ok()).collect())
                            .unwrap_or_default();
                        if !out.iter().any(|(_, p)| same_dir(p, &exe)) {
                            out.push((key, exe));
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Every tool the heavy legs run, resolved; the first absence is the RED.
pub fn all(host: &Host) -> Result<Vec<Tool>> {
    let mut tools = Vec::with_capacity(LLVM_STEMS.len() + 1);
    for stem in LLVM_STEMS {
        tools.push(llvm(stem, host)?);
    }
    tools.push(symbolizer()?);
    Ok(tools)
}

fn versioned(name: &'static str, path: PathBuf, source: &'static str) -> Result<Tool> {
    let out = Command::new(&path)
        .arg("--version")
        .output()
        .map_err(|e| Red::new(RedKind::ToolFailed, format!("{} could not be run: {e}", path.display())))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(line) = text.lines().find(|l| l.contains("LLVM version")) else {
        return Err(Red::new(
            RedKind::ToolFailed,
            format!(
                "{} --version (exit {:?}) printed no `LLVM version` line; a tool whose identity cannot be \
                 printed into the receipt is not one this gate may run. stdout:\n{text}",
                path.display(),
                out.status.code()
            ),
        ));
    };
    Ok(Tool { name, path, version: line.trim().to_owned(), source })
}

fn on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe)).find(|c| c.is_file())
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    v.sort();
    v
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}
