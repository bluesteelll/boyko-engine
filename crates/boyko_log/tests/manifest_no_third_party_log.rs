//! L8b's tidy check: **no workspace manifest may DECLARE a third-party logging facade.**
//!
//! # What this can prove, and what it deliberately cannot
//!
//! It walks every `Cargo.toml` under `crates/`, plus the workspace root, and refuses a dependency
//! **named** `log`, `tracing`, `env_logger`, `console_log`, `tracing-subscriber` or
//! `slog` in any dependency table.
//!
//! It is about **direct declarations only**, and the failure text says so, because the alternative
//! reading is false and would be worse than no check. `log 0.4` is in this workspace's build graph
//! today regardless of what these manifests say: `eframe`, `egui`, `naga`, `gpu-allocator` and
//! `bevy_ecs` all depend on it, and no rule this repository can write removes it from a third
//! party's manifest. That is `substrate/tree-verification`'s open Q5, and the consequence it
//! forces is exactly this scope.
//!
//! So the property is *"this engine's own code reaches for `boyko_log` and nothing else"* — which
//! is a real property, worth a gate, and is not the same as *"no `log` crate is compiled"*.
//!
//! # Why a test and not `clippy.toml`
//!
//! `disallowed-types` cannot see a manifest. `clippy.toml:21-25` also records, empirically, that
//! clippy **silently ignores a config path it cannot resolve** — so a lint-based attempt here
//! would have the failure mode this campaign keeps finding: a gate that cannot fail.

use std::fs;
use std::path::{Path, PathBuf};

/// Facades this engine replaced with `boyko_log`. Split so the file does not match itself when a
/// future census greps for these names.
const BANNED: &[&str] = &[
    concat!("lo", "g"),
    concat!("traci", "ng"),
    concat!("env_log", "ger"),
    concat!("console_", "log"),
    concat!("tracing-subscri", "ber"),
    concat!("sl", "og"),
];

/// The repository root, resolved from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("invariant: crates/boyko_log has a grandparent")
        .to_path_buf()
}

/// Every `Cargo.toml` this workspace owns.
fn workspace_manifests() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = vec![root.join("Cargo.toml")];
    let crates = root.join("crates");
    let entries = fs::read_dir(&crates).expect("invariant: crates/ is readable");
    for e in entries.flatten() {
        let m = e.path().join("Cargo.toml");
        if m.is_file() {
            out.push(m);
        }
    }
    // `tools/` holds `prof_decode`, which is a workspace member with its own manifest.
    let tools = root.join("tools");
    if tools.is_dir() {
        for e in fs::read_dir(&tools).expect("invariant: tools/ is readable").flatten() {
            let m = e.path().join("Cargo.toml");
            if m.is_file() {
                out.push(m);
            }
        }
    }
    out
}

/// Every banned dependency KEY declared by one manifest, as `(line_no, key)`.
///
/// A dependency key is a line whose first token before `=` is the crate name, inside a table whose
/// header ends in `dependencies`. Comments are stripped first — this file's own module doc names
/// every banned crate, and `boyko_demo`'s manifest explains at length which ones it deleted and
/// why. A check that could not tell a name from a note would red on the note.
fn banned_deps(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            // `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, and the
            // `[target.'cfg(..)'.dependencies]` forms all end in `dependencies]`.
            in_deps = line.ends_with("dependencies]");
            continue;
        }
        if !in_deps || line.is_empty() {
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        if BANNED.contains(&key) {
            out.push((i + 1, key.to_string()));
        }
    }
    out
}

#[test]
fn no_workspace_manifest_declares_a_third_party_logging_facade() {
    let manifests = workspace_manifests();
    assert!(
        manifests.len() > 10,
        "vacuity guard: only {} manifests were found, so a green here would mean the walk \
         resolved the wrong root rather than that the workspace is clean",
        manifests.len()
    );

    let mut offenders = Vec::new();
    for m in &manifests {
        let text = fs::read_to_string(m)
            .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", m.display()));
        for (line, key) in banned_deps(&text) {
            offenders.push(format!("{}:{line} declares `{key}`", m.display()));
        }
    }

    assert!(
        offenders.is_empty(),
        "these manifests declare a third-party logging facade: {offenders:?}. This engine logs \
         through `boyko_log`; a second facade means two subsystems with two sets of levels, two \
         sinks and two answers to \"is logging on\". NOTE THE SCOPE: this check is about DIRECT \
         declarations only. `log 0.4` remains in the build graph transitively through \
         eframe/egui/naga/gpu-allocator/bevy_ecs, and no rule in this repository can change that \
         -- so a green here means \"our code reaches for one facade\", NOT \"no log crate is \
         compiled\"."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parser_finds_a_banned_key_and_ignores_a_comment_that_names_one() {
        // The RED this gate needs, run as a unit rather than by editing a real manifest: the
        // positive control proves the walk would see a declaration, and the negative controls
        // prove it does not red on the prose that necessarily surrounds a deletion.
        let manifest = "\
[package]\n\
name = \"x\"\n\
\n\
[dependencies]\n\
# L8b deleted the third-party log = \"0.4\" that stood here.\n\
boyko-log = { path = \"../boyko_log\" }\n\
serde = \"1\"\n\
";
        assert!(
            banned_deps(manifest).is_empty(),
            "a COMMENT naming a banned crate is a record of its deletion, not a declaration"
        );

        let with_dep = manifest.replace("serde = \"1\"", "log = \"0.4\"");
        let hits = banned_deps(&with_dep);
        assert_eq!(hits.len(), 1, "the declaration must be found: {hits:?}");
        assert_eq!(hits[0].1, "log");

        // A target-gated table is still a dependency table.
        let target_gated = "\
[target.'cfg(not(target_arch = \"wasm32\"))'.dependencies]\n\
env_logger = \"0.11\"\n\
";
        assert_eq!(
            banned_deps(target_gated).len(),
            1,
            "`[target.'cfg(..)'.dependencies]` is where env_logger and console_log actually lived"
        );

        // And a key that merely CONTAINS a banned name is not one.
        let near_miss = "[dependencies]\nboyko-log = { path = \"..\" }\nlog4rs = \"1\"\n";
        assert!(
            banned_deps(near_miss).is_empty(),
            "matching must be on the whole key, not a substring: {:?}",
            banned_deps(near_miss)
        );
    }
}
