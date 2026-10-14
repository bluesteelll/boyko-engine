//! **AD-9 / RK-16 (render-wave RD3): no `RhiCommandEncoder` default-body seam is silently
//! inherited by the Vulkan backend.**
//!
//! `RhiCommandEncoder` gives most of its verbs a default body — a `#[cold] #[inline(never)]`
//! no-op — so a backend without the capability (the test Mock) compiles unchanged. The price is
//! that a backend that SHOULD implement a verb and forgets inherits a no-op that records nothing,
//! and nothing says so: `dispatch_indirect` was exactly that (RK-16) — zero trait call sites, a
//! loaded `vkCmdDispatchIndirect`, and a trait default that dispatched nothing.
//!
//! This census reads the two source files with `syn` and holds them to one rule: every defaulted
//! seam of the trait has an UNCONDITIONAL override in `impl RhiCommandEncoder<Vulkan> for
//! VulkanCommandEncoder`, or a row in [`ALLOW`] carrying the reason it may default there.
//!
//! # Red when
//!
//! - **(a) the class gate:** a defaulted seam has no unconditional Vulkan override and no
//!   [`ALLOW`] row (a `#[cfg]`-gated override alone does not count — it is absent from the
//!   default build);
//! - **(b)** an [`ALLOW`] row names no defaulted seam;
//! - **(c)** an [`ALLOW`] row's seam has an unconditional override (the row is stale);
//! - **(d)** a [`Kind::CfgGated`] row's override is missing or not gated on exactly that feature;
//! - **(e)** a [`Kind::ProvidedWrapper`] row's default body is not exactly one
//!   `self.<delegates_to>(..)` statement, or `delegates_to` has no unconditional override;
//! - **(f) anti-vacuity:** fewer than [`MIN_DEFAULTED`] defaulted seams or [`MIN_OVERRIDES`]
//!   Vulkan trait fns were parsed, or [`ALLOW`] names a seam twice;
//! - **(g)** a Vulkan override has an EMPTY body: inheriting a silent no-op is (a), and writing one
//!   out by hand is the same defect.
//!
//! The run prints the defaulted seams, the overrides, the allow rows and the violations.
//!
//! # Scope
//!
//! The Vulkan backend only. The test Mock (`boyko_rhi/src/handle.rs`, `#[cfg(test)]`) inherits
//! every default by design and is not read. RD3 makes `dispatch_indirect` a required verb rather
//! than an allow row, so a backend that omits it does not compile.
//!
//! # Why this lives in the root package
//!
//! `CARGO_MANIFEST_DIR` **is** the repository root here, so no `../..` walking can point the scan
//! at the wrong tree, and the package links no engine crate (`Cargo.toml`'s dev-dependency note).

use std::fs;
use std::path::Path;

use quote::ToTokens;
use syn::{Expr, ImplItem, Item, Stmt, TraitItem, Type};

/// The trait's source file, repo-relative.
const TRAIT_FILE: &str = "crates/boyko_rhi/src/encoder.rs";
/// The Vulkan backend's encoder source file, repo-relative.
const IMPL_FILE: &str = "crates/boyko_rhi_vulkan/src/rhi_impl/encoder.rs";
/// The trait's name.
const TRAIT_NAME: &str = "RhiCommandEncoder";
/// The Vulkan implementor's name.
const IMPL_SELF: &str = "VulkanCommandEncoder";

/// Anti-vacuity floor on defaulted seams (23 since RD3; 24 before it).
const MIN_DEFAULTED: usize = 20;
/// Anti-vacuity floor on the Vulkan trait impl's fns (30 since RD3; 29 before it).
const MIN_OVERRIDES: usize = 25;

/// Why a defaulted seam may stay inherited by the Vulkan backend.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// A provided method whose whole default body forwards to another trait verb, which the
    /// backend overrides — inheriting it records the override's commands, not a no-op.
    ProvidedWrapper {
        /// The verb the default body calls.
        delegates_to: &'static str,
    },
    /// Overridden only under a cargo feature whose every caller is gated on the same feature.
    CfgGated {
        /// The feature name in `#[cfg(feature = "..")]`.
        feature: &'static str,
    },
}

/// One reasoned allow row.
struct Allow {
    name: &'static str,
    kind: Kind,
    reason: &'static str,
}

/// The seams the Vulkan backend may inherit, each with its reason. Exactly three since RD3.
const ALLOW: &[Allow] = &[
    Allow {
        name: "bind_descriptor_set",
        kind: Kind::ProvidedWrapper { delegates_to: "bind_descriptor_set_at" },
        reason: "provided wrapper; its body is one `self.bind_descriptor_set_at(..)` call, which \
                 Vulkan overrides",
    },
    Allow {
        name: "cmd_build_acceleration_structures",
        kind: Kind::CfgGated { feature: "hwrt" },
        reason: "overridden under `hwrt`; every trait caller is `hwrt`-gated (boyko_rhi_vulkan's \
                 `accel_build.rs`, a `#[cfg(feature = \"hwrt\")]` module, and \
                 `tests/hwrt_blas_smoke.rs`, a `#![cfg(feature = \"hwrt\")]` binary)",
    },
    Allow {
        name: "cmd_acceleration_structure_barrier",
        kind: Kind::CfgGated { feature: "hwrt" },
        reason: "overridden under `hwrt`; it has no trait caller (the passes record the raw \
                 `crate::accel` helper under `hwrt`), so nothing outside the feature reaches it",
    },
];

/// How the Vulkan impl provides a fn.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Override {
    /// No `#[cfg]` attribute: present in every build.
    Unconditional,
    /// Present only under the `#[cfg(..)]` whose whitespace-free tokens are carried.
    Cfg(String),
}

/// A defaulted trait seam: its name and its default body's statements.
struct Defaulted {
    name: String,
    body: Vec<Stmt>,
}

/// A fn of the Vulkan trait impl.
struct ImplFn {
    name: String,
    how: Override,
    empty_body: bool,
}

fn parse(rel: &str) -> syn::File {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", path.display()));
    syn::parse_file(&text).unwrap_or_else(|e| panic!("{rel} does not parse as Rust: {e}"))
}

/// Whitespace-free token text (`feature="hwrt"`), so formatting cannot change a comparison.
fn squash(tokens: impl ToTokens) -> String {
    tokens.to_token_stream().to_string().chars().filter(|c| !c.is_whitespace()).collect()
}

/// Every defaulted fn of `trait RhiCommandEncoder` (exactly one such trait must exist).
fn defaulted_seams(file: &syn::File) -> Vec<Defaulted> {
    let traits: Vec<&syn::ItemTrait> = file
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Trait(t) if t.ident == TRAIT_NAME => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(traits.len(), 1, "{TRAIT_FILE} defines `trait {TRAIT_NAME}` exactly once");
    traits[0]
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(f) => f.default.as_ref().map(|block| Defaulted {
                name: f.sig.ident.to_string(),
                body: block.stmts.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Every fn of `impl RhiCommandEncoder<..> for VulkanCommandEncoder` (exactly one such impl).
fn vulkan_overrides(file: &syn::File) -> Vec<ImplFn> {
    let impls: Vec<&syn::ItemImpl> = file
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Impl(imp) => Some(imp),
            _ => None,
        })
        .filter(|imp| {
            let trait_ok = imp
                .trait_
                .as_ref()
                .and_then(|(_, path, _)| path.segments.last())
                .is_some_and(|seg| seg.ident == TRAIT_NAME);
            let self_ok = matches!(&*imp.self_ty, Type::Path(p)
                if p.path.segments.last().is_some_and(|seg| seg.ident == IMPL_SELF));
            trait_ok && self_ok
        })
        .collect();
    assert_eq!(impls.len(), 1, "{IMPL_FILE} has exactly one `impl {TRAIT_NAME}<..> for {IMPL_SELF}`");
    impls[0]
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Fn(f) => {
                let cfgs: Vec<String> = f
                    .attrs
                    .iter()
                    .filter(|a| a.path().is_ident("cfg"))
                    .map(|a| match &a.meta {
                        syn::Meta::List(list) => squash(&list.tokens),
                        other => squash(other),
                    })
                    .collect();
                let how = if cfgs.is_empty() {
                    Override::Unconditional
                } else {
                    Override::Cfg(cfgs.join("&&"))
                };
                Some(ImplFn {
                    name: f.sig.ident.to_string(),
                    how,
                    empty_body: f.block.stmts.is_empty(),
                })
            }
            _ => None,
        })
        .collect()
}

/// `true` iff `body` is exactly one statement `self.<method>(..)` (with or without `;`).
fn body_is_one_self_call(body: &[Stmt], method: &str) -> bool {
    let [stmt] = body else {
        return false;
    };
    let expr = match stmt {
        Stmt::Expr(e, _) => e,
        _ => return false,
    };
    let Expr::MethodCall(call) = expr else {
        return false;
    };
    let receiver_is_self =
        matches!(&*call.receiver, Expr::Path(p) if p.qself.is_none() && p.path.is_ident("self"));
    receiver_is_self && call.method == method
}

/// The census verdict over parsed inputs: one line per violation.
fn violations(seams: &[Defaulted], overrides: &[ImplFn], allow: &[Allow]) -> Vec<String> {
    let mut bad = Vec::new();
    let unconditional =
        |name: &str| overrides.iter().any(|o| o.name == name && o.how == Override::Unconditional);
    let seam = |name: &str| seams.iter().find(|s| s.name == name);
    let allow_row = |name: &str| allow.iter().find(|a| a.name == name);

    for s in seams {
        if !unconditional(&s.name) && allow_row(&s.name).is_none() {
            bad.push(format!(
                "(a) `{}` is a defaulted seam with no unconditional Vulkan override and no ALLOW \
                 row - the Vulkan backend silently inherits its default body",
                s.name
            ));
        }
    }
    for (i, a) in allow.iter().enumerate() {
        if allow[..i].iter().any(|b| b.name == a.name) {
            bad.push(format!("(f) ALLOW names `{}` twice", a.name));
        }
        let Some(s) = seam(a.name) else {
            bad.push(format!("(b) ALLOW row `{}` names no defaulted seam of the trait", a.name));
            continue;
        };
        if unconditional(a.name) {
            bad.push(format!(
                "(c) ALLOW row `{}` is stale: the Vulkan backend overrides it unconditionally",
                a.name
            ));
        }
        match a.kind {
            Kind::CfgGated { feature } => {
                let want = format!("feature=\"{feature}\"");
                let gated = overrides
                    .iter()
                    .any(|o| o.name == a.name && o.how == Override::Cfg(want.clone()));
                if !gated {
                    bad.push(format!(
                        "(d) ALLOW row `{}` says cfg-gated on `{feature}`, but the Vulkan impl has \
                         no `#[cfg(feature = \"{feature}\")]` override of it",
                        a.name
                    ));
                }
            }
            Kind::ProvidedWrapper { delegates_to } => {
                if !body_is_one_self_call(&s.body, delegates_to) {
                    bad.push(format!(
                        "(e) ALLOW row `{}` says provided wrapper over `{delegates_to}`, but its \
                         default body is not exactly one `self.{delegates_to}(..)` statement",
                        a.name
                    ));
                }
                if !unconditional(delegates_to) {
                    bad.push(format!(
                        "(e) ALLOW row `{}` delegates to `{delegates_to}`, which has no \
                         unconditional Vulkan override",
                        a.name
                    ));
                }
            }
        }
    }
    if seams.len() < MIN_DEFAULTED || overrides.len() < MIN_OVERRIDES {
        bad.push(format!(
            "(f) anti-vacuity: {} defaulted seams (floor {MIN_DEFAULTED}) and {} Vulkan trait fns \
             (floor {MIN_OVERRIDES}) parsed",
            seams.len(),
            overrides.len()
        ));
    }
    for o in overrides.iter().filter(|o| o.empty_body) {
        bad.push(format!(
            "(g) the Vulkan override of `{}` has an empty body - a silent no-op written out by hand",
            o.name
        ));
    }
    bad
}

#[test]
fn no_encoder_default_seam_is_silently_inherited_by_vulkan() {
    let seams = defaulted_seams(&parse(TRAIT_FILE));
    let overrides = vulkan_overrides(&parse(IMPL_FILE));

    println!("defaulted seams of `{TRAIT_NAME}` ({}):", seams.len());
    for s in &seams {
        println!("  {}", s.name);
    }
    println!("fns of `impl {TRAIT_NAME} for {IMPL_SELF}` ({}):", overrides.len());
    for o in &overrides {
        match &o.how {
            Override::Unconditional => println!("  {}", o.name),
            Override::Cfg(c) => println!("  {}  [cfg({c})]", o.name),
        }
    }
    println!("ALLOW ({}):", ALLOW.len());
    for a in ALLOW {
        println!("  {} - {:?}: {}", a.name, a.kind, a.reason);
    }

    let bad = violations(&seams, &overrides, ALLOW);
    println!("violations ({}):", bad.len());
    for b in &bad {
        println!("  {b}");
    }
    assert!(bad.is_empty(), "{} encoder default-seam violation(s):\n{}", bad.len(), bad.join("\n"));
}
