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
//!
//! # Broken constructs participate (§7.3, rung A7)
//!
//! Every rule below runs over `constructs ∪ broken`. A construct that failed to PARSE still holds
//! its name and its kind, and reading it as absent manufactures a second fault out of the first:
//! a half-typed `plugin` would make every sibling clause report "needs a plugin", and a duplicate
//! `material gold` would go unreported here and surface as rustc's E0428 on the macro token —
//! the two shapes this table exists to prevent. Only rules whose failure could not exist without
//! the break (the broken plugin's own registration contents, a scene's reference INTO a construct
//! that never parsed) stay suppressed, at their own sites.

use syn::Ident;

use crate::ast::{AetherBlock, BrokenConstruct, Construct, MaterialDef};
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
    /// The names of `material` constructs that did NOT parse (§7.3). A `material: gold` pointing
    /// at one of these is not an unknown symbol — the material is right there, unread — so the
    /// consumer SUPPRESSES itself instead of reporting a message that contradicts the source.
    broken_materials: Vec<&'a Ident>,
}

impl<'a> AetherCtx<'a> {
    /// Build the table, running the whole-block validation §4 puts at ctx-build time.
    ///
    /// The rules, in the order a reader meets them: duplicate fn-producing names, one `plugin` per
    /// block, and the §3.3 requirement that scheduling clauses (and a `machine`) have a plugin to
    /// hold their registrations.
    pub fn build(block: &'a AetherBlock) -> syn::Result<Self> {
        duplicate_fn_names(block)?;

        // The plugin slot, over the union: a `plugin` that failed to parse still OCCUPIES it.
        // Two named plugins are a real duplicate whichever of them parsed (the fault survives the
        // author finishing the line); a nameless broken one can only hold the slot, since a
        // diagnostic has nothing to print for it.
        let mut plugin: Option<&Ident> = None;
        let mut plugin_declared = false;
        for item in block_symbols(block) {
            if item.keyword != "plugin" {
                continue;
            }
            plugin_declared = true;
            let Some(name) = item.name else { continue };
            if let Some(first) = plugin {
                let mut e = diag::err(
                    name.span(),
                    format!(
                        "one `plugin` per aether block — `{first}` already holds this block's registrations"
                    ),
                );
                e.combine(diag::err(first.span(), "the first `plugin` is here"));
                return Err(e);
            }
            plugin = Some(name);
        }

        if !plugin_declared {
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
        let broken_materials = block
            .broken
            .iter()
            .filter(|b| b.keyword == Some("material"))
            .filter_map(BrokenConstruct::name)
            .collect();

        Ok(AetherCtx { materials, broken_materials })
    }

    /// `true` iff `name` is a `material` this block declares but the parser could not read.
    ///
    /// The consumer's cue to suppress ITSELF (§7.3): a scene that mints this material cannot
    /// expand, and "no material `gold` in this aether block" would be false — `gold` is declared
    /// three lines up. One fault, one error.
    pub fn material_is_broken(&self, name: &Ident) -> bool {
        self.broken_materials.contains(&name)
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

/// One declared symbol of the block, from EITHER list — the view §4's whole-block rules read.
struct BlockSymbol<'a> {
    /// The construct's keyword.
    keyword: &'static str,
    /// Its declared name; `None` only for a broken construct whose name never parsed.
    name: Option<&'a Ident>,
    /// Whether this construct's name occupies a fn item (§4's rule splits on it).
    emits_fn: bool,
    /// The noun a duplicate diagnostic uses for that fn ("builder fn", "spawn fn").
    fn_noun: &'static str,
}

/// Every declared symbol in the block, in SOURCE order across both lists.
///
/// The merge is what makes "the first `material` of this name is here" point at the earlier
/// declaration rather than at whichever list happened to hold it —
/// [`BrokenConstruct::after`](crate::ast::BrokenConstruct::after) is the ordering key, since two
/// separate vectors cannot answer "which came first" on their own.
fn block_symbols(block: &AetherBlock) -> Vec<BlockSymbol<'_>> {
    let mut out: Vec<BlockSymbol<'_>> = Vec::with_capacity(block.constructs.len());
    let mut broken = block.broken.iter().peekable();
    for (i, c) in block.constructs.iter().enumerate() {
        while broken.peek().is_some_and(|b| b.after <= i) {
            let b = broken.next().expect("invariant: peek said Some");
            push_broken(&mut out, b);
        }
        out.push(BlockSymbol {
            keyword: c.keyword(),
            name: Some(c.name()),
            emits_fn: c.emits_fn(),
            fn_noun: c.fn_noun(),
        });
    }
    for b in broken {
        push_broken(&mut out, b);
    }
    out
}

fn push_broken<'a>(out: &mut Vec<BlockSymbol<'a>>, b: &'a BrokenConstruct) {
    let Some(keyword) = b.keyword else { return };
    let emits_fn = b.stub.as_ref().is_some_and(crate::ast::Stub::emits_fn);
    out.push(BlockSymbol {
        keyword,
        name: b.name(),
        emits_fn,
        // The parsed half derives this from the construct; the recovery half has only the
        // keyword, and the two must agree — `material` says "builder fn" on both paths.
        fn_noun: match keyword {
            "material" => "builder fn",
            "scene" => "spawn fn",
            _ => "fn",
        },
    });
}

/// §4's duplicate-name rule over the fn-producing half of the registry (see the module docs for
/// why the type-producing half stays with rustc).
///
/// The error lands on the SECOND declaration and carries the first's span — the shape `plugin` ×
/// `plugin` and `material` × `material` already ship with, generalized rather than re-invented.
///
/// Runs over the UNION (§7.3): a `material gold` that did not parse still occupies the name
/// `gold`, and skipping it does not make the collision go away — it moves the report to rustc's
/// E0428, which for two macro-generated fns puts both of its labels on the `aether!` token and
/// names no user token anywhere (the A5 measurement this rule exists for).
fn duplicate_fn_names(block: &AetherBlock) -> syn::Result<()> {
    let symbols = block_symbols(block);
    for (i, c) in symbols.iter().enumerate() {
        let (true, Some(name)) = (c.emits_fn, c.name) else {
            continue;
        };
        let Some(first) = symbols[..i]
            .iter()
            .find(|p| p.emits_fn && p.name.is_some_and(|n| n == name))
        else {
            continue;
        };
        let first_name = first.name.expect("invariant: the finder required a name");
        let msg = if first.keyword == c.keyword {
            format!(
                "duplicate {kw} `{name}` — each {kw} expands to a {noun} of its own name, and two of one name is one fn defined twice",
                kw = c.keyword,
                noun = c.fn_noun,
            )
        } else {
            format!(
                "`{name}` is declared twice in this aether block — the `{a}` and the `{b}` both expand to a fn of that name",
                a = first.keyword,
                b = c.keyword,
            )
        };
        let mut e = diag::err(name.span(), msg);
        e.combine(diag::err(
            first_name.span(),
            format!("the first `{}` of this name is here", first.keyword),
        ));
        return Err(e);
    }
    Ok(())
}
