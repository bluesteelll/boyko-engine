//! One subject, one profile, one post-LTO object — built by THIS invocation and paired with the
//! image the linker wrote from it, or RED.
//!
//! # Why the object, not the image
//!
//! `link.exe` writes no COFF symbol table into the image and its PDB carries publics only, so a
//! fat-LTO local is invisible in both (`crates/profile_fixture/tests/profile_axis_census.rs`, its
//! SECOND FINDING). The census therefore reads the linker's INPUT: `cargo rustc … -- --emit=obj`,
//! which rustc unions with the `--emit=dep-info,link` cargo passes, so the image is still linked.
//!
//! # Why every build is forced dirty (B3 critique W4, O2, O7)
//!
//! `--emit=obj` outputs are outside cargo's fingerprint, and on msvc cargo omits the metadata hash
//! from a bin's or an example's filename, so two recipes (this one, and a plain `cargo build
//! --release` by anyone sharing the target dir) write the same `deps/<stem>.exe`. The `[C]`
//! instrument tolerated that residual because each of its legs had a private target dir; this one
//! runs in a shared dir. A unit cargo reports FRESH may therefore sit beside an image a foreign
//! recipe linked, and nothing about the pair tells them apart.
//!
//! So this module never reads a fresh unit. Before the build it bumps the modification time of
//! the target's root source file (content untouched, so `git status` stays clean), which dirties
//! exactly the final unit. After the build it requires, from cargo's own JSON, `fresh: false` for
//! that unit, and from the file system, that the object and the image were both written after the
//! bump. Any other state is RED. This also retires `[C]`'s one-retry deletion of the deps image
//! (O7): nothing inside the target dir is ever deleted here.
//!
//! # Pairing (PC-8)
//!
//! `[C]` identified the unit through the byte-identical uplift of `release/<bin>`. Cargo does not
//! uplift bench executables, and every leg-(2) hot body lives in a bench, so the pairing here goes
//! through cargo's `compiler-artifact.executable` instead: a bench's executable IS its deps image;
//! a bin's or an example's is matched to its deps image by byte identity, as `[C]` did. The object
//! is `<live stem>.o` in the unit's output dir: `deps/` for a bin or a bench, `examples/` for an
//! example (critique O2).

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use crate::host::{self, Host};
use crate::json::{self, Json};
use crate::red::{Red, RedKind, Result};
use crate::sha256;

/// The six crates the modding design names for the source census (03 §6 leg 1); leg (7b) reads the
/// rlibs among them. `boyko_macros` is a proc-macro and links into no binary, so it has no rlib.
pub const CENSUS_CRATES: [&str; 6] =
    ["boyko_ecs", "boyko_utils", "boyko_threadpool", "boyko_log", "boyko_diag", "boyko_macros"];

/// A cargo target kind a subject can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `[[bin]]`.
    Bin,
    /// `examples/*.rs`.
    Example,
    /// `[[bench]]`.
    Bench,
}

impl Kind {
    /// The `cargo rustc` selection flag.
    #[must_use]
    pub const fn flag(self) -> &'static str {
        match self {
            Self::Bin => "--bin",
            Self::Example => "--example",
            Self::Bench => "--bench",
        }
    }

    /// The `target.kind` string cargo prints for it.
    #[must_use]
    pub const fn cargo_kind(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::Example => "example",
            Self::Bench => "bench",
        }
    }
}

/// A binary the gate builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subject {
    /// The short name recipes use (`--subject <key>`).
    pub key: &'static str,
    /// The `[package] name`.
    pub package: &'static str,
    /// The target kind.
    pub kind: Kind,
    /// The target name.
    pub name: &'static str,
}

/// The five subjects of B3 (cut §3): the game-shaped binaries and the three benches that hold the
/// leg-(2) hot bodies. `clear` is the smallest binary that links `EnginePlugins` (PC-1).
pub const SUBJECTS: [Subject; 5] = [
    Subject { key: "boyko_demo", package: "boyko_demo", kind: Kind::Bin, name: "boyko_demo" },
    Subject { key: "clear", package: "boyko-app", kind: Kind::Example, name: "clear" },
    Subject { key: "swap_remove", package: "boyko-ecs", kind: Kind::Bench, name: "swap_remove" },
    Subject { key: "query_dsl", package: "boyko-ecs", kind: Kind::Bench, name: "query_dsl" },
    Subject { key: "phase9_scheduler", package: "boyko-ecs", kind: Kind::Bench, name: "phase9_scheduler" },
];

/// The subject named `key`.
pub fn subject(key: &str) -> Result<Subject> {
    SUBJECTS.iter().copied().find(|s| s.key == key).ok_or_else(|| {
        let keys: Vec<&str> = SUBJECTS.iter().map(|s| s.key).collect();
        Red::new(RedKind::Usage, format!("no subject `{key}`; the subjects are {keys:?}"))
    })
}

/// Where and with what the instrument builds.
#[derive(Clone, Debug)]
pub struct Ctx {
    /// The workspace root (the tree this binary was compiled from).
    pub root: PathBuf,
    /// `$CARGO_TARGET_DIR`, host-marked.
    pub target: PathBuf,
    /// The active toolchain.
    pub host: Host,
}

impl Ctx {
    /// Resolves the tree, the toolchain and the target dir; RED on any refusal of [`host`].
    pub fn new() -> Result<Self> {
        host::check_cwd()?;
        let host = host::host()?;
        let target = host::target_dir(&host)?;
        Ok(Self { root: host::workspace_root(), target, host })
    }
}

/// A deliberate instrument fault, for the instrument's own red controls only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// Every real leg.
    None,
    /// Control O: `--emit=obj` is not passed, so no object is written by this build.
    DropEmitObj,
    /// Control P: the root source is not touched, so a repeated build finds the unit fresh.
    NoTouch,
}

/// One build request.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    /// The subject.
    pub subject: Subject,
    /// The cargo profile name.
    pub profile: &'a str,
    /// Extra rustc arguments for the final unit, after `--emit=obj --print link-args`.
    pub extra_rustc: &'a [String],
    /// [`Control::None`] for every real leg.
    pub control: Control,
}

impl<'a> Request<'a> {
    /// A real-leg request.
    #[must_use]
    pub const fn new(subject: Subject, profile: &'a str, extra_rustc: &'a [String]) -> Self {
        Self { subject, profile, extra_rustc, control: Control::None }
    }
}

/// A paired build.
#[derive(Clone, Debug)]
pub struct Built {
    /// The subject.
    pub subject: Subject,
    /// The profile it was built under.
    pub profile: String,
    /// The image the linker wrote: `deps/<stem>.exe` for a bin, the bench exe, the example exe.
    pub image: PathBuf,
    /// cargo's `compiler-artifact.executable` (for a bin: the uplifted `<profile>/<bin>.exe`).
    pub executable: PathBuf,
    /// The post-LTO object the linker consumed.
    pub object: PathBuf,
    /// The PDB beside the image, if the host writes one.
    pub pdb: Option<PathBuf>,
    /// The `/MAP` file, when `extra_rustc` asked for one.
    pub map: Option<PathBuf>,
    /// `(crate, rlib)` for every census crate cargo reported an rlib for in this build.
    pub rlibs: Vec<(String, PathBuf)>,
    /// The final unit's `--print link-args` output.
    pub link_args: String,
    /// The final unit's rustc command line, from cargo's `-v` output.
    pub final_rustc: String,
    /// sha256 of [`Built::object`].
    pub object_sha256: String,
    /// sha256 of [`Built::image`].
    pub image_sha256: String,
    stamps: Vec<(PathBuf, SystemTime)>,
}

impl Built {
    /// RED if the object or the image changed since the build returned.
    ///
    /// Called by every reader after it has read, because the target dir is shared: a foreign
    /// build that starts after ours exits can rewrite `deps/<stem>.exe` under a reader.
    pub fn check_intact(&self) -> Result<()> {
        for (path, stamp) in &self.stamps {
            if mtime(path) != Some(*stamp) {
                return Err(Red::new(
                    RedKind::PairGuard,
                    format!(
                        "{} changed while this command was reading it. Another recipe shares the \
                         target dir; re-run when it has finished.",
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The receipt block for this build.
    #[must_use]
    pub fn receipt(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("subject      {} ({} {} {})\n", self.subject.key, self.subject.package, self.subject.kind.flag(), self.subject.name));
        s.push_str(&format!("profile      {}\n", self.profile));
        s.push_str(&format!("object       {}\n", self.object.display()));
        s.push_str(&format!("object sha   {}\n", self.object_sha256));
        s.push_str(&format!("image        {}\n", self.image.display()));
        s.push_str(&format!("image sha    {}\n", self.image_sha256));
        if let Some(p) = &self.pdb {
            s.push_str(&format!("pdb          {}\n", p.display()));
        }
        if let Some(m) = &self.map {
            s.push_str(&format!("map          {}\n", m.display()));
        }
        s.push_str(&format!("link args    {}\n", self.link_args.trim()));
        // The final unit's rustc line carries its `--cfg feature=…` set and every codegen flag, so a
        // move caused by feature unification or a flag can be attributed from the receipt (O8).
        s.push_str(&format!("final rustc  {}\n", self.final_rustc.trim()));
        s
    }
}

/// The directory name cargo uses for `profile`.
#[must_use]
pub fn profile_dir(profile: &str) -> &str {
    match profile {
        "dev" | "test" => "debug",
        "release" | "bench" => "release",
        other => other,
    }
}

/// Builds `req.subject` under `req.profile`, forced dirty, and returns the paired artifacts.
pub fn build(ctx: &Ctx, req: &Request<'_>) -> Result<Built> {
    let s = req.subject;
    let src = target_src_path(ctx, s)?;
    let prof_dir = ctx.target.join(profile_dir(req.profile));
    let unit_dir = match s.kind {
        Kind::Example => prof_dir.join("examples"),
        Kind::Bin | Kind::Bench => prof_dir.join("deps"),
    };
    let stem = s.name.replace('-', "_");

    let bumped = if req.control == Control::NoTouch { SystemTime::UNIX_EPOCH } else { touch(&src)? };

    let mut cmd = Command::new(cargo_program());
    cmd.current_dir(&ctx.root)
        .args(["rustc", "-v", "-p", s.package, s.kind.flag(), s.name, "--profile", req.profile])
        .args(["--message-format=json-render-diagnostics", "--"]);
    if req.control != Control::DropEmitObj {
        cmd.arg("--emit=obj");
    }
    cmd.args(["--print", "link-args"]).args(req.extra_rustc);
    cmd.env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("CARGO_TARGET_DIR", &ctx.target)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    eprintln!("ug15: building {} ({} {} {}) under `{}`", s.key, s.package, s.kind.flag(), s.name, req.profile);
    let mut child = cmd
        .spawn()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn cargo: {e}")))?;

    let stderr = child.stderr.take().expect("invariant: stderr was piped");
    let needle = format!("--crate-name {stem} ");
    let echo = std::thread::spawn(move || {
        let mut kept = Vec::new();
        for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
            let t = line.trim_start();
            if t.starts_with("Running `") {
                if line.contains(&needle) {
                    kept.push(line.clone());
                }
                let cut: String = line.chars().take(160).collect();
                eprintln!("{cut}{}", if cut.len() < line.len() { " …" } else { "" });
            } else {
                eprintln!("{line}");
            }
        }
        kept
    });
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("invariant: stdout was piped")
        .read_to_string(&mut stdout)
        .map_err(|e| Red::new(RedKind::Io, format!("reading cargo's stdout: {e}")))?;
    let status = child.wait().map_err(|e| Red::new(RedKind::Io, format!("waiting for cargo: {e}")))?;
    let running = echo.join().unwrap_or_default();

    let mut link_args = String::new();
    let mut artifact: Option<Json> = None;
    let mut finished_ok: Option<bool> = None;
    let mut rlibs: Vec<(String, PathBuf)> = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if !t.starts_with('{') {
            link_args.push_str(t);
            link_args.push('\n');
            continue;
        }
        let msg = json::parse(t)?;
        match msg.get("reason").and_then(Json::as_str) {
            Some("compiler-artifact") => {
                let tname = msg.get("target").and_then(|t| t.get("name")).and_then(Json::as_str).unwrap_or("");
                let kinds: Vec<&str> = msg
                    .get("target")
                    .and_then(|t| t.get("kind"))
                    .map(|k| k.items().iter().filter_map(Json::as_str).collect())
                    .unwrap_or_default();
                if tname == s.name && kinds.contains(&s.kind.cargo_kind()) && package_matches(&msg, s.package) {
                    artifact = Some(msg);
                } else if CENSUS_CRATES.contains(&tname) {
                    for f in msg.get("filenames").map(Json::items).unwrap_or(&[]) {
                        if let Some(p) = f.as_str().filter(|p| p.ends_with(".rlib")) {
                            rlibs.push((tname.to_owned(), PathBuf::from(p)));
                        }
                    }
                }
            }
            Some("build-finished") => finished_ok = msg.get("success").and_then(Json::as_bool),
            _ => {}
        }
    }

    if !status.success() || finished_ok != Some(true) {
        return Err(Red::new(
            RedKind::BuildFailed,
            format!("cargo rustc for {} under `{}` failed (exit {:?}); the leg has no artifact", s.key, req.profile, status.code()),
        ));
    }
    let Some(artifact) = artifact else {
        return Err(Red::new(
            RedKind::BuildFailed,
            format!("cargo's message stream has no compiler-artifact for {} {} {}", s.package, s.kind.flag(), s.name),
        ));
    };
    if artifact.get("fresh").and_then(Json::as_bool) != Some(false) {
        return Err(Red::new(
            RedKind::PairGuard,
            format!(
                "cargo reports {} FRESH although {} was touched before the build, so rustc did not \
                 run and neither the object nor the image can be attributed to this invocation.",
                s.key,
                src.display()
            ),
        ));
    }
    let executable = artifact
        .get("executable")
        .and_then(Json::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| Red::new(RedKind::BuildFailed, format!("the artifact for {} names no executable", s.key)))?;

    let (image, live) = if s.kind == Kind::Bench {
        let live = file_stem(&executable);
        (executable.clone(), live)
    } else {
        live_image(&unit_dir, &stem, &executable)?
    };
    let object = unit_dir.join(format!("{live}.o"));
    if !object.is_file() {
        let others = files_of(&unit_dir, &stem, "o");
        return Err(Red::new(
            RedKind::NoObject,
            format!(
                "no post-LTO object {} after the build (other objects of `{stem}`: {others:?}). The \
                 census reads the LINKER'S INPUT, not the image; without `-- --emit=obj` there is \
                 nothing to count, and a missing subject is a RED, never a zero.",
                object.display()
            ),
        ));
    }
    for (what, path) in [("object", &object), ("image", &image)] {
        let written = mtime(path).is_some_and(|t| t >= bumped);
        if !written {
            return Err(Red::new(
                RedKind::PairGuard,
                format!(
                    "the {what} {} was not written by this build (its modification time precedes the \
                     bump of {}). An object that stood still while its image moved is the \
                     stale-object trap `profile_axis_census.rs` records: the census would count a \
                     file the linker did not consume this time.",
                    path.display(),
                    src.display()
                ),
            ));
        }
    }
    if link_args.trim().is_empty() {
        return Err(Red::new(
            RedKind::Malformed,
            "`--print link-args` printed nothing for a unit that was rebuilt. Empty is never read as \
             `no /OPT:REF` (critique O2).",
        ));
    }
    let final_rustc = running.into_iter().last().unwrap_or_default();
    if isa_baseline_expected(&ctx.host) && !final_rustc.contains("target-cpu=x86-64-v3") {
        return Err(Red::new(
            RedKind::Mismatch,
            format!(
                "the final unit's rustc line does not carry `-C target-cpu=x86-64-v3`, so the \
                 `.cargo/config.toml` ISA baseline did not reach it (a RUSTFLAGS-style override \
                 replaces it). Line: {final_rustc}"
            ),
        ));
    }
    let pdb = Some(unit_dir.join(format!("{live}.pdb"))).filter(|p| p.is_file());
    let map = match map_arg(req.extra_rustc) {
        Some(m) => {
            if !mtime(&m).is_some_and(|t| t >= bumped) {
                return Err(Red::new(RedKind::PairGuard, format!("/MAP file {} was not written by this build", m.display())));
            }
            Some(m)
        }
        None => None,
    };
    let object_sha256 = sha256::file_hex(&object)?;
    let image_sha256 = sha256::file_hex(&image)?;
    let mut stamps = Vec::with_capacity(2);
    for p in [&object, &image] {
        stamps.push((p.clone(), mtime(p).ok_or_else(|| Red::new(RedKind::Io, format!("{} vanished", p.display())))?));
    }
    Ok(Built {
        subject: s,
        profile: req.profile.to_owned(),
        image,
        executable,
        object,
        pdb,
        map,
        rlibs,
        link_args,
        final_rustc,
        object_sha256,
        image_sha256,
        stamps,
    })
}

/// Bumps the modification time of each census crate's `src/lib.rs`, which makes cargo rebuild
/// those crates and every dependent from the front end (P6-1 arm B), without deleting anything.
pub fn touch_census_crate_roots(ctx: &Ctx) -> Result<Vec<PathBuf>> {
    let mut out = Vec::with_capacity(CENSUS_CRATES.len());
    for c in CENSUS_CRATES {
        let p = ctx.root.join("crates").join(c).join("src").join("lib.rs");
        touch(&p)?;
        out.push(p);
    }
    Ok(out)
}

fn cargo_program() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn isa_baseline_expected(host: &Host) -> bool {
    host.triple.starts_with("x86_64-") && (host.triple.contains("windows") || host.triple.contains("linux"))
}

/// Sets `path`'s modification time to now without touching its content; returns the time set.
fn touch(path: &Path) -> Result<SystemTime> {
    let f = std::fs::OpenOptions::new().write(true).open(path).map_err(|e| Red::io(path, &e))?;
    let now = SystemTime::now();
    f.set_modified(now).map_err(|e| Red::io(path, &e))?;
    drop(f);
    // Read back, because the file system's resolution decides what cargo will compare.
    mtime(path).ok_or_else(|| Red::new(RedKind::Io, format!("{} has no modification time", path.display())))
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn file_stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn package_matches(msg: &Json, package: &str) -> bool {
    let id = msg.get("package_id").and_then(Json::as_str).unwrap_or("");
    // `path+file:///…/crates/boyko_ecs#boyko-ecs@0.1.0`, or `…/crates/boyko_demo#0.1.0` when the
    // directory name equals the package name.
    id.contains(&format!("#{package}@")) || id.contains(&format!("/{package}#"))
}

/// `<stem>.<ext>` or `<stem>-<hash>.<ext>` in `dir`. The `-` is load-bearing: a bare prefix match
/// would claim a neighbour such as `<stem>_extra.o`.
fn files_of(dir: &Path, stem: &str, ext: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .filter(|p| {
            let fs = file_stem(p);
            fs == stem || fs.strip_prefix(stem).is_some_and(|r| r.starts_with('-'))
        })
        .collect();
    out.sort();
    out
}

/// The unit-dir image byte-identical to cargo's `executable`, and its stem.
fn live_image(unit_dir: &Path, stem: &str, executable: &Path) -> Result<(PathBuf, String)> {
    let ext = std::env::consts::EXE_EXTENSION;
    let want = std::fs::read(executable).map_err(|e| Red::io(executable, &e))?;
    for cand in files_of(unit_dir, stem, ext) {
        let Ok(meta) = std::fs::metadata(&cand) else { continue };
        if meta.len() != want.len() as u64 {
            continue;
        }
        if std::fs::read(&cand).is_ok_and(|got| got == want) {
            let live = file_stem(&cand);
            return Ok((cand, live));
        }
    }
    Err(Red::new(
        RedKind::PairGuard,
        format!(
            "no image in {} is byte-identical to {}, so nothing names the unit whose object the \
             linker consumed",
            unit_dir.display(),
            executable.display()
        ),
    ))
}

/// The `/MAP:<file>` path among the extra rustc arguments, if any.
fn map_arg(extra: &[String]) -> Option<PathBuf> {
    extra.iter().find_map(|a| {
        let i = a.find("/MAP:")?;
        Some(PathBuf::from(a[i + 5..].trim_matches('"')))
    })
}

/// The root source file of `s`, from `cargo metadata --no-deps`.
pub fn target_src_path(ctx: &Ctx, s: Subject) -> Result<PathBuf> {
    let out = Command::new(cargo_program())
        .current_dir(&ctx.root)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .env("CARGO_TARGET_DIR", &ctx.target)
        .output()
        .map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn cargo metadata: {e}")))?;
    if !out.status.success() {
        return Err(Red::new(RedKind::BuildFailed, format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr))));
    }
    let meta = json::parse(&String::from_utf8_lossy(&out.stdout))?;
    for pkg in meta.get("packages").map(Json::items).unwrap_or(&[]) {
        if pkg.get("name").and_then(Json::as_str) != Some(s.package) {
            continue;
        }
        for t in pkg.get("targets").map(Json::items).unwrap_or(&[]) {
            let kinds: Vec<&str> = t.get("kind").map(|k| k.items().iter().filter_map(Json::as_str).collect()).unwrap_or_default();
            if t.get("name").and_then(Json::as_str) == Some(s.name)
                && kinds.contains(&s.kind.cargo_kind())
                && let Some(p) = t.get("src_path").and_then(Json::as_str)
            {
                return Ok(PathBuf::from(p));
            }
        }
    }
    Err(Red::new(RedKind::Usage, format!("cargo metadata has no target {} {} in package {}", s.kind.flag(), s.name, s.package)))
}
