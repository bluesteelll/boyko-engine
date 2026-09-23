//! `ug15` — the heavy legs and B3's probes of UG-15 (03 §6). Invoke it directly, from the tree it
//! was compiled from, with the recipe's environment prefix (`CARGO_TARGET_DIR` set):
//!
//! ```text
//! ug15 tools
//! ug15 build  --subject <S> [--profile <P>] [--map <file>] [-- <rustc args>]
//!             [--control-drop-emit-obj | --control-no-touch]   # the instrument's controls O / P
//! ug15 nm     <file>                          # the instrument's control E: RED on a msvc image
//! ug15 rlibs  --subject <S> [--profile <P>]   # leg (7b)'s member formats (PC-7)
//! ug15 leg7 snapshot --subject <S> [--profile <P>] --out <file>
//! ug15 textcheck --subject <S> --seam <snapshot>  # .text of release == seam-census
//! ug15 leg7 compare  <parent> <child> [--rename <file>] [--expect <name>]…  # --expect: a red control
//! ug15 probe i   [--subjects a,b,…] [--out <file>]      # probes (i) and (ii)
//! ug15 probe iii [--out <file>]
//! ug15 probe iv  [--subjects a,b,…] [--arms ABC] [--out <file>]
//! ug15 probe v   --parent-dir <dir> [--out <file>]
//! ```
//!
//! Every command prints what it read and from where; a RED prints `RED [<kind>]: …` and exits 1.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use boyko_symcensus::llvm;
use boyko_symcensus::normalize::RenameList;
use boyko_symcensus::objbuild::{self, Control, Ctx, Request, Subject};
use boyko_symcensus::objview::{Llvm, ObjView};
use boyko_symcensus::probe;
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
            let ctx = Ctx::new()?;
            let mut text = probe::header(&ctx, &Llvm::resolve(&ctx.host)?, "ug15 tools");
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
            let ctx = Ctx::new()?;
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
            let mut text = probe::header(&ctx, &llvm, "ug15 build");
            text.push_str(&built.receipt());
            text.push_str(&format!("symbols      {} defined, {} undefined\n", view.syms.defined.len(), view.syms.undefined.len()));
            built.check_intact()?;
            emit(&text, opt(args, "--out"))
        }
        "nm" => {
            let ctx = Ctx::new()?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let file = args.get(1).ok_or_else(|| Red::new(RedKind::Usage, "nm <file>"))?;
            let tab = llvm::nm(&llvm.nm, Path::new(file))?;
            emit(&format!("{file}: {} defined, {} undefined symbols", tab.defined.len(), tab.undefined.len()), None)
        }
        "rlibs" => {
            let ctx = Ctx::new()?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            let profile = opt(args, "--profile").unwrap_or("seam-census");
            let built = objbuild::build(&ctx, &Request::new(s, profile, &[]))?;
            let mut text = probe::header(&ctx, &llvm, "ug15 rlibs (leg 7b member formats)");
            text.push_str(&built.receipt());
            text.push_str(&probe::rlib_formats(&llvm, &built)?);
            built.check_intact()?;
            emit(&text, opt(args, "--out"))
        }
        "textcheck" => {
            // 03 §6 leg (7): the seam-census profile must change symbols only, so its image's
            // `.text` must equal `release`'s. The seam-census side comes from a leg-(7) snapshot.
            let ctx = Ctx::new()?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            let seam_file = required(args, "--seam")?;
            let seam = Snapshot::parse(&std::fs::read_to_string(seam_file).map_err(|e| Red::io(Path::new(seam_file), &e))?)?;
            let built = objbuild::build(&ctx, &Request::new(s, "release", &[]))?;
            let rel = llvm::size_a(&llvm.size, &built.image)?;
            built.check_intact()?;
            let mut text = probe::header(&ctx, &llvm, &format!(".text check of {}: release vs seam-census", s.key));
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
        "probe" => probe_cmd(args),
        other => Err(Red::new(RedKind::Usage, format!("unknown command `{other}`"))),
    }
}

fn leg7(args: &[String]) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("snapshot") => {
            let ctx = Ctx::new()?;
            let llvm = Llvm::resolve(&ctx.host)?;
            let s = objbuild::subject(required(args, "--subject")?)?;
            let profile = opt(args, "--profile").unwrap_or("seam-census");
            let out = required(args, "--out")?;
            let map = maps_dir(&ctx)?.join(format!("leg7-{}-{profile}.map", s.key));
            let extra = vec!["-C".to_owned(), format!("link-arg=/MAP:{}", map.display())];
            let built = objbuild::build(&ctx, &Request::new(s, profile, &extra))?;
            let mut header: Vec<String> = probe::header(&ctx, &llvm, &format!("leg (7) snapshot of {} under {profile}", s.key))
                .lines()
                .map(str::to_owned)
                .collect();
            header.extend(built.receipt().lines().map(str::to_owned));
            let snap = Snapshot::capture(&llvm, &built, header)?;
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
    let ctx = Ctx::new()?;
    let llvm = Llvm::resolve(&ctx.host)?;
    let out = opt(args, "--out");
    match args.get(1).map(String::as_str) {
        Some("i") | Some("ii") => {
            let subs = subjects(args, &["swap_remove", "boyko_demo", "clear", "query_dsl", "phase9_scheduler"])?;
            emit(&probe::probe_i_ii(&ctx, &llvm, &subs)?, out)
        }
        Some("iii") => {
            let map = maps_dir(&ctx)?.join("probe-iii-swap_remove.map");
            emit(&probe::probe_iii(&ctx, &llvm, &map)?, out)
        }
        Some("iv") => {
            let subs = subjects(args, &["swap_remove", "query_dsl", "phase9_scheduler", "boyko_demo", "clear"])?;
            emit(&probe::probe_iv(&ctx, &llvm, &subs, opt(args, "--arms").unwrap_or("ABC"))?, out)
        }
        Some("v") => {
            let parent = required(args, "--parent-dir")?;
            let maps = maps_dir(&ctx)?;
            emit(&probe::probe_v(&ctx, &llvm, Path::new(parent), &maps)?, out)
        }
        _ => Err(Red::new(RedKind::Usage, "probe i|iii|iv|v")),
    }
}
