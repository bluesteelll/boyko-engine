//! `state_chart!`'s surface syntax and its parser.
//!
//! The grammar is deliberately **Rust-native**: a param is `name: RealRustType`, never a sugared
//! shorthand. That is what makes the macro usable from hand-written Rust, and it is the seam
//! Aether's `machine` front-end lowers onto — Aether owns its own sugar table and hands this
//! macro finished types, so there is exactly one flattening implementation and exactly one place
//! that knows what `res<T>` means.

use proc_macro2::{Span, TokenStream};
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, Path, Token, Type};

/// One `name: Type` binding in an `enter` / `exit` / `on` parameter list.
pub struct Param {
    /// A `mut` binding. Merged by UNION across the handlers a leaf's system fuses: one site
    /// needing `mut` makes the merged binding `mut`, since it is one signature now.
    pub mutable: bool,
    pub name: Ident,
    pub ty: Type,
}

/// An `enter` / `exit` action: params plus a verbatim body.
pub struct Handler {
    pub params: Vec<Param>,
    /// The braced block's INNER tokens, re-emitted untouched (spans and formatting preserved).
    pub body: TokenStream,
}

/// One `on EVENT (params)? (if GUARD)? => TARGET (BLOCK | ;)` route.
pub struct Transition {
    /// The event type path, verbatim.
    pub event: Path,
    /// The `on` keyword's span — duplicate-handler errors point at the SECOND `on`.
    pub kw_span: Span,
    /// Source position among the CHART's transitions, assigned as the parser walks.
    ///
    /// Arbitration is **first declared wins**, and the per-leaf inheritance walk runs
    /// innermost-first, which is not declaration order. This index is the only exact record of
    /// the order the author wrote, so the model re-sorts on it. A span cannot serve:
    /// `proc_macro2::Span::start()` needs the `span-locations` feature, which this crate does not
    /// take.
    pub decl_index: usize,
    pub params: Vec<Param>,
    /// `if EXPR` — verbatim. A failed guard SKIPS the event rather than consuming the frame.
    pub guard: Option<Expr>,
    /// The root-anchored target path (`Playing.Paused`), unresolved segments.
    pub target: Vec<Ident>,
    /// The action block's inner tokens (`None` for the `;` form).
    pub action: Option<TokenStream>,
}

/// One `state` node — composite when `children` is non-empty.
pub struct StateNode {
    pub name: Ident,
    pub initial: Option<Ident>,
    pub enter: Option<Handler>,
    pub exit: Option<Handler>,
    /// Routes in source order.
    pub transitions: Vec<Transition>,
    /// Nested states in source order. Empty ⇒ LEAF.
    pub children: Vec<StateNode>,
}

/// A whole `state_chart! { chart N; initial S; state … }` invocation.
pub struct ChartInput {
    pub name: Ident,
    pub initial: Ident,
    pub states: Vec<StateNode>,
}

impl Parse for ChartInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let head: Ident = input
            .parse()
            .map_err(|e| err(e.span(), "a chart opens with `chart <Name>;`"))?;
        if head != "chart" {
            return Err(err(head.span(), "a chart opens with `chart <Name>;`"));
        }
        let name: Ident = input
            .parse()
            .map_err(|e| err(e.span(), "`chart` takes the chart's type name"))?;
        input
            .parse::<Token![;]>()
            .map_err(|e| err(e.span(), "`chart <Name>` ends with `;`"))?;

        let init_kw: Ident = input
            .parse()
            .map_err(|e| err(e.span(), "a chart declares `initial <State>;` before its states"))?;
        if init_kw != "initial" {
            return Err(err(
                init_kw.span(),
                "a chart declares `initial <State>;` before its states",
            ));
        }
        let initial: Ident = input
            .parse()
            .map_err(|e| err(e.span(), "`initial` names a top-level state"))?;
        input
            .parse::<Token![;]>()
            .map_err(|e| err(e.span(), "`initial <State>` ends with `;`"))?;

        let mut decl = 0usize;
        let mut states = Vec::new();
        while !input.is_empty() {
            let head: Ident = input.fork().parse().map_err(|_| {
                err(input.span(), "expected a `state` declaration")
            })?;
            if head != "state" {
                return Err(err(
                    head.span(),
                    format!("expected `state`, found `{head}`"),
                ));
            }
            states.push(parse_state(input, &mut decl)?);
        }
        Ok(ChartInput { name, initial, states })
    }
}

/// `state NAME { … }`, recursively. `decl` is the chart-wide transition counter that records
/// declaration order for arbitration.
fn parse_state(input: ParseStream, decl: &mut usize) -> syn::Result<StateNode> {
    let _kw: Ident = input.parse()?; // `state`
    let name: Ident = input
        .parse()
        .map_err(|e| err(e.span(), "expected a state name after `state`"))?;

    let body;
    syn::braced!(body in input);
    let mut node = StateNode {
        name,
        initial: None,
        enter: None,
        exit: None,
        transitions: Vec::new(),
        children: Vec::new(),
    };

    while !body.is_empty() {
        let head: Ident = body.fork().parse().map_err(|_| {
            err(body.span(), "expected `initial`, `enter`, `exit`, `on`, or a nested `state`")
        })?;
        match head.to_string().as_str() {
            "initial" => {
                let kw: Ident = body.parse()?;
                if node.initial.is_some() {
                    return Err(err(kw.span(), "duplicate `initial` in this state"));
                }
                let target: Ident = body
                    .parse()
                    .map_err(|e| err(e.span(), "`initial` names a child state"))?;
                body.parse::<Token![;]>()
                    .map_err(|e| err(e.span(), "`initial <State>` ends with `;`"))?;
                node.initial = Some(target);
            }
            "enter" | "exit" => {
                let kw: Ident = body.parse()?;
                let is_enter = kw == "enter";
                let params =
                    if body.peek(syn::token::Paren) { parse_params(&body)? } else { Vec::new() };
                let block;
                syn::braced!(block in body);
                let handler = Handler { params, body: block.parse()? };
                let slot = if is_enter { &mut node.enter } else { &mut node.exit };
                if slot.is_some() {
                    return Err(err(kw.span(), format!("duplicate `{kw}` in this state")));
                }
                *slot = Some(handler);
            }
            "on" => {
                let kw: Ident = body.parse()?;
                let event: Path = body
                    .parse()
                    .map_err(|e| err(e.span(), "`on` takes an event type path"))?;
                let params =
                    if body.peek(syn::token::Paren) { parse_params(&body)? } else { Vec::new() };
                // `parse_without_eager_brace` is load-bearing: a bare `Expr` parse would read the
                // target's own `{ … }` action block as a struct literal on the guard's tail.
                let guard = if body.peek(Token![if]) {
                    let _: Token![if] = body.parse()?;
                    Some(
                        body.call(Expr::parse_without_eager_brace)
                            .map_err(|e| err(e.span(), "`if` takes a guard expression"))?,
                    )
                } else {
                    None
                };
                body.parse::<Token![=>]>().map_err(|e| {
                    err(e.span(), "a transition points at its target: `on Event => State.Path`")
                })?;
                let mut target = vec![body.parse::<Ident>().map_err(|e| {
                    err(e.span(), "the transition target is a state path (`Playing.Paused`)")
                })?];
                while body.peek(Token![.]) {
                    let _: Token![.] = body.parse()?;
                    target.push(body.parse::<Ident>().map_err(|e| {
                        err(e.span(), "the state path continues with a state name after `.`")
                    })?);
                }
                let action = if body.peek(syn::token::Brace) {
                    let block;
                    syn::braced!(block in body);
                    Some(block.parse()?)
                } else {
                    body.parse::<Token![;]>()
                        .map_err(|e| err(e.span(), "a transition ends with an action block or `;`"))?;
                    None
                };
                node.transitions.push(Transition {
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
            "state" => node.children.push(parse_state(&body, decl)?),
            other => {
                return Err(err(
                    head.span(),
                    format!(
                        "unknown state item `{other}`; state items are: initial, enter, exit, on, state"
                    ),
                ));
            }
        }
    }
    Ok(node)
}

/// `( (mut)? name: Type , … )` — Rust types verbatim.
fn parse_params(input: ParseStream) -> syn::Result<Vec<Param>> {
    let inner;
    syn::parenthesized!(inner in input);
    let mut out = Vec::new();
    while !inner.is_empty() {
        let mutable = inner.peek(Token![mut]) && {
            let _: Token![mut] = inner.parse()?;
            true
        };
        let name: Ident = inner
            .parse()
            .map_err(|e| err(e.span(), "a parameter is `name: Type`"))?;
        inner
            .parse::<Token![:]>()
            .map_err(|e| err(e.span(), "a parameter is `name: Type`"))?;
        let ty: Type = inner
            .parse()
            .map_err(|e| err(e.span(), "a parameter's type must be a Rust type"))?;
        out.push(Param { mutable, name, ty });
        if inner.is_empty() {
            break;
        }
        inner
            .parse::<Token![,]>()
            .map_err(|e| err(e.span(), "parameters are separated by `,`"))?;
    }
    Ok(out)
}

/// A span-anchored error. Every diagnostic in this module lands on the offending TOKEN, never on
/// the macro call site — a `Span::call_site()` error puts the caret on `state_chart!` and tells
/// the reader nothing about which of their forty lines is meant.
pub fn err(span: Span, msg: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(span, msg)
}
