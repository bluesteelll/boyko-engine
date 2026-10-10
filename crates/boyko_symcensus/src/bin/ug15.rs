//! `ug15` — the heavy legs and B3's probes of UG-15 (03 §6). Invoke it directly, from the tree it
//! was compiled from, with the recipe's environment prefix (`CARGO_TARGET_DIR` set):
//!
//! ```text
//! ug15 tools
//! ug15 build  --subject <S> [--profile <P>] [--map <file>] [-- <rustc args>]
//!             [--control-drop-emit-obj | --control-no-touch]   # the instrument's controls O / P
//! ug15 nm     <file>                          # the instrument's control E: RED on a msvc image
//! ug15 members <archive>                     # the instrument's control M: RED [member count] on an rmeta-only archive
//! ug15 rlibs  --subject <S> [--profile <P>]   # leg (7b)'s member formats (PC-7)
//! ug15 leg7 snapshot --subject <S> [--profile <P>] --out <file>
//! ug15 textcheck --subject <S> --seam <snapshot>  # .text of release == seam-census
//! ug15 leg7 compare  <parent> <child> [--rename <file>] [--expect <name>]…  # --expect: a red control
//! ug15 probe i   [--subjects a,b,…] [--out <file>]      # probes (i) and (ii)
//! ug15 probe iii [--out <file>]
//! ug15 probe iv  [--subjects a,b,…] [--arms ABC] [--out <file>]
//! ug15 probe v   --parent-dir <dir> [--out <file>]
//! ug15 leg1 census | deps [--write]                  # leg (1) by hand; `deps --write` re-pins
//! ug15 leg2 capture [--out <file>]                    # B3 only: writes ug15/pins/* and the frozen list
//! ug15 leg2 check [--rename <file>] [--named <file>] [--subjects a,b,…] [--out <file>]
//! ug15 leg7b --subject <S> [--out <file>]             # rlib object census + generated sources
//! ug15 map a capture [--out <file>]                    # writes ug15/maps/file-map.tsv
//! ug15 map b capture --arm <label> --out <file>        # on a layout control branch
//! ug15 map b diff --type <T> <arm1> <arm2>             # rewrites T's rows of ug15/maps/layout-map.tsv
//! ug15 map c capture|check                             # ug15/maps/containment.tsv
//! ```
//!
//! Every command prints what it read and from where; a RED prints `RED [<kind>]: …` and exits 1.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use boyko_symcensus::contain;
use boyko_symcensus::leg7b;
use boyko_symcensus::llvm;
use boyko_symcensus::maps;
use boyko_symcensus::normalize::RenameList;
use boyko_symcensus::pins;
use boyko_symcensus::objbuild::{self, Control, Ctx, Request, Subject};
use boyko_symcensus::objview::{Llvm, ObjView};
use boyko_symcensus::probes;
use boyko_symcensus::source;
use boyko_symcensus::red::{Red, RedKind, Result};
use boyko_symcensus::snapshot::{self, Snapshot};
use boyko_symcensus::tools;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(red) => {
            eprintln!("{red}");
            println!("{red}");
            ExitCode::FAILURE
        }
    }
}

/// The library's build progress ([`objbuild::Echo`]) goes to this binary's stderr.
fn echo_stderr(line: &str) {
    eprintln!("{line}");
}

/// `--name value` from `args`.
fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn required<'a>(args: &'a [String], name: &str) -> Result<&'a str> {
    opt(args, name).ok_or_else(|| Red::new(RedKind::Usage, format!("missing {name}")))
}

fn subjects(args: &[String], default: &[&str]) -> Result<Vec<Subject>> {
    match opt(args, "--subjects") {
        Some(list) => list.split(',').map(objbuild::subject).collect(),
        None => default.iter().map(|k| objbuild::subject(k)).collect(),
    }
}

fn emit(text: &str, out: Option<&str>) -> Result<()> {
    println!("{text}");
    if let Some(path) = out {
        let p = Path::new(path);
        if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| Red::io(parent, &e))?;
        }
        std::fs::write(p, text).map_err(|e| Red::io(p, &e))?;
        eprintln!("ug15: wrote {}", p.display());
    }
    Ok(())
}

fn maps_dir(ctx: &Ctx) -> Result<PathBuf> {
    let d = ctx.target.join("ug15").join("maps");
    std::fs::create_dir_all(&d).map_err(|e| Red::io(&d, &e))?;
    Ok(d)
}

fn run(args: &[String]) -> Result<()> {
    let Some(cmd) = args.first().map(String::as_str) else {
        return Err(Red::new(RedKind::Usage, "no command; see the header of src/bin/ug15.rs"));
    };
    match cmd {
        "tools" => {
            let ctx = Ctx::new(echo_stderr)?;
            let mut text = probes::header(&ctx, &Llvm::resolve(&ctx.host)?, "ug15 tools");
            text.push_str(&tools::symbolizer()?.receipt_line());
            text.push('\n');
            let found = tools::msvc_symbolizers();
            text.push_str(&format!("msvc symbolizers found: {}\n", found.len()));
            for (v, p) in found {
                text.push_str(&format!("  {v:?} {}\n", p.display()));
            }
            emit(&text, opt(args, "--out"))
        }
        "build" => {
            let ctx = Ctx::new(echo_stderr)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            let profile = opt(args, "--profile").unwrap_or("release");
            let mut extra: Vec<String> = args.iter().position(|a| a == "--").map(|i| args[i + 1..].to_vec()).unwrap_or_default();
            if let Some(m) = opt(args, "--map") {
                extra.push("-C".to_owned());
                extra.push(format!("link-arg=/MAP:{m}"));
            }
            let control = if flag(args, "--control-drop-emit-obj") {
                Control::DropEmitObj
            } else if flag(args, "--control-no-touch") {
                Control::NoTouch
            } else {
                Control::None
            };
            let built = objbuild::build(&ctx, &Request { subject: s, profile, extra_rustc: &extra, control })?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let view = ObjView::load(&llvm, &built.object)?;
            let mut text = probes::header(&ctx, &llvm, "ug15 build");
            text.push_str(&built.receipt());
            text.push_str(&format!("symbols      {} defined, {} undefined\n", view.syms.defined.len(), view.syms.undefined.len()));
            built.check_intact()?;
            emit(&text, opt(args, "--out"))
        }
        "nm" => {
            let ctx = Ctx::new(echo_stderr)?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let file = args.get(1).ok_or_else(|| Red::new(RedKind::Usage, "nm <file>"))?;
            let tab = llvm::nm(&llvm.nm, Path::new(file))?;
            emit(&format!("{file}: {} defined, {} undefined symbols", tab.defined.len(), tab.undefined.len()), None)
        }
        "members" => {
            // The instrument's control M: an archive with no object member once `lib.rmeta` is
            // skipped must RED [member count], never read as "a census of zero".
            let ctx = Ctx::new(echo_stderr)?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let file = args.get(1).ok_or_else(|| Red::new(RedKind::Usage, "members <archive>"))?;
            let members = llvm::rlib_members(&llvm.ar, Path::new(file))?;
            let mut text = format!("{file}: {} member(s)\n", members.len());
            for m in &members {
                text.push_str(&format!("  {} {:?} {} B\n", m.name, m.format, m.size));
            }
            emit(&text, None)
        }
        "rlibs" => {
            let ctx = Ctx::new(echo_stderr)?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            let profile = opt(args, "--profile").unwrap_or("seam-census");
            let built = objbuild::build(&ctx, &Request::new(s, profile, &[]))?;
            let mut text = probes::header(&ctx, &llvm, "ug15 rlibs (leg 7b member formats)");
            text.push_str(&built.receipt());
            text.push_str(&probes::rlib_formats(&llvm, &built)?);
            built.check_intact()?;
            emit(&text, opt(args, "--out"))
        }
        "textcheck" => {
            // 03 §6 leg (7): the seam-census profile must change symbols only, so its image's
            // `.text` must equal `release`'s. The seam-census side comes from a leg-(7) snapshot.
            let ctx = Ctx::new(echo_stderr)?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            let seam_file = required(args, "--seam")?;
            let seam = Snapshot::parse(&std::fs::read_to_string(seam_file).map_err(|e| Red::io(Path::new(seam_file), &e))?)?;
            let built = objbuild::build(&ctx, &Request::new(s, "release", &[]))?;
            let rel = llvm::size_a(&llvm.size, &built.image)?;
            built.check_intact()?;
            let mut text = probes::header(&ctx, &llvm, &format!(".text check of {}: release vs seam-census", s.key));
            text.push_str(&built.receipt());
            text.push_str(&format!("{:<10} {:>12} {:>12}\n", "section", "release", "seam-census"));
            for (name, size) in &rel {
                let other = seam.sections.iter().find(|(n, _)| n == name).map(|(_, v)| *v);
                text.push_str(&format!("{name:<10} {size:>12} {:>12}\n", other.map_or_else(|| "-".to_owned(), |v| v.to_string())));
            }
            let get = |rows: &[(String, u64)]| rows.iter().find(|(n, _)| n == ".text").map(|(_, v)| *v);
            let (a, b) = (get(&rel), get(&seam.sections));
            let same = a.is_some() && a == b;
            text.push_str(&format!(".text equal: {same}\n"));
            emit(&text, opt(args, "--out"))?;
            if same {
                Ok(())
            } else {
                Err(Red::new(RedKind::Mismatch, format!(".text differs: release {a:?} vs seam-census {b:?}")))
            }
        }
        "leg7" => leg7(args),
        "leg2" => leg2(args),
        "leg1" => leg1(args),
        "leg7b" => {
            let ctx = Ctx::new(echo_stderr)?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            emit(&leg7b::run(&ctx, &llvm, s)?, opt(args, "--out"))
        }
        "map" => map_cmd(args),
        "probe" => probe_cmd(args),
        other => Err(Red::new(RedKind::Usage, format!("unknown command `{other}`"))),
    }
}

fn leg7(args: &[String]) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("snapshot") => {
            let ctx = Ctx::new(echo_stderr)?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            let profile = opt(args, "--profile").unwrap_or("seam-census");
            let out = required(args, "--out")?;
            // Read before the build, so a pin set that is not the frozen one REDs [pin set] at once
            // (B3 review W1): a missing pin file is never "nothing to check".
            let pinned = pins::read_pins(&ctx.root, s.key)?;
            let map = maps_dir(&ctx)?.join(format!("leg7-{}-{profile}.map", s.key));
            let extra = vec!["-C".to_owned(), format!("link-arg=/MAP:{}", map.display())];
            let built = objbuild::build(&ctx, &Request::new(s, profile, &extra))?;
            let mut header: Vec<String> = probes::header(&ctx, &llvm, &format!("leg (7) snapshot of {} under {profile}", s.key))
                .lines()
                .map(str::to_owned)
                .collect();
            header.extend(built.receipt().lines().map(str::to_owned));
            let snap = Snapshot::capture(&llvm, &built, header)?;
            // Anti-vacuity (03 §6 leg 7): every leg-(2) pin of this subject is present in (b).
            // `None` only when the frozen list holds no pin of this subject (`read_pins`).
            match pinned {
                Some(file) => {
                    let names: BTreeSet<&str> = snap.multiset.keys().map(|(n, _, _)| n.as_str()).collect();
                    let missing: Vec<&str> = file.pins.iter().map(|p| p.name.as_str()).filter(|n| !names.contains(n)).collect();
                    if !missing.is_empty() {
                        return Err(Red::new(RedKind::SymbolAbsent, format!("leg (7): leg-(2) pin(s) of {} absent from the object: {missing:?}", s.key)));
                    }
                    println!("leg (7): all {} leg-(2) pin(s) of {} present in (b)", file.pins.len(), s.key);
                }
                None => println!("leg (7): the frozen leg-(2) set holds no pin of {}, so there is no presence to check", s.key),
            }
            let text = snap.to_text();
            let p = Path::new(out);
            if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| Red::io(parent, &e))?;
            }
            std::fs::write(p, &text).map_err(|e| Red::io(p, &e))?;
            let total: usize = snap.multiset.values().sum();
            println!(
                "leg (7) snapshot {}: {} section rows, {} distinct / {total} symbols, {} exports -> {}",
                s.key,
                snap.sections.len(),
                snap.multiset.len(),
                snap.exports.len(),
                p.display()
            );
            Ok(())
        }
        Some("compare") => {
            let (Some(a), Some(b)) = (args.get(2), args.get(3)) else {
                return Err(Red::new(RedKind::Usage, "leg7 compare <parent> <child> [--rename <file>]"));
            };
            let read = |p: &str| std::fs::read_to_string(p).map_err(|e| Red::io(Path::new(p), &e));
            let parent = Snapshot::parse(&read(a)?)?;
            let child = Snapshot::parse(&read(b)?)?;
            let rename = match opt(args, "--rename") {
                Some(f) => RenameList::parse(&read(f)?)?,
                None => RenameList::default(),
            };
            let diffs = snapshot::compare(&parent, &child, &rename);
            let total: usize = parent.multiset.values().sum();
            println!("leg (7) compare: parent {a} ({total} symbols), child {b}, rename entries {}", rename.0.len());
            let expect: Vec<&str> = args.iter().enumerate().filter(|(_, x)| *x == "--expect").filter_map(|(i, _)| args.get(i + 1)).map(String::as_str).collect();
            if !expect.is_empty() {
                // A red CONTROL (critique C1): the diff must name each expected survivor among the
                // child-only (b) entries. A section-size-only or unrelated-symbol diff is a FAILED
                // control, because the edit that keeps a survivor alive moves `.text` by itself.
                for d in &diffs {
                    println!("  {d}");
                }
                let missing: Vec<&str> =
                    expect.iter().copied().filter(|e| !diffs.iter().any(|d| d.starts_with("(b) child only") && d.contains(e))).collect();
                return if missing.is_empty() {
                    println!("control: leg (7) RED on the named survivor(s) {expect:?} — the control PASSES");
                    Ok(())
                } else {
                    Err(Red::new(
                        RedKind::Mismatch,
                        format!("control FAILED: leg (7)(b) has no child-only entry naming {missing:?} ({} diff lines)", diffs.len()),
                    ))
                };
            }
            if diffs.is_empty() {
                println!("leg (7): (a), (b) and (c) identical");
                Ok(())
            } else {
                for d in &diffs {
                    println!("  {d}");
                }
                Err(Red::new(RedKind::Mismatch, format!("leg (7): {} difference(s) between {a} and {b}", diffs.len())))
            }
        }
        _ => Err(Red::new(RedKind::Usage, "leg7 snapshot|compare")),
    }
}

fn probe_cmd(args: &[String]) -> Result<()> {
    let ctx = Ctx::new(echo_stderr)?;
    let llvm = Llvm::resolve(&ctx.host)?;
    let out = opt(args, "--out");
    match args.get(1).map(String::as_str) {
        Some("i") | Some("ii") => {
            let subs = subjects(args, &["swap_remove", "boyko_demo", "clear", "query_dsl", "phase9_scheduler"])?;
            emit(&probes::probe_i_ii(&ctx, &llvm, &subs)?, out)
        }
        Some("iii") => {
            let map = maps_dir(&ctx)?.join("probe-iii-swap_remove.map");
            emit(&probes::probe_iii(&ctx, &llvm, &map)?, out)
        }
        Some("iv") => {
            let subs = subjects(args, &["swap_remove", "query_dsl", "phase9_scheduler", "boyko_demo", "clear"])?;
            emit(&probes::probe_iv(&ctx, &llvm, &subs, opt(args, "--arms").unwrap_or("ABC"))?, out)
        }
        Some("v") => {
            let parent = required(args, "--parent-dir")?;
            let maps = maps_dir(&ctx)?;
            emit(&probes::probe_v(&ctx, &llvm, Path::new(parent), &maps)?, out)
        }
        _ => Err(Red::new(RedKind::Usage, "probe i|iii|iv|v")),
    }
}

fn leg2(args: &[String]) -> Result<()> {
    let ctx = Ctx::new(echo_stderr)?;
    let llvm = Llvm::resolve(&ctx.host)?;
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| Red::io(Path::new(p), &e));
    match args.get(1).map(String::as_str) {
        Some("capture") => emit(&pins::capture(&ctx, &llvm)?, opt(args, "--out")),
        Some("check") => {
            let rename = match opt(args, "--rename") {
                Some(f) => RenameList::parse(&read(f)?)?,
                None => RenameList::default(),
            };
            let named = match opt(args, "--named") {
                Some(f) => pins::parse_named(&read(f)?)?,
                None => Vec::new(),
            };
            let subs = subjects(args, &["boyko_demo", "clear", "swap_remove", "query_dsl", "phase9_scheduler"])?;
            match pins::check(&ctx, &llvm, &rename, &named, &subs) {
                Ok(text) => emit(&text, opt(args, "--out")),
                Err(red) => {
                    if let Some(p) = opt(args, "--out") {
                        let _ = std::fs::write(p, red.to_string());
                    }
                    Err(red)
                }
            }
        }
        _ => Err(Red::new(RedKind::Usage, "leg2 capture|check")),
    }
}

fn map_cmd(args: &[String]) -> Result<()> {
    let ctx = Ctx::new(echo_stderr)?;
    let data = pins::data_dir(&ctx.root).join("maps");
    std::fs::create_dir_all(&data).map_err(|e| Red::io(&data, &e))?;
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| Red::io(Path::new(p), &e));
    match (args.get(1).map(String::as_str), args.get(2).map(String::as_str)) {
        (Some("a"), Some("capture")) => {
            let llvm = Llvm::resolve(&ctx.host)?;
            let symbolizer = tools::symbolizer()?;
            let mut text = probes::header(&ctx, &llvm, "sensitivity map (a) capture (route a1)");
            text.push_str(&symbolizer.receipt_line());
            text.push('\n');
            let subs: Vec<Subject> = maps::committed_pins(&ctx.root)?.into_iter().map(|(s, _)| s).collect();
            let m = maps::capture_a(&ctx, &llvm, &symbolizer, &subs, &mut text)?;
            let path = data.join("file-map.tsv");
            let tsv = maps::file_map_tsv(&m);
            std::fs::write(&path, &tsv).map_err(|e| Red::io(&path, &e))?;
            // The absent list's carrier column: a pinned body whose inline frames name the candidate.
            let ap = pins::absent_path(&ctx.root);
            let mut absent = pins::parse_absent(&read(&ap.to_string_lossy())?)?;
            for a in &mut absent {
                let needle = absent_function(&a.cand);
                let carriers: Vec<String> = m
                    .iter()
                    .filter(|((subj, _, _), pm)| {
                        *subj == a.subject && pm.as_ref().is_some_and(|pm| pm.functions.iter().any(|f| needle.is_some_and(|n| f.contains(n))))
                    })
                    .map(|((_, cand, name), _)| format!("{cand} `{name}`"))
                    .collect();
                let base = a.reason.split(" | carrier").next().unwrap_or(&a.reason).to_owned();
                a.reason = if carriers.is_empty() {
                    format!("{base} | carrier: no pinned body of this subject lists it among its inline frames (map (a))")
                } else {
                    format!("{base} | carrier per map (a): {}", carriers.join("; "))
                };
            }
            std::fs::write(&ap, pins::absent_text(&absent)).map_err(|e| Red::io(&ap, &e))?;
            text.push_str(&format!("wrote {} ({} rows) and the carrier column of {}\n", path.display(), tsv.lines().count().saturating_sub(2), ap.display()));
            emit(&text, opt(args, "--out"))
        }
        (Some("b"), Some("capture")) => {
            let llvm = Llvm::resolve(&ctx.host)?;
            let label = required(args, "--arm")?;
            let out = required(args, "--out")?;
            let subs: Vec<Subject> = maps::committed_pins(&ctx.root)?.into_iter().map(|(s, _)| s).collect();
            let body = maps::capture_b_arm(&ctx, &llvm, &subs, label)?;
            let head: String = probes::header(&ctx, &llvm, &format!("map (b) arm {label}")).lines().map(|l| format!("# {l}\n")).collect();
            emit(&format!("{head}{body}"), Some(out))
        }
        (Some("b"), Some("diff")) => {
            let ty = required(args, "--type")?;
            let rest: Vec<&String> = args.iter().skip(3).filter(|a| !a.starts_with("--") && a.as_str() != ty).collect();
            let (Some(a), Some(b)) = (rest.first(), rest.get(1)) else {
                return Err(Red::new(RedKind::Usage, "map b diff --type <T> <arm1> <arm2>"));
            };
            let rows = maps::diff_b(ty, &read(a)?, &read(b)?, &data.join("layout-map.tsv"))?;
            let mut text = format!("map (b) {ty}: {} pin(s) differ between {a} and {b}\n", rows.len());
            for r in &rows {
                text.push_str(&format!("  {r}\n"));
            }
            emit(&text, opt(args, "--out"))
        }
        (Some("c"), Some(which @ ("capture" | "check"))) => {
            let c = contain::capture(&ctx.root)?;
            let tsv = contain::to_tsv(&c);
            let path = data.join("containment.tsv");
            let mut text = format!("map (c): {} file(s) walked, {} type definition(s), {} row(s)\n", c.files, c.defs, c.rows.len());
            if which == "capture" {
                std::fs::write(&path, &tsv).map_err(|e| Red::io(&path, &e))?;
                text.push_str(&format!("wrote {}\n", path.display()));
                return emit(&text, opt(args, "--out"));
            }
            let committed = read(&path.to_string_lossy())?.replace("\r\n", "\n");
            if committed == tsv {
                text.push_str("map (c): identical to the committed map\n");
                return emit(&text, opt(args, "--out"));
            }
            let a: BTreeSet<&str> = committed.lines().collect();
            let b: BTreeSet<&str> = tsv.lines().collect();
            for l in a.difference(&b) {
                text.push_str(&format!("  committed only: {l}\n"));
            }
            for l in b.difference(&a) {
                text.push_str(&format!("  tree only:      {l}\n"));
            }
            emit(&text, opt(args, "--out"))?;
            Err(Red::new(RedKind::Mismatch, "map (c) differs from the committed map; re-capture it in this rung's merge"))
        }
        _ => Err(Red::new(RedKind::Usage, "map a capture | map b capture|diff | map c capture|check")),
    }
}

/// The function name a candidate's inlined frames carry, for the absent list's carrier column.
fn absent_function(cand: &str) -> Option<&'static str> {
    Some(match cand {
        "R-1" => "register_new<",
        "R-2" => "component_id",
        "D-2" | "D-3" => "register_new",
        "D-4" => "resource_registry::register_new",
        "D-5" => "register_event_new",
        "R-3" => "try_register_dynamic",
        "R-4" => "register_layout",
        "S-1" => "add_system",
        "H-3" => "Schedule::run",
        "P29-1" => "grow_rows",
        "P29-2" => "commit_subregion",
        "P29-3" => "run_check_ticks_scan",
        "P29-4" => "ScopeBlock::grow",
        _ => return None,
    })
}

fn leg1(args: &[String]) -> Result<()> {
    let root = boyko_symcensus::host::workspace_root();
    boyko_symcensus::host::check_cwd()?;
    match args.get(1).map(String::as_str) {
        Some("census") => {
            let (findings, stats, per) = source::census(&root)?;
            let mut text = format!("leg (1) read: {stats}\n");
            for (c, s) in &per {
                text.push_str(&format!("  {c}: {s}\n"));
            }
            text.push_str(&format!("counted: {}\n", findings.len()));
            for f in &findings {
                text.push_str(&format!("  {f}\n"));
            }
            emit(&text, opt(args, "--out"))
        }
        Some("deps") => {
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let rows = source::census_crate_deps(&root, &cargo)?;
            let mut text = String::from(
                "# UG-15 leg (1) dependency pin (critique W6): every direct dependency of every census crate, by kind.\n\
                 # A dependency's macros emit tokens into the invoking crate, where the source walk cannot see them;\n\
                 # a change here is RED until someone has read what the new dependency emits. Re-pin: `ug15 leg1 deps --write`.\n\
                 # crate\tdependency\tkind\ttarget\n",
            );
            for r in &rows {
                text.push_str(&format!("{r}\n"));
            }
            if flag(args, "--write") {
                let p = pins::data_dir(&root).join("pins").join("leg1-deps.pins");
                if let Some(d) = p.parent() {
                    std::fs::create_dir_all(d).map_err(|e| Red::io(d, &e))?;
                }
                std::fs::write(&p, &text).map_err(|e| Red::io(&p, &e))?;
                eprintln!("ug15: wrote {}", p.display());
            }
            emit(&text, None)
        }
        _ => Err(Red::new(RedKind::Usage, "leg1 census|deps [--write]")),
    }
}
