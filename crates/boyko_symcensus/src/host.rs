//! Which toolchain, which target dir, which tree — and the refusals that keep them honest.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::red::{Red, RedKind, Result};

/// The toolchain every nested build and tool resolution in this process uses.
#[derive(Clone, Debug)]
pub struct Host {
    /// `host:` from `rustc -vV`, e.g. `x86_64-pc-windows-msvc`.
    pub triple: String,
    /// `"msvc"`, `"gnu"` or `"other"`, from the triple's last component.
    pub env: &'static str,
    /// The whole `rustc -vV` text, printed into every receipt.
    pub rustc_vv: String,
    /// `rustc --print sysroot`: where the toolchain's own `llvm-tools` live.
    pub sysroot: PathBuf,
}

/// Reads the active toolchain from `rustc -vV` and `rustc --print sysroot`.
///
/// `rustc` is resolved through `PATH` with the inherited `RUSTUP_TOOLCHAIN`, which is the same
/// resolution the nested `cargo` performs, so the tool and the build cannot disagree about the
/// toolchain.
pub fn host() -> Result<Host> {
    let vv = run_text("rustc", &["-vV"])?;
    let triple = vv
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::trim)
        .ok_or_else(|| Red::new(RedKind::Malformed, format!("`rustc -vV` has no `host:` line:\n{vv}")))?
        .to_owned();
    let env = if triple.ends_with("-msvc") {
        "msvc"
    } else if triple.ends_with("-gnu") {
        "gnu"
    } else {
        "other"
    };
    let sysroot = PathBuf::from(run_text("rustc", &["--print", "sysroot"])?.trim());
    Ok(Host { triple, env, rustc_vv: vv.trim().to_owned(), sysroot })
}

/// The workspace root this binary was compiled from.
///
/// The `ug15` executable is built into a SHARED target dir, so an executable built by another
/// worktree could be sitting at the path a recipe invokes. [`check_cwd`] refuses that case.
#[must_use]
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("invariant: the crate lives at <root>/crates/boyko_symcensus")
        .to_path_buf()
}

/// RED unless the current directory is inside the tree this binary was compiled from.
///
/// The recipe is `cd <worktree> && <exe> …`; an executable from another worktree would silently
/// build and census the other tree's sources.
pub fn check_cwd() -> Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Red::io(Path::new("."), &e))?;
    let root = workspace_root();
    let (Ok(cwd_c), Ok(root_c)) = (cwd.canonicalize(), root.canonicalize()) else {
        return Err(Red::new(
            RedKind::Io,
            format!("cannot canonicalise {} or {}", cwd.display(), root.display()),
        ));
    };
    if cwd_c.starts_with(&root_c) {
        Ok(())
    } else {
        Err(Red::new(
            RedKind::Usage,
            format!(
                "this ug15 was compiled from {} but runs in {}. The executable lives in a shared \
                 target dir; rebuild it from the tree you mean to census.",
                root.display(),
                cwd.display()
            ),
        ))
    }
}

/// The name of the host marker file inside the target dir.
pub const HOST_MARKER: &str = ".ug15-host";

/// `$CARGO_TARGET_DIR`, which must be set and absolute; its `.ug15-host` marker must name `host`.
///
/// RED if unset: the fat-LTO trees this crate builds would otherwise land inside the worktree.
/// RED if the marker names another triple: two toolchains in one target dir name the same unit's
/// object differently (msvc `<stem>.o`, gnu `<stem>-<hash>.o`), and a census that cannot say which
/// linker's input it holds is the defect `profile_axis_census.rs` keyed its dirs by host to avoid.
pub fn target_dir(host: &Host) -> Result<PathBuf> {
    let Some(raw) = std::env::var_os("CARGO_TARGET_DIR").filter(|v| !v.is_empty()) else {
        return Err(Red::new(
            RedKind::TargetDir,
            "CARGO_TARGET_DIR is unset. The heavy legs are fat-LTO builds; without an explicit \
             target dir they would be written inside the worktree. Use the recipe's prefix.",
        ));
    };
    let dir = PathBuf::from(raw);
    if !dir.is_absolute() {
        return Err(Red::new(
            RedKind::TargetDir,
            format!("CARGO_TARGET_DIR={} is relative; it would resolve per cwd", dir.display()),
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|e| Red::io(&dir, &e))?;
    let marker = dir.join(HOST_MARKER);
    match std::fs::read_to_string(&marker) {
        Ok(text) => {
            if text.trim() != host.triple {
                return Err(Red::new(
                    RedKind::HostMismatch,
                    format!(
                        "{} names host `{}` but this process runs `{}`. One target dir per host.",
                        marker.display(),
                        text.trim(),
                        host.triple
                    ),
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(&marker, format!("{}\n", host.triple)).map_err(|e| Red::io(&marker, &e))?;
        }
        Err(e) => return Err(Red::io(&marker, &e)),
    }
    Ok(dir)
}

/// `git rev-parse HEAD` plus the count of `git status --porcelain` lines, for receipts.
#[must_use]
pub fn git_provenance(root: &Path) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let head = git(&["rev-parse", "HEAD"]).map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned());
    let dirty = git(&["status", "--porcelain"]).map_or(0, |s| s.lines().count());
    format!("HEAD {head}, {dirty} uncommitted path(s)")
}

/// Runs `program args…` and returns stdout as text; RED on spawn failure or non-zero exit.
pub fn run_text(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn `{program}`: {e}")))?;
    if !out.status.success() {
        return Err(Red::new(
            RedKind::ToolFailed,
            format!(
                "`{program} {}` exited {:?}: {}",
                args.join(" "),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
