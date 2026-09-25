//! `boyko-symcensus` is a DEV-ONLY crate: it spawns `cargo`, `llvm-*` and `llvm-symbolizer`, which
//! belong in no process that ships. This test reads `cargo metadata`'s resolve graph and is RED if
//! any package reaches it through a normal or build edge (cut §1).

use std::path::Path;

use boyko_symcensus::json::{self, Json};

#[test]
fn no_package_has_a_normal_or_build_edge_onto_the_instrument() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("invariant: crates/boyko_symcensus");
    let out = std::process::Command::new(env!("CARGO"))
        .current_dir(root)
        .args(["metadata", "--format-version", "1"])
        .output()
        .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    let meta = json::parse(&String::from_utf8_lossy(&out.stdout)).unwrap_or_else(|r| panic!("{r}"));
    let nodes = meta.get("resolve").and_then(|r| r.get("nodes")).map(Json::items).unwrap_or(&[]);
    assert!(nodes.len() > 20, "the resolve graph has {} nodes; the check read nothing", nodes.len());
    let mut edges = 0usize;
    let mut bad = Vec::new();
    for n in nodes {
        let from = n.get("id").and_then(Json::as_str).unwrap_or("?");
        for d in n.get("deps").map(Json::items).unwrap_or(&[]) {
            if d.get("name").and_then(Json::as_str) != Some("boyko_symcensus") {
                continue;
            }
            for k in d.get("dep_kinds").map(Json::items).unwrap_or(&[]) {
                edges += 1;
                let kind = k.get("kind").and_then(Json::as_str).unwrap_or("normal");
                if kind != "dev" {
                    bad.push(format!("{from} --{kind}--> boyko-symcensus"));
                }
            }
        }
    }
    println!("edges onto boyko-symcensus: {edges} (every one must be `dev`)");
    assert!(edges > 0, "no package names boyko-symcensus at all, so M-b's edge from boyko-macros is missing");
    assert!(bad.is_empty(), "boyko-symcensus reached by a shipping edge: {bad:?}");
}
