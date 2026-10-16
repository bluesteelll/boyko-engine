//! Retention (IB design Rev 5 W11, Rev 6 "Retention layout"): every IB build is copied out of the
//! shared target dir, hashed and checked BEFORE the next build starts.
//!
//! On msvc a bin's image, object and pdb are `deps/<stem>.{exe,o,pdb}` with no hash in the name
//! (`objbuild.rs`'s module docs), so the next build of the same stem — another feature set, a plain
//! `cargo build --release` — overwrites them. A reader that opened `deps/` after that would read
//! another invocation's bytes and attribute them to this one. So `ug15 unit`, `unit-controls` and
//! the scratch arms read only the copies in `<retain>/<tag>/`; the `/MAP` file is written straight
//! there by the linker and never has a shared path.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::key::{self, Key};
use crate::objbuild::{self, ArtifactKind, Built, Ctx, Request};
use crate::red::{Red, RedKind, Result};
use crate::sha256;

/// A retain directory: absolute, outside the tree and the target dir, empty or absent at start.
#[derive(Clone, Debug)]
pub struct RetainDir {
    root: PathBuf,
}

/// Lexical `child` ⊆ `parent`, separator- and (on Windows) case-insensitive, component-wise.
#[must_use]
pub fn is_within(child: &Path, parent: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        let s = s.trim_end_matches('/').to_owned();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    let (c, p) = (norm(child), norm(parent));
    c == p || c.strip_prefix(&p).is_some_and(|rest| rest.starts_with('/'))
}

impl RetainDir {
    /// Opens `path` as a retain directory for a run over the tree `tree` and the target dir
    /// `target`. RED unless `path` is absolute, outside both, and empty or absent (it is created).
    ///
    /// Outside the tree because the IB bracket needs clean trees; outside the target dir because
    /// `cargo clean` deletes it; empty because a tag already present would be read as this run's.
    pub fn open(path: &Path, tree: &Path, target: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Red::new(RedKind::Usage, format!("--retain {} is relative; it would resolve per cwd", path.display())));
        }
        for (what, base) in [("the tree", tree), ("the target dir", target)] {
            if is_within(path, base) || is_within(&canonical_or_self(path), &canonical_or_self(base)) {
                return Err(Red::new(RedKind::Usage, format!("--retain {} is inside {what} ({})", path.display(), base.display())));
            }
        }
        if path.exists() {
            let mut rd = std::fs::read_dir(path).map_err(|e| Red::io(path, &e))?;
            if rd.next().is_some() {
                return Err(Red::new(
                    RedKind::Usage,
                    format!("--retain {} is not empty; a tag already there would be read as this run's", path.display()),
                ));
            }
        } else {
            std::fs::create_dir_all(path).map_err(|e| Red::io(path, &e))?;
        }
        Ok(Self { root: path.to_path_buf() })
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Creates `<retain>/<tag>/`; RED on a tag outside the vocabulary or already present.
    pub fn tag_dir(&self, tag: &str) -> Result<PathBuf> {
        check_tag(tag)?;
        let d = self.root.join(tag);
        if d.exists() {
            return Err(Red::new(RedKind::Usage, format!("retained tag `{tag}` is already present in {}", self.root.display())));
        }
        std::fs::create_dir_all(&d).map_err(|e| Red::io(&d, &e))?;
        Ok(d)
    }
}

fn canonical_or_self(p: &Path) -> PathBuf {
    // The nearest existing ancestor, canonicalised, with the missing tail re-appended: `path` may
    // not exist yet, and a junction or a short name must not smuggle it into the tree.
    let mut tail = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for t in tail.iter().rev() {
                out.push(t);
            }
            return out;
        }
        match (cur.file_name().map(std::ffi::OsStr::to_os_string), cur.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                tail.push(name);
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

fn is_word(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn is_base_tag(t: &str) -> bool {
    match t {
        "runner" | "probe" | "probe-wide" => true,
        _ => t.strip_prefix("runner-").or_else(|| t.strip_prefix("probe-")).is_some_and(is_word),
    }
}

fn is_rc_tag(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() > 6
        && t.starts_with("rc")
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4].is_ascii_lowercase()
        && b[5] == b'-'
        && is_base_tag(&t[6..])
}

/// RED unless `tag` is in the closed vocabulary: `runner`, `runner-<pin>`, `probe`, `probe-<pin>`,
/// `probe-wide`; `rc<NN><x>-<tag>` for controls; `plain-<tag>` for V11 (c).
pub fn check_tag(tag: &str) -> Result<()> {
    let core = tag.strip_prefix("plain-").unwrap_or(tag);
    if is_base_tag(core) || is_rc_tag(core) {
        Ok(())
    } else {
        Err(Red::new(
            RedKind::Usage,
            format!(
                "retained tag `{tag}` is outside the vocabulary (runner, runner-<pin>, probe, probe-<pin>, probe-wide, \
                 rc<NN><x>-<tag>, plain-<tag>)"
            ),
        ))
    }
}

/// One library unit of a retained build: an rlib, or a proc-macro's dll, hashed right after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibRow {
    /// The crate name (`target.name`).
    pub crate_name: String,
    /// cargo's package id.
    pub package_id: String,
    /// `lib` or `proc-macro`.
    pub kind: ArtifactKind,
    /// Built for the host (build scripts, proc macros and their dependencies).
    pub host: bool,
    /// The file.
    pub path: PathBuf,
    /// Its sha256, taken right after the build.
    pub sha256: String,
}

/// A retained build.
#[derive(Debug)]
pub struct Retained {
    /// Its tag.
    pub tag: String,
    /// `<retain>/<tag>/`.
    pub dir: PathBuf,
    /// The build itself (its paths point into the shared target dir: never read them again).
    pub built: Built,
    /// The retained image.
    pub image: PathBuf,
    /// The retained object.
    pub object: PathBuf,
    /// The retained pdb, if the host wrote one.
    pub pdb: Option<PathBuf>,
    /// The map, written by the linker into `dir`.
    pub map: PathBuf,
    /// sha256 of the retained image (= the build's).
    pub image_sha256: String,
    /// sha256 of the retained object (= the build's).
    pub object_sha256: String,
    /// sha256 of the map.
    pub map_sha256: String,
    /// sha256 of the retained pdb.
    pub pdb_sha256: Option<String>,
    /// Every library unit of the build, hashed.
    pub libs: Vec<LibRow>,
}

/// Classifies a build artifact as host or target: cargo builds host units (build scripts, proc
/// macros, their dependencies) under the build-override profile, whose default `opt-level` is `0`,
/// while the requested profile's is `3`. RED on any other pair: a root manifest that overrode
/// either would shift the split silently ([`check_profile_premise`] reds on it first).
pub fn is_host(opt_level: &str, final_opt_level: &str) -> Result<bool> {
    match (opt_level, final_opt_level) {
        ("0", "3") => Ok(true),
        ("3", "3") => Ok(false),
        (a, f) => Err(Red::new(
            RedKind::Malformed,
            format!("an artifact at opt-level `{a}` under a final unit at `{f}`: the host/target split is read from (0, 3) only"),
        )),
    }
}

/// RED if the root manifest has a `[profile.*.build-override]` or a `[profile.*.package.*]` table:
/// either moves a unit's `opt-level` and with it the host/target split of [`is_host`] (W14, O8).
pub fn check_profile_premise(root_manifest_text: &str) -> Result<()> {
    for line in root_manifest_text.lines() {
        let t = line.trim();
        if t.starts_with("[profile.") && (t.contains("build-override") || t.contains(".package.")) {
            return Err(Red::new(
                RedKind::Malformed,
                format!("the root manifest has `{t}`, which moves units' opt-level; the host/target split cannot be read"),
            ));
        }
    }
    Ok(())
}

/// The value of `key` in a comment-stripped manifest line `key = value`, quotes removed.
fn manifest_value<'t>(line: &'t str, key: &str) -> Option<&'t str> {
    let (k, v) = line.split_once('=')?;
    (k.trim() == key).then(|| v.trim().trim_matches('"'))
}

/// The codegen-units an IB build of `profile` passes to its final unit (orchestrator ruling 57).
/// This is the profile's own `codegen-units`, or the nearest one up its `inherits` chain
/// (`bench` → `release`, `test`/`doc` → `dev` when not stated). If none is set, it is rustc's
/// non-incremental default, [`objbuild::RUSTC_NON_INCREMENTAL_CODEGEN_UNITS`].
///
/// Only the root manifest's `[profile.<name>]` tables are read. `objbuild::build` checks the
/// result against the final unit's rustc line, so a value from anywhere else is RED there, never
/// silently overridden ([`objbuild::check_final_codegen_units`]). RED [`Malformed`](RedKind::Malformed)
/// on a value that is not a positive integer, on a custom profile without `inherits`, or on an
/// `inherits` cycle.
pub fn profile_codegen_units(root_manifest_text: &str, profile: &str) -> Result<objbuild::CodegenUnits> {
    let mut tables: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in root_manifest_text.lines() {
        let t = line.split('#').next().unwrap_or("").trim();
        if t.starts_with('[') {
            current = t.strip_prefix("[profile.").and_then(|r| r.strip_suffix(']')).filter(|n| !n.contains(['.', '[', ']'])).map(|n| {
                tables.push((n.trim().to_owned(), None, None));
                tables.len() - 1
            });
            continue;
        }
        let Some(i) = current else { continue };
        if let Some(v) = manifest_value(t, "codegen-units") {
            tables[i].1 = Some(v.to_owned());
        } else if let Some(v) = manifest_value(t, "inherits") {
            tables[i].2 = Some(v.to_owned());
        }
    }
    let mut name = profile.to_owned();
    let mut seen: Vec<String> = Vec::new();
    loop {
        if seen.contains(&name) {
            return Err(Red::new(RedKind::Malformed, format!("the profiles' `inherits` chain from `{profile}` loops at `{name}`")));
        }
        seen.push(name.clone());
        let table = tables.iter().find(|(n, _, _)| *n == name);
        if let Some(v) = table.and_then(|(_, c, _)| c.as_deref()) {
            let n = v.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| {
                Red::new(RedKind::Malformed, format!("[profile.{name}] codegen-units = {v} is not a positive integer"))
            })?;
            return Ok(objbuild::CodegenUnits { n, set_by_profile: true });
        }
        let parent = match (table.and_then(|(_, _, p)| p.clone()), name.as_str()) {
            (Some(p), _) => Some(p),
            (None, "bench") => Some("release".to_owned()),
            (None, "test" | "doc") => Some("dev".to_owned()),
            (None, "release" | "dev") => None,
            (None, other) => {
                return Err(Red::new(RedKind::Malformed, format!("[profile.{other}] has no `inherits` (from `{profile}`); its codegen-units cannot be read")));
            }
        };
        match parent {
            Some(p) => name = p,
            None => return Ok(objbuild::CodegenUnits { n: objbuild::RUSTC_NON_INCREMENTAL_CODEGEN_UNITS, set_by_profile: false }),
        }
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn copy_into(src: &Path, dir: &Path) -> Result<PathBuf> {
    let name = src.file_name().ok_or_else(|| Red::new(RedKind::Io, format!("{} has no file name", src.display())))?;
    let dst = dir.join(name);
    std::fs::copy(src, &dst).map_err(|e| Red::io(src, &e))?;
    Ok(dst)
}

/// Builds `req` through [`objbuild::build`] with the `/MAP` written into `<retain>/<tag>/`, copies
/// the image, object and pdb there, hashes every copy against the build's own hash and every
/// library unit in place, and only then runs [`Built::check_intact`] (plus the pdb's and the map's
/// stamps), so nothing that was read can have been rewritten by a foreign build in between.
///
/// `ib_key`: the final unit's compiler must be the one the key names ([`key::check_invoked_rustc`]).
///
/// The final unit is compiled at the profile's effective codegen-units ([`profile_codegen_units`],
/// orchestrator ruling 57). Without it, `--emit=obj` would make it a codegen-units = 1 build and not
/// the shipped one. The value is the profile's, never the caller's: a request that already names
/// one is RED.
pub fn build_retained(ctx: &Ctx, req: &Request<'_>, retain: &RetainDir, tag: &str, ib_key: &Key) -> Result<Retained> {
    if ctx.host.env != "msvc" {
        return Err(Red::new(RedKind::Usage, "IB builds are Windows (msvc) only: the `/MAP` link argument and every IB claim are"));
    }
    if req.codegen_units.is_some() {
        return Err(Red::new(RedKind::Usage, "an IB build's codegen-units is the profile's effective value (ruling 57); a request does not choose it"));
    }
    let manifest = ctx.root.join("Cargo.toml");
    let cgu = profile_codegen_units(&std::fs::read_to_string(&manifest).map_err(|e| Red::io(&manifest, &e))?, req.profile)?;
    let dir = retain.tag_dir(tag)?;
    let map = dir.join(format!("{tag}.map"));
    let mut extra: Vec<String> = req.extra_rustc.to_vec();
    extra.push("-C".to_owned());
    extra.push(format!("link-arg=/MAP:{}", map.display()));
    let r = Request {
        subject: req.subject,
        profile: req.profile,
        extra_rustc: &extra,
        codegen_units: Some(cgu),
        features: req.features,
        env: req.env,
        locked: true,
        control: req.control,
    };
    let built = objbuild::build(ctx, &r)?;
    key::check_invoked_rustc(&ctx.root, &built.final_rustc, ib_key)?;
    let final_opt = built.final_opt_level.clone();
    let mut libs = Vec::new();
    for a in &built.artifacts {
        if !matches!(a.kind, ArtifactKind::Lib | ArtifactKind::ProcMacro) {
            continue;
        }
        let Some(path) = &a.path else {
            return Err(Red::new(RedKind::Malformed, format!("library unit {} ({}) names no rlib or dll", a.crate_name, a.package_id)));
        };
        libs.push(LibRow {
            crate_name: a.crate_name.clone(),
            package_id: a.package_id.clone(),
            kind: a.kind,
            host: is_host(&a.opt_level, &final_opt)?,
            path: path.clone(),
            sha256: sha256::file_hex(path)?,
        });
    }
    if libs.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, format!("{tag}: the build reported no library unit")));
    }
    let pdb_stamp = built.pdb.as_ref().map(|p| (p.clone(), mtime(p)));
    let map_stamp = mtime(&map).ok_or_else(|| Red::new(RedKind::PairGuard, format!("{tag}: the linker wrote no map at {}", map.display())))?;
    let image = copy_into(&built.image, &dir)?;
    let object = copy_into(&built.object, &dir)?;
    let image_sha256 = sha256::file_hex(&image)?;
    let object_sha256 = sha256::file_hex(&object)?;
    if image_sha256 != built.image_sha256 || object_sha256 != built.object_sha256 {
        return Err(Red::new(
            RedKind::PairGuard,
            format!("{tag}: a retained copy does not hash to its build's (image {image_sha256} vs {}, object {object_sha256} vs {})", built.image_sha256, built.object_sha256),
        ));
    }
    let (pdb, pdb_sha256) = match &built.pdb {
        Some(p) => {
            let src_sha = sha256::file_hex(p)?;
            let copy = copy_into(p, &dir)?;
            let copy_sha = sha256::file_hex(&copy)?;
            if copy_sha != src_sha {
                return Err(Red::new(RedKind::PairGuard, format!("{tag}: the retained pdb does not hash to the build's")));
            }
            (Some(copy), Some(copy_sha))
        }
        None => (None, None),
    };
    let map_sha256 = sha256::file_hex(&map)?;
    built.check_intact()?;
    if let Some((p, stamp)) = &pdb_stamp
        && mtime(p) != *stamp
    {
        return Err(Red::new(RedKind::PairGuard, format!("{} changed while {tag} was retained", p.display())));
    }
    if mtime(&map) != Some(map_stamp) {
        return Err(Red::new(RedKind::PairGuard, format!("{} changed while {tag} was retained", map.display())));
    }
    let retained = Retained {
        tag: tag.to_owned(),
        dir: dir.clone(),
        built,
        image,
        object,
        pdb,
        map,
        image_sha256,
        object_sha256,
        map_sha256,
        pdb_sha256,
        libs,
    };
    let receipt = dir.join("build.txt");
    std::fs::write(&receipt, retained.receipt()).map_err(|e| Red::io(&receipt, &e))?;
    Ok(retained)
}

impl Retained {
    /// The tag's own receipt (`<retain>/<tag>/build.txt`).
    #[must_use]
    pub fn receipt(&self) -> String {
        let mut s = self.built.receipt();
        s.push_str(&format!("retained     {} image {}\n", self.tag, self.image_sha256));
        s.push_str(&format!("retained     {} object {}\n", self.tag, self.object_sha256));
        s.push_str(&format!("retained     {} map {}\n", self.tag, self.map_sha256));
        if let Some(p) = &self.pdb_sha256 {
            s.push_str(&format!("retained     {} pdb {p}\n", self.tag));
        }
        for l in &self.libs {
            s.push_str(&format!(
                "lib          {}\t{}\t{}\t{}\t{}\t{}\n",
                l.crate_name,
                l.package_id,
                l.kind.label(),
                if l.host { "host" } else { "target" },
                l.path.display(),
                l.sha256
            ));
        }
        s
    }
}
