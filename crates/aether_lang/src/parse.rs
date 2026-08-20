//! The block parser: dispatch on the leading contextual keyword (§6.1's registry), one `Parse`
//! per construct. Rung A0 registers `component` and `tag`; an unregistered-but-planned keyword
//! gets an honest "not implemented at this rung" rather than the unknown-construct error, so a
//! user tracking the roadmap is told the truth about which failure they hit.

use proc_macro2::TokenStream;
use syn::Ident;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Path, Token, Type, parenthesized};

use crate::ast::{
    AetherBlock, BundleDef, ComponentDef, Construct, EvField, EventDef, FilterKind, HandlerDef,
    HookKind, MachineDef, OrderKind, PluginDef, Schedule, StateDef, SysParam, SysParamTy,
    SystemDef, TagDef, TransitionDef,
};
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
                "system" => constructs.push(Construct::System(parse_system(input)?)),
                "plugin" => constructs.push(Construct::Plugin(parse_plugin(input)?)),
                "machine" => constructs.push(Construct::Machine(parse_machine(input)?)),
                // Planned constructs (§9, rungs A5..A6): name the rung rather than pretending
                // the keyword is unknown — a misspelling and a not-yet-shipped construct are
                // different failures and deserve different messages.
                "material" => {
                    return Err(diag::err(
                        head.span(),
                        "`material` is an Aether construct but lands at rung A5; this build carries rungs A0..A3 (component, tag, bundle, event, system, plugin, machine)",
                    ));
                }
                "scene" => {
                    return Err(diag::err(
                        head.span(),
                        "`scene` is an Aether construct but lands at rung A6; this build carries rungs A0..A3 (component, tag, bundle, event, system, plugin, machine)",
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
    // §2 case convention, diagnosed EARLY with the name's own span: components expand to
    // types, and a lowercase type would fail far away in the derive's output.
    upper_camel_gate(&name, "component names are UpperCamelCase — they expand to types")?;

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
                    let after_ident = fork.fork();
                    if let Ok(next) = after_ident.parse::<Ident>() {
                        let ns = next.to_string();
                        // `ident ::` CONTINUES A PATH no matter which ident it is — the
                        // keyword names too (`requires A, no_bundle::C` is a path whose first
                        // segment happens to spell a keyword; only bare keywords open items).
                        // `ident :` (single colon) opens a FIELD.
                        let is_item_head = !after_ident.peek(Token![::])
                            && (ns == "requires"
                                || ns == "no_bundle"
                                || HookKind::from_str(&ns).is_some()
                                || after_ident.peek(Token![:]));
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
    upper_camel_gate(&name, "tag names are UpperCamelCase — they expand to types")?;
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

/// The §2 case gate for type-producing names. Unicode-correct — `char::is_uppercase`, not the
/// ASCII probe (a name titled with a non-ASCII capital must pass), and the rename suggestion is
/// attached only when [`upper_camel`] actually changes the spelling — a self-identical rename
/// explains nothing.
///
/// A RAW ident (`r#Foo`) prints as `r#Foo`, whose first char is the escape's `r` — so the gate
/// must classify the ESCAPED spelling, otherwise `r#Foo` is refused for being lowercase and the
/// suggestion reads `R#Foo`, which is not a legal ident at all.
fn upper_camel_gate(name: &Ident, base: &str) -> syn::Result<()> {
    let raw = name.to_string();
    let s = raw.strip_prefix("r#").unwrap_or(&raw);
    if s.starts_with(char::is_uppercase) {
        return Ok(());
    }
    let sugg = upper_camel(s);
    let msg = if !sugg.is_empty() && sugg != s {
        format!("{base} (rename `{raw}` to `{sugg}`)")
    } else {
        base.to_string()
    };
    Err(diag::err(name.span(), msg))
}

/// Refuse a `let` binding where a plain `bool` is required.
///
/// `if let Some(x) = y` and `when let …` both parse — `Expr::Let` is a real expression node,
/// legal only inside an `if`/`while` scrutinee. Aether drops the surrounding `if` and splices
/// the expression into `if !(…)` (a guard) or `.run_if(…)` (a condition), where a `let` is not
/// valid Rust at all. Caught here, the error names the user's own `let`; passed through, rustc
/// reports it against a synthesized `if` the user never wrote.
fn reject_let_binding(e: &Expr, role: &str, kw: &str) -> syn::Result<()> {
    let Expr::Let(l) = e else {
        return Ok(());
    };
    Err(diag::err(
        l.let_token.span,
        format!(
            "`let` bindings are not usable as {role} — {kw} takes a plain bool expression (bind with a `local<…>` param or match inside the body instead)"
        ),
    ))
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
    upper_camel_gate(&name, "bundle names are UpperCamelCase — they expand to types")?;
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
    upper_camel_gate(&name, "event names are UpperCamelCase — they expand to types")?;
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
                // The derive's `components = "…"` channel is a comma-separated list of BARE
                // idents (boyko_macros splits on `,` and mints an Ident from each piece) — a
                // qualified path would panic the downstream macro with no user span, and a
                // generic argument's own comma would corrupt the split. Refuse both HERE, on
                // the user's tokens (the never-panic contract).
                if p.leading_colon.is_some()
                    || p.segments.len() != 1
                    || !p.segments[0].arguments.is_none()
                {
                    let shown = quote::quote!(#p).to_string().replace(' ', "");
                    return Err(diag::err(
                        p.segments[0].ident.span(),
                        format!("participant context components are bare component idents (the `#[event]` channel is comma-separated identifiers) — found `{shown}`; import the component and name it unqualified"),
                    ));
                }
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

/// The clause keywords of §3.3, for the unknown-clause did-you-mean.
const CLAUSE_KEYWORDS: &[&str] = &["on", "in", "before", "after", "when"];

/// The filter keywords of §3.3, for the unknown-filter did-you-mean.
const FILTER_KEYWORDS: &[&str] = &["with", "without", "added", "changed", "enabled", "disabled"];

/// `system NAME(params) clause* { body }` (§3.3).
fn parse_system(input: ParseStream) -> syn::Result<SystemDef> {
    let _kw: Ident = input.parse()?; // `system`
    let name: Ident = input
        .parse()
        .map_err(|e| diag::err(e.span(), "expected a system name after `system`"))?;
    let name_str = name.to_string();
    if name_str.starts_with(char::is_uppercase) {
        // The §2 case convention mirrored: systems expand to FNS, and an UpperCamelCase fn
        // name reads like a type everywhere the plugin registers it.
        return Err(diag::err(
            name.span(),
            format!("system names are snake_case — they expand to fns (rename `{name_str}`)"),
        ));
    }

    let params = parse_params_paren(input)?;

    let mut def = SystemDef {
        name,
        params,
        schedule: None,
        in_sets: Vec::new(),
        orders: Vec::new(),
        whens: Vec::new(),
        body: TokenStream::new(),
    };

    // Clauses run until the body brace. `in` is a real Rust keyword, the rest are contextual.
    let mut first_non_on_span: Option<proc_macro2::Span> = None;
    loop {
        if input.peek(syn::token::Brace) {
            break;
        }
        if input.peek(Token![in]) {
            let kw: Token![in] = input.parse()?;
            let p: Path = input
                .parse()
                .map_err(|e| diag::err(e.span(), "`in` takes a SystemSet path"))?;
            def.in_sets.push((p, kw.span));
            first_non_on_span.get_or_insert(kw.span);
            continue;
        }
        let head: Ident = input.fork().parse().map_err(|_| {
            diag::err(
                input.span(),
                "expected a clause (`on`, `in`, `before`, `after`, `when`) or the system body",
            )
        })?;
        match head.to_string().as_str() {
            "on" => {
                let kw: Ident = input.parse()?;
                if def.schedule.is_some() {
                    return Err(diag::err(
                        kw.span(),
                        "duplicate schedule clause; a system runs on exactly one schedule",
                    ));
                }
                let target: Ident = input.parse().map_err(|e| {
                    diag::err(e.span(), "`on` takes one of: startup, update, fixed")
                })?;
                def.schedule = Some(match target.to_string().as_str() {
                    "startup" => Schedule::Startup,
                    "update" => Schedule::Update,
                    "fixed" => Schedule::Fixed,
                    other => {
                        return Err(diag::err(
                            target.span(),
                            format!("unknown schedule `{other}`; `on` takes one of: startup, update, fixed"),
                        ));
                    }
                });
            }
            "before" | "after" => {
                let kw: Ident = input.parse()?;
                let kind = if kw == "before" { OrderKind::Before } else { OrderKind::After };
                let p: Path = input.parse().map_err(|e| {
                    diag::err(
                        e.span(),
                        format!("`{kw}` takes a SystemSet path or a sibling aether system name"),
                    )
                })?;
                def.orders.push((kind, p, kw.span()));
                first_non_on_span.get_or_insert(kw.span());
            }
            "when" => {
                let kw: Ident = input.parse()?;
                // `parse_without_eager_brace`: the body brace that follows must never be
                // swallowed as a struct-literal tail of the condition expression.
                let e: Expr = input.call(Expr::parse_without_eager_brace).map_err(|e| {
                    diag::err(e.span(), "`when` takes a condition expression (a fn implementing IntoSystem<(), bool, _>)")
                })?;
                reject_let_binding(&e, "a run condition", "`when`")?;
                def.whens.push((e, kw.span()));
                first_non_on_span.get_or_insert(kw.span());
            }
            other => {
                let mut msg = format!(
                    "unknown clause `{other}`; clauses are: on, in, before, after, when"
                );
                if let Some(sugg) = diag::did_you_mean(other, CLAUSE_KEYWORDS) {
                    msg.push_str(&format!(" (did you mean `{sugg}`?)"));
                }
                return Err(diag::err(head.span(), msg));
            }
        }
    }

    // §3.3: startup systems run once, pre-loop — every clause but `on` is meaningless there
    // and rejected on the offending clause's own keyword span.
    if def.schedule == Some(Schedule::Startup)
        && let Some(span) = first_non_on_span
    {
        return Err(diag::err(
            span,
            "scheduling clauses other than `on` are rejected on startup systems — the engine runs them once, pre-loop",
        ));
    }

    let body;
    syn::braced!(body in input);
    def.body = body.parse()?; // verbatim tokens, spans preserved
    Ok(def)
}

/// A parenthesized param list in the SYSTEM grammar — shared by `system` and the `machine`
/// handlers/transitions (§3.5: "same param grammar as system").
fn parse_params_paren(input: ParseStream) -> syn::Result<Vec<SysParam>> {
    let paren_body;
    parenthesized!(paren_body in input);
    let mut params = Vec::new();
    while !paren_body.is_empty() {
        params.push(parse_sys_param(&paren_body)?);
        if paren_body.peek(Token![,]) {
            let _: Token![,] = paren_body.parse()?;
        } else if !paren_body.is_empty() {
            return Err(diag::err(paren_body.span(), "expected `,` between params"));
        }
    }
    Ok(params)
}

/// One system param: `mut? NAME ':' param_ty` (§3.3).
fn parse_sys_param(input: ParseStream) -> syn::Result<SysParam> {
    let explicit_mut = if input.peek(Token![mut]) {
        let _: Token![mut] = input.parse()?;
        true
    } else {
        false
    };
    let name: Ident = input
        .parse()
        .map_err(|e| diag::err(e.span(), "expected a system param name"))?;
    let _: Token![:] = input
        .parse()
        .map_err(|_| diag::err(name.span(), format!("expected `:` after system param `{name}`")))?;
    let ty = parse_sys_param_ty(input)?;
    Ok(SysParam { explicit_mut, name, ty })
}

/// The §3.3 `param_ty` sugar table. Everything unclaimed passes through as a verbatim type —
/// the escape hatch that makes any real `SystemParam` work day one.
fn parse_sys_param_ty(input: ParseStream) -> syn::Result<SysParamTy> {
    // `mut res<T>` — the one two-token sugar; `mut` in the TYPE position only pairs with `res`.
    if input.peek(Token![mut]) {
        let _: Token![mut] = input.parse()?;
        let kw: Ident = input.parse().map_err(|e| {
            diag::err(e.span(), "in the type position `mut` pairs only with `res`: `mut res<T>`")
        })?;
        if kw != "res" {
            return Err(diag::err(
                kw.span(),
                format!("in the type position `mut` pairs only with `res`: `mut res<T>` (found `{kw}`)"),
            ));
        }
        return Ok(SysParamTy::ResMut(parse_angle_type(input, "res")?));
    }

    let fork = input.fork();
    let Ok(head) = fork.parse::<Ident>() else {
        // Not ident-led (`&T`, `(A, B)`, …) — verbatim.
        return Ok(SysParamTy::Verbatim(input.parse()?));
    };
    let sugar = head.to_string();
    match sugar.as_str() {
        // The §2 contextual rule: a sugar keyword is only a sugar when its OWN syntax follows
        // (angle brackets, or nothing for `commands`); `query::Thing`, `res` as a bare user
        // type, etc. fall through verbatim.
        "query" if fork.peek(syn::token::Paren) => {
            let _: Ident = input.parse()?;
            Err(diag::err(
                input.span(),
                "query takes angle brackets: `query<&mut Transform>`",
            ))
        }
        "query" if fork.peek(Token![<]) => {
            let _: Ident = input.parse()?;
            let _: Token![<] = input.parse()?;
            let data: Type = input.parse().map_err(|e| {
                diag::err(e.span(), "`query<…>` opens with the query data (a type: `&T`, `&mut T`, a tuple, …)")
            })?;
            let mut filters = Vec::new();
            while input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
                if input.peek(Token![>]) {
                    break; // trailing comma before `>`
                }
                let fkw: Ident = input.parse().map_err(|e| {
                    diag::err(e.span(), "expected a query filter: with, without, added, changed, enabled, disabled")
                })?;
                let Some(kind) = FilterKind::from_str(&fkw.to_string()) else {
                    let mut msg = format!(
                        "unknown query filter `{fkw}`; filters are: with, without, added, changed, enabled, disabled"
                    );
                    if let Some(sugg) = diag::did_you_mean(&fkw.to_string(), FILTER_KEYWORDS) {
                        msg.push_str(&format!(" (did you mean `{sugg}`?)"));
                    }
                    return Err(diag::err(fkw.span(), msg));
                };
                let p: Path = input.parse().map_err(|e| {
                    diag::err(e.span(), format!("`{fkw}` takes a component path"))
                })?;
                filters.push((kind, p));
            }
            let _: Token![>] = input
                .parse()
                .map_err(|e| diag::err(e.span(), "expected `>` to close `query<…>`"))?;
            Ok(SysParamTy::Query { data, filters })
        }
        "res" if fork.peek(Token![<]) => {
            let _: Ident = input.parse()?;
            Ok(SysParamTy::Res(parse_angle_type(input, "res")?))
        }
        "local" if fork.peek(Token![<]) => {
            let _: Ident = input.parse()?;
            Ok(SysParamTy::Local(parse_angle_type(input, "local")?))
        }
        "events" if fork.peek(Token![<]) => {
            let _: Ident = input.parse()?;
            Ok(SysParamTy::Events(parse_angle_type(input, "events")?))
        }
        "emit" if fork.peek(Token![<]) => {
            let _: Ident = input.parse()?;
            Ok(SysParamTy::Emit(parse_angle_type(input, "emit")?))
        }
        "commands" if !fork.peek(Token![::]) && !fork.peek(Token![<]) => {
            let _: Ident = input.parse()?;
            Ok(SysParamTy::Commands)
        }
        _ => Ok(SysParamTy::Verbatim(input.parse()?)),
    }
}

/// `'<' TYPE '>'` after a sugar keyword (`res` / `local` / `events` / `emit`).
fn parse_angle_type(input: ParseStream, kw: &str) -> syn::Result<Type> {
    let _: Token![<] = input
        .parse()
        .map_err(|e| diag::err(e.span(), format!("`{kw}` takes angle brackets: `{kw}<T>`")))?;
    let ty: Type = input.parse()?;
    let _: Token![>] = input
        .parse()
        .map_err(|e| diag::err(e.span(), format!("expected `>` to close `{kw}<…>`")))?;
    Ok(ty)
}

/// `machine NAME { initial X; state* }` (§3.5).
fn parse_machine(input: ParseStream) -> syn::Result<MachineDef> {
    let _kw: Ident = input.parse()?; // `machine`
    let name: Ident = input
        .parse()
        .map_err(|e| diag::err(e.span(), "expected a machine name after `machine`"))?;
    upper_camel_gate(&name, "machine names are UpperCamelCase — they expand to enums")?;
    let body;
    syn::braced!(body in input);

    // The required leading `initial LEAF;` — a machine without one has no inserted value.
    let init_kw: Ident = body
        .parse()
        .map_err(|e| diag::err(e.span(), "a machine opens with `initial <State>;`"))?;
    if init_kw != "initial" {
        return Err(diag::err(init_kw.span(), "a machine opens with `initial <State>;`"));
    }
    let initial: Ident = body
        .parse()
        .map_err(|e| diag::err(e.span(), "`initial` names a state"))?;
    body.parse::<Token![;]>()
        .map_err(|e| diag::err(e.span(), "`initial <State>` ends with `;`"))?;

    // The declaration counter threads through the whole body, nested states included, so the
    // index it stamps is exact SOURCE order across the machine (see `TransitionDef::decl_index`).
    let mut decl = 0usize;
    let mut states = Vec::new();
    while !body.is_empty() {
        states.push(parse_state(&body, &mut decl)?);
    }
    Ok(MachineDef { name, initial, states })
}

/// `state NAME { (initial X;)? (enter|exit|on|state)* }` (§3.5).
fn parse_state(input: ParseStream, decl: &mut usize) -> syn::Result<StateDef> {
    let kw: Ident = input.parse().map_err(|e| {
        diag::err(e.span(), "expected `state` (a machine body holds only states after `initial`)")
    })?;
    if kw != "state" {
        return Err(diag::err(
            kw.span(),
            format!("expected `state`, found `{kw}` (a machine body holds only states after `initial`)"),
        ));
    }
    let name: Ident = input
        .parse()
        .map_err(|e| diag::err(e.span(), "expected a state name after `state`"))?;
    upper_camel_gate(&name, "state names are UpperCamelCase — leaves become enum variants")?;

    let body;
    syn::braced!(body in input);
    let mut def = StateDef {
        name,
        initial: None,
        enter: None,
        exit: None,
        transitions: Vec::new(),
        children: Vec::new(),
    };

    while !body.is_empty() {
        let head: Ident = body.fork().parse().map_err(|_| {
            diag::err(body.span(), "expected `initial`, `enter`, `exit`, `on`, or a nested `state`")
        })?;
        match head.to_string().as_str() {
            "initial" => {
                let kw: Ident = body.parse()?;
                if def.initial.is_some() {
                    return Err(diag::err(kw.span(), "duplicate `initial` in this state"));
                }
                let target: Ident = body
                    .parse()
                    .map_err(|e| diag::err(e.span(), "`initial` names a child state"))?;
                body.parse::<Token![;]>()
                    .map_err(|e| diag::err(e.span(), "`initial <State>` ends with `;`"))?;
                def.initial = Some(target);
            }
            "enter" | "exit" => {
                let kw: Ident = body.parse()?;
                let is_enter = kw == "enter";
                let params = if body.peek(syn::token::Paren) {
                    parse_params_paren(&body)?
                } else {
                    Vec::new()
                };
                let block;
                syn::braced!(block in body);
                let handler = HandlerDef { params, body: block.parse()? };
                let slot = if is_enter { &mut def.enter } else { &mut def.exit };
                if slot.is_some() {
                    return Err(diag::err(kw.span(), format!("duplicate `{kw}` in this state")));
                }
                *slot = Some(handler);
            }
            "on" => {
                let kw: Ident = body.parse()?;
                let event: Path = body
                    .parse()
                    .map_err(|e| diag::err(e.span(), "`on` takes an event type path"))?;
                let params = if body.peek(syn::token::Paren) {
                    parse_params_paren(&body)?
                } else {
                    Vec::new()
                };
                let guard = if body.peek(Token![if]) {
                    let _: Token![if] = body.parse()?;
                    let g = body.call(Expr::parse_without_eager_brace).map_err(|e| {
                        diag::err(e.span(), "`if` takes a guard expression")
                    })?;
                    reject_let_binding(&g, "a transition guard", "`if`")?;
                    Some(g)
                } else {
                    None
                };
                body.parse::<Token![=>]>().map_err(|e| {
                    diag::err(e.span(), "a transition points at its target: `on Event => State.Path`")
                })?;
                let mut target = Vec::new();
                target.push(body.parse::<Ident>().map_err(|e| {
                    diag::err(e.span(), "the transition target is a state path (`Playing.Paused`)")
                })?);
                while body.peek(Token![.]) {
                    let _: Token![.] = body.parse()?;
                    target.push(body.parse::<Ident>().map_err(|e| {
                        diag::err(e.span(), "the state path continues with a state name after `.`")
                    })?);
                }
                let action = if body.peek(syn::token::Brace) {
                    let block;
                    syn::braced!(block in body);
                    Some(block.parse()?)
                } else {
                    body.parse::<Token![;]>().map_err(|e| {
                        diag::err(e.span(), "a transition ends with an action block or `;`")
                    })?;
                    None
                };
                def.transitions.push(TransitionDef {
                    event,
                    kw_span: kw.span(),
                    decl_index: *decl,
                    params,
                    guard,
                    target,
                    action,
                });
                *decl += 1;
            }
            "state" => def.children.push(parse_state(&body, decl)?),
            other => {
                return Err(diag::err(
                    head.span(),
                    format!("unknown state item `{other}`; state items are: initial, enter, exit, on, state"),
                ));
            }
        }
    }
    Ok(def)
}

/// `plugin NAME;` (§3.3).
fn parse_plugin(input: ParseStream) -> syn::Result<PluginDef> {
    let _kw: Ident = input.parse()?; // `plugin`
    let name: Ident = input
        .parse()
        .map_err(|e| diag::err(e.span(), "expected a plugin name after `plugin`"))?;
    upper_camel_gate(&name, "plugin names are UpperCamelCase — they expand to types")?;
    input.parse::<Token![;]>().map_err(|e| {
        diag::err(e.span(), "a plugin declaration ends with `;` (the systems it registers are sibling `system` items)")
    })?;
    Ok(PluginDef { name })
}