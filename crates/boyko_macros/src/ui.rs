//! `ui! { .. }` function-like macro implementation.

use proc_macro::TokenStream;

/// Implementation of the `ui!` macro (see the public entry in `lib.rs`).
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    ui_macro::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// The `ui!` authoring macro: parser, compile-time validation, and two-pass
/// codegen. Kept in a dedicated module so its `use`s do not perturb the
/// derive/attribute macros above.
mod ui_macro {
    // Proc-macro expansion: every `HashMap` in this module is the `ui!` parser's
    // duplicate-`#name` validation table, built and dropped inside rustc while
    // compiling the invocation. Nothing here exists in the shipped binary, so it
    // has no engine path at all — let alone a per-frame one.
    #![allow(clippy::disallowed_types)]

    use std::collections::HashMap;

    use proc_macro2::{Span, TokenStream as TokenStream2};
    use quote::{format_ident, quote, quote_spanned};
    use syn::parse::{Parse, ParseStream};
    use syn::spanned::Spanned;
    use syn::{Expr, Ident, LitStr, Token, braced, bracketed};

    // ── Emitted absolute paths (proc-macro `$crate` does not exist — leading-`::`
    //    is the only correct choice; mirrors the bundle/component emit) ──────────

    /// `quote!` fragment for `::boyko_ui::bundles::UiNodeBundle`.
    fn path_ui_node_bundle() -> TokenStream2 {
        quote! { ::boyko_ui::bundles::UiNodeBundle }
    }
    /// `quote!` fragment for `::boyko_ui::components::ComputedRect`.
    fn path_computed_rect() -> TokenStream2 {
        quote! { ::boyko_ui::components::ComputedRect }
    }
    /// `quote!` fragment for `::boyko_ui::components::UiName`.
    fn path_ui_name() -> TokenStream2 {
        quote! { ::boyko_ui::components::UiName }
    }

    /// `UiName` inline-buffer ceiling — mirrors `UiName::CAP` so over-length
    /// names are rejected at macro time (the runtime path only debug-asserts).
    const UI_NAME_CAP: usize = 60;

    // ── Parsed AST (host-side only; never emitted) ──────────────────────────────

    /// One parsed UI node.
    struct UiNode {
        /// From `#name`; the user's ident (call-site span preserved).
        name: Option<Ident>,
        /// Component literal expressions in author declaration order.
        components: Vec<Expr>,
        /// Child nodes (from the `children:` clause).
        children: Vec<UiNode>,
        /// The node's brace span (for node-level diagnostics).
        brace_span: Span,
    }

    /// The whole `ui!` invocation: an optional `commands:` override plus one or
    /// more top-level (root) nodes.
    struct UiInvocation {
        /// `commands: <ident>;` override (default `cmds`).
        commands: Ident,
        /// Top-level nodes (multiple → a tuple of roots).
        roots: Vec<UiNode>,
    }

    // ── Parsing (explicit recursive descent) ────────────────────────────────────

    impl Parse for UiInvocation {
        fn parse(input: ParseStream) -> syn::Result<Self> {
            // Optional preamble: `commands : IDENT ;`
            let commands = if peek_keyword(input, "commands") {
                let kw: Ident = input.parse()?;
                if input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        kw.span(),
                        "expected `:` after `commands`, found `=`",
                    ));
                }
                input.parse::<Token![:]>()?;
                let ident: Ident = input.parse()?;
                input.parse::<Token![;]>()?;
                ident
            } else {
                Ident::new("cmds", Span::call_site())
            };

            // Top-level forms:
            //   * braced-node list — `#name { .. }` / `{ .. }`, comma-separated
            //     (one or more roots, supports multiple sibling roots);
            //   * a single implicit-body root — the macro's own outer braces ARE
            //     this root's braces, so the remaining stream is the body directly
            //     (the common `ui! { UiLayout { .. }, .. }` form).
            let mut roots = Vec::new();
            if input.peek(Token![#]) || input.peek(syn::token::Brace) {
                roots.push(parse_node(input)?);
                while input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                    if input.is_empty() {
                        break;
                    }
                    roots.push(parse_node(input)?);
                }
                if !input.is_empty() {
                    return Err(input.error("unexpected trailing tokens after the ui! tree"));
                }
            } else {
                // Implicit single-root body — `parse_body` consumes the whole
                // remaining stream (it loops until `input.is_empty()`).
                let brace_span = input.span();
                roots.push(parse_body(input, None, brace_span)?);
            }

            Ok(UiInvocation { commands, roots })
        }
    }

    /// Parses one `name? '{' body '}'` node.
    fn parse_node(input: ParseStream) -> syn::Result<UiNode> {
        // Optional `#name` prefix.
        let name = if input.peek(Token![#]) {
            input.parse::<Token![#]>()?;
            let ident: Ident = input.parse()?;
            Some(ident)
        } else {
            None
        };

        if !input.peek(syn::token::Brace) {
            return Err(input.error("expected `{` to open a ui node body"));
        }

        let body;
        let brace = braced!(body in input);
        let brace_span = brace.span.join();

        parse_body(&body, name, brace_span)
    }

    /// Parses a node body: `items? children?`.
    fn parse_body(body: ParseStream, name: Option<Ident>, brace_span: Span) -> syn::Result<UiNode> {
        let mut components: Vec<Expr> = Vec::new();
        let mut children: Vec<UiNode> = Vec::new();
        let mut children_seen = false;

        while !body.is_empty() {
            // `children:` keyed clause — must be last.
            if peek_keyword(body, "children") {
                if children_seen {
                    let kw: Ident = body.parse()?;
                    return Err(syn::Error::new(
                        kw.span(),
                        "`children:` appears twice in one node body",
                    ));
                }
                let kw: Ident = body.parse()?;
                if body.peek(Token![=]) {
                    return Err(syn::Error::new(
                        kw.span(),
                        "expected `:` after `children`, found `=`",
                    ));
                }
                body.parse::<Token![:]>()?;
                children = parse_children(body)?;
                children_seen = true;
                body.parse::<Option<Token![,]>>()?;
                if !body.is_empty() {
                    return Err(body.error("`children:` must be the last clause in a node body"));
                }
                break;
            }

            // Targeted diagnostics for the high-frequency authoring mistakes —
            // produced before the generic `syn::Expr` parse so the message is
            // specific instead of an opaque expression-parse failure.
            if body.peek(Token![#]) {
                return Err(body.error(
                    "a node reference must appear inside a component field; \
                     a bare `#name` is not a component",
                ));
            }
            if body.peek(syn::token::Brace) {
                return Err(body.error(
                    "a child node must appear inside a `children: [ ... ]` clause",
                ));
            }
            if body.peek(syn::token::Bracket) {
                return Err(body.error(
                    "expected a `children: [ ... ]` clause; a bare `[ ... ]` is not a component",
                ));
            }

            let expr: Expr = body.parse()?;
            components.push(expr);

            if body.peek(Token![,]) {
                body.parse::<Token![,]>()?;
            } else if !body.is_empty() {
                return Err(body.error("expected `,` between ui node components"));
            }
        }

        if components.is_empty() {
            let msg = if children_seen {
                "a ui node needs at least one component (a `UiLayout`) besides `children:`"
            } else {
                "a ui node needs at least one component (a `UiLayout`)"
            };
            return Err(syn::Error::new(brace_span, msg));
        }

        Ok(UiNode { name, components, children, brace_span })
    }

    /// Parses a `'[' node ( ',' node )* ','? ']'` child list.
    fn parse_children(input: ParseStream) -> syn::Result<Vec<UiNode>> {
        if !input.peek(syn::token::Bracket) {
            return Err(input.error("expected `[` to open the children list"));
        }
        let inner;
        bracketed!(inner in input);

        let mut nodes = Vec::new();
        while !inner.is_empty() {
            nodes.push(parse_node(&inner)?);
            if inner.peek(Token![,]) {
                inner.parse::<Token![,]>()?;
            } else if !inner.is_empty() {
                return Err(inner.error("expected `,` between child nodes"));
            }
        }
        Ok(nodes)
    }

    /// Peeks whether the next token is the bare identifier `kw` (a reserved
    /// context keyword). Uses a fork so no input is consumed on a miss.
    fn peek_keyword(input: ParseStream, kw: &str) -> bool {
        input.peek(Ident) && input.fork().parse::<Ident>().map(|i| i == kw).unwrap_or(false)
    }

    // ── Validation (macro-time, span-precise, accumulated via `.combine()`) ──────

    /// Walks the parsed tree collecting macro-time errors (duplicate names,
    /// over-length names, name/commands collision, no-`UiLayout` nodes). Returns
    /// the first combined error if any.
    fn validate(inv: &UiInvocation) -> syn::Result<()> {
        let mut errors: Option<syn::Error> = None;
        let push = |e: syn::Error, acc: &mut Option<syn::Error>| match acc {
            Some(existing) => existing.combine(e),
            None => *acc = Some(e),
        };

        // First pass: collect all declared names (for the duplicate check). The
        // value of the map is the first declaration span.
        let mut declared: HashMap<String, Span> = HashMap::new();
        collect_names(&inv.roots, &inv.commands, &mut declared, &mut |e| push(e, &mut errors));

        // Second pass: per-node structural checks (every node has a `UiLayout`).
        for node in &inv.roots {
            validate_node(node, &mut |e| push(e, &mut errors));
        }

        match errors {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Collects `#name` declarations, flagging duplicates, over-length names, and
    /// collisions with the `commands` binding.
    fn collect_names(
        nodes: &[UiNode],
        commands: &Ident,
        declared: &mut HashMap<String, Span>,
        push: &mut dyn FnMut(syn::Error),
    ) {
        for node in nodes {
            if let Some(name) = &node.name {
                let s = name.to_string();
                if name == commands {
                    push(syn::Error::new(
                        name.span(),
                        format!(
                            "ui name `{s}` collides with the commands binding; choose another name"
                        ),
                    ));
                }
                if s.len() > UI_NAME_CAP {
                    push(syn::Error::new(
                        name.span(),
                        format!("ui name `{s}` exceeds {UI_NAME_CAP} bytes"),
                    ));
                }
                if let Some(_prev) = declared.insert(s.clone(), name.span()) {
                    push(syn::Error::new(
                        name.span(),
                        format!(
                            "duplicate ui name `{s}`; names must be unique within a `ui!` invocation"
                        ),
                    ));
                }
            }
            collect_names(&node.children, commands, declared, push);
        }
    }

    /// Per-node validation: rejects a node with no `UiLayout` literal, then
    /// recurses into children.
    fn validate_node(node: &UiNode, push: &mut dyn FnMut(syn::Error)) {
        if !node.components.iter().any(is_ui_layout_literal) {
            push(syn::Error::new(
                node.brace_span,
                "a ui node requires a `UiLayout` component (the last path segment \
                 must be `UiLayout`, e.g. `UiLayout { .. }` or a qualified spelling \
                 ending in `::UiLayout`)",
            ));
        }
        for child in &node.children {
            validate_node(child, push);
        }
    }

    // ── Head-path recognition (syntactic, pre-type-resolution) ──────────────────

    /// Whether `expr`'s component-type path ends in the segment `UiLayout`.
    fn is_ui_layout_literal(expr: &Expr) -> bool {
        head_ident_is(expr, "UiLayout")
    }

    /// Whether `expr`'s component-type path ends in the segment `ComputedRect`.
    fn is_computed_rect_literal(expr: &Expr) -> bool {
        head_ident_is(expr, "ComputedRect")
    }

    /// Whether the type path of a component literal ends in the segment `name`.
    /// Recognises the struct-literal (`UiLayout { .. }`), bare-path (`UiRoot`)
    /// and associated-call (`UiLayout::default()`) forms.
    ///
    /// Matching the LAST path segment (not the first) means a re-pathed /
    /// qualified spelling — `crate::components::UiLayout`, `::boyko_ui::…`,
    /// `components::UiLayout { .. }` — is recognised identically to the bare
    /// `UiLayout`, so validation never false-rejects a qualified node and the
    /// `UiNodeBundle` fast path applies to both spellings. Only an alias that
    /// renames the type away from `UiLayout` (`use … as Foo`) is missed, which
    /// then correctly falls to the generic insert path.
    fn head_ident_is(expr: &Expr, name: &str) -> bool {
        let path = match expr {
            Expr::Struct(s) => &s.path,
            Expr::Call(c) => match &*c.func {
                Expr::Path(p) => &p.path,
                _ => return false,
            },
            Expr::Path(p) => &p.path,
            _ => return false,
        };
        // The last segment carries the type name; a leading `::` or extra
        // segments only re-path it. For the associated-call form
        // (`UiLayout::default()`) the type is the *next-to-last* segment, so
        // also accept a penultimate match when the final segment is a bare
        // function-call tail (no generic args on the matched segment).
        let segs = &path.segments;
        if let Some(last) = segs.last()
            && last.ident == name
            && last.arguments.is_empty()
        {
            return true;
        }
        // `Path::Type::method()` → match the segment before a trailing call tail.
        if segs.len() >= 2 {
            let penultimate = &segs[segs.len() - 2];
            if penultimate.ident == name && penultimate.arguments.is_empty() {
                return true;
            }
        }
        false
    }

    // ── Codegen (single-pass pre-order DFS, spawn-and-capture) ──────────────────
    //
    // O1 (verified): `entity(reserved_id).insert(base)` does NOT materialise a
    // reserved-but-unspawned id — `InsertCommand::apply` no-ops on a null inland
    // (commands core is locked; there is no public `spawn_at(reserved_id, bundle)`
    // API). So Phase B uses `cmds.spawn(<base>)` and captures the entity from the
    // returned handle's `.id()`. Each `#name` binding is captured at its own spawn
    // site and exposed as a post-invocation `let`, so cross-node wiring is done by
    // the caller after the macro expands (P2 has no value-position `#name` refs:
    // a bare `#ident` is not parseable inside a component `syn::Expr`).
    //
    // Pre-order materialisation keeps the load-bearing ordering invariant: a
    // parent's spawn is enqueued before its descendants' `add_child` (`ChildOf`
    // insert), so the dangling-parent guard passes at the FIFO apply drain.

    /// Top-level expansion entry point.
    pub fn expand(input: TokenStream2) -> syn::Result<TokenStream2> {
        let inv: UiInvocation = syn::parse2(input)?;
        validate(&inv)?;

        let cmds = &inv.commands;
        let mut counter: usize = 0;
        let mut stmts: Vec<TokenStream2> = Vec::new();
        let mut root_bindings: Vec<Ident> = Vec::with_capacity(inv.roots.len());

        for node in &inv.roots {
            let binding = lower_node(node, cmds, &mut counter, &mut stmts);
            root_bindings.push(binding);
        }

        let result = if root_bindings.len() == 1 {
            let only = &root_bindings[0];
            quote! { #only }
        } else {
            quote! { ( #(#root_bindings),* ) }
        };

        Ok(quote! {
            {
                #(#stmts)*
                #result
            }
        })
    }

    /// Lowers one node and its subtree (pre-order). Pushes statements into
    /// `stmts` and returns the `Entity` binding ident for this node.
    fn lower_node(
        node: &UiNode,
        cmds: &Ident,
        counter: &mut usize,
        stmts: &mut Vec<TokenStream2>,
    ) -> Ident {
        // The binding: the user's ident for `#named`, else a hidden `__ui_n{}`.
        let (binding, is_named) = match &node.name {
            Some(name) => (name.clone(), true),
            None => {
                let id = format_ident!("__ui_n{}", *counter, span = node.brace_span);
                *counter += 1;
                (id, false)
            }
        };

        // Split the component set: pull the UiLayout + ComputedRect literals for
        // the canonical bundle (set-based, position-independent); the rest become
        // chained inserts in author declaration order.
        let layout_idx = node.components.iter().position(is_ui_layout_literal);
        let rect_idx = node.components.iter().position(is_computed_rect_literal);

        let mut inserts: Vec<TokenStream2> = Vec::new();
        let spawn_base: TokenStream2;

        match (layout_idx, rect_idx) {
            (Some(li), Some(ri)) => {
                // Canonical bundle fast path: spawn `UiNodeBundle { layout, rect }`.
                let layout_lit = &node.components[li];
                let rect_lit = &node.components[ri];
                let bundle = path_ui_node_bundle();
                let layout_span = layout_lit.span();
                let rect_span = rect_lit.span();
                let layout_field = quote_spanned! { layout_span => layout: #layout_lit };
                let rect_field = quote_spanned! { rect_span => rect: #rect_lit };
                spawn_base = quote! { #bundle { #layout_field, #rect_field } };
                // Remaining components → inserts, skipping the two pulled out.
                for (i, c) in node.components.iter().enumerate() {
                    if i == li || i == ri {
                        continue;
                    }
                    inserts.push(quote! { #c });
                }
            }
            (Some(li), None) => {
                // Spawn the UiLayout literal; inject ComputedRect::default().
                let layout_lit = &node.components[li];
                spawn_base = quote! { #layout_lit };
                let rect = path_computed_rect();
                inserts.push(quote! { #rect::default() });
                for (i, c) in node.components.iter().enumerate() {
                    if i == li {
                        continue;
                    }
                    inserts.push(quote! { #c });
                }
            }
            // Validation already rejected a node without a UiLayout literal; this
            // arm is unreachable, but emit a sound spawn so a validation regression
            // produces a type error rather than a panic.
            (None, _) => {
                spawn_base = quote! { ::core::compile_error!("internal: ui node without UiLayout") };
            }
        }

        // `#named` → also insert UiName::new("name").
        if is_named {
            let name_str = LitStr::new(&binding.to_string(), binding.span());
            let ui_name = path_ui_name();
            inserts.push(quote! { #ui_name::new(#name_str) });
        }

        // Spawn-and-capture: bind the entity id, then chain inserts as standalone
        // statements (no EntityCommands held across a sibling `#name` read).
        let spawn_span = node.brace_span;
        stmts.push(quote_spanned! { spawn_span =>
            let #binding = #cmds.spawn(#spawn_base).id();
        });
        for ins in &inserts {
            stmts.push(quote! { #cmds.entity(#binding).insert(#ins); });
        }

        // Recurse into children, then link each after the parent is materialised
        // (pre-order ⇒ parent spawn precedes the child's ChildOf insert).
        for child in &node.children {
            let child_binding = lower_node(child, cmds, counter, stmts);
            stmts.push(quote! { #cmds.entity(#binding).add_child(#child_binding); });
        }

        // Suppress `unused_variables` for an unreferenced `#named` handle under
        // `-D warnings` (the binding is the user's ident; they may never read it).
        if is_named {
            stmts.push(quote! { let _ = &#binding; });
        }

        binding
    }
}
