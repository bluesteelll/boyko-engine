//! B3's probe build, items (i)–(v) (02 B3; cut §3). Each probe returns its receipt text; the
//! decision each answer feeds is printed as a `DECISION:` line so the receipt is self-reading.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::candidates::{self, CANDIDATES};
use crate::host;
use crate::llvm::{self, Frame};
use crate::normalize::{self, RenameList};
use crate::objbuild::{self, Built, Ctx, Request, Subject};
use crate::objview::{Body, Llvm, ObjView};
use crate::pe;
use crate::red::{Red, RedKind, Result};
use crate::sha256;
use crate::snapshot::{self, Snapshot};
use crate::tools;

/// The receipt header every probe and leg prints: tree, toolchain, tools.
#[must_use]
pub fn header(ctx: &Ctx, llvm: &Llvm, what: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "== {what}");
    let _ = writeln!(s, "tree         {} ({})", ctx.root.display(), host::git_provenance(&ctx.root));
    let _ = writeln!(s, "target dir   {}", ctx.target.display());
    let _ = writeln!(s, "host         {} ({})", ctx.host.triple, ctx.host.env);
    for l in ctx.host.rustc_vv.lines() {
        let _ = writeln!(s, "rustc -vV    {l}");
    }
    s.push_str(&llvm.receipt());
    s
}

fn release(ctx: &Ctx, s: Subject) -> Result<Built> {
    objbuild::build(ctx, &Request { subject: s, profile: "release", extra_rustc: &[], emit_obj: true })
}

/// A decoded `core::panic::Location` referenced as anonymous data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocationRef {
    /// The anonymous symbol the body references.
    pub anon: String,
    /// The embedded file path.
    pub file: String,
    /// The line.
    pub line: u32,
    /// The column.
    pub col: u32,
}

/// Decodes `anon` as a `Location { file: &str, line: u32, col: u32 }` (24 bytes, an `ADDR64` to the
/// path at offset 0); `None` when it is not that shape.
fn decode_location(
    llvm: &Llvm,
    view: &ObjView,
    relocs: &llvm::Relocations,
    anon: &str,
) -> Result<Option<LocationRef>> {
    let Some(n) = view.section_of(anon) else { return Ok(None) };
    if view.coff.section(n).map(|s| s.raw_size) != Some(24) {
        return Ok(None);
    }
    let Some(target) = relocs.get(&n).and_then(|v| v.iter().find(|(o, k, _)| *o == 0 && k.ends_with("ADDR64"))).map(|(_, _, t)| t.clone()) else {
        return Ok(None);
    };
    let bytes = llvm::section_bytes(&llvm.readobj, &view.path, n)?;
    if bytes.len() != 24 {
        return Ok(None);
    }
    let len = u64::from_le_bytes(bytes[8..16].try_into().expect("invariant: 8 bytes")) as usize;
    let line = u32::from_le_bytes(bytes[16..20].try_into().expect("invariant: 4 bytes"));
    let col = u32::from_le_bytes(bytes[20..24].try_into().expect("invariant: 4 bytes"));
    let Some(sn) = view.section_of(&target) else { return Ok(None) };
    let off = view.coff.symbols.iter().find(|s| s.name == target && s.aux.is_none()).map_or(0, |s| s.value) as usize;
    let sbytes = llvm::section_bytes(&llvm.readobj, &view.path, sn)?;
    let Some(slice) = sbytes.get(off..off + len) else { return Ok(None) };
    let Ok(file) = std::str::from_utf8(slice) else { return Ok(None) };
    if !file.ends_with(".rs") {
        return Ok(None);
    }
    Ok(Some(LocationRef { anon: anon.to_owned(), file: file.to_owned(), line, col }))
}

/// The form class of a relocation target (probe (i)'s histogram key).
fn form_of(target: &str, classes: &BTreeMap<&str, char>) -> String {
    if target.starts_with("anon.") {
        return "anon.<hash>.<n> (anonymous; <anon-data>)".to_owned();
    }
    if target.starts_with('.') {
        return "section symbol (anonymous; <anon-data>)".to_owned();
    }
    for p in ["__unnamed_", "alloc_", "str."] {
        if target.starts_with(p) {
            return format!("{p}* (anonymous; <anon-data>)");
        }
    }
    if target.starts_with("switch.table.") {
        return "switch.table.<owner>[.N][.rel] (owned; named by owner)".to_owned();
    }
    if normalize::CONTENT_NAMED_PREFIXES.iter().any(|p| target.starts_with(p)) {
        return "__real@/__xmm@/__ymm@ (content-named constant)".to_owned();
    }
    if normalize::SEH_PREFIXES.iter().any(|p| target.starts_with(p)) {
        return "$<seh table>$<owner> (owned; named by owner)".to_owned();
    }
    if target.starts_with('?') {
        return "?dtor$/?catch$ funclet (owned; named by owner)".to_owned();
    }
    if target.starts_with("__imp_") {
        return "__imp_* (import)".to_owned();
    }
    let mangled = target.starts_with("_R") || target.starts_with("_ZN");
    match classes.get(target) {
        Some(c) if candidates::is_code(*c) => {
            if mangled { "defined code, mangled".to_owned() } else { format!("defined code, C name ({target})") }
        }
        Some(c) => {
            if mangled { format!("defined data class {c}, mangled") } else { format!("UNKNOWN defined data class {c}: {target}") }
        }
        None => {
            if mangled { "undefined, mangled".to_owned() } else { format!("undefined C name ({target})") }
        }
    }
}

/// Probes (i) and (ii) over `subjects`, built under `release`.
///
/// (i) records every relocation target form in every candidate body, the mangling scheme, and each
/// panic `Location` a body references (decoded to file:line:col). (ii) records, per candidate body,
/// its section, COMDAT selection, `RawDataSize`, aux `Length`, the symbols sharing the section and
/// the distance from the function to the next of them; and whether `llvm-nm -S` prints sizes on
/// COFF at all.
pub fn probe_i_ii(ctx: &Ctx, llvm: &Llvm, subjects: &[Subject]) -> Result<String> {
    let mut out = header(ctx, llvm, "probe (i) anonymous-data forms + (ii) symbol-size source");
    let mut forms: BTreeMap<String, usize> = BTreeMap::new();
    let mut unknown: BTreeSet<String> = BTreeSet::new();
    let mut absent: Vec<String> = Vec::new();
    let mut size_rows: Vec<String> = Vec::new();
    let mut all_at_zero = true;
    let mut all_alone = true;
    let mut aux_eq_raw = true;
    for &s in subjects {
        let built = release(ctx, s)?;
        out.push_str(&built.receipt());
        let view = ObjView::load(llvm, &built.object)?;
        let relocs = llvm::relocations(&llvm.readobj, &built.object)?;
        let classes: BTreeMap<&str, char> = view.syms.defined.iter().map(|x| (x.raw.as_str(), x.class)).collect();
        let mut mangling: BTreeMap<&str, usize> = BTreeMap::new();
        for d in &view.syms.defined {
            let k = if d.raw.starts_with("_R") {
                "_R (v0)"
            } else if d.raw.starts_with("_ZN") {
                "_ZN (legacy)"
            } else if d.raw.starts_with("anon.") {
                "anon."
            } else if d.raw.starts_with('$') || d.raw.starts_with('?') {
                "SEH ($… / ?…)"
            } else {
                "other"
            };
            *mangling.entry(k).or_insert(0) += 1;
        }
        let _ = writeln!(out, "defined symbols {} ; by raw prefix {mangling:?}", view.syms.defined.len());
        if let Some(sample) = view.syms.defined.iter().find(|d| d.raw.starts_with("_R")) {
            let _ = writeln!(out, "demangled sample: {}  <=  {}", sample.name, sample.raw);
        }
        let nm_s = llvm.nm.output([std::ffi::OsStr::new("-S"), "--defined-only".as_ref(), built.object.as_os_str()])?;
        let nm_s_text = String::from_utf8_lossy(&nm_s.stdout);
        let nonzero = nm_s_text.lines().filter(|l| l.split_whitespace().nth(1).is_some_and(|z| !z.trim_start_matches('0').is_empty())).count();
        let _ = writeln!(out, "llvm-nm -S: {} lines, {nonzero} with a non-zero size field", nm_s_text.lines().count());

        for cand in CANDIDATES.iter().filter(|c| c.subjects.contains(&s.key)) {
            let mut syms = Vec::new();
            for rule in cand.rules {
                for x in candidates::locate(&view.syms.defined, *rule) {
                    if !syms.iter().any(|y: &&llvm::Sym| y.raw == x.raw) {
                        syms.push(x);
                    }
                }
            }
            if syms.is_empty() {
                absent.push(format!("{} in {}", cand.id, s.key));
                let _ = writeln!(out, "-- {} in {}: ABSENT (no defined code symbol matches {:?})", cand.id, s.key, cand.rules);
                continue;
            }
            for sym in syms {
                let body = view.body(&llvm.objdump, &sym.raw, &RenameList::default())?;
                let main_at = body.members.iter().find(|(n, _)| *n == sym.raw).map_or(0, |(_, v)| *v);
                let next = body.members.iter().map(|(_, v)| *v).filter(|v| *v > main_at).min().unwrap_or(body.section_len);
                all_at_zero &= main_at == 0;
                all_alone &= body.members.len() == 1;
                aux_eq_raw &= body.aux_len == Some(body.section_len);
                let row = format!(
                    "{} {} | {} | sec {} `{}` sel={} raw={} aux={:?} | members={} main@{} next@{} → main-only={} section={} | sha {}",
                    cand.id,
                    s.key,
                    body.name,
                    body.section,
                    body.section_name,
                    if body.selection.is_empty() { "-" } else { &body.selection },
                    body.section_len,
                    body.aux_len,
                    body.members.len(),
                    main_at,
                    next,
                    next - main_at,
                    body.section_len,
                    &body.sha256[..16]
                );
                let _ = writeln!(out, "-- {row}");
                size_rows.push(row);
                if body.members.len() > 1 {
                    for (m, v) in &body.members {
                        let _ = writeln!(out, "     member @{v}: {}", normalize::norm_target(&view.map, m, &RenameList::default()));
                    }
                }
                let mut body_forms: BTreeMap<String, usize> = BTreeMap::new();
                let mut anon_targets: BTreeSet<String> = BTreeSet::new();
                for part in &body.parts {
                    for insn in &part.insns {
                        for r in &insn.relocs {
                            let f = form_of(&r.target, &classes);
                            if f.starts_with("UNKNOWN") {
                                unknown.insert(format!("{} in {} {}", r.target, cand.id, s.key));
                            }
                            *body_forms.entry(f.clone()).or_insert(0) += 1;
                            *forms.entry(f).or_insert(0) += 1;
                            if normalize::is_anon(&r.target) {
                                anon_targets.insert(r.target.clone());
                            }
                        }
                    }
                }
                for (f, n) in &body_forms {
                    let _ = writeln!(out, "     ref ×{n}: {f}");
                }
                for a in &anon_targets {
                    if let Some(loc) = decode_location(llvm, &view, &relocs, a)? {
                        let _ = writeln!(out, "     Location {}:{}:{} via {}", loc.file, loc.line, loc.col, loc.anon);
                    }
                }
                // Every defined engine CODE symbol the body references, by call, tail jump,
                // conditional jump (a cold callee is often reached by `jne`) or address-of.
                let code_refs: BTreeSet<String> = body
                    .parts
                    .iter()
                    .flat_map(|p| p.insns.iter())
                    .flat_map(|i| i.relocs.iter())
                    .filter(|r| classes.get(r.target.as_str()).is_some_and(|c| candidates::is_code(*c)))
                    .map(|r| normalize::norm_target(&view.map, &r.target, &RenameList::default()))
                    .filter(|n| n.contains("boyko_"))
                    .collect();
                for c in code_refs {
                    let _ = writeln!(out, "     code ref {c}");
                }
            }
        }
        built.check_intact()?;
    }
    let _ = writeln!(out, "\n== form histogram over every candidate body");
    for (f, n) in &forms {
        let _ = writeln!(out, "{n:>7}  {f}");
    }
    let _ = writeln!(out, "UNKNOWN forms: {}", if unknown.is_empty() { "none".to_owned() } else { format!("{unknown:?}") });
    let _ = writeln!(out, "ABSENT candidates: {absent:?}");
    let _ = writeln!(
        out,
        "DECISION (ii): every candidate at offset 0 of its section: {all_at_zero}; alone in its section: {all_alone}; aux Length == RawDataSize: {aux_eq_raw}"
    );
    if all_at_zero {
        let _ = writeln!(
            out,
            "DECISION (ii): pin size = the section's RawDataSize, and the pinned body = every symbol of that section (the function plus its SEH funclets), so the disassembly ends at the section end"
        );
    } else {
        let _ = writeln!(out, "DECISION (ii): pin size = distance to the next symbol in the section, or to its end; both numbers are printed per row above");
    }
    Ok(out)
}

/// Probe (iii): does the MSVC `llvm-symbolizer` resolve the inlined `VmColumn::swap_remove` frame
/// inside the `swap_remove/10k` body from a `sensitivity-map` PDB?
///
/// Critique W3: the frame must first be shown INLINED in the chosen body from the object — a panic
/// `Location` from `vm_column.rs` referenced, and no call to a `VmColumn…::swap_remove` symbol —
/// so that a FAIL can only mean "the PDB carries no inline site", never "wrong body".
pub fn probe_iii(ctx: &Ctx, llvm: &Llvm, map_path: &Path) -> Result<String> {
    let symbolizer = tools::symbolizer()?;
    let mut out = header(ctx, llvm, "probe (iii) inline-frame resolution (route a1 vs a2)");
    out.push_str(&symbolizer.receipt_line());
    out.push('\n');
    let s = objbuild::subject("swap_remove")?;
    let extra = vec!["-C".to_owned(), format!("link-arg=/MAP:{}", map_path.display())];
    let built = objbuild::build(ctx, &Request { subject: s, profile: "sensitivity-map", extra_rustc: &extra, emit_obj: true })?;
    out.push_str(&built.receipt());
    let view = ObjView::load(llvm, &built.object)?;
    let relocs = llvm::relocations(&llvm.readobj, &built.object)?;

    let mut chosen: Option<(Body, LocationRef)> = None;
    for rule in CANDIDATES.iter().find(|c| c.id == "H-2").map(|c| c.rules).unwrap_or(&[]) {
        for sym in candidates::locate(&view.syms.defined, *rule) {
            let body = view.body(&llvm.objdump, &sym.raw, &RenameList::default())?;
            let calls_swap = body.parts.iter().flat_map(|p| p.insns.iter()).flat_map(|i| i.relocs.iter()).any(|r| {
                let n = normalize::norm_target(&view.map, &r.target, &RenameList::default());
                n.contains("VmColumn") && n.ends_with("::swap_remove")
            });
            let mut loc = None;
            for part in &body.parts {
                for r in part.insns.iter().flat_map(|i| i.relocs.iter()).filter(|r| normalize::is_anon(&r.target)) {
                    if let Some(l) = decode_location(llvm, &view, &relocs, &r.target)?
                        && l.file.ends_with("vm_column.rs")
                    {
                        loc = Some(l);
                    }
                }
            }
            let _ = writeln!(out, "H-2 body {}: calls a VmColumn::swap_remove symbol: {calls_swap}; vm_column.rs Location: {loc:?}", body.name);
            if let (false, Some(l), None) = (calls_swap, loc, &chosen) {
                chosen = Some((body, l));
            }
        }
    }
    let Some((body, loc)) = chosen else {
        let _ = writeln!(out, "DECISION (iii): NO H-2 body is shown by the object to carry VmColumn::swap_remove inlined; the probe picks no body (W3) and does not read the symbolizer");
        return Ok(out);
    };
    let _ = writeln!(out, "chosen body {} (section len {}), inlined frame witnessed by Location {}:{}", body.name, body.section_len, loc.file, loc.line);

    let map_text = std::fs::read_to_string(map_path).map_err(|e| Red::io(map_path, &e))?;
    let base = map_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Preferred load address is "))
        .and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
        .ok_or_else(|| Red::new(RedKind::Malformed, "the /MAP has no `Preferred load address is` line"))?;
    let mut va: Option<u64> = None;
    let mut map_line = String::new();
    for l in map_text.lines() {
        let mut it = l.split_whitespace();
        let (Some(_secoff), Some(name), Some(rvabase)) = (it.next(), it.next(), it.next()) else { continue };
        if name == body.raw {
            va = u64::from_str_radix(rvabase, 16).ok();
            map_line = l.trim().to_owned();
            break;
        }
    }
    let Some(va) = va else {
        let _ = writeln!(out, "DECISION (iii): the /MAP does not list {} — the body is not locatable in the image by name; route (a2)", body.raw);
        return Ok(out);
    };
    let _ = writeln!(out, "/MAP: {map_line}\nimage base 0x{base:x}, body VA 0x{va:x}, RVA 0x{:x}", va - base);

    let dis = llvm.objdump.output([
        std::ffi::OsString::from("-d"),
        "--no-show-raw-insn".into(),
        format!("--start-address=0x{va:x}").into(),
        format!("--stop-address=0x{:x}", va + body.section_len).into(),
        built.image.as_os_str().to_owned(),
    ])?;
    let mut rvas: Vec<u64> = Vec::new();
    for l in String::from_utf8_lossy(&dis.stdout).lines() {
        if let Some((a, _)) = l.trim_start().split_once(':')
            && let Ok(v) = u64::from_str_radix(a.trim(), 16)
            && v >= va
            && v < va + body.section_len
        {
            rvas.push(v - base);
        }
    }
    let _ = writeln!(out, "instructions in the image range: {}", rvas.len());
    if rvas.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, "no instruction disassembled in the body's image range"));
    }
    let frames = llvm::symbolize(&symbolizer, &built.image, built.pdb.as_deref(), &rvas)?;
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut fns: BTreeMap<String, usize> = BTreeMap::new();
    let mut hit: Option<(u64, Vec<Frame>)> = None;
    let mut outer_names_body = 0usize;
    for (rva, fr) in rvas.iter().zip(&frames) {
        for f in fr {
            files.insert(f.file.clone());
            *fns.entry(f.function.clone()).or_insert(0) += 1;
        }
        if fr.last().is_some_and(|f| f.function.contains("delete_entity")) {
            outer_names_body += 1;
        }
        if hit.is_none() && fr.iter().any(|f| f.function.contains("VmColumn") && f.function.contains("swap_remove") && f.file.ends_with("vm_column.rs")) {
            hit = Some((*rva, fr.clone()));
        }
    }
    let _ = writeln!(out, "addresses whose outermost frame names delete_entity: {outer_names_body} of {}", rvas.len());
    let _ = writeln!(out, "distinct frame functions ({}):", fns.len());
    for (f, n) in &fns {
        let _ = writeln!(out, "  {n:>5}  {f}");
    }
    let _ = writeln!(out, "file map for this body ({} files):", files.len());
    for f in &files {
        let _ = writeln!(out, "  {f}");
    }
    match hit {
        Some((rva, fr)) => {
            let _ = writeln!(out, "PASS: RVA 0x{rva:x} resolves the inlined frame chain:");
            for f in fr {
                let _ = writeln!(out, "  {} @ {}:{}", f.function, f.file, f.line);
            }
            let _ = writeln!(out, "DECISION (iii): route (a1) — the file map comes from the msvc image and its sensitivity-map PDB");
        }
        None => {
            let _ = writeln!(out, "FAIL: no address resolves a VmColumn::swap_remove frame in vm_column.rs, although the object shows it inlined here (W3 precheck passed)");
            let _ = writeln!(out, "DECISION (iii): route (a2) — the PDB carries no usable inline site; the file map comes from a windows-gnu build of the same commit");
        }
    }
    built.check_intact()?;
    Ok(out)
}

/// What probe (iv) compares between two builds of one commit.
struct Capture {
    object_sha: String,
    image_sha: String,
    image: Vec<u8>,
    bodies: BTreeMap<String, String>,
    multiset_sha: String,
}

fn capture(ctx: &Ctx, llvm: &Llvm, s: Subject, extra: &[String]) -> Result<Capture> {
    let built = objbuild::build(ctx, &Request { subject: s, profile: "release", extra_rustc: extra, emit_obj: true })?;
    let view = ObjView::load(llvm, &built.object)?;
    let mut bodies = BTreeMap::new();
    for cand in CANDIDATES.iter().filter(|c| c.subjects.contains(&s.key)) {
        for rule in cand.rules {
            for sym in candidates::locate(&view.syms.defined, *rule) {
                let b = view.body(&llvm.objdump, &sym.raw, &RenameList::default())?;
                bodies.insert(format!("{} {}", cand.id, b.name), b.sha256);
            }
        }
    }
    let ms = view.multiset(&RenameList::default())?;
    let mut ms_text = String::new();
    for ((n, c, z), k) in &ms {
        let _ = writeln!(ms_text, "{k}\t{c}\t{z}\t{n}");
    }
    let image = std::fs::read(&built.image).map_err(|e| Red::io(&built.image, &e))?;
    built.check_intact()?;
    Ok(Capture {
        object_sha: built.object_sha256.clone(),
        image_sha: built.image_sha256.clone(),
        image,
        bodies,
        multiset_sha: sha256::bytes_hex(ms_text.as_bytes()),
    })
}

fn compare_captures(out: &mut String, label: &str, a: &Capture, b: &Capture) -> bool {
    let obj_same = a.object_sha == b.object_sha;
    let bodies_same = a.bodies == b.bodies;
    let ms_same = a.multiset_sha == b.multiset_sha;
    let _ = writeln!(
        out,
        "{label}: object byte-identical {obj_same} ({} / {}); candidate bodies normalised-identical {bodies_same} ({} bodies); (7)(b) multiset identical {ms_same}; image byte-identical {}",
        &a.object_sha[..16],
        &b.object_sha[..16],
        a.bodies.len(),
        a.image_sha == b.image_sha
    );
    if !bodies_same {
        for (k, v) in &a.bodies {
            if b.bodies.get(k) != Some(v) {
                let _ = writeln!(out, "  body differs: {k}");
            }
        }
    }
    if a.image_sha != b.image_sha {
        if let (Some(h), Some(ranges)) = (pe::headers(&a.image), pe::diff_ranges(&a.image, &b.image, 32)) {
            for (s, e) in ranges {
                let _ = writeln!(out, "  image bytes 0x{s:x}..0x{e:x} ({} B) differ: {}", e - s, pe::locate(&h, s));
            }
        } else {
            let _ = writeln!(out, "  image lengths differ: {} / {}", a.image.len(), b.image.len());
        }
    }
    obj_same || (bodies_same && ms_same)
}

/// Probe (iv), P6-1 reproducibility: (A) final-unit rebuilds of each subject; (B) a front-end
/// rebuild of the six census crates, then `swap_remove`; (C) `boyko_demo` and `clear` linked with
/// `/Brepro`, twice. Nothing is deleted: rebuilds are forced by modification time only (O7).
pub fn probe_iv(ctx: &Ctx, llvm: &Llvm, subjects: &[Subject], arms: &str) -> Result<String> {
    let mut out = header(ctx, llvm, "probe (iv) P6-1 reproducibility");
    let mut reproducible = true;
    if arms.contains('A') {
        for &s in subjects {
            let a = capture(ctx, llvm, s, &[])?;
            let b = capture(ctx, llvm, s, &[])?;
            reproducible &= compare_captures(&mut out, &format!("(A) {} final-unit rebuild", s.key), &a, &b);
        }
    }
    if arms.contains('B') {
        let s = objbuild::subject("swap_remove")?;
        let a = capture(ctx, llvm, s, &[])?;
        let touched = objbuild::touch_census_crate_roots(ctx)?;
        let _ = writeln!(out, "(B) touched {} crate roots: {:?}", touched.len(), touched);
        let b = capture(ctx, llvm, s, &[])?;
        reproducible &= compare_captures(&mut out, "(B) swap_remove after a front-end rebuild of the census crates", &a, &b);
    }
    if arms.contains('C') {
        let brepro = vec!["-C".to_owned(), "link-arg=/Brepro".to_owned()];
        for key in ["boyko_demo", "clear"] {
            let s = objbuild::subject(key)?;
            let a = capture(ctx, llvm, s, &brepro)?;
            let b = capture(ctx, llvm, s, &brepro)?;
            let _ = compare_captures(&mut out, &format!("(C) {key} with /Brepro"), &a, &b);
            let _ = writeln!(out, "(C) {key}: /Brepro image byte-identical {}", a.image_sha == b.image_sha);
        }
    }
    let _ = writeln!(
        out,
        "DECISION (iv): {}",
        if reproducible {
            "profile.release is reproducible for the object census (byte-identical objects, or normalised-identical bodies and multisets); legs (2), (7b) and (7)(b) use profile.release (legs 7/7b through seam-census); no [profile.ug15-det]"
        } else {
            "profile.release is NOT reproducible for the census; [profile.ug15-det] is needed with the key the differing bodies point at"
        }
    );
    Ok(out)
}

/// The names probe (v) plants on `u/b3-ctl-0`.
pub const PROBE_V_NAMES: [&str; 5] =
    ["ug15_probe_v_uncalled", "UG15_PROBE_V_TABLE", "ug15_probe_v_target", "UG15_PROBE_V_LIVE", "ug15_probe_v_live_target"];

/// Probe (v): which of the planted items survive fat LTO into the object, and `/OPT:REF` into the
/// image (by `/MAP`), and what the exports and the section deltas against commit A's snapshots are.
///
/// Critique C1: the (b′) reading is decided by NAMED symbols — the static and its pointee must be
/// present in the object — never by a section-size delta, because the live read that keeps them
/// also edits `EcsMaster::new` and moves `.text` on its own.
pub fn probe_v(ctx: &Ctx, llvm: &Llvm, parent_dir: &Path, map_dir: &Path) -> Result<String> {
    let mut out = header(ctx, llvm, "probe (v) survival under fat LTO and /OPT:REF (branch u/b3-ctl-0)");
    for key in ["boyko_demo", "clear"] {
        let s = objbuild::subject(key)?;
        let map = map_dir.join(format!("probe-v-{key}.map"));
        let extra = vec!["-C".to_owned(), format!("link-arg=/MAP:{}", map.display())];
        let built = objbuild::build(ctx, &Request { subject: s, profile: "seam-census", extra_rustc: &extra, emit_obj: true })?;
        out.push_str(&built.receipt());
        let view = ObjView::load(llvm, &built.object)?;
        let map_text = std::fs::read_to_string(&map).map_err(|e| Red::io(&map, &e))?;
        let _ = writeln!(out, "link line has /OPT:REF: {} ; /DEBUG: {}", built.link_args.contains("/OPT:REF"), built.link_args.contains("\"/DEBUG\""));
        for name in PROBE_V_NAMES {
            let hits: Vec<String> = view
                .syms
                .defined
                .iter()
                .filter(|d| d.name.ends_with(name) || d.raw.contains(name))
                .map(|d| format!("{} {} (raw {})", d.class, d.name, d.raw))
                .collect();
            let in_map: Vec<String> = map_text.lines().filter(|l| l.contains(name)).map(|l| l.trim().to_owned()).collect();
            let _ = writeln!(out, "{key} {name}: object {hits:?}; /MAP {in_map:?}");
        }
        let snap_header = vec![format!("subject {key}, profile seam-census, probe (v) child")];
        let child = Snapshot::capture(llvm, &built, snap_header)?;
        let _ = writeln!(out, "{key} exports: {:?}", child.exports);
        let parent_file = parent_dir.join(format!("leg7-{key}-A.snap"));
        match std::fs::read_to_string(&parent_file) {
            Ok(text) => {
                let parent = Snapshot::parse(&text)?;
                let diffs = snapshot::compare(&parent, &child, &RenameList::default());
                let _ = writeln!(out, "{key} leg-(7) diff against {} ({} lines):", parent_file.display(), diffs.len());
                for d in diffs.iter().take(60) {
                    let _ = writeln!(out, "  {d}");
                }
            }
            Err(e) => {
                let _ = writeln!(out, "{key}: no parent snapshot at {} ({e})", parent_file.display());
            }
        }
        built.check_intact()?;
    }
    Ok(out)
}

/// Leg (7b)'s member-format record (PC-7): every census crate rlib cargo reported for `built`.
pub fn rlib_formats(llvm: &Llvm, built: &Built) -> Result<String> {
    let mut out = String::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for (krate, rlib) in &built.rlibs {
        if !seen.insert(rlib.clone()) {
            continue;
        }
        let members = llvm::rlib_members(&llvm.ar, rlib)?;
        let mut by: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for m in &members {
            let e = by.entry(format!("{:?}", m.format)).or_insert((0, 0));
            e.0 += 1;
            e.1 += m.size;
        }
        let _ = writeln!(out, "{krate} {}: {} members {by:?}", rlib.display(), members.len());
    }
    if seen.is_empty() {
        return Err(Red::new(RedKind::MemberCount, "cargo reported no census-crate rlib for this build"));
    }
    Ok(out)
}
