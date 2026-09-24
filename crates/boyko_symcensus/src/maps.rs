//! Sensitivity maps (a) and (b) (03 §6 "(2) Sensitivity map"). They ATTRIBUTE a leg-(2) move to
//! a source file or a layout change; they never pin anything, and a body missing from a map is
//! still RED when it moves unnamed.
//!
//! **(a) File map.** Route (a1), decided by probe (iii): the `sensitivity-map` build's msvc image
//! plus its PDB, read by the MSVC Build Tools `llvm-symbolizer`. Name → address comes from the
//! linker's `/MAP` ("Static symbols" and "Publics"), the body's length from the post-LTO object's
//! section, the instruction addresses from `llvm-objdump` over the image range. Every frame of
//! every instruction, inlined frames included, contributes its source file.
//!
//! Frames name files in three spellings (probe (iii)): the worktree's absolute path, the rustup
//! source tree, and rustc's remapped `/rustc/<hash>/library`. [`norm_file`] makes a row
//! repo-relative (`crates/…`), `library/…` or `registry/<crate>-<ver>/…`, with `/` separators, so
//! the map does not differ between worktrees or machines.
//!
//! **(b) Layout map.** For each leg-(3) type, the leg-(2) bodies that differ between two
//! never-committed builds: arm 1 forces `#[repr(C)]` on the type, arm 2 adds a leading
//! `[u8; align_of::<T>()]`, which shifts every field displacement by the type's alignment. The
//! arms are made on local control branches; this module only captures and diffs bodies.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::llvm::{self, Frame};
use crate::objbuild::{self, Ctx, Request, Subject};
use crate::objview::{Llvm, ObjView};
use crate::pins::{self, PinFile};
use crate::red::{Red, RedKind, Result};
use crate::tools::Tool;

/// The profile map (a) reads (03 §6; B3 adds it).
pub const MAP_PROFILE: &str = "sensitivity-map";

/// Repo-relative (or `library/…` / `registry/…`) spelling of a frame's file.
#[must_use]
pub fn norm_file(root: &Path, file: &str) -> Option<String> {
    if file.is_empty() || file == "??" || file.starts_with('<') {
        return None;
    }
    let f = file.replace('\\', "/");
    let root_s = root.to_string_lossy().replace('\\', "/");
    let lower = f.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix(&format!("{}/", root_s.to_ascii_lowercase())) {
        return Some(f[f.len() - rest.len()..].to_owned());
    }
    if let Some(i) = f.find("/library/")
        && (f.contains("/rustlib/src/rust/") || f.starts_with("/rustc/"))
    {
        return Some(f[i + 1..].to_owned());
    }
    if let Some(i) = f.find("/registry/src/") {
        let after = &f[i + "/registry/src/".len()..];
        // `<index dir>/<crate>-<ver>/…`
        if let Some((_, rest)) = after.split_once('/') {
            return Some(format!("registry/{rest}"));
        }
    }
    if let Some(i) = f.find("/crates/") {
        // Another worktree's path to the same repo layout still names the crate file.
        return Some(format!("other-tree:{}", &f[i + 1..]));
    }
    Some(format!("external:{f}"))
}

/// `/MAP` name → VA, from the "Publics" and "Static symbols" sections, and the preferred load
/// address.
fn parse_map(text: &str) -> Result<(u64, BTreeMap<String, Vec<u64>>)> {
    let base = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Preferred load address is "))
        .and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
        .ok_or_else(|| Red::new(RedKind::Malformed, "the /MAP has no `Preferred load address is` line"))?;
    let mut m: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for l in text.lines() {
        let mut it = l.split_whitespace();
        let (Some(secoff), Some(name), Some(va)) = (it.next(), it.next(), it.next()) else { continue };
        if !secoff.contains(':') || secoff.len() != 13 {
            continue;
        }
        if let Ok(v) = u64::from_str_radix(va, 16)
            && v >= base
        {
            m.entry(name.to_owned()).or_default().push(v);
        }
    }
    if m.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, "the /MAP lists no symbol with an address"));
    }
    Ok((base, m))
}

/// The RVAs of every instruction `llvm-objdump` decodes in `[va, va + len)` of `image`.
fn insn_rvas(objdump: &Tool, image: &Path, base: u64, va: u64, len: u64) -> Result<Vec<u64>> {
    let dis = objdump.output([
        std::ffi::OsString::from("-d"),
        "--no-show-raw-insn".into(),
        format!("--start-address=0x{va:x}").into(),
        format!("--stop-address=0x{:x}", va + len).into(),
        image.as_os_str().to_owned(),
    ])?;
    let mut rvas = Vec::new();
    for l in String::from_utf8_lossy(&dis.stdout).lines() {
        if let Some((a, _)) = l.trim_start().split_once(':')
            && let Ok(v) = u64::from_str_radix(a.trim(), 16)
            && v >= va
            && v < va + len
        {
            rvas.push(v - base);
        }
    }
    Ok(rvas)
}

/// One pin's map row set.
#[derive(Clone, Debug, Default)]
pub struct PinMap {
    /// Normalised files.
    pub files: BTreeSet<String>,
    /// Frame function names (for the absent list's carrier column).
    pub functions: BTreeSet<String>,
    /// Addresses symbolized.
    pub addresses: usize,
}

/// Map (a) of every committed pin of `subjects`: `(subject, cand, name) → PinMap`. A pin whose
/// name is not defined in the `sensitivity-map` build is returned as `None` (unmapped), never
/// dropped.
pub fn capture_a(ctx: &Ctx, llvm: &Llvm, symbolizer: &Tool, subjects: &[Subject], receipt: &mut String) -> Result<BTreeMap<(String, String, String), Option<PinMap>>> {
    let mut out = BTreeMap::new();
    for &s in subjects {
        let Some(file) = pins::read_pins(&ctx.root, s.key)? else { continue };
        let map_path = ctx.target.join("ug15").join("maps").join(format!("sensmap-{}.map", s.key));
        if let Some(d) = map_path.parent() {
            std::fs::create_dir_all(d).map_err(|e| Red::io(d, &e))?;
        }
        let extra = vec!["-C".to_owned(), format!("link-arg=/MAP:{}", map_path.display())];
        let built = objbuild::build(ctx, &Request::new(s, MAP_PROFILE, &extra))?;
        receipt.push_str(&built.receipt());
        if built.pdb.is_none() {
            return Err(Red::new(RedKind::NoObject, format!("no PDB beside {}: the {MAP_PROFILE} profile must write one", built.image.display())));
        }
        let view = ObjView::load(llvm, &built.object)?;
        let map_text = std::fs::read_to_string(&map_path).map_err(|e| Red::io(&map_path, &e))?;
        let (base, vas) = parse_map(&map_text)?;
        let names: BTreeSet<(String, String)> = file.pins.iter().map(|p| (p.cand.clone(), p.name.clone())).collect();
        for (cand, name) in names {
            let syms = pins::by_name(&view, &name);
            if syms.is_empty() {
                let _ = writeln!(receipt, "{} {cand} {name}: UNMAPPED (not defined in the {MAP_PROFILE} object)", s.key);
                out.insert((s.key.to_owned(), cand, name), None);
                continue;
            }
            let mut pm = PinMap::default();
            for sym in syms {
                let Some(section) = view.section_of(&sym.raw) else { continue };
                let len = view.coff.section(section).map_or(0, |x| x.raw_size);
                let Some(addrs) = vas.get(&sym.raw) else {
                    let _ = writeln!(receipt, "{} {cand} {name}: copy {} not in the /MAP (removed by /OPT:REF or folded)", s.key, sym.raw);
                    continue;
                };
                for &va in addrs {
                    let rvas = insn_rvas(&llvm.objdump, &built.image, base, va, len)?;
                    if rvas.is_empty() {
                        return Err(Red::new(RedKind::EmptyCensus, format!("map (a): no instruction decoded for {name} at 0x{va:x} in {}", built.image.display())));
                    }
                    let frames: Vec<Vec<Frame>> = llvm::symbolize(symbolizer, &built.image, &rvas)?;
                    pm.addresses += rvas.len();
                    for f in frames.iter().flatten() {
                        if let Some(nf) = norm_file(&ctx.root, &f.file) {
                            pm.files.insert(nf);
                        }
                        if !f.function.is_empty() {
                            pm.functions.insert(f.function.clone());
                        }
                    }
                }
            }
            if pm.files.is_empty() {
                return Err(Red::new(RedKind::EmptyCensus, format!("map (a): {} {cand} {name} resolved no source file over {} address(es)", s.key, pm.addresses)));
            }
            let _ = writeln!(receipt, "{} {cand} {name}: {} address(es), {} file(s), {} frame function(s)", s.key, pm.addresses, pm.files.len(), pm.functions.len());
            out.insert((s.key.to_owned(), cand, name), Some(pm));
        }
        built.check_intact()?;
    }
    if out.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, "map (a): no committed pin to map"));
    }
    Ok(out)
}

/// The TSV of map (a): `subject\tcand\tpin name\tfile` (unmapped pins carry `UNMAPPED`).
#[must_use]
pub fn file_map_tsv(m: &BTreeMap<(String, String, String), Option<PinMap>>) -> String {
    let mut s = String::from("# UG-15 sensitivity map (a): source files of every frame, inlined frames included, over each pinned body (03 §6; route a1).\n# subject\tcand\tpin\tfile\n");
    for ((subj, cand, name), pm) in m {
        match pm {
            Some(pm) => {
                for f in &pm.files {
                    let _ = writeln!(s, "{subj}\t{cand}\t{name}\t{f}");
                }
            }
            None => {
                let _ = writeln!(s, "{subj}\t{cand}\t{name}\tUNMAPPED");
            }
        }
    }
    s
}

/// Map (b), one arm: the normalised body hashes of every committed pin, per subject.
pub fn capture_b_arm(ctx: &Ctx, llvm: &Llvm, subjects: &[Subject], label: &str) -> Result<String> {
    let mut s = format!("# ug15 map b arm {label}\n");
    let mut total = 0usize;
    for &subj in subjects {
        let Some(file) = pins::read_pins(&ctx.root, subj.key)? else { continue };
        let built = objbuild::build(ctx, &Request::new(subj, pins::PROFILE, &[]))?;
        let view = ObjView::load(llvm, &built.object)?;
        let names: BTreeSet<(String, String)> = file.pins.iter().map(|p| (p.cand.clone(), p.name.clone())).collect();
        for (cand, name) in names {
            let syms = pins::by_name(&view, &name);
            let mut shas: Vec<String> = Vec::with_capacity(syms.len());
            for sym in syms {
                shas.push(view.body(&llvm.objdump, &sym.raw, &crate::normalize::RenameList::default())?.sha256);
            }
            shas.sort();
            let joined = if shas.is_empty() { "ABSENT".to_owned() } else { shas.join(",") };
            let _ = writeln!(s, "{}\t{cand}\t{name}\t{joined}", subj.key);
            total += 1;
        }
        built.check_intact()?;
    }
    if total == 0 {
        return Err(Red::new(RedKind::EmptyCensus, "map (b): no committed pin to capture"));
    }
    Ok(s)
}

fn parse_arm(text: &str) -> BTreeMap<(String, String, String), String> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.splitn(4, '\t');
            Some(((it.next()?.to_owned(), it.next()?.to_owned(), it.next()?.to_owned()), it.next()?.to_owned()))
        })
        .collect()
}

/// Map (b) diff: rewrites `type`'s rows of the layout map at `map` with every pin whose bodies
/// differ between the two arms. Returns the rows written.
pub fn diff_b(ty: &str, arm1: &str, arm2: &str, map: &Path) -> Result<Vec<String>> {
    let (a, b) = (parse_arm(arm1), parse_arm(arm2));
    if a.is_empty() || a.len() != b.len() {
        return Err(Red::new(RedKind::Mismatch, format!("map (b) arms are not comparable: {} vs {} rows", a.len(), b.len())));
    }
    let rows: Vec<String> = a
        .iter()
        .filter(|(k, v)| b.get(*k) != Some(*v))
        .map(|((subj, cand, name), _)| format!("{ty}\t{subj}\t{cand}\t{name}"))
        .collect();
    let existing = std::fs::read_to_string(map).unwrap_or_default();
    let mut keep: BTreeSet<String> =
        existing.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty() && !l.starts_with(&format!("{ty}\t"))).map(str::to_owned).collect();
    keep.extend(rows.iter().cloned());
    let mut text = String::from(
        "# UG-15 sensitivity map (b): leg-(2) bodies that differ between repr(C) and repr(C)+[u8; align] builds of each leg-(3) type (03 §6).\n# type\tsubject\tcand\tpin\n",
    );
    for l in &keep {
        text.push_str(l);
        text.push('\n');
    }
    std::fs::write(map, text).map_err(|e| Red::io(map, &e))?;
    Ok(rows)
}

/// Reads the pin files of every subject that has one.
pub fn committed_pins(root: &Path) -> Result<Vec<(Subject, PinFile)>> {
    let mut v = Vec::new();
    for s in objbuild::SUBJECTS {
        if let Some(f) = pins::read_pins(root, s.key)? {
            v.push((s, f));
        }
    }
    Ok(v)
}
