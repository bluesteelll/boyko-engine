//! UG-15 legs (4) and (6) are N/A before any modding crate exists (03 §6), and are reported N/A,
//! never green. This test asserts the PRECONDITION of N/A: no workspace member is a modding crate.
//! The day one appears it goes RED, which forces the real legs to be written (control N).

use std::path::Path;

/// Package-name prefixes of the modding crates (05 §3.2: `boyko_mod_api`, `boyko_mod_host`,
/// `boyko_mod_registry`).
const MOD_PREFIXES: [&str; 2] = ["boyko-mod", "boyko_mod"];

#[test]
fn legs_4_and_6_are_na_because_no_modding_crate_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("invariant: crates/boyko_symcensus");
    let out = std::process::Command::new(env!("CARGO"))
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    let meta = boyko_symcensus::json::parse(&String::from_utf8_lossy(&out.stdout)).unwrap_or_else(|r| panic!("{r}"));
    let names: Vec<&str> = meta
        .get("packages")
        .map(boyko_symcensus::json::Json::items)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| p.get("name").and_then(boyko_symcensus::json::Json::as_str))
        .collect();
    assert!(names.len() > 20, "cargo metadata listed {} workspace packages; the precondition read nothing", names.len());
    let modding: Vec<&&str> = names.iter().filter(|n| MOD_PREFIXES.iter().any(|p| n.starts_with(p))).collect();
    println!("UG-15 legs (4)/(6): {} workspace package(s) read, {} modding crate(s) matched", names.len(), modding.len());
    assert!(
        modding.is_empty(),
        "legs (4)/(6) are no longer N/A: modding crate(s) {modding:?} exist, so the startup-equality and two-arm linked-size legs must be written (03 §6)"
    );
    println!("UG-15 leg (4): N/A — no modding crate (0 members matched)");
    println!("UG-15 leg (6): N/A — no modding crate (0 members matched)");
}
