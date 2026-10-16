//! Render wave PX0 (`px0/critique.md` W6) — the ONE discovery rule for the external interpreters
//! the root self-tests drive (`golden_diff_selftest`, `golden_tolerance_selftest`).
//!
//! # Why a rule, and not "python3, then python"
//!
//! On a stock Windows workstation `python3.exe` (and often `python.exe`) is the Microsoft Store App
//! Execution Alias stub (`%LOCALAPPDATA%\Microsoft\WindowsApps`): it SPAWNS successfully and exits
//! non-zero with "Python was not found", so a first-hit rule never reaches the real interpreter and
//! the test is red on correct code. And `tools/golden_diff.py` needs Python ≥ 3.11 (`tomllib`),
//! which nothing else checks. So every candidate must answer `--version` with exit 0 AND report
//! `Python 3.N` with `N >= 11`; the first that does is used; none ⇒ a RED naming every candidate
//! and what it answered.
//!
//! PowerShell is spawned as `powershell -NoProfile -ExecutionPolicy Bypass -File …`, so a client
//! machine's `Restricted` default policy cannot refuse the script under test.

#![allow(dead_code)] // each including test uses a subset

use std::path::PathBuf;
use std::process::Command;

/// The smallest Python minor version the tools accept (`tomllib` is 3.11).
pub const MIN_PYTHON_MINOR: u32 = 11;

/// A resolved interpreter: the program and the arguments that select it (`py -3`).
#[derive(Clone, Debug)]
pub struct Interpreter {
    /// The program.
    pub program: String,
    /// Leading arguments.
    pub args: Vec<String>,
    /// What `--version` printed.
    pub version: String,
}

impl Interpreter {
    /// A `Command` for this interpreter, its selecting arguments already applied.
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args);
        c
    }
}

/// The candidates in the order tried: Windows prefers `python`, then the launcher `py -3`, then
/// `python3`; elsewhere `python3`, then `python`.
pub fn python_candidates() -> Vec<(String, Vec<String>)> {
    let c = |p: &str, a: &[&str]| (p.to_owned(), a.iter().map(|s| (*s).to_owned()).collect());
    if cfg!(windows) {
        vec![c("python", &[]), c("py", &["-3"]), c("python3", &[])]
    } else {
        vec![c("python3", &[]), c("python", &[])]
    }
}

/// Parses `Python 3.N[.M]` from a `--version` answer; the minor version, or `None`.
pub fn python_minor(answer: &str) -> Option<u32> {
    let v = answer.trim().strip_prefix("Python 3.")?;
    v.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// The first candidate that answers `--version` with exit 0 and Python ≥ 3.11.
///
/// # Errors
/// Every candidate tried and its answer, when none qualifies.
pub fn find_python() -> Result<Interpreter, String> {
    let mut tried = Vec::new();
    for (program, args) in python_candidates() {
        let out = Command::new(&program).args(&args).arg("--version").output();
        match out {
            Err(e) => tried.push(format!("`{program} {}`: did not spawn ({e})", args.join(" "))),
            Ok(o) => {
                let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&o.stderr));
                let text = text.trim().to_owned();
                match (o.status.success(), python_minor(&text)) {
                    (true, Some(minor)) if minor >= MIN_PYTHON_MINOR => {
                        return Ok(Interpreter { program, args, version: text });
                    }
                    _ => tried.push(format!(
                        "`{program} {}`: exit {:?}, answered {:?}",
                        args.join(" "),
                        o.status.code(),
                        text.lines().next().unwrap_or("")
                    )),
                }
            }
        }
    }
    Err(format!(
        "no Python >= 3.{MIN_PYTHON_MINOR} interpreter qualifies (a Store alias stub answers with a non-zero exit):\n  {}",
        tried.join("\n  ")
    ))
}

/// `powershell -NoProfile -ExecutionPolicy Bypass -File <script>` (Windows PowerShell 5.1, which
/// `scripts/golden.ps1` targets).
pub fn powershell_file(script: &PathBuf) -> Command {
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(script);
    c
}

/// The repository root (the root package's manifest directory).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
