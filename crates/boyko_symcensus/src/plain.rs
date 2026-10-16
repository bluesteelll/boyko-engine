//! V11 (c), the bridge to a plain shipped build (IB design Rev 5 O9, Rev 6 W12 (i)).
//!
//! After a retained ug15 build of a timed image, the same package and features are built the way a
//! game ships: `cargo build --release --locked -p <pkg> --bin <bin> [--features …]
//! --message-format=json`, in the same target dir, RUSTFLAGS removed, after a touch of the final
//! unit's root source. Then:
//!
//! * **(c1) freshness.** Every non-final artifact cargo reports — rlibs, proc-macro dlls, build-script
//!   executables — is `fresh: true`; the plain build's library set {(crate, path)} equals the ug15
//!   build's member for member; each of those files still hashes to what the ug15 build recorded; the
//!   final unit is `fresh: false`. So cargo itself certifies that the plain build linked the very
//!   files ug15's did, and that the plain image is not ug15's uplifted.
//! * **(c2) content.** The `.text` raw bytes of the two RETAINED images are equal (sha256 over
//!   `PointerToRawData..+SizeOfRawData`). Neither is read from `deps/` or `release/`: the plain build
//!   has just rewritten both.
//!
//! RC-14 (a) and (b) are the red controls: an extra codegen argument on the ug15 side reds (c2) only,
//! and a touched `boyko_input` reds (c1) with exactly its dependents non-fresh.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::json::{self, Json};
use crate::objbuild::{self, Ctx, Subject};
use crate::pe;
use crate::red::{Red, RedKind, Result};
use crate::retain::{RetainDir, Retained};
use crate::sha256;

/// One artifact of the plain build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlainArtifact {
    /// `target.name`.
    pub crate_name: String,
    /// cargo's package id.
    pub package_id: String,
    /// `target.kind`, comma-joined.
    pub kind: String,
    /// The rlib or dll, for a library unit.
    pub lib: Option<PathBuf>,
    /// cargo's `fresh`.
    pub fresh: bool,
}

/// The plain build, retained as `plain-<tag>`.
#[derive(Clone, Debug)]
pub struct Plain {
    /// `plain-<tag>`.
    pub tag: String,
    /// The retained image.
    pub image: PathBuf,
    /// Its sha256.
    pub image_sha256: String,
    /// Every non-final artifact.
    pub artifacts: Vec<PlainArtifact>,
    /// The final unit's `fresh`.
    pub final_fresh: bool,
}

/// A leg's outcome: green, or red with what was measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Green.
    pub green: bool,
    /// One line per finding (empty when green, except the summary).
    pub lines: Vec<String>,
}

impl Verdict {
    /// Green with a summary.
    #[must_use]
    pub fn green(summary: String) -> Self {
        Self { green: true, lines: vec![summary] }
    }
    /// Red with findings.
    #[must_use]
    pub fn red(lines: Vec<String>) -> Self {
        Self { green: false, lines }
    }
}

/// Runs the plain build of `subject` with `features` and retains it as `plain-<reference.tag>`.
pub fn plain_build(ctx: &Ctx, subject: Subject, features: &[String], retain: &RetainDir, reference: &Retained) -> Result<Plain> {
    let tag = format!("plain-{}", reference.tag);
    let dir = retain.tag_dir(&tag)?;
    let src = objbuild::target_src_path(ctx, subject)?;
    objbuild::touch(&src)?;
    let mut cmd = Command::new(objbuild::cargo_program());
    cmd.current_dir(&ctx.root)
        .args(["build", "--release", "--locked", "-p", subject.package, subject.kind.flag(), subject.name, "--message-format=json"]);
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    cmd.env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("CARGO_TARGET_DIR", &ctx.target)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    (ctx.echo)(&format!("ug15: plain build {} ({} {} {}) under `release` -> {tag}", subject.key, subject.package, subject.kind.flag(), subject.name));
    let mut child = cmd.spawn().map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn cargo: {e}")))?;
    let stderr = child.stderr.take().expect("invariant: stderr was piped");
    let sink = ctx.echo;
    let echo = std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
            sink(&line);
        }
    });
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("invariant: stdout was piped")
        .read_to_string(&mut stdout)
        .map_err(|e| Red::new(RedKind::Io, format!("reading cargo's stdout: {e}")))?;
    let status = child.wait().map_err(|e| Red::new(RedKind::Io, format!("waiting for cargo: {e}")))?;
    let _ = echo.join();
    if !status.success() {
        return Err(Red::new(RedKind::BuildFailed, format!("the plain build of {} failed (exit {:?})", subject.key, status.code())));
    }
    let mut artifacts = Vec::new();
    let mut final_fresh = None;
    let mut executable = None;
    for line in stdout.lines().map(str::trim).filter(|l| l.starts_with('{')) {
        let msg = json::parse(line)?;
        if msg.get("reason").and_then(Json::as_str) != Some("compiler-artifact") {
            continue;
        }
        let target = msg.get("target");
        let name = target.and_then(|t| t.get("name")).and_then(Json::as_str).unwrap_or("").to_owned();
        let kinds: Vec<&str> = target.and_then(|t| t.get("kind")).map(|k| k.items().iter().filter_map(Json::as_str).collect()).unwrap_or_default();
        let fresh = msg.get("fresh").and_then(Json::as_bool).ok_or_else(|| Red::new(RedKind::Malformed, format!("artifact {name} has no `fresh`")))?;
        let package_id = msg.get("package_id").and_then(Json::as_str).unwrap_or("").to_owned();
        if name == subject.name && kinds.contains(&subject.kind.cargo_kind()) {
            final_fresh = Some(fresh);
            executable = msg.get("executable").and_then(Json::as_str).map(PathBuf::from);
            continue;
        }
        let lib = msg
            .get("filenames")
            .map(Json::items)
            .unwrap_or(&[])
            .iter()
            .filter_map(Json::as_str)
            .find(|p| p.ends_with(".rlib") || (kinds.contains(&"proc-macro") && p.ends_with(std::env::consts::DLL_SUFFIX)))
            .map(PathBuf::from);
        artifacts.push(PlainArtifact { crate_name: name, package_id, kind: kinds.join(","), lib, fresh });
    }
    let final_fresh = final_fresh.ok_or_else(|| Red::new(RedKind::BuildFailed, format!("the plain build reported no artifact for {}", subject.key)))?;
    let executable = executable.ok_or_else(|| Red::new(RedKind::BuildFailed, format!("the plain build's {} names no executable", subject.key)))?;
    let name = executable.file_name().ok_or_else(|| Red::new(RedKind::Io, "the plain executable has no file name"))?;
    let image = dir.join(name);
    let before = sha256::file_hex(&executable)?;
    std::fs::copy(&executable, &image).map_err(|e| Red::io(&executable, &e))?;
    let image_sha256 = sha256::file_hex(&image)?;
    if image_sha256 != before {
        return Err(Red::new(RedKind::PairGuard, format!("{tag}: the retained plain image does not hash to the built one")));
    }
    Ok(Plain { tag, image, image_sha256, artifacts, final_fresh })
}

/// (c1): the plain build is fresh over the ug15 build's whole library set, at the same bytes.
pub fn check_c1(plain: &Plain, reference: &Retained) -> Result<Verdict> {
    let mut lines = Vec::new();
    for a in plain.artifacts.iter().filter(|a| !a.fresh) {
        lines.push(format!("(c1) non-fresh {} ({}, {})", a.crate_name, a.kind, a.package_id));
    }
    if plain.final_fresh {
        lines.push("(c1) the final unit is FRESH although its root source was touched: the plain image would be ug15's, uplifted".to_owned());
    }
    let plain_libs: BTreeSet<(String, PathBuf)> = plain.artifacts.iter().filter_map(|a| a.lib.clone().map(|p| (a.crate_name.clone(), p))).collect();
    let ug15_libs: BTreeSet<(String, PathBuf)> = reference.libs.iter().map(|l| (l.crate_name.clone(), l.path.clone())).collect();
    for (c, p) in plain_libs.difference(&ug15_libs) {
        lines.push(format!("(c1) plain only: {c} {}", p.display()));
    }
    for (c, p) in ug15_libs.difference(&plain_libs) {
        lines.push(format!("(c1) ug15 only: {c} {}", p.display()));
    }
    for l in &reference.libs {
        let now = sha256::file_hex(&l.path)?;
        if now != l.sha256 {
            lines.push(format!("(c1) {} {} now hashes {now}, recorded {}", l.crate_name, l.path.display(), l.sha256));
        }
    }
    Ok(if lines.is_empty() {
        Verdict::green(format!(
            "(c1) GREEN: {} non-final artifact(s) fresh, the library set equals ug15's ({} member(s)) and re-hashes equal, the final unit rebuilt",
            plain.artifacts.len(),
            ug15_libs.len()
        ))
    } else {
        Verdict::red(lines)
    })
}

/// The non-fresh artifacts of the plain build, by package id (RC-14 (b)'s set).
#[must_use]
pub fn non_fresh(plain: &Plain) -> BTreeSet<String> {
    plain.artifacts.iter().filter(|a| !a.fresh).map(|a| a.package_id.clone()).collect()
}

/// sha256 of an image's `.text` raw bytes, and the parsed headers.
pub fn text_sha256(image: &Path) -> Result<(String, pe::PeHeaders, Vec<u8>)> {
    let bytes = std::fs::read(image).map_err(|e| Red::io(image, &e))?;
    let h = pe::headers(&bytes).ok_or_else(|| Red::new(RedKind::Malformed, format!("{} is not a PE image", image.display())))?;
    let s = h
        .sections
        .iter()
        .find(|s| s.name == ".text")
        .ok_or_else(|| Red::new(RedKind::Malformed, format!("{} has no .text section", image.display())))?;
    let (a, n) = (s.raw_ptr as usize, s.raw_size as usize);
    let text = bytes.get(a..a + n).ok_or_else(|| Red::new(RedKind::Malformed, format!("{}: .text runs past the file", image.display())))?;
    if text.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, format!("{}: an empty .text", image.display())));
    }
    Ok((sha256::bytes_hex(text), h, bytes))
}

/// (c2): the two retained images' `.text` raw bytes are equal.
pub fn check_c2(ug15_image: &Path, plain_image: &Path) -> Result<Verdict> {
    let (a, ha, ba) = text_sha256(ug15_image)?;
    let (b, _, bb) = text_sha256(plain_image)?;
    if a == b {
        return Ok(Verdict::green(format!("(c2) GREEN: .text sha256 {a} in both retained images")));
    }
    let mut lines = vec![format!("(c2) .text differs: ug15 {a}, plain {b}")];
    match pe::diff_ranges(&ba, &bb, 8) {
        Some(ranges) => {
            for (s, e) in ranges {
                lines.push(format!("(c2)   bytes 0x{s:x}..0x{e:x}: {}", pe::locate(&ha, s)));
            }
        }
        None => lines.push(format!("(c2)   the images differ in length ({} vs {} B)", ba.len(), bb.len())),
    }
    Ok(Verdict::red(lines))
}
