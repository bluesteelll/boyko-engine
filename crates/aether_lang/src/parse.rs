//! The block parser: dispatch on the leading contextual keyword (§6.1's registry), one `Parse`
//! per construct. Rung A0 registers `component` and `tag`; an unregistered-but-planned keyword
//! gets an honest "not implemented at this rung" rather than the unknown-construct error, so a
//! user tracking the roadmap is told the truth about which failure they hit.

use syn::Ident;
use syn::parse::{Parse, ParseStream};
use syn::{Path, Token, Type, parenthesized};

use crate::ast::{AetherBlock, ComponentDef, Construct, HookKind, TagDef};
use crate::diag;

impl Parse for AetherBlock {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut constructs = Vec::new();
        while !input.is_empty() {
            let head: Ident = input.fork().parse().map_err(|_| {
                diag::err(input.span(), "expected a construct keyword (component, tag, …)")
            })?;
            let kw = head.to_string();
            match kw.as_str() {
                "component" => constructs.push(Construct::Component(parse_component(input)?)),
                "tag" => constructs.push(Construct::Tag(parse_tag(input)?)),
                // Planned constructs (§9, rungs A1..A6): name the rung rather than pretending
                // the keyword is unknown — a misspelling and a not-yet-shipped construct are
                // different failures and deserve different messages.
                "bundle" | "event" => {
                    return Err(diag::err(
                        head.span(),
                        format!("`{kw}` is an Aether construct but lands at rung A1 (docs/AETHER-LANG-PLAN.md §9); this build carries rung A0 (component, tag)"),
                    ));
                }
                "system" | "plugin" => {
                    return Err(diag::err(
                        head.span(),
                        format!("`{kw}` is an Aether construct but lands at rung A2; this build carries rung A0 (component, tag)"),
                    ));
                }
                "machine" => {
                    return Err(diag::err(
                        head.span(),
                        "`machine` is an Aether construct but lands at rung A3; this build carries rung A0 (component, tag)",
                    ));
                }
                "material" => {
                    return Err(diag::err(
                        head.span(),
                        "`material` is an Aether construct but lands at rung A5; this build carries rung A0 (component, tag)",
                    ));
                }
                "scene" => {
                    return Err(diag::err(
                        head.span(),
                        "`scene` is an Aether construct but lands at rung A6; this build carries rung A0 (component, tag)",
                    ));
                }
                other => return Err(diag::unknown_construct(head.span(), other)),
            }
        }
        Ok(AetherBlock { constructs })
    }
}

/// `component NAME { item* }` — items: `field: Type,` | `requires P, Q,` | `on_* = path,`
/// | `no_bundle,`. Trailing commas always permitted (§2).
fn parse_component(input: ParseStream) -> syn::Result<ComponentDef> {
    let _kw: Ident = input.parse()?; // `component` — peeked by the dispatcher
    let name: Ident = input.parse().map_err(|e| {
        diag::err(e.span(), "expected a component name after `component`")
    })?;
    let name_str = name.to_string();
    if !name_str.starts_with(|c: char| c.is_ascii_uppercase()) {
        // §2 case convention, diagnosed EARLY with the name's own span: components expand to
        // types, and a lowercase type would fail far away in the derive's output.
        return Err(diag::err(
            name.span(),
            format!("component names are UpperCamelCase — they expand to types (rename `{name_str}` to `{}`)",
                upper_camel(&name_str)),
        ));
    }

    let body;
    syn::braced!(body in input);

    let mut def = ComponentDef {
        name,
        fields: Vec::new(),
        requires: Vec::new(),
        hooks: Vec::new(),
        no_bundle: false,
    };

    while !body.is_empty() {
        let item_head: Ident = body.fork().parse().map_err(|_| {
            diag::err(body.span(), "expected a field, `requires`, a hook key, or `no_bundle`")
        })?;
        let head_str = item_head.to_string();

        if head_str == "requires" {
            let _: Ident = body.parse()?;
            // One or more comma-separated paths; the shared trailing comma is consumed by the
            // item loop's own `,` eat below, so `requires A, B,` and `requires A, B` both parse.
            loop {
                let p: Path = body.parse().map_err(|e| {
                    diag::err(e.span(), "`requires` takes one or more component paths")
                })?;
                def.requires.push(p);
                if body.peek(Token![,]) && !body.peek2(Token![,]) {
                    // A comma may separate the next path OR terminate the item; look ahead: if
                    // what follows the comma is another item head (a known key or `ident :`),
                    // stop consuming paths and let the item loop handle it.
                    let fork = body.fork();
                    let _: Token![,] = fork.parse()?;
                    if fork.is_empty() {
                        break;
                    }
                    if let Ok(next) = fork.fork().parse::<Ident>() {
                        let ns = next.to_string();
                        // `ident :` opens a FIELD — but `ident ::` continues a PATH, so the
                        // `::` check must come first (`requires A, b::C` vs the field `b: T`).
                        let is_item_head = ns == "requires"
                            || ns == "no_bundle"
                            || HookKind::from_str(&ns).is_some()
                            || (fork.peek2(Token![:]) && !fork.peek2(Token![::]));
                        if is_item_head {
                            break;
                        }
                    }
                    let _: Token![,] = body.parse()?;
                    continue;
                }
                break;
            }
        } else if head_str == "no_bundle" {
            let flag: Ident = body.parse()?;
            if def.no_bundle {
                return Err(diag::err(flag.span(), "duplicate `no_bundle`"));
            }
            def.no_bundle = true;
        } else if let Some(kind) = HookKind::from_str(&head_str) {
            let key: Ident = body.parse()?;
            let _: Token![=] = body.parse().map_err(|e| {
                diag::err(e.span(), format!("hook `{head_str}` takes `= path` (a fn path)"))
            })?;
            let path: Path = body.parse()?;
            if def.hooks.iter().any(|(k, _)| *k == kind) {
                return Err(diag::err(key.span(), format!("duplicate hook `{head_str}`")));
            }
            def.hooks.push((kind, path));
        } else {
            // A field: `name: Type`.
            let fname: Ident = body.parse()?;
            let _: Token![:] = body.parse().map_err(|_| {
                diag::err(
                    fname.span(),
                    format!("expected `:` after field `{fname}` (or a known item: requires / on_add / on_insert / on_replace / on_remove / no_bundle)"),
                )
            })?;
            let ty: Type = body.parse()?;
            def.fields.push((fname, ty));
        }

        if body.peek(Token![,]) {
            let _: Token![,] = body.parse()?;
        } else if !body.is_empty() {
            return Err(diag::err(body.span(), "expected `,` between component items"));
        }
    }

    Ok(def)
}

/// `tag NAME;` / `tag NAME(bitset);`
fn parse_tag(input: ParseStream) -> syn::Result<TagDef> {
    let _kw: Ident = input.parse()?; // `tag`
    let name: Ident = input.parse().map_err(|e| diag::err(e.span(), "expected a tag name after `tag`"))?;
    let name_str = name.to_string();
    if !name_str.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Err(diag::err(
            name.span(),
            format!("tag names are UpperCamelCase — they expand to types (rename `{name_str}` to `{}`)",
                upper_camel(&name_str)),
        ));
    }
    let mut bitset = false;
    if input.peek(syn::token::Paren) {
        let inner;
        parenthesized!(inner in input);
        let flag: Ident = inner.parse().map_err(|_| {
            diag::err(inner.span(), "the only tag modifier is `(bitset)` — the EnableTag backend")
        })?;
        if flag != "bitset" {
            return Err(diag::err(
                flag.span(),
                format!("unknown tag modifier `{flag}`; the only one is `bitset` (the EnableTag backend)"),
            ));
        }
        if !inner.is_empty() {
            return Err(diag::err(inner.span(), "`(bitset)` takes nothing else"));
        }
        bitset = true;
    }
    input.parse::<Token![;]>().map_err(|e| {
        diag::err(e.span(), "a tag declaration ends with `;` (tags have no body — a component with fields wants `component`)")
    })?;
    Ok(TagDef { name, bitset })
}

/// Best-effort UpperCamelCase suggestion for the §2 case diagnostics.
fn upper_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = true;
    for c in s.chars() {
        if c == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}
