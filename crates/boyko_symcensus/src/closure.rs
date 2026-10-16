//! `boyko-physics`'s resolved normal closure, the "(e) enumerator" every V11 floor uses (IB design
//! Rev 5 (d)/(e), Rev 6 (b)/(d1) anti-vacuity).
//!
//! Read from `cargo metadata --format-version 1 --filter-platform <host> --locked`: its `resolve`
//! graph, following NORMAL edges only (a `dep_kinds` entry whose `kind` is `null`). The platform
//! filter drops every `cfg` edge the host does not take, so on msvc `boyko-input → windows-sys` is
//! in and every `cfg(unix)` edge is out.
//!
//! # With or without dev-activated features (critique open question 2)
//!
//! WITH. `cargo metadata` resolves the whole workspace at once, and that resolve unifies every
//! member's requests, `[dev-dependencies]` features included. An optional normal dependency of a
//! closure crate that only some member's dev table activates is therefore an edge here although no
//! binary build links it. The enumerator can only over-count, never under-count, and an over-count
//! is fail-safe: a floor (`(b)`, `(d1)`) names that crate as missing from the build and goes RED,
//! never green. `ug15 unit`'s first run at P0r (RF-5a) shows the floor green, so no such crate
//! exists on that tree; if one appears later the red names it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

use crate::json::{self, Json};
use crate::objbuild::{ArtifactKind, Ctx};
use crate::red::{Red, RedKind, Result};

/// One package of the resolve graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// cargo's package id.
    pub id: String,
    /// `[package] name`.
    pub name: String,
    /// The lib target's crate name (`-` → `_`), or `None` for a package without a lib target.
    pub crate_name: Option<String>,
    /// `lib` or `proc-macro`, for a package with a lib target.
    pub kind: Option<ArtifactKind>,
    /// A workspace member (a path package with no registry source).
    pub workspace: bool,
    /// The lib target's root source file.
    pub root_src: Option<PathBuf>,
    /// Ids of its normal dependencies on this platform.
    pub deps: Vec<String>,
}

/// The resolve graph of one tree, for one platform.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    /// Every package of the resolve, by id.
    pub nodes: BTreeMap<String, Node>,
}

/// The crate the closure is rooted at.
pub const CLOSURE_ROOT: &str = "boyko-physics";

impl Graph {
    /// Runs `cargo metadata --filter-platform <host triple> --locked` in the tree and reads it.
    pub fn load(ctx: &Ctx) -> Result<Self> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let out = Command::new(cargo)
            .current_dir(&ctx.root)
            .args(["metadata", "--format-version", "1", "--locked", "--filter-platform", &ctx.host.triple])
            .env("CARGO_TARGET_DIR", &ctx.target)
            .output()
            .map_err(|e| Red::new(RedKind::ToolAbsent, format!("could not spawn cargo metadata: {e}")))?;
        if !out.status.success() {
            return Err(Red::new(RedKind::BuildFailed, format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr))));
        }
        Self::parse(&String::from_utf8_lossy(&out.stdout))
    }

    /// Reads a `cargo metadata --format-version 1` document.
    pub fn parse(text: &str) -> Result<Self> {
        let meta = json::parse(text)?;
        let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
        for pkg in meta.get("packages").map(Json::items).unwrap_or(&[]) {
            let id = str_of(pkg, "id")?;
            let name = str_of(pkg, "name")?;
            let workspace = matches!(pkg.get("source"), None | Some(Json::Null));
            let mut crate_name = None;
            let mut kind = None;
            let mut root_src = None;
            for t in pkg.get("targets").map(Json::items).unwrap_or(&[]) {
                let kinds: Vec<&str> = t.get("kind").map(|k| k.items().iter().filter_map(Json::as_str).collect()).unwrap_or_default();
                let k = if kinds.contains(&"proc-macro") {
                    Some(ArtifactKind::ProcMacro)
                } else if kinds.iter().any(|k| matches!(*k, "lib" | "rlib" | "dylib" | "staticlib" | "cdylib")) {
                    Some(ArtifactKind::Lib)
                } else {
                    None
                };
                if let Some(k) = k {
                    crate_name = t.get("name").and_then(Json::as_str).map(|n| n.replace('-', "_"));
                    kind = Some(k);
                    root_src = t.get("src_path").and_then(Json::as_str).map(PathBuf::from);
                }
            }
            nodes.insert(id.clone(), Node { id, name, crate_name, kind, workspace, root_src, deps: Vec::new() });
        }
        let resolve = meta.get("resolve").ok_or_else(|| Red::new(RedKind::Malformed, "cargo metadata has no `resolve`"))?;
        for n in resolve.get("nodes").map(Json::items).unwrap_or(&[]) {
            let id = str_of(n, "id")?;
            let mut deps = Vec::new();
            for d in n.get("deps").map(Json::items).unwrap_or(&[]) {
                let normal = d
                    .get("dep_kinds")
                    .map(Json::items)
                    .unwrap_or(&[])
                    .iter()
                    .any(|k| matches!(k.get("kind"), None | Some(Json::Null)));
                if normal {
                    deps.push(str_of(d, "pkg")?);
                }
            }
            deps.sort();
            deps.dedup();
            match nodes.get_mut(&id) {
                Some(node) => node.deps = deps,
                None => return Err(Red::new(RedKind::Malformed, format!("resolve node {id} is not a package"))),
            }
        }
        if nodes.is_empty() {
            return Err(Red::new(RedKind::EmptyCensus, "cargo metadata listed no package"));
        }
        Ok(Self { nodes })
    }

    /// The id of the workspace package named `name`.
    pub fn workspace_id(&self, name: &str) -> Result<&str> {
        let ids: Vec<&str> = self.nodes.values().filter(|n| n.workspace && n.name == name).map(|n| n.id.as_str()).collect();
        match ids.as_slice() {
            [id] => Ok(id),
            [] => Err(Red::new(RedKind::Usage, format!("no workspace package `{name}` in the resolve"))),
            _ => Err(Red::new(RedKind::Malformed, format!("{} workspace packages named `{name}`", ids.len()))),
        }
    }

    /// Every package reachable from `root` over normal edges, `root` included.
    #[must_use]
    pub fn reachable(&self, root: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![root.to_owned()];
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(n) = self.nodes.get(&id) {
                stack.extend(n.deps.iter().cloned());
            }
        }
        seen
    }

    /// `boyko-physics`'s resolved normal closure, sorted by crate name.
    pub fn closure(&self) -> Result<Vec<Node>> {
        let root = self.workspace_id(CLOSURE_ROOT)?.to_owned();
        let mut v: Vec<Node> = self.reachable(&root).iter().filter_map(|id| self.nodes.get(id).cloned()).collect();
        v.sort_by(|a, b| (&a.crate_name, &a.id).cmp(&(&b.crate_name, &b.id)));
        if v.iter().any(|n| n.crate_name.is_none()) {
            return Err(Red::new(RedKind::Malformed, "a closure package has no lib target"));
        }
        Ok(v)
    }

    /// The packages reachable from `from` (over normal edges) that depend on `on`, transitively,
    /// `on` excluded: the units a change of `on` dirties in `from`'s build.
    #[must_use]
    pub fn dependents_within(&self, from: &str, on: &str) -> BTreeSet<String> {
        let within = self.reachable(from);
        let mut out = BTreeSet::new();
        for id in &within {
            if id != on && self.reachable(id).contains(on) {
                out.insert(id.clone());
            }
        }
        out
    }
}

fn str_of(v: &Json, key: &str) -> Result<String> {
    v.get(key)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Red::new(RedKind::Malformed, format!("cargo metadata: an entry has no string `{key}`")))
}

/// Bumps the modification time of the root source of every workspace crate in `closure`,
/// proc-macro included (content untouched, so `git status` stays clean): the next build rebuilds
/// each of them and every dependent from the front end and prints its `Running` line, which V11
/// (d2) reads. `touch_census_crate_roots` generalised to the closure; probe (iv) arm B's method.
pub fn touch_closure_roots(closure: &[Node]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for n in closure.iter().filter(|n| n.workspace) {
        let src = n
            .root_src
            .clone()
            .ok_or_else(|| Red::new(RedKind::Malformed, format!("workspace closure crate {} has no root source", n.name)))?;
        crate::objbuild::touch(&src)?;
        out.push(src);
    }
    if out.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, "boyko-physics's closure holds no workspace crate to touch"));
    }
    Ok(out)
}
