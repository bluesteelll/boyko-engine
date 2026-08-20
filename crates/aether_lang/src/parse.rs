//! The block parser: dispatch on the leading contextual keyword (§6.1's registry), one `Parse`
//! per construct. Rung A0 registers `component` and `tag`; an unregistered-but-planned keyword
//! gets an honest "not implemented at this rung" rather than the unknown-construct error, so a
//! user tracking the roadmap is told the truth about which failure they hit.

use syn::Ident;
use syn::parse::{Parse, ParseStream};
use syn::{Path, Token, Type, parenthesized};

use crate::ast::{AetherBlock, BundleDef, ComponentDef, Construct, EvField, EventDef, HookKind, TagDef};
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
                "bundle" => constructs.push(Construct::Bundle(parse_bundle(input)?)),
                "event" => constructs.push(Construct::Event(parse_event(input)?)),
                // Planned constructs (§9, rungs A2..A6): name the rung rather than pretending
                // the keyword is unknown — a misspelling and a not-yet-shipped construct are
                // different failures and deserve different messages.
                "system" | "plugin" => {
                    return Err(diag::err(
                        head.span(),
                        format!("`{kw}` is an Aether construct but lands at rung A2; this build carries rungs A0..A1 (component, tag, bundle, event)"),
                    ));
                }
                "machine" => {
                    return Err(diag::err(
                        head.span(),
                        "`machine` is an Aether construct but lands at rung A3; this build carries rungs A0..A1 (component, tag, bundle, event)",
                    ));
                }
                "material" => {
                    return Err(diag::err(
                        head.span(),
                        "`material` is an Aether construct but lands at rung A5; this build carries rungs A0..A1 (component, tag, bundle, event)",
                    ));
                }
                "scene" => {
                    return Err(diag::err(
                        head.span(),
                        "`scene` is an Aether construct but lands at rung A6; this build carries rungs A0..A1 (component, tag, bundle, event)",
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

/// §3.2's own arity cap mirror — the derive owns the rule; Aether owns the friendlier span.
const MAX_BUNDLE_ARITY: usize = 16;

/// `bundle NAME { field: Type, … }` (§3.2).
fn parse_bundle(input: ParseStream) -> syn::Result<BundleDef> {
    let _kw: Ident = input.parse()?; // `bundle`
    let name: Ident = input.parse().map_err(|e| diag::err(e.span(), "expected a bundle name after `bundle`"))?;
    let name_str = name.to_string();
    if !name_str.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Err(diag::err(
            name.span(),
            format!("bundle names are UpperCamelCase — they expand to types (rename `{name_str}` to `{}`)",
                upper_camel(&name_str)),
        ));
    }
    let body;
    syn::braced!(body in input);
    let mut fields = Vec::new();
    while !body.is_empty() {
        let fname: Ident = body.parse().map_err(|e| diag::err(e.span(), "expected a bundle field"))?;
        let _: Token![:] = body.parse().map_err(|_| diag::err(fname.span(), format!("expected `:` after bundle field `{fname}`")))?;
        let ty: Type = body.parse()?;
        if fields.len() == MAX_BUNDLE_ARITY {
            // The friendlier span the §3.2 pre-check exists for: ON the 17th field, before the
            // derive's own downstream refusal.
            return Err(diag::err(
                fname.span(),
                "bundle arity is capped at 16 (`MAX_BUNDLE_ARITY`) — split it",
            ));
        }
        fields.push((fname, ty));
        if body.peek(Token![,]) {
            let _: Token![,] = body.parse()?;
        } else if !body.is_empty() {
            return Err(diag::err(body.span(), "expected `,` between bundle fields"));
        }
    }
    Ok(BundleDef { name, fields })
}

/// `event NAME { participant/parameter fields }` (§3.4). A participant is TYPE-SHAPED —
/// `name: entity(A, B)` — and the empty context is deliberately not defaulted.
fn parse_event(input: ParseStream) -> syn::Result<EventDef> {
    let _kw: Ident = input.parse()?; // `event`
    let name: Ident = input.parse().map_err(|e| diag::err(e.span(), "expected an event name after `event`"))?;
    let name_str = name.to_string();
    if !name_str.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Err(diag::err(
            name.span(),
            format!("event names are UpperCamelCase — they expand to types (rename `{name_str}` to `{}`)",
                upper_camel(&name_str)),
        ));
    }
    let body;
    syn::braced!(body in input);
    let mut fields = Vec::new();
    while !body.is_empty() {
        let fname: Ident = body.parse().map_err(|e| diag::err(e.span(), "expected an event field"))?;
        let _: Token![:] = body.parse().map_err(|_| diag::err(fname.span(), format!("expected `:` after event field `{fname}`")))?;
        // `entity` in the type position is the participant marker — contextual (§2): a PARAMETER
        // may still be of a user type named `entity` via a qualified path.
        if input_is_bare_entity(&body) {
            let ent: Ident = body.parse()?; // `entity`
            if !body.peek(syn::token::Paren) {
                return Err(diag::err(
                    ent.span(),
                    "participant fields name their component context: `entity(ComponentA, ComponentB)`",
                ));
            }
            let inner;
            parenthesized!(inner in body);
            let mut components = Vec::new();
            loop {
                let p: Path = inner.parse().map_err(|e| {
                    diag::err(e.span(), "participant fields name their component context: `entity(ComponentA, ComponentB)`")
                })?;
                components.push(p);
                if inner.peek(Token![,]) {
                    let _: Token![,] = inner.parse()?;
                    if inner.is_empty() {
                        break;
                    }
                } else {
                    break;
                }
            }
            fields.push(EvField::Participant { name: fname, components });
        } else {
            let ty: Type = body.parse()?;
            fields.push(EvField::Parameter { name: fname, ty });
        }
        if body.peek(Token![,]) {
            let _: Token![,] = body.parse()?;
        } else if !body.is_empty() {
            return Err(diag::err(body.span(), "expected `,` between event fields"));
        }
    }
    Ok(EventDef { name, fields })
}

/// `true` iff the next tokens are the bare `entity` marker (not a path like `foo::entity` and
/// not a generic type) — the §2 contextual-keyword rule at the event-field type position.
fn input_is_bare_entity(body: ParseStream) -> bool {
    let fork = body.fork();
    match fork.parse::<Ident>() {
        Ok(id) => id == "entity" && !fork.peek(Token![::]) && !fork.peek(Token![<]),
        Err(_) => false,
    }
}