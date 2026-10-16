//! IB P0r, V11's pure pieces (no cargo build): the closed normalisation list of `ug15
//! unit-compare` (O17's order, (d3)'s list), the instrument key, the retain directory's refusals,
//! the tag vocabulary, (f)'s diagnostic match, the host/target premise, the receipt format,
//! ruling 57's codegen-units (the profile chain and the final-line check), and the controls'
//! package key and scratch edits.
//!
//! Each assertion is the one its red-first mutation turned red (recorded in the commit message).

use std::path::{Path, PathBuf};

use boyko_symcensus::controls;
use boyko_symcensus::key;
use boyko_symcensus::objbuild::{self, CodegenUnits};
use boyko_symcensus::red::RedKind;
use boyko_symcensus::retain::{self, RetainDir};
use boyko_symcensus::unit::{self, ClosureRow, LibLine, Receipt};

/// O17: the target dir lies under the root, so the LONGER string must be substituted first.
/// Root-first would leave `<R>/target` here and `<T>` on a tree whose target dir is elsewhere.
#[test]
fn d2_substitutes_the_longer_path_first() {
    let (root, target) = ("D:/x", "D:/x/target");
    let line = "--out-dir 'D:/x/target\\release\\deps' -L 'dependency=D:\\x\\target\\release\\deps' 'D:/x/crates/a/src/lib.rs'";
    assert_eq!(
        unit::normalize_d2(line, target, root),
        "--out-dir '<T>\\release\\deps' -L 'dependency=<T>\\release\\deps' '<R>/crates/a/src/lib.rs'"
    );
    // The same tree token in the other separator form, and a target dir outside the root.
    assert_eq!(unit::normalize_d2("D:\\t\\release D:/r/x", "D:/t", "D:\\r"), "<T>\\release <R>/x");
}

/// (d3)'s closed list: exactly `-C metadata`, `-C extra-filename`, the pin features and the one
/// `/MAP:` value; anything else stays a difference.
#[test]
fn d3_normalises_exactly_the_closed_list() {
    let (t, r) = ("D:/tgt", "D:/root");
    let a = "Running `rustc --crate-name jolt_parity_pyramid --cfg 'feature=\"ib-pin-s4b1x2f\"' --check-cfg 'cfg(feature, values(\"bench-alloc\", \"ib-pin-s4b1x2f\", \"narrowphase-counts\"))' -C metadata=aaaa -C extra-filename=-aaaa --extern 'boyko_app=D:/tgt\\release\\deps\\libboyko_app-1111.rlib' -C 'link-arg=/MAP:D:/ret/pre/runner/runner.map' -C target-cpu=x86-64-v3`";
    let b = "Running `rustc --crate-name jolt_parity_pyramid --check-cfg 'cfg(feature, values(\"bench-alloc\", \"narrowphase-counts\"))' -C metadata=bbbb -C extra-filename=-bbbb --extern 'boyko_app=D:/tgt\\release\\deps\\libboyko_app-1111.rlib' -C 'link-arg=/MAP:D:/ret/tip/runner-x/runner-x.map' -C target-cpu=x86-64-v3`";
    let na = unit::normalize_d3(a, t, r).expect("one /MAP token");
    let nb = unit::normalize_d3(b, t, r).expect("one /MAP token");
    assert_eq!(na, nb);
    assert!(na.contains("--extern 'boyko_app=<T>\\release\\deps\\libboyko_app-1111.rlib'"), "{na}");
    assert!(na.contains("-C 'link-arg=/MAP:<MAP>'"), "{na}");

    // An unknown `-C` difference stays red.
    let c = b.replace("-C target-cpu=x86-64-v3", "-C target-cpu=x86-64-v2");
    let nc = unit::normalize_d3(&c, t, r).expect("one /MAP token");
    assert_ne!(nb, nc);
    assert_eq!(unit::word_difference(&nc, &nb), vec!["target-cpu=x86-64-v2`".to_owned()]);
    // An `--extern` file name is never normalised (O13).
    let d = b.replace("libboyko_app-1111", "libboyko_app-2222");
    assert_ne!(nb, unit::normalize_d3(&d, t, r).expect("one /MAP token"));
    // Exactly one `/MAP:` token per side.
    let two = b.replace("-C target-cpu", "-C 'link-arg=/MAP:D:/elsewhere.map' -C target-cpu");
    assert_eq!(unit::normalize_d3(&two, t, r).map_err(|e| e.kind), Err(RedKind::Mismatch));
    let none = b.replace("-C 'link-arg=/MAP:D:/ret/tip/runner-x/runner-x.map' ", "");
    assert_eq!(unit::normalize_d3(&none, t, r).map_err(|e| e.kind), Err(RedKind::Mismatch));
}

fn files(pairs: &[(&str, &[u8])]) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = pairs.iter().map(|(p, b)| ((*p).to_owned(), b.to_vec())).collect();
    key::sort_key_order(&mut v);
    v
}

/// The key hashes `\r\n`-normalised source, so a CRLF checkout keys as an LF one; one changed
/// byte, one changed toolchain line, or a file on disk the executable does not carry, does not.
#[test]
fn the_key_is_line_ending_blind_and_byte_sensitive() {
    let lf = files(&[("Cargo.toml", b"[package]\nname = \"x\"\n"), ("src/lib.rs", b"fn a() {}\n")]);
    let crlf = files(&[("Cargo.toml", b"[package]\r\nname = \"x\"\r\n"), ("src/lib.rs", b"fn a() {}\r\n")]);
    let edited = files(&[("Cargo.toml", b"[package]\nname = \"x\"\n"), ("src/lib.rs", b"fn b() {}\n")]);
    let k = |f: &[(String, Vec<u8>)], rustc: &str| key::key_hex(f, rustc, "cargo 1.98.1");
    assert_eq!(k(&lf, "rustc 1.98.1"), k(&crlf, "rustc 1.98.1\r\n"));
    assert_ne!(k(&lf, "rustc 1.98.1"), k(&edited, "rustc 1.98.1"));
    assert_ne!(k(&lf, "rustc 1.98.1"), k(&lf, "rustc 1.99.0"));
    assert_eq!(key::source_digest(&lf).0, key::source_digest(&crlf).0);
    // `Cargo.toml` sorts first whatever the other names are.
    assert_eq!(files(&[("src/a.rs", b""), ("Cargo.toml", b"")])[0].0, "Cargo.toml");

    assert!(key::check_fresh(&lf, &crlf).is_ok());
    assert_eq!(key::check_fresh(&lf, &edited).map_err(|e| e.kind), Err(RedKind::Stale));
    let extra = files(&[("Cargo.toml", b"[package]\nname = \"x\"\n"), ("src/lib.rs", b"fn a() {}\n"), ("src/new.rs", b"")]);
    assert_eq!(key::check_fresh(&lf, &extra).map_err(|e| e.kind), Err(RedKind::Stale));
    assert_eq!(key::check_fresh(&extra, &lf).map_err(|e| e.kind), Err(RedKind::Stale));
}

/// The executable's compiled sources are the tree's: this test binary links the very library the
/// `ug15` it sits beside was built from.
#[test]
fn the_compiled_sources_are_this_trees() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    key::check_fresh(&key::compiled(), &key::on_disk(crate_dir).expect("the crate's sources are readable"))
        .expect("every file under src/ is listed in key::COMPILED_SOURCES and unedited");
}

/// The `Running` line's compiler program, plain or single-quoted.
#[test]
fn the_running_program_is_read_from_the_line() {
    assert_eq!(key::running_program("     Running `C:\\t\\bin\\rustc.exe --crate-name a`").as_deref(), Some("C:\\t\\bin\\rustc.exe"));
    assert_eq!(key::running_program("Running `'C:\\Program Files\\rustc.exe' --crate-name a`").as_deref(), Some("C:\\Program Files\\rustc.exe"));
    assert_eq!(key::running_program("Compiling a v0.1.0"), None);
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ug15-pure-{}-{name}", std::process::id()))
}

/// A retain directory is absolute, outside the tree and the target dir, and empty or absent.
#[test]
fn the_retain_directory_refuses_every_unsafe_place() {
    let base = scratch("retain");
    let tree = base.join("tree");
    let target = base.join("target");
    let usage = |p: &Path| RetainDir::open(p, &tree, &target).map(|_| ()).map_err(|e| e.kind);
    assert_eq!(usage(Path::new("relative/retained")), Err(RedKind::Usage));
    assert_eq!(usage(&tree.join("retained")), Err(RedKind::Usage));
    assert_eq!(usage(&target.join("retained")), Err(RedKind::Usage));
    let full = base.join("full");
    std::fs::create_dir_all(&full).expect("scratch dir");
    std::fs::write(full.join("x"), b"x").expect("scratch file");
    assert_eq!(usage(&full), Err(RedKind::Usage));
    let fresh = base.join("fresh");
    let rd = RetainDir::open(&fresh, &tree, &target).expect("an absent directory outside both is accepted");
    assert!(fresh.is_dir());
    rd.tag_dir("runner").expect("a first tag");
    assert_eq!(rd.tag_dir("runner").map_err(|e| e.kind), Err(RedKind::Usage), "a tag already present is RED");
    std::fs::remove_dir_all(&base).expect("scratch cleanup");
    assert!(retain::is_within(Path::new("D:\\A\\b\\c"), Path::new("d:/a/b")) == cfg!(windows));
    assert!(!retain::is_within(Path::new("D:/a/bc"), Path::new("D:/a/b")));
}

/// The tag vocabulary is closed.
#[test]
fn tags_come_from_the_closed_vocabulary() {
    for ok in ["runner", "probe", "probe-wide", "runner-s4b1x2f", "probe-s4b1x2f", "rc11c-probe", "rc14a-runner", "plain-runner", "plain-rc14b-runner"] {
        assert!(retain::check_tag(ok).is_ok(), "{ok}");
    }
    for bad in ["", "foo", "runner-", "rc11-runner", "rc1c-probe", "rc11C-probe", "plain-", "plain-foo", "plain-plain-runner", "probe-wide-x", "Runner"] {
        assert_eq!(retain::check_tag(bad).map_err(|e| e.kind), Err(RedKind::Usage), "{bad:?}");
    }
}

/// (f) needs a positive match on the lint's own text; ANSI codes do not hide it.
#[test]
fn the_host_unused_diagnostic_is_matched_positively() {
    // Codes inside the message as well as around it: a styled crate name must not hide the match.
    let ansi = vec!["\u{1b}[0m\u{1b}[1m\u{1b}[33mwarning\u{1b}[0m\u{1b}[0m\u{1b}[1m: extern crate `\u{1b}[1mboyko_app\u{1b}[0m` is unused in crate `jolt_parity_pyramid`\u{1b}[0m".to_owned()];
    assert!(unit::host_unused_line(&ansi).is_some());
    assert!(unit::host_unused_line(&["warning: unused import: `x`".to_owned()]).is_none(), "absent");
    assert!(unit::host_unused_line(&["warning: extern crate `boyko_app` is unused in crate `physics_probe`".to_owned()]).is_none(), "another crate");
    assert!(unit::host_unused_line(&["warning: extern crate `boyko_render` is unused in crate `jolt_parity_pyramid`".to_owned()]).is_none(), "another extern");
    assert!(unit::host_unused_line(&[]).is_none(), "an allow leaves nothing to match");
}

/// The host/target split is read from (0, 3) only, and a profile override that would move it is
/// refused up front (W14, O8).
#[test]
fn the_host_split_and_its_premise() {
    assert_eq!(retain::is_host("0", "3").ok(), Some(true));
    assert_eq!(retain::is_host("3", "3").ok(), Some(false));
    assert_eq!(retain::is_host("1", "3").map_err(|e| e.kind), Err(RedKind::Malformed));
    assert_eq!(retain::is_host("0", "0").map_err(|e| e.kind), Err(RedKind::Malformed));
    assert!(retain::check_profile_premise("[profile.release]\nlto = \"fat\"\n[profile.parity]\ninherits = \"release\"\n").is_ok());
    assert!(retain::check_profile_premise("[profile.release.build-override]\nopt-level = 3\n").is_err());
    assert!(retain::check_profile_premise("[profile.release.package.\"*\"]\nopt-level = 2\n").is_err());
}

/// Ruling 57: an IB build's final unit gets the profile's effective codegen-units. That is the
/// profile's own value, else the nearest one up `inherits`, else rustc's non-incremental 16.
#[test]
fn the_effective_codegen_units_follow_the_profile_chain() {
    let cgu = |text: &str, p: &str| retain::profile_codegen_units(text, p).map(|c| (c.n, c.set_by_profile)).map_err(|e| e.kind);
    let shaped = "[workspace]\nmembers = []\n[profile.release]\nlto = \"fat\"\n# codegen-units = 1\n[profile.bench]\ncodegen-units = 1 # explicit\nlto = false\n[profile.parity] # the runner's\ninherits = \"release\"\n[[bin]]\nname = \"x\"\ncodegen-units = 3\n";
    assert_eq!(cgu(shaped, "release"), Ok((16, false)));
    assert_eq!(cgu(shaped, "parity"), Ok((16, false)));
    assert_eq!(cgu(shaped, "bench"), Ok((1, true)));
    assert_eq!(cgu(shaped, "dev"), Ok((16, false)));
    // RC-11 (e)'s edit: the value is inherited by every profile below `release`.
    let rc11e = shaped.replace("lto = \"fat\"\n", "lto = \"fat\"\ncodegen-units = 8\n");
    assert_eq!(cgu(&rc11e, "release"), Ok((8, true)));
    assert_eq!(cgu(&rc11e, "parity"), Ok((8, true)));
    assert_eq!(cgu("[profile.a]\ninherits = \"bench\"\n[profile.bench]\ncodegen-units = 4\n", "a"), Ok((4, true)));
    assert_eq!(cgu("[profile.a]\nlto = true\n", "a"), Err(RedKind::Malformed));
    assert_eq!(cgu("[profile.a]\ninherits = \"b\"\n[profile.b]\ninherits = \"a\"\n", "a"), Err(RedKind::Malformed));
    assert_eq!(cgu("[profile.release]\ncodegen-units = 0\n", "release"), Err(RedKind::Malformed));
    assert_eq!(cgu("[profile.release]\ncodegen-units = \"x\"\n", "release"), Err(RedKind::Malformed));
    // This tree's own manifest, through its comments and its non-ASCII lines.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let text = std::fs::read_to_string(&root).expect("the root manifest");
    assert_eq!(cgu(&text, "release"), Ok((16, false)), "the root [profile.release] sets no codegen-units (its comment says why)");
    assert_eq!(cgu(&text, "bench"), Ok((1, true)));
}

/// Ruling 57's check on the final line: exactly our token (plus cargo's when the profile sets
/// one), every value the effective one, and a non-incremental build.
#[test]
fn the_final_line_carries_exactly_the_effective_codegen_units() {
    let (sixteen, eight) = (CodegenUnits { n: 16, set_by_profile: false }, CodegenUnits { n: 8, set_by_profile: true });
    let check = |line: &str, c: CodegenUnits| objbuild::check_final_codegen_units(line, c).map_err(|e| e.kind);
    let ours = "Running `rustc --crate-name r -C opt-level=3 -C lto=fat --emit=obj --print link-args -C codegen-units=16 -C 'link-arg=/MAP:D:/m/r.map' -C target-cpu=x86-64-v3`";
    assert_eq!(check(ours, sixteen), Ok(()));
    assert_eq!(check(&ours.replace(" -C codegen-units=16", ""), sixteen), Err(RedKind::Mismatch), "no token: rustc would codegen one unit");
    assert_eq!(check(&ours.replace("codegen-units=16", "codegen-units=1"), sixteen), Err(RedKind::Mismatch));
    let foreign = ours.replace("x86-64-v3`", "x86-64-v3 -C codegen-units=4`");
    assert_eq!(check(&foreign, sixteen), Err(RedKind::Mismatch), "a rustflags value the manifest does not show");
    let both = ours.replace("-C lto=fat", "-C lto=fat -C codegen-units=8").replace("codegen-units=16", "codegen-units=8");
    assert_eq!(check(&both, eight), Ok(()));
    assert_eq!(check(&ours.replace("codegen-units=16", "codegen-units=8"), eight), Err(RedKind::Mismatch), "the profile's value but not cargo's token");
    assert_eq!(check(&ours.replace("-C lto=fat", "-C lto=fat -Ccodegen-units=8").replace("codegen-units=16", "codegen-units=8"), eight), Ok(()));
    assert_eq!(check(&ours.replace("-C lto=fat", "-C lto=fat -C 'incremental=D:/t/i'"), sixteen), Err(RedKind::Mismatch));
}

/// The controls compare a scratch tree's tables with this tree's through a tree-independent package
/// key (a path package's id carries its checkout's path).
#[test]
fn package_keys_are_tree_independent() {
    assert_eq!(controls::stable_pkg("path+file:///D:/wt/fm/crates/boyko_log#boyko-log@0.1.0"), "boyko-log@0.1.0");
    assert_eq!(controls::stable_pkg("path+file:///D:/tmp/rc/g-d/tree/crates/boyko_log#boyko-log@0.1.0"), "boyko-log@0.1.0");
    assert_eq!(controls::stable_pkg("path+file:///D:/wt/fm/crates/boyko_demo#0.1.0"), "boyko_demo@0.1.0");
    assert_eq!(controls::stable_pkg("registry+https://github.com/rust-lang/crates.io-index#syn@2.0.106"), "syn@2.0.106");
}

/// Every scratch control's edit applies to this tree exactly as often as it says, so a 420-minute
/// `unit-controls` cannot fail on a stale needle after its main-tree controls (critique W3 (i)).
#[test]
fn the_scratch_edits_apply_to_this_tree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(controls::SCRATCH.len(), 7);
    for c in &controls::SCRATCH {
        for e in c.edits {
            let raw = std::fs::read(root.join(e.file)).expect("the edited file");
            let text = String::from_utf8_lossy(&key::normalize_eol(&raw)).into_owned();
            assert_eq!(text.matches(e.old).count(), e.count, "{}: {}", c.id, e.file);
        }
    }
    let ids: Vec<&str> = controls::ORDER.to_vec();
    assert_eq!(ids, unit::REQUIRED_CONTROLS.to_vec(), "every required control runs, in ORDER");
}

/// What `unit` writes, `unit-compare` reads back unchanged.
#[test]
fn the_receipt_round_trips() {
    let r = Receipt {
        command: "unit".to_owned(),
        key: "ab".repeat(32),
        key_lines: vec!["key file     crates/boyko_symcensus/Cargo.toml 10 cd".to_owned()],
        target: "D:/t".to_owned(),
        root: "D:\\r".to_owned(),
        tree: "HEAD 0123, 0 uncommitted path(s)".to_owned(),
        boyko_profile: "unset".to_owned(),
        rustc_vv: vec!["rustc 1.98.1 (48a229cea 2026-09-01)".to_owned(), "host: x86_64-pc-windows-msvc".to_owned()],
        cargo_v: "cargo 1.98.1".to_owned(),
        config_sha256: "11".repeat(32),
        lock_sha256: "22".repeat(32),
        closure: vec![ClosureRow { crate_name: "boyko_ecs".to_owned(), package_id: "path+file:///D:/r/crates/boyko_ecs#boyko-ecs@0.1.0".to_owned(), kind: "lib".to_owned(), workspace: true }],
        builds: vec![("runner".to_owned(), "parity_runner".to_owned(), "-".to_owned())],
        libs: vec![LibLine {
            tag: "runner".to_owned(),
            crate_name: "boyko_ecs".to_owned(),
            package_id: "path+file:///D:/r/crates/boyko_ecs#boyko-ecs@0.1.0".to_owned(),
            kind: "lib".to_owned(),
            host: false,
            path: "D:/t\\release\\deps\\libboyko_ecs-0123.rlib".to_owned(),
            sha256: "33".repeat(32),
        }],
        running: vec![("runner".to_owned(), "boyko_ecs".to_owned(), "Running `rustc --crate-name boyko_ecs`".to_owned())],
        finals: vec![("runner".to_owned(), "Running `rustc --crate-name jolt_parity_pyramid`".to_owned())],
        diags: vec![("runner".to_owned(), "warning: x".to_owned())],
        retained: vec![("runner".to_owned(), "image".to_owned(), "44".repeat(32))],
        legs: vec![("(b)".to_owned(), true, "equal".to_owned()), ("(f)".to_owned(), false, "no line".to_owned())],
        certified: "none".to_owned(),
    };
    let text = r.to_text();
    assert_eq!(Receipt::parse(&text).expect("its own format"), r);
    assert_eq!(Receipt::parse(&text.replace("\n", "\r\n")).expect("a CRLF copy"), r);
    assert_eq!(r.libs[0].file_name(), "libboyko_ecs-0123.rlib");
    assert_eq!(Receipt::parse("ug15-ib-receipt v0\n").map_err(|e| e.kind), Err(RedKind::Malformed));
    assert_eq!(Receipt::parse(&format!("{}bogus\tx\n", unit::RECEIPT_FORMAT.to_owned() + "\n")).map_err(|e| e.kind), Err(RedKind::Malformed));
}
