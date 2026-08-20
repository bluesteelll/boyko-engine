//! `AetherCtx` — the per-block symbol table (§4), built between parse and expand.
//!
//! Build-time only: this is transpiler state, never runtime state, so the keyed lookups here are
//! exempt from the engine's hot-path collection rules exactly as `boyko_shaderdsl`'s emit arena is.
//! The scan is linear over a block's construct list (a handful of entries), which is why there is
//! no map: an `O(n)` walk over eight names beats a hash every time and prints in a stable order.
//!
//! # Scope (§4's resolution rules v1)
//!
//! One `aether!` block. Cross-block references are ordinary Rust name resolution on the EXPANDED
//! items — Aether itself never resolves across blocks, and holds no global macro state.
//!
//! # The duplicate-name rule, and where it stops
//!
//! §4 states duplicate names across kinds are a ctx-build error with both spans. §7.1 states
//! Aether pre-checks a downstream fault ONLY when it can produce a strictly better span or
//! message. Those two meet here, and the split is drawn by MEASUREMENT, not by preference:
//!
//! * The **fn-producing** constructs (`system`, `material`, `scene`) expand to a bare `pub fn` and
//!   nothing else — no derive, no trait bound. Rung A5 measured real rustc on that shape: E0428
//!   puts BOTH of its labels on the `aether!` token and names no user token anywhere. Aether owns
//!   these, with both spans. `scene` widened the class rather than adding a case: a `scene lab`
//!   beside a `material lab` is the same two-fns-one-name fault across kinds.
//! * The **type-producing** constructs (`component`, `tag`, `bundle`, `event`, `machine`,
//!   `plugin`) carry a derive, so rustc reports the duplicate definition AND a second, localized
//!   error against the user's own item. §7.1 defers those — a duplicated check there could only be
//!   worse, and duplicated checks drift.

use syn::Ident;

use crate::ast::{AetherBlock, Construct, MaterialDef, PluginDef};
use crate::diag;

/// The per-block symbol table (§4) — the ONLY channel constructs use to see each other.
///
/// Deliberately narrow: it carries the one symbol class a consumer actually resolves against
/// today (`scene`'s `material:` prop) plus the whole-block validation. `system`'s sibling
/// ordering and `plugin`'s collection each walk `block.constructs` at their own site and need no
/// entry here — a table row nothing reads is a datum that rots.
pub struct AetherCtx<'a> {
    /// Every sibling `material`, in declaration order — `scene`'s `material:` prop resolves here,
    /// and the order is what makes the hoisted mint sequence deterministic.
    materials: Vec<&'a MaterialDef>,
}

impl<'a> AetherCtx<'a> {
    /// Build the table, running the whole-block validation §4 puts at ctx-build time.
    ///
    /// The rules, in the order a reader meets them: duplicate fn-producing names, one `plugin` per
    /// block, and the §3.3 requirement that scheduling clauses (and a `machine`) have a plugin to
    /// hold their registrations.
    pub fn build(block: &'a AetherBlock) -> syn::Result<Self> {
        duplicate_fn_names(block)?;

        let mut plugin: Option<&PluginDef> = None;
        for c in &block.constructs {
            let Construct::Plugin(p) = c else { continue };
            if let Some(first) = plugin {
                let mut e = diag::err(
                    p.name.span(),
                    format!(
                        "one `plugin` per aether block — `{}` already holds this block's registrations",
                        first.name
                    ),
                );
                e.combine(diag::err(first.name.span(), "the first `plugin` is here"));
                return Err(e);
            }
            plugin = Some(p);
        }

        if plugin.is_none() {
            for c in &block.constructs {
                match c {
                    Construct::System(s) if s.has_clauses() => {
                        return Err(diag::err(
                            s.name.span(),
                            "scheduling clauses (`on`, `after`, `when`, …) need a `plugin <Name>;` declaration in this block to hold the generated registration",
                        ));
                    }
                    Construct::Machine(m) => {
                        return Err(diag::err(
                            m.name.span(),
                            "a `machine` needs a `plugin <Name>;` declaration in this block to hold its `insert_state` and transition registrations",
                        ));
                    }
                    _ => {}
                }
            }
        }

        let materials = block
            .constructs
            .iter()
            .filter_map(|c| match c {
                Construct::Material(m) => Some(&**m),
                _ => None,
            })
            .collect();

        Ok(AetherCtx { materials })
    }

    /// Resolve a `material: NAME` reference against the sibling `material` constructs.
    pub fn material(&self, name: &Ident) -> Option<&'a MaterialDef> {
        self.materials.iter().copied().find(|m| m.name == *name)
    }

    /// Every sibling material, in BLOCK declaration order — the order a consumer must emit in if
    /// its output is to be stable under an edit that only moves reference sites around.
    pub fn materials(&self) -> &[&'a MaterialDef] {
        &self.materials
    }

    /// Every sibling material name in declaration order — the §3.7 "materials here: …" list.
    pub fn material_names(&self) -> Vec<String> {
        self.materials.iter().map(|m| m.name.to_string()).collect()
    }
}

/// §4's duplicate-name rule over the fn-producing half of the registry (see the module docs for
/// why the type-producing half stays with rustc).
///
/// The error lands on the SECOND declaration and carries the first's span — the shape `plugin` ×
/// `plugin` and `material` × `material` already ship with, generalized rather than re-invented.
fn duplicate_fn_names(block: &AetherBlock) -> syn::Result<()> {
    for (i, c) in block.constructs.iter().enumerate() {
        if !c.emits_fn() {
            continue;
        }
        let Some(first) =
            block.constructs[..i].iter().find(|p| p.emits_fn() && p.name() == c.name())
        else {
            continue;
        };
        let msg = if first.keyword() == c.keyword() {
            format!(
                "duplicate {kw} `{name}` — each {kw} expands to a {noun} of its own name, and two of one name is one fn defined twice",
                kw = c.keyword(),
                name = c.name(),
                noun = c.fn_noun(),
            )
        } else {
            format!(
                "`{name}` is declared twice in this aether block — the `{a}` and the `{b}` both expand to a fn of that name",
                name = c.name(),
                a = first.keyword(),
                b = c.keyword(),
            )
        };
        let mut e = diag::err(c.name().span(), msg);
        e.combine(diag::err(
            first.name().span(),
            format!("the first `{}` of this name is here", first.keyword()),
        ));
        return Err(e);
    }
    Ok(())
}
