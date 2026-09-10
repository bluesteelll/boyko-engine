//! **G14 and G16 — the cross-profile symbol census**, run over artifacts this file builds itself.
//!
//! # What it measures
//!
//! One source, two `BOYKO_PROFILE` legs, one symbol question per fixture:
//!
//! | binary | site's tier / level | `dev` | `shipping` |
//! |---|---|---|---|
//! | `deep_zone` | `ZoneTier::Deep` | present | **absent** |
//! | `always_zone` | `ZoneTier::Always` | present | present |
//! | `profile-fixture-log` | `Level::Debug` | present | **absent** |
//!
//! The profiling half is a **2×2, not the 1×2 the corpus specifies**, and the extra row is what
//! makes the interesting cell mean anything. A `shipping` binary with no emission symbol is
//! ambiguous on its own: it is equally consistent with "the tier fold deleted this site" and with
//! "a shipping build contains no profiler at all". `always_zone` — the same fixture one tier down,
//! nothing else changed — answers that, and it must be **present** in `shipping`. Exactly one of
//! the four cells is zero, and it is the one under test.
//!
//! # THE FINDING: without LTO this census is INERT, and `--gc-sections` does not fix it
//!
//! MEASURED on this box, `x86_64-pc-windows-gnu`, release, over `deep_zone` (over the linked image,
//! which is what this file censused at the time; the SECOND FINDING below moved the subject to the
//! post-LTO object and did not disturb any cell of this table):
//!
//! | link configuration | `dev` | `shipping` | decidable? |
//! |---|---|---|---|
//! | default release | `mint_cold` = 1 | `mint_cold` = **1** | no |
//! | `-C link-arg=-Wl,--gc-sections` | 1 | **1** | no — *no effect whatsoever* |
//! | `lto = "fat"`, `codegen-units = 1` | 1 | **0** | yes |
//!
//! The reason is visible in the same output: the default-release `deep_zone` image contains
//! `core::ptr::drop_glue::<boyko_diag::telemetry::Block>`, in a binary whose source never mentions
//! telemetry. The whole `boyko_diag` rlib is carried into the image and nothing collects it, so a
//! whole-image census answers *"was this symbol codegen'd into some rlib on the way here?"* rather
//! than *"can this program reach it?"*. Those are different questions and only the second is G14's.
//!
//! The corpus anticipated an instrument failure here and predicted the wrong side of it: it warned
//! that `open`/`record` might **inline away in the `dev` leg**, leaving the census with no subject,
//! and required the `dev` leg as the control that would catch it. The control was right and the
//! prediction was backwards — the subject survives in the `shipping` leg instead, and the `dev` leg
//! would have looked perfect while the gate could not fail. Two REDs of the same shape had to be
//! run before this file was believed: the `--gc-sections` leg (which changed nothing at all) and
//! the first draft of the logging fixture (below).
//!
//! # THE SECOND FINDING: the subject is the post-LTO OBJECT, because the msvc IMAGE has no symtab
//!
//! This file censused the linked **image** until 2026-09-10. Under
//! `stable-x86_64-pc-windows-msvc` (rustc 1.98.1, LLVM 22.1.8) every cell of it read **0** —
//! `g14a` reporting four zeros and `g16ab` failing on its own `dev` positive control. What had gone
//! was the instrument, not the fold:
//!
//! | measured under msvc, on the six images this file builds | result |
//! |---|---|
//! | `llvm-nm <image>` | exit **0**, stdout **0 bytes**, `<image>: no symbols` on **stderr** |
//! | `llvm-readobj --file-headers <image>` | `PointerToSymbolTable: 0x0`, `SymbolCount: 0` |
//! | byte scan of the six `.pdb` | `mint_cold` 0, `emit_impl` 0, `boyko_diag` 0 — while `std` = 836 |
//! | `llvm-nm <post-LTO object>` | dev **1**, shipping **0**; 965–1009 total symbol lines |
//!
//! The instrument's full reading, taken by hand on both hosts after the rewrite (2026-09-10):
//!
//! | object | msvc total / `mint_cold` / `emit_impl` | gnu total / `mint_cold` / `emit_impl` |
//! |---|---|---|
//! | `deep_zone` dev | 975 / **1** / 0 | 590 / **1** / 0 |
//! | `deep_zone` shipping | 967 / **0** / 0 | 575 / **0** / 0 |
//! | `always_zone` dev | 975 / 1 / 0 | 590 / 1 / 0 |
//! | `always_zone` shipping | 975 / 1 / 0 | 590 / 1 / 0 |
//! | `profile_fixture_log` dev | 1009 / 0 / **1** | 605 / 0 / **1** |
//! | `profile_fixture_log` shipping | 965 / 0 / **0** | 575 / 0 / **0** |
//!
//! The subject's **kind** is the reason, and it is the same property the section below turns on.
//! After fat-LTO internalisation `mint_cold` is a **local** symbol: `llvm-readobj --symbols` on the
//! msvc object says `StorageClass: Static (0x3)`, and `llvm-nm` on the gnu image prints it in class
//! `t`. GNU `ld` copies an object's locals into the image's symbol table, so the gnu census was
//! reading a local all along and never had to notice the dependency. `link.exe` writes **no COFF
//! symbol table into the image at all**, and the PDB carries the linker's *publics* — of which a
//! `Static` symbol is not one. On msvc the image and the PDB are therefore both blind to the very
//! symbol this gate names, while the object and the linker map are not.
//!
//! **So the census reads the linker's INPUT, on both hosts.** That is the same question one step
//! earlier, and the step it skips subtracts nothing: the FINDING above is precisely that LTO's
//! internalize+DCE is what makes the census decidable and the link adds nothing (`--gc-sections`
//! moved not one cell), while `link.exe /OPT:REF` can only remove — so an object zero implies an
//! image zero, and the `dev` cell stays the positive control that catches an instrument with no
//! subject. Cross-validated on msvc with `link.exe /MAP`, whose *Static symbols* section names the
//! subject as arriving from this very object
//! (`… 9mint_cold 0000000140014360 f deep_zone.deep_zone.<hash>-cgu.0.rcgu.o`): the linker consumed
//! the file this census counts, and kept what is in it.
//!
//! **And an EMPTY census is now a RED under its own name.** The old instrument counted needle
//! matches in stdout and checked only the exit status, so *"the tool censused nothing"* and *"the
//! tool censused everything and found no match"* produced the same number — which is how an entire
//! gate went dark on one host while its own failure text sent the reader after the link
//! configuration. `symbols_matching` now reds when `llvm-nm` says `no symbols` on stderr, or prints
//! no symbol line at all, **before** any filtering. The same repair was made at the other copy of
//! this instrument, `boyko_diag::storage::gate::parse_nm`, where an empty stdout used to parse to
//! `SymbolReport::NoSuchSymbol` — a silent wrong answer rather than a red.
//!
//! # The two subsystems do not need the same instrument, and the reason is their symbol's KIND
//!
//! MEASURED by running the no-LTO RED through this file: `g14a` failed and **`g16ab` passed
//! unchanged**. The logging census is decidable without LTO and the profiling one is not, because
//! the two symbols are different kinds of thing:
//!
//! - `emit_impl` is **generic** (`emit_impl<A: LogArgs>`), so a monomorphisation exists only if
//!   some site instantiated it. Delete the site and the symbol was never codegen'd anywhere.
//! - `mint_cold` is a **plain function in a dependency's rlib**. It is codegen'd when `boyko_diag`
//!   is compiled, whether or not anything reaches it, and on this target nothing collects it out of
//!   the final image.
//!
//! ⚠ **The no-LTO RED changed SHAPE when the subject moved to the object, and it is worth knowing
//! which shape you are looking at.** Over the image, `g14a` failed with all four cells at **1** —
//! the rlib rides in and nothing collects it. Over the object it fails with all four cells at
//! **0**, MEASURED 2026-09-10 with `--config profile.release.lto=false` on gnu: the bin's own
//! object is a 31-symbol (dev) / 17-symbol (shipping) stub that never contained `mint_cold` at all,
//! because the function lives in `boyko_diag`'s rlib and only fat LTO's internalisation brings it
//! into this compilation unit. **Both are REDs and neither is inert** — but the object's is the
//! stronger failure, because the zero lands on the `dev` positive control, where a reader cannot
//! mistake it for the fold. What the stub *does* carry is the undefined reference
//! `U …profiling_abi7zone_id`, dev **1** / shipping **0**: a no-LTO census of this object would be
//! decidable with `zone_id` as its needle. That is recorded, not adopted — LTO plus `mint_cold`
//! measures the callee's codegen, `zone_id` would measure only the caller's reference.
//!
//! Both legs are still built with LTO here — one instrument, not two, so a future reader does not
//! have to remember which clause tolerates which link. But the asymmetry is recorded because it is
//! the thing that decides whether *any* new census clause needs LTO: ask what kind of symbol it
//! names, not which subsystem it belongs to.
//!
//! The same refusal governs the host axis. `link.exe /MAP` is decidable on msvc and was measured so
//! (table above), but GNU `ld -Map` writes a section-oriented format that is not the same parse, so
//! adopting it would mean **two** instruments differing by host — and a reader would then have to
//! remember which host's answer he is holding. The post-LTO object is one file format, read by one
//! tool, on both.
//!
//! **What this does and does not license.** The tier gate is `const { … } && …`, so the *call* is
//! deleted by the compiler in every configuration; LTO is not what deletes it. LTO is what lets a
//! census SEE that nothing references the callee. The claim is therefore "the fold removes the
//! site's codegen", proved under a link that can observe it — not "a shipping game must use LTO".
//!
//! # Cost, stated rather than hidden
//!
//! Six census legs — three fixture binaries × two profiles, each an LTO build of a 2- or 3-crate
//! graph — and one `llvm-nm` run per leg. Each leg gets its **own `CARGO_TARGET_DIR`** under the
//! system temp dir: the profile changes `boyko_diag`'s generated table, so two legs sharing a
//! target dir would rebuild each other in a loop, and a nested cargo sharing the outer sweep's
//! target dir is the linker `permission denied` this campaign has already paid for once.
//!
//! That directory is keyed by profile **and by host** since 2026-09-10. It was keyed by profile
//! alone, and the two toolchains on this machine overwrote each other's fixtures — which cost a
//! full rebuild on every host switch, but the real hazard is sharper than disk: the two hosts name
//! the object differently (msvc `deep_zone.o`, gnu `deep_zone-<hash>.o`), so both survive in one
//! `deps/` and the census cannot tell which linker's input it is holding.
//!
//! # What it cannot claim
//!
//! Nothing about the **runtime** flag: a symbol present in `dev` says nothing about whether it
//! executes, which is `GJ1`'s question. Nothing about a profile CI does not build (`custom`).
//! Nothing about a **dynamic** logging site — `dyn_debug!` is logging rung L10 and does not exist
//! yet, so the `emit_impl` clause covers the static path only.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The mangled fragment naming the profiler's out-of-line emission path.
///
/// `ZoneGuard::open` and its `Drop` are both `#[inline]` and trivial, so neither survives a release
/// build as a symbol — the corpus names them and they cannot be the subject. `mint_cold` can:
/// it is `#[cold] #[inline(never)]`, it is on the path every zone site takes on its first use, and
/// it is reachable from a `zone!` and from nothing else in these fixtures.
const ZONE_EMIT_SYMBOL: &str = "mint_cold";

/// The mangled fragment naming the logger's out-of-line emission path.
const LOG_EMIT_SYMBOL: &str = "emit_impl";

/// The `<env>` component of [`host_tag`], decided at compile time from the test binary's own target.
///
/// It cannot disagree with the toolchain that builds the fixtures: `build` removes `RUSTFLAGS` and
/// nothing else from the environment it hands the nested `cargo`, so `RUSTUP_TOOLCHAIN` is
/// inherited and the fixture is compiled by the toolchain that compiled this test. That holds for
/// both ways of choosing a toolchain on this box — the environment variable directly, and rustup's
/// `+toolchain`, which sets the same variable for the child — which is why this is read out of the
/// TEST binary's own `cfg` rather than out of a nested `rustc -vV`.
const TARGET_ENV: &str = if cfg!(target_env = "msvc") {
    "msvc"
} else if cfg!(target_env = "musl") {
    "musl"
} else if cfg!(target_env = "gnu") {
    "gnu"
} else {
    "none"
};

/// A path component naming the build host, so two toolchains on one machine get two target dirs.
///
/// See the header's cost section for why this is not a tidiness measure: with the directory keyed
/// by profile alone, an msvc `deep_zone.o` and a gnu `deep_zone-<hash>.o` end up in one `deps/` and
/// the census has no way to say which linker's input it is counting.
fn host_tag() -> String {
    format!("{}-{}-{TARGET_ENV}", std::env::consts::ARCH, std::env::consts::OS)
}

/// One fixture leg's two artifacts, both produced by the single `cargo rustc` invocation in
/// [`build`].
struct Artifact {
    /// The linked image. `g14b` and `g16ab` **execute** it — the behavioural half of the axis,
    /// which no symbol can answer.
    exe: PathBuf,
    /// The post-LTO object rustc handed the linker: what the census counts. The header's SECOND
    /// FINDING is why the image cannot be the subject on every host.
    object: PathBuf,
}

/// Builds one fixture binary under one profile, LTO-linked, emitting the post-LTO object beside the
/// image, and returns both.
///
/// Panics rather than returns on failure: a census whose artifact could not be produced has not
/// measured anything, and the RED-not-SKIP rule applies to the build step exactly as it applies to
/// the tool.
///
/// `cargo rustc` rather than `cargo build`, because `--emit=obj` has to reach rustc and only
/// `cargo rustc` passes extra arguments — to **one** target, which is why `--bin` is mandatory here
/// (`profile-fixture` carries two). `--emit=obj` does not suppress the link: rustc unions it with
/// the `--emit=dep-info,link` cargo already passes, and the image is still produced, so [`run`]
/// keeps working. MEASURED 2026-09-10 on both hosts.
fn build(package: &str, bin: &str, profile: &str) -> Artifact {
    let target = std::env::temp_dir().join(format!("boyko-axis-census-{profile}-{}", host_tag()));
    let deps = target.join("release").join("deps");
    let exe = target.join("release").join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
    // `--emit=obj` outputs are OUTSIDE cargo's fingerprint, so the pairing of image and object has
    // to be established here rather than assumed. Both of the obvious guards were TRIED and both
    // are wrong, each measured on 2026-09-10:
    //
    //   * **Delete the object before the build.** Then the object's existence afterwards would
    //     prove this invocation emitted it. It does not: cargo's fingerprint does not know the
    //     file is gone, so on the second identical recipe it reports the unit fresh, never runs
    //     rustc, and nothing recreates the object. `g14b` -- which rebuilds the same
    //     `always_zone`/`shipping` leg `g14a` has already built -- red with "no post-LTO object"
    //     on a perfectly good tree.
    //   * **Assert the object is not older than the image.** False on every honest build: rustc
    //     writes the object and the linker writes the image AFTERWARDS, so on a fresh leg the
    //     object's stamp is legitimately the earlier of the two.
    //
    // What IS invariant is that the object and the `deps/` image cargo linked out of it move
    // together. Both halves of that sentence were WRONG in this comment's first draft, and both
    // were corrected by running it (2026-09-10):
    //
    //   * **The image to stamp is the one in `deps/`, not `release/<bin>`.** The extra arguments
    //     after `--` are not "part of cargo's fingerprint" in the sense that would make the next
    //     run of this recipe rebuild: they go into the unit's METADATA HASH, so `-- --emit=obj` and
    //     a plain `cargo build` are two DIFFERENT units, each with its own fingerprint. Switching
    //     back finds THIS recipe's unit fresh, spawns no rustc at all, and merely re-uplifts its
    //     `deps/` image onto `release/<bin>` -- a hardlink swap, which moves the uplift's stamp
    //     while the object correctly stands still. Stamping the uplift therefore RED-ed a pristine
    //     tree once after any foreign recipe had touched this directory, and passed on the
    //     immediate re-run: the signature of a guard watching the wrong file.
    //   * **The object to stamp is the one that will be SELECTED.** `objects_for(..).first()` is
    //     `read_dir` order while `post_lto_object` selects by pairing, so whenever a stale
    //     `<stem>-<hash>.o` from another recipe's metadata hash sorted first the two were different
    //     files and `after != before` held for free -- the guard measured nothing at all. MEASURED
    //     in the same session: with an orphan present the `deep_zone` legs passed vacuously while
    //     `always_zone`, whose live object happened to sort first, fired.
    //
    // So: stamp every `deps/` artifact of this binary by exact path, and after the build red only
    // if the selected object's own paired image was rewritten while that object stood still.
    //
    // Residual, stated rather than hidden: on msvc cargo omits the metadata hash from a binary's
    // deps FILENAME (so the `.pdb` name is stable), so both units write `deps/<stem>.exe` and no
    // stem can tell them apart. This guard catches the foreign recipe while it RUNS -- MEASURED
    // 2026-09-10, all three msvc binaries red at this assert with `-- --emit=obj` removed. What it
    // has no way to see is a foreign build that finished before this process started, there being
    // no earlier stamp to compare against; measured in the same session, the census run after that
    // one rewrote object and image together anyway, so no stale pair survived to be counted. On gnu
    // the two units' stems differ and the pairing in `post_lto_object` refuses that case outright.
    let before = deps_stamps(&deps, bin);

    cargo_rustc(package, bin, profile, &target);

    if objects_for(&deps, bin).is_empty() {
        // The one state the pair-move guard above cannot repair, and it is self-inflicted the
        // moment anything removes an object: cargo's fingerprint does not list `--emit=obj`
        // outputs, so a fresh unit whose object is gone STAYS that way -- cargo reports the unit
        // fresh, rustc is never spawned, nothing recreates it. A gate stuck red until a human
        // clears a temp directory is a gate nobody will keep. Deleting cargo's REAL output for the
        // unit dirties it (MEASURED 2026-09-10: `release/always_zone.exe` is only the uplifted
        // hardlink and removing it merely re-links from `deps/`; removing `deps/always_zone.exe`
        // recompiles the unit and the object comes back). One retry, then the hard RED below.
        for stale in deps_files(&deps, bin, std::env::consts::EXE_EXTENSION) {
            let _ = std::fs::remove_file(&stale);
        }
        cargo_rustc(package, bin, profile, &target);
    }

    assert!(exe.is_file(), "{} was not produced", exe.display());

    let live = live_stem(&deps, bin, &exe).unwrap_or_else(|| {
        panic!(
            "no image in {} is byte-identical to {}, so nothing names the unit whose object the \
             linker consumed. Cargo *uplifts* one `deps/` image onto `release/<bin>` byte for \
             byte, and that identity is the only relation tying an object to an image here.",
            deps.display(),
            exe.display()
        )
    });
    let object = post_lto_object(&deps, bin, &live);
    let deps_image = deps.join(format!("{live}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        mtime(&object) != stamp_of(&before, &object)
            || mtime(&deps_image) == stamp_of(&before, &deps_image),
        "{} was rewritten by this build while {} was not, so the census subject is not the object \
         this image was linked from. The extra arguments after `--` go into the unit's METADATA \
         HASH, so any other recipe in this target directory -- a plain `cargo build` is the \
         measured one -- is a DIFFERENT unit that relinks {bin} and emits no object at all, which \
         is exactly this shape.",
        deps_image.display(),
        object.display()
    );
    Artifact { exe, object }
}

/// The modification time of `path`, or `None` if it does not exist or has none.
fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Every `deps/` object and image belonging to `bin`, stamped with its modification time.
///
/// Keyed by the **exact path**, because the pair-move guard in [`build`] has to ask about the two
/// files it will later select and not about whichever file `read_dir` happened to yield first —
/// see that guard's comment for the vacuity that cost.
fn deps_stamps(deps: &Path, bin: &str) -> Vec<(PathBuf, std::time::SystemTime)> {
    let mut out = Vec::new();
    for ext in ["o", std::env::consts::EXE_EXTENSION] {
        for path in deps_files(deps, bin, ext) {
            if let Some(stamp) = mtime(&path) {
                out.push((path, stamp));
            }
        }
    }
    out
}

/// The stamp [`deps_stamps`] took for `path`, or `None` if that file did not exist then.
///
/// A linear scan over at most a handful of entries, which is cheaper than the map this codebase
/// bans on the hot path and would be no clearer here.
fn stamp_of(
    stamps: &[(PathBuf, std::time::SystemTime)],
    path: &Path,
) -> Option<std::time::SystemTime> {
    stamps.iter().find(|(known, _)| known == path).map(|(_, stamp)| *stamp)
}

/// Spawns the one build recipe this file uses, and refuses to continue if it failed.
fn cargo_rustc(package: &str, bin: &str, profile: &str, target: &Path) {
    let status = Command::new(env!("CARGO"))
        .args(["rustc", "-p", package, "--bin", bin, "--release"])
        // `--config` rather than a `[profile.release]` edit in the workspace manifest: LTO here is
        // the INSTRUMENT's requirement, not the engine's, and writing it into the shared profile
        // would change every release build in the repository to satisfy one gate. (The workspace
        // manifest does now set `lto = "fat"` of its own accord -- see the SECOND RED on `g14a` --
        // but the gate must not depend on a decision that is the owner's to reverse.)
        .args(["--config", "profile.release.lto=\"fat\""])
        .args(["--config", "profile.release.codegen-units=1"])
        .args(["--", "--emit=obj"])
        .env("BOYKO_PROFILE", profile)
        .env("CARGO_TARGET_DIR", target)
        // Inherited incremental state is per-target-dir, but an inherited RUSTFLAGS is not, and
        // `-C embed-bitcode=no` from an outer invocation is incompatible with `-C lto`.
        .env_remove("RUSTFLAGS")
        .status()
        .unwrap_or_else(|e| panic!("could not spawn cargo to build {package} under {profile}: {e}"));
    assert!(
        status.success(),
        "building {package} under BOYKO_PROFILE={profile} failed, so the census has no artifact"
    );
}

/// The `deps/` stem cargo names a binary target's object after: the CRATE name, so a target whose
/// name carries `-` is looked up under `_` (`profile-fixture-log` -> `profile_fixture_log.o`).
fn object_stem(bin: &str) -> String {
    bin.replace('-', "_")
}

/// Every file in `deps` with extension `ext` belonging to `bin`: `<stem>.<ext>` (msvc, where cargo
/// omits the metadata hash for binaries so the PDB name is stable) or `<stem>-<hash>.<ext>` (gnu).
///
/// The `-` in the second arm is load-bearing: a bare `starts_with(stem)` would also claim a
/// hypothetical `deep_zone_extra.o`, and the census must never silently count a neighbour.
///
/// `ext` is compared against a missing extension as the empty string, so `std::env::consts::
/// EXE_EXTENSION` selects the image on every platform, `""` and all.
fn deps_files(deps: &Path, bin: &str, ext: &str) -> Vec<PathBuf> {
    let stem = object_stem(bin);
    let Ok(entries) = std::fs::read_dir(deps) else {
        // A first build into a fresh target dir has no `deps/` yet; that is not an error here, and
        // the missing object is caught after the build by `post_lto_object`.
        return Vec::new();
    };
    let mut found = Vec::new();
    for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        let ext_matches = match path.extension() {
            Some(e) => e == ext,
            None => ext.is_empty(),
        };
        if !ext_matches {
            continue;
        }
        let Some(file_stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if file_stem == stem || file_stem.strip_prefix(&stem).is_some_and(|r| r.starts_with('-')) {
            found.push(path);
        }
    }
    found
}

/// The post-LTO objects in `deps` belonging to `bin`.
fn objects_for(deps: &Path, bin: &str) -> Vec<PathBuf> {
    deps_files(deps, bin, "o")
}

/// The `deps/` stem cargo used for THIS build's image, identified by content identity with the
/// uplifted `release/<bin>` copy — which is what an uplift IS.
///
/// Consulted on **every** build rather than only on an ambiguous count, because it is the one
/// relation that names the unit, and both the object selection and the pair-move guard are stated
/// in terms of it. Cheap enough to be unremarkable: the fixture images are 118 KB (msvc) to 359 KB
/// (gnu), read once per `build` alongside the one candidate whose length matches.
fn live_stem(deps: &Path, bin: &str, exe: &Path) -> Option<String> {
    let want = std::fs::read(exe).ok()?;
    for cand in deps_files(deps, bin, std::env::consts::EXE_EXTENSION) {
        let Ok(meta) = std::fs::metadata(&cand) else { continue };
        if meta.len() != want.len() as u64 {
            continue;
        }
        if std::fs::read(&cand).is_ok_and(|got| got == want) {
            return cand.file_stem().map(|s| s.to_string_lossy().into_owned());
        }
    }
    None
}

/// The object emitted by the unit `live` names — the stem [`live_stem`] paired with this build's
/// image — and nothing else.
///
/// **Zero objects of any stem** is the shape a build without `-- --emit=obj` leaves, and it must
/// red naming the missing artifact rather than counting zero matches in a file nobody opened — that
/// confusion is the whole defect this instrument was rewritten to remove.
///
/// **Zero objects of the LIVE stem, with others present** is the same mutation met on a directory
/// that still holds an earlier run's object: the census would otherwise count a file that no longer
/// has anything to do with the image beside it. It is the one arm of that RED that a `found.len()`
/// check alone cannot see, so the pairing is required **unconditionally** and not only when the
/// count is ambiguous.
///
/// **More than one object** has a cause more ordinary than two hosts sharing a directory, and it
/// was met while running this file's own mutations on 2026-09-10: cargo's metadata hash is a
/// function of the recipe, so `--config profile.release.lto=false` produces
/// `always_zone-dc2cd986….{exe,o}` beside the fat-LTO `always_zone-ce20e634….{exe,o}` — and cargo
/// removes neither when the recipe changes back. A gate that simply refused would then be stuck red
/// until someone cleared a temp directory, which is not a gate anybody keeps. The same pairing
/// resolves it: cargo *uplifts* one `deps/` image onto `release/<bin>` byte for byte, and the
/// object sharing that image's stem is the one the linker consumed.
fn post_lto_object(deps: &Path, bin: &str, live: &str) -> PathBuf {
    let found = objects_for(deps, bin);
    let stem = object_stem(bin);
    assert!(
        !found.is_empty(),
        "no post-LTO object `{stem}[-<hash>].o` in {} after the build. The census reads the \
         LINKER'S INPUT, not the image (see this file's SECOND FINDING), so without it there is \
         nothing to count -- and a missing subject is a RED, never a zero. If `build` no longer \
         passes `-- --emit=obj` to `cargo rustc`, that is the cause.",
        deps.display()
    );
    let mut paired: Vec<PathBuf> = found
        .iter()
        .filter(|path| path.file_stem().is_some_and(|f| f.to_string_lossy() == live))
        .cloned()
        .collect();
    assert!(
        paired.len() == 1,
        "{} of the {} objects matching `{stem}[-<hash>].o` in {} carry `{live}`, the stem of the \
         unit whose image this build uplifted, and exactly one must. Candidates were {:?}. Cargo \
         leaves an orphan `<stem>-<hash>.{{exe,o}}` pair behind whenever the recipe's metadata \
         hash changes -- and a recipe that emits no object at all leaves ONLY orphans, which is \
         what zero here means.",
        paired.len(),
        found.len(),
        deps.display(),
        found
    );
    paired.remove(0)
}

/// Counts symbols in `object` whose name contains `needle`.
///
/// **Tool absence is a RED, never a SKIP** — the rule G22a states and this campaign has caught
/// vacuity under more than once. A gate that passes on every machine without the tool is a gate
/// that passes.
///
/// **An empty census is a RED too, and under its own name.** MEASURED 2026-09-10: on a `link.exe`
/// image `llvm-nm` exits **0**, prints **nothing** to stdout and `<image>: no symbols` to stderr,
/// so a version of this function that checked only the exit status and filtered stdout reported
/// `0` — indistinguishable from "the tool read the whole symbol table and your needle is not in
/// it". Both the stderr diagnostic and a stdout with no symbol line at all are therefore checked
/// **before** the filter, because the two conditions answer different questions and only one of
/// them is this gate's.
fn symbols_matching(object: &Path, needle: &str) -> usize {
    let tool = resolve_tool("llvm-nm").unwrap_or_else(|| {
        panic!(
            "llvm-nm is on neither PATH nor any rustup toolchain's rustlib bin. That is a RED, not \
             a skip: without it this gate cannot distinguish a folded site from a present one. \
             Install it with `rustup component add llvm-tools`."
        )
    });
    let out = Command::new(&tool)
        .arg(object)
        .output()
        .unwrap_or_else(|e| panic!("{} could not be run: {e}", tool.display()));
    assert!(out.status.success(), "{} exited non-zero on {}", tool.display(), object.display());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("no symbols"),
        "{} reported `no symbols` for {}: the tool CENSUSED NOTHING, which is not the answer \
         `no symbol matches {needle}` and must never be recorded as that number. stderr was:\n{}",
        tool.display(),
        object.display(),
        stderr.trim()
    );

    let text = String::from_utf8_lossy(&out.stdout);
    let total = text.lines().filter(|l| !l.trim().is_empty()).count();
    assert!(
        total > 0,
        "{} printed no symbol line at all for {}, so this census has no subject and its zero would \
         mean nothing. For reference the post-LTO objects of these fixtures carry 575-605 symbols \
         under gnu and 965-1009 under msvc (MEASURED 2026-09-10).",
        tool.display(),
        object.display()
    );

    text.lines().filter(|l| l.contains(needle)).count()
}

/// Runs a built fixture and returns its one stdout line.
fn run(image: &Path) -> String {
    let out = Command::new(image)
        .output()
        .unwrap_or_else(|e| panic!("{} could not be run: {e}", image.display()));
    assert!(out.status.success(), "{} exited non-zero", image.display());
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Locates an LLVM binutil: `PATH` first, then the rustup toolchains' `rustlib` bins.
///
/// This is the same resolution order `boyko_diag::storage`'s probe uses, and it is deliberately
/// **not** shared with it. Sharing would mean a `[dev-dependencies] boyko-diag = { features =
/// ["section-gate"] }` in this package — and cargo unifies features across one build, so that entry
/// would switch `section-gate` on for the fixture BINARIES too, compiling `std::process` and
/// `std::fs` into the very images this file takes a census of. The duplication is ~30 lines; the
/// alternative perturbs the measurement.
fn resolve_tool(stem: &str) -> Option<PathBuf> {
    let mut exe = String::from(stem);
    if cfg!(windows) {
        exe.push_str(".exe");
    }

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join(&exe);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }

    let home = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(|h| PathBuf::from(h).join(".rustup"))
    })?;

    let toolchains = home.join("toolchains");
    let named: Vec<PathBuf> = match std::env::var_os("RUSTUP_TOOLCHAIN") {
        Some(t) => vec![toolchains.join(t)],
        None => std::fs::read_dir(&toolchains).ok()?.filter_map(|e| e.ok().map(|e| e.path())).collect(),
    };

    let mut fallback = None;
    for tc in named {
        let Ok(targets) = std::fs::read_dir(tc.join("lib").join("rustlib")) else {
            continue;
        };
        for target in targets.filter_map(|e| e.ok()) {
            let cand = target.path().join("bin").join(&exe);
            if !cand.is_file() {
                continue;
            }
            let name = target.file_name();
            let name = name.to_string_lossy();
            if name.contains(std::env::consts::ARCH) && name.contains(std::env::consts::OS) {
                return Some(cand);
            }
            if fallback.is_none() {
                fallback = Some(cand);
            }
        }
    }
    fallback
}

/// **G14(a)** — the tier fold is per-site, shown across two profiles over single-site binaries.
///
/// RED: delete `const { $h::TIER as u8 <= GLOBAL_TIER as u8 }` from `zone_enabled!` ⇒ the `Deep`
/// site's emission path appears in the `shipping` leg ⇒ the one zero cell fills in.
///
/// SECOND RED, and the one that matters more: take away the link configuration that lets a census
/// see the difference. Over the image that read **every cell 1** and the gate could not fail at
/// all — the state this gate would have shipped in.
///
/// ⚠ Two things about that second RED were rewritten on 2026-09-10, both because they were RUN.
///
/// 1. It **no longer reproduces by dropping** `--config profile.release.lto="fat"` from `build`,
///    and this note said it did. `Cargo.toml:111-112` now sets `[profile.release] lto = "fat"` for
///    the whole workspace, so the flag is redundant and removing it changes nothing at all. The
///    mutation is `--config profile.release.lto=false`.
/// 2. Run that way on `x86_64-pc-windows-gnu`, the census over the OBJECT reds with **every cell
///    0**, not 1: `deep_zone dev 0 / shipping 0 / always_zone dev 0 / shipping 0`, three
///    mismatches, the dev positive control among them. The bin's own object without LTO is a
///    31-symbol stub and `mint_cold` was never in it (see the header's ⚠ on the no-LTO RED's
///    shape). The gate is therefore *not* inert without LTO under this instrument — it fails on
///    its control — which is strictly better than the image's silent 1/1/1/1, and it is why the
///    failure text below routes on ALL-ZERO as well as on ALL-NON-ZERO.
///
/// THIRD RED, run the same day, in both of its arms: build without `-- --emit=obj` ⇒ no object to
/// census ⇒ a RED naming the missing artifact instead of counting zero matches in a file that was
/// never opened. With an earlier run's object still lying in `deps/` the count is not zero, and the
/// arm that catches it is the pairing: that recipe is a different unit, so on gnu its image has a
/// different stem and `0 of the N objects … carry `<live>`` reds, while on msvc — where cargo omits
/// the hash from a binary's deps filename — the relink moves `deps/<stem>.exe` while the object
/// stands still and the pair-move guard reds. That is the stale-object trap itself, caught; it is
/// the shape the pre-2026-09-10 instrument had on msvc, now made impossible rather than fixed.
///
/// ⚠ The first version of that pairing guard, written the same day, stamped the **uplift** rather
/// than the `deps/` image and stamped `read_dir`'s first object rather than the selected one. It
/// red-ed a pristine tree once per foreign recipe and was vacuous whenever an orphan sorted first.
/// The build comment records both mechanisms; a guard on this pair is worth having and was worth
/// getting right twice.
#[test]
fn g14a_the_deep_site_is_deleted_by_the_shipping_ceiling_and_only_there() {
    let cells = [
        ("deep_zone", "dev", true),
        ("deep_zone", "shipping", false),
        ("always_zone", "dev", true),
        ("always_zone", "shipping", true),
    ];

    let mut report = String::new();
    let mut wrong = Vec::new();
    for (bin, profile, expect_present) in cells {
        let leg = build("profile-fixture", bin, profile);
        let n = symbols_matching(&leg.object, ZONE_EMIT_SYMBOL);
        report.push_str(&format!("  {bin:<12} {profile:<9} {ZONE_EMIT_SYMBOL} = {n}\n"));
        if (n > 0) != expect_present {
            wrong.push(format!("{bin}/{profile}: expected {expect_present}, got {n}"));
        }
    }

    assert!(
        wrong.is_empty(),
        "the cross-profile census does not have the shape the tier fold requires:\n{report}\n\
         mismatched: {}\n\
         Exactly one cell may be zero -- `deep_zone` under `shipping` -- and the other three are \
         what make that zero mean the fold rather than an empty binary.\n\
         If ALL FOUR are NON-ZERO, the fold deleted nothing: that is this gate's own subject and \
         the fold is the first suspect.\n\
         If ALL FOUR are ZERO, the fold is NOT the suspect -- suspect the link configuration. \
         MEASURED 2026-09-10 with `--config profile.release.lto=false`: without fat LTO this bin's \
         object is a 31-symbol stub that never contained the subject at all, because it lives in \
         `boyko_diag`'s rlib and only internalisation brings it into this compilation unit.\n\
         Neither of those is `the tool censused nothing`, which is a THIRD and distinct condition \
         and can no longer arrive at this message: since 2026-09-10 an empty census reds earlier \
         and under its own name, in `symbols_matching` (an `llvm-nm` that printed no symbol line, \
         or said `no symbols` on stderr) or in `post_lto_object` (no object to census at all). \
         Every count printed above therefore came out of a symbol table that was genuinely read.",
        wrong.join("; ")
    );
}

/// **G14(b)** — the shipping build is not vacuous: its `Always` tier still records.
///
/// The clause rev 3 of the corpus wanted, obtained from behaviour instead of from a symbol. A
/// ceiling that folded everything gives zero samples here, and a profiler that records nothing in a
/// shipping title is indistinguishable from one that was never compiled in.
///
/// RED: give `ZoneTier` an `Off` position below `Always` and select it ⇒ `calls=0` ⇒ red.
#[test]
fn g14b_the_shipping_build_still_runs_its_always_tier() {
    let leg = build("profile-fixture", "always_zone", "shipping");
    let line = run(&leg.exe);
    assert!(
        line.contains("profile=shipping"),
        "the artifact reports {line:?}, so the build did not use the profile this test asked for -- \
         which is the half of `the generated value matches the profile that was requested` that only \
         a harness spawning its own build can check"
    );
    assert!(
        line.contains("tier=0"),
        "shipping must compile at ZoneTier::Always (0); the artifact says {line:?}"
    );
    assert!(
        line.contains("calls=10"),
        "the shipping build opened and closed its Always-tier zone ten times and recorded {line:?}"
    );
}

/// **G16(a)/(b)** — the per-profile logging ceiling deletes a `debug!` site, and only in the
/// profile whose ceiling is below it.
///
/// RED: drop `$crate::GLOBAL_CEILING as u8 >= $crate::Level::Debug as u8` from `debug!` ⇒
/// `emit_impl` appears in the `shipping` leg.
///
/// SECOND RED, run and recorded because it is the reason the fixture calls `set_target_level`:
/// remove that call ⇒ `emit_impl = 0` in **both** legs. `CONTROL` is `.bss`-zero, LTO proves the
/// runtime gate false for the whole program, and the site vanishes everywhere — a census with no
/// subject, which reads exactly like a pass.
#[test]
fn g16ab_the_debug_site_is_deleted_by_the_shipping_ceiling() {
    let dev = build("profile-fixture-log", "profile-fixture-log", "dev");
    let ship = build("profile-fixture-log", "profile-fixture-log", "shipping");

    let n_dev = symbols_matching(&dev.object, LOG_EMIT_SYMBOL);
    let n_ship = symbols_matching(&ship.object, LOG_EMIT_SYMBOL);

    assert!(
        n_dev > 0,
        "the `dev` leg carries no {LOG_EMIT_SYMBOL} at all, so the `shipping` leg's zero would \
         measure nothing. The dev leg is this gate's positive control and its absence is \
         NOT RESOLVED (census inert), never a pass."
    );
    assert_eq!(
        n_ship, 0,
        "a `shipping` build (GLOBAL_CEILING = Info) still references {LOG_EMIT_SYMBOL} from a \
         `debug!` site, so the per-profile compile ceiling did not delete it (dev leg: {n_dev})"
    );

    // The ceiling, read out of the artifacts, is the second half of "the build used the profile
    // that was asked for" -- and across the five rows it is a one-to-one label for the profile.
    assert!(run(&dev.exe).contains("ceiling=5"), "the dev artifact does not report a Trace ceiling");
    assert!(
        run(&ship.exe).contains("ceiling=3"),
        "the shipping artifact does not report an Info ceiling"
    );
}

/// Runs `cargo check` on one package under one environment and returns (succeeded, stderr).
fn check(package: &str, envs: &[(&str, &str)], features: &[&str]) -> (bool, String) {
    let target = std::env::temp_dir().join("boyko-axis-refusal");
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["check", "-p", package]).env("CARGO_TARGET_DIR", &target).env_remove("RUSTFLAGS");
    // Every variable this axis reads is cleared first, so an outer invocation's environment cannot
    // decide the answer. A refusal gate inheriting the very variable it is testing would report
    // whatever the operator happened to be running under.
    for k in ["BOYKO_PROFILE", "BOYKO_PROFILING_TIER", "BOYKO_PROFILING_REGION_CAPACITY", "BOYKO_PROFILING_DYN_CAP", "BOYKO_LOG_MAX_LEVEL"] {
        cmd.env_remove(k);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if !features.is_empty() {
        cmd.args(["--features", &features.join(",")]);
    }
    let out = cmd.output().unwrap_or_else(|e| panic!("could not spawn cargo: {e}"));
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// **G14(c)** — a profile that does not admit the analysis half REFUSES a build that enables it.
///
/// This replaces the corpus's symbol census over `ConcurrencyReport` / `resolve` / the TOML writer,
/// and it is strictly stronger rather than a weakening. A census answers *"did this particular
/// artifact end up containing them?"* — and, per the LTO finding above, on this target it would
/// have answered that question wrongly. A build refusal answers *"can any shipping artifact contain
/// them?"*, which is what the row in `SEAM.md` actually claims.
///
/// Both directions are asserted. A one-sided version would pass on a workspace that failed to build
/// for any reason at all, which is the same vacuity the `dev` control exists to catch above.
#[test]
fn g14c_a_shipping_profile_refuses_the_analysis_half() {
    let (ok, stderr) = check("boyko-ecs", &[("BOYKO_PROFILE", "shipping")], &["profiling-analysis"]);
    assert!(
        !ok,
        "BOYKO_PROFILE=shipping accepted `--features profiling-analysis`, so a shipping build can \
         carry `ConcurrencyReport`, `resolve` and the TOML writer while its own profile says the \
         analysis half is absent"
    );
    assert!(
        stderr.contains("does not admit the analysis half"),
        "the refusal did not name the conflict; a build that fails for an unnamed reason teaches \
         the operator nothing. stderr was:\n{stderr}"
    );

    let (ok, stderr) = check("boyko-ecs", &[("BOYKO_PROFILE", "dev")], &["profiling-analysis"]);
    assert!(ok, "BOYKO_PROFILE=dev must ACCEPT the analysis half, or the refusal above proves \
                 nothing about the profile. stderr was:\n{stderr}");
}

/// **G16(c)** — a per-knob override beside a named profile is a `compile_error!`, not a silent
/// winner or a silent loser.
///
/// This is the single-axis rule made mechanical. With two axes a binary ends up printing a ceiling
/// its profile does not name, and no test downstream can tell which of the two produced the value.
///
/// RED: delete the knob loop from `crates/boyko_diag/build.rs` ⇒ the build succeeds and the ceiling
/// silently comes from whichever side the script checked last.
#[test]
fn g16c_a_stray_knob_beside_a_named_profile_refuses_to_build() {
    let (ok, stderr) = check(
        "boyko-diag",
        &[("BOYKO_PROFILE", "shipping"), ("BOYKO_LOG_MAX_LEVEL", "trace")],
        &[],
    );
    assert!(!ok, "BOYKO_PROFILE=shipping accepted BOYKO_LOG_MAX_LEVEL=trace beside it");
    assert!(
        stderr.contains("One build axis"),
        "the refusal did not name the rule it is enforcing. stderr was:\n{stderr}"
    );

    // The same knob under `custom`, which is the one value that honours it, must BUILD -- otherwise
    // the refusal above is indistinguishable from "this knob is simply broken".
    let (ok, stderr) = check(
        "boyko-diag",
        &[("BOYKO_PROFILE", "custom"), ("BOYKO_LOG_MAX_LEVEL", "trace")],
        &[],
    );
    assert!(ok, "BOYKO_PROFILE=custom must honour BOYKO_LOG_MAX_LEVEL. stderr was:\n{stderr}");

    // And a value that names no profile is refused by name rather than defaulted to `dev`, which
    // would ship a typo as a full-fat development build.
    let (ok, stderr) = check("boyko-diag", &[("BOYKO_PROFILE", "retail")], &[]);
    assert!(!ok, "BOYKO_PROFILE=retail was accepted; a misspelt profile must not fall back");
    assert!(
        stderr.contains("names no profile"),
        "the refusal did not name the typo. stderr was:\n{stderr}"
    );
}
