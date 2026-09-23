//! Leg (7b), the rlib object census (03 §6; 05 §6), and leg (1)'s generated-source scan.
//!
//! **(7b).** `llvm-nm --defined-only --demangle` over the OBJECT members of every census crate's
//! rlib, as cargo built them for the subject under `seam-census`. Under fat LTO those members
//! are LLVM bitcode (PC-7), read by rustup's `llvm-nm`, which is rustc's own LLVM. Pass: no
//! defined symbol names a seam item that is not doc-only. MS-08's doc-only items are defined
//! non-generic engine functions, so they ARE in the bitcode; they are exempt (PC-6) and are the
//! census's positive control: with doc-only rows in the inventory, zero exempt hits means the
//! matcher is blind, which is RED.
//!
//! **Anti-vacuity:** zero census rlibs, zero object members, or zero defined symbols read is RED.
//!
//! **Generated sources (critique O3).** A census crate's build script writes Rust into its
//! `OUT_DIR` (`boyko_diag`'s `profile_axis.rs`, `include!`d by `profile.rs`), which the leg-(1)
//! source walk cannot reach. The build this leg already runs reports each `OUT_DIR`; every `.rs`
//! in it goes through the one visitor, and any counted form is RED (the leg-(1) allowlist names
//! source files, never generated ones).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::llvm::{self, MemberFormat};
use crate::objbuild::{self, Built, Ctx, Request, Subject};
use crate::objview::Llvm;
use crate::red::{Red, RedKind, Result};
use crate::seam::{self, SEAM_INVENTORY};
use crate::source::{self, Via};

/// The profile leg (7b) reads (03 §6).
pub const PROFILE: &str = "seam-census";

/// Defined symbols of one extracted member, demangled; an empty list is allowed per member
/// (the per-rlib and total counts carry the anti-vacuity).
fn member_symbols(nm: &crate::tools::Tool, file: &Path) -> Result<Vec<String>> {
    let out = nm.output([std::ffi::OsStr::new("--defined-only"), "--demangle".as_ref(), "--no-sort".as_ref(), file.as_os_str()])?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-nm exited {:?} on {}: {}", out.status.code(), file.display(), String::from_utf8_lossy(&out.stderr).trim())));
    }
    let mut v = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()) {
        // `-------- T name` for bitcode (no value), `00000000 T name` for COFF.
        let mut it = line.splitn(3, ' ');
        let (Some(_), Some(_), Some(name)) = (it.next(), it.next(), it.next()) else {
            return Err(Red::new(RedKind::Malformed, format!("llvm-nm line not understood: {line}")));
        };
        v.push(name.to_owned());
    }
    Ok(v)
}

/// Runs leg (7b) and the generated-source scan for `s`; returns the receipt, RED on a violation.
pub fn run(ctx: &Ctx, llvm: &Llvm, s: Subject) -> Result<String> {
    let built = objbuild::build(ctx, &Request::new(s, PROFILE, &[]))?;
    let mut out = crate::probe::header(ctx, llvm, &format!("leg (7b) rlib object census of {} under {PROFILE}", s.key));
    out.push_str(&built.receipt());
    let census = census_rlibs(ctx, llvm, &built, &mut out)?;
    let generated = generated_sources(ctx, &built, &mut out)?;
    built.check_intact()?;
    let mut problems = census;
    problems.extend(generated);
    if problems.is_empty() {
        let _ = writeln!(out, "leg (7b): GREEN");
        Ok(out)
    } else {
        Err(Red::new(RedKind::Mismatch, format!("{out}leg (7b) RED:\n  {}", problems.join("\n  "))))
    }
}

fn census_rlibs(ctx: &Ctx, llvm: &Llvm, built: &Built, out: &mut String) -> Result<Vec<String>> {
    let rlibs: BTreeSet<(String, PathBuf)> = built.rlibs.iter().cloned().collect();
    if rlibs.is_empty() {
        return Err(Red::new(RedKind::MemberCount, "cargo reported no census-crate rlib for this build"));
    }
    let scratch = ctx.target.join("ug15").join("7b");
    let mut members_read = 0usize;
    let mut symbols_read = 0usize;
    let mut exempt: Vec<String> = Vec::new();
    let mut violations: Vec<String> = Vec::new();
    for (krate, rlib) in &rlibs {
        let members = llvm::rlib_members(&llvm.ar, rlib)?;
        let dir = scratch.join(krate);
        std::fs::create_dir_all(&dir).map_err(|e| Red::io(&dir, &e))?;
        let mut crate_syms = 0usize;
        let mut crate_members = 0usize;
        for m in members.iter().filter(|m| m.format != MemberFormat::Metadata) {
            let body = llvm.ar.output([std::ffi::OsStr::new("p"), rlib.as_os_str(), m.name.as_ref()])?;
            if !body.status.success() || body.stdout.len() != m.size {
                return Err(Red::new(RedKind::ToolFailed, format!("llvm-ar p {} {} did not reproduce the member", rlib.display(), m.name)));
            }
            let file = dir.join(&m.name);
            std::fs::write(&file, &body.stdout).map_err(|e| Red::io(&file, &e))?;
            let syms = member_symbols(&llvm.nm, &file)?;
            crate_members += 1;
            crate_syms += syms.len();
            for name in &syms {
                for row in SEAM_INVENTORY {
                    if seam::names_item(name, row) {
                        let line = format!("{krate}/{}: {} names {} {}", m.name, name, row.ms, row.item);
                        if row.doc_only { exempt.push(line) } else { violations.push(line) }
                    }
                }
            }
        }
        let _ = writeln!(out, "{krate} {}: {crate_members} object member(s), {crate_syms} defined symbol(s)", rlib.display());
        if crate_members == 0 {
            return Err(Red::new(RedKind::MemberCount, format!("{} has zero object members once `lib.rmeta` is skipped", rlib.display())));
        }
        members_read += crate_members;
        symbols_read += crate_syms;
    }
    let _ = writeln!(out, "read: {} rlib(s), {members_read} object member(s), {symbols_read} defined symbol(s)", rlibs.len());
    if symbols_read == 0 {
        return Err(Red::new(RedKind::EmptyCensus, "leg (7b) read zero defined symbols across every object member"));
    }
    let _ = writeln!(out, "exempt (doc-only MOD-SEAM) hits: {}", exempt.len());
    for e in &exempt {
        let _ = writeln!(out, "  {e}");
    }
    let _ = writeln!(out, "non-exempt hits: {}", violations.len());
    let mut problems: Vec<String> = violations.iter().map(|v| format!("seam item defined in an rlib object: {v}")).collect();
    if SEAM_INVENTORY.iter().any(|r| r.doc_only) && exempt.is_empty() {
        problems.push(
            "positive control failed: the inventory holds doc-only seam items that are defined engine functions, and no symbol named any of them — the matcher is blind"
                .to_owned(),
        );
    }
    Ok(problems)
}

fn generated_sources(ctx: &Ctx, built: &Built, out: &mut String) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let dirs: BTreeSet<(String, PathBuf)> = built.out_dirs.iter().cloned().collect();
    let _ = writeln!(out, "leg (1) generated sources: {} census build-script OUT_DIR(s)", dirs.len());
    for (krate, dir) in &dirs {
        let files = if dir.is_dir() { source::rs_files(dir)? } else { Vec::new() };
        let mut items = 0usize;
        for f in &files {
            let parsed = source::parse_path(f)?;
            let label = format!("{krate}:OUT_DIR/{}", source::rel(dir, f));
            let (findings, st) = source::count_file(&label, &parsed, Via::SynItem);
            items += st.items;
            for fnd in findings {
                problems.push(format!("generated source: {fnd}"));
            }
        }
        let _ = writeln!(out, "  {krate} {}: {} .rs file(s), {items} item(s)", source::rel(&ctx.target, dir), files.len());
        if files.is_empty() {
            problems.push(format!("{krate}'s build script ran and its OUT_DIR holds no .rs file; the generated-source scan read nothing"));
        }
    }
    Ok(problems)
}
