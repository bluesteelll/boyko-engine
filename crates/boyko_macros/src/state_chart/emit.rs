//! Codegen: the flat enum, the composite predicates, the initial-enter chain, ONE merged system
//! per leaf, and the two registration fns a plugin calls.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::Ident;

use super::ast::Param;
use super::model::{
    Chart, MergedParam, initial_enter_fn_ident, install_fn_ident, leaf_fn_ident, snake,
    systems_fn_ident,
};

/// `::boyko_ecs::…` paths are TOKENS resolved in the downstream crate, never a dependency of this
/// proc-macro crate (see this crate's Cargo.toml note).
fn sys() -> TokenStream {
    quote!(::boyko_ecs::ecs::core::system)
}
fn state_mod() -> TokenStream {
    quote!(::boyko_ecs::ecs::core::state)
}

fn arity_allow() -> TokenStream {
    quote!(#[allow(clippy::too_many_arguments)])
}

fn param_tokens(p: &MergedParam<'_>) -> TokenStream {
    let name = p.name;
    let ty = p.ty;
    let mut_kw = p.mutable.then(|| quote!(mut));
    quote!(#mut_kw #name: #ty)
}

/// The whole expansion for one chart.
pub fn chart_items(chart: &Chart<'_>) -> syn::Result<TokenStream> {
    let cname = &chart.input.name;
    let variants: Vec<Ident> = chart.leaves.iter().map(|&l| chart.variant(l)).collect();

    // Composite predicates: `in_playing(self)` = compile-time group membership.
    let mut predicates: Vec<TokenStream> = Vec::new();
    for (i, n) in chart.nodes.iter().enumerate() {
        if n.state.children.is_empty() {
            continue;
        }
        let members: Vec<Ident> = chart
            .leaves
            .iter()
            .filter(|&&l| chart.lineage(l).contains(&i))
            .map(|&l| chart.variant(l))
            .collect();
        let pred = quote::format_ident!("in_{}", snake(&n.cat));
        predicates.push(quote! {
            /// Zero-cost superstate predicate (compile-time group membership).
            #[inline]
            pub const fn #pred(self) -> bool {
                matches!(self, #( Self::#members )|*)
            }
        });
    }
    let predicate_impl =
        (!predicates.is_empty()).then(|| quote! { impl #cname { #( #predicates )* } });

    let initial_enter = initial_enter_fn(chart)?;

    let mut fns: Vec<TokenStream> = Vec::new();
    for (li, &leaf) in chart.leaves.iter().enumerate() {
        if let Some(f) = leaf_fn(chart, li, leaf)? {
            fns.push(f);
        }
    }

    let install = install_fn(chart);
    let systems = systems_fn(chart);
    let states_trait = state_mod();

    // Spanned at the chart's own name, so a downstream error against the generated type points at
    // the user's declaration. The generated FNS keep their own spans (each is built from its
    // leaf's tokens).
    Ok(quote_spanned! {cname.span()=>
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum #cname {
            #( #variants ),*
        }
        impl #states_trait::States for #cname {}
        #predicate_impl
        #initial_enter
        #( #fns )*
        #install
        #systems
    })
}

/// The chart's **initial enter chain**. `insert_state` seeds the VALUE, but nothing in the kernel
/// runs an entry action for a state nobody transitioned into — so the `enter` bodies along the
/// initial leaf's ancestor path run as ONE startup system, outermost-first (the order the LCA
/// rule uses on a transition's enter side). Emitted only when that chain has a body: an empty
/// startup system would be pure expansion volume.
fn initial_enter_fn(chart: &Chart<'_>) -> syn::Result<Option<TokenStream>> {
    if !chart.has_initial_enter() {
        return Ok(None);
    }
    let chain = chart.lineage(chart.initial_leaf);
    let all: Vec<&Param> = chain
        .iter()
        .filter_map(|&i| chart.nodes[i].state.enter.as_ref())
        .flat_map(|h| h.params.iter())
        .collect();
    let merged = super::model::merge_params(&all, "the initial state's merged `enter` chain")?;
    let params = merged.iter().map(param_tokens);
    let bodies = chain.iter().filter_map(|&i| {
        chart.nodes[i].state.enter.as_ref().map(|h| {
            let b = &h.body;
            quote! { { #b } }
        })
    });
    let fn_name = initial_enter_fn_ident(&chart.input.name);
    let allow = arity_allow();
    Ok(Some(quote! {
        #allow
        fn #fn_name( #( #params ),* ) {
            #( #bodies )*
        }
    }))
}

/// **The route merge.** One system for the whole leaf, in three phases:
///
/// 1. **Drain**, one lane per route, in declaration order. Every lane is drained to completion
///    whether or not an earlier route already won — the kernel's `EventIter` advances the cursor
///    only past what it YIELDED, so breaking out mid-drain leaves the rest of this frame's events
///    unread and the next frame re-fires on them. The remainder is observed and discarded, which
///    is also exactly what the per-route systems did to their own lanes before the merge.
/// 2. **Arbitrate**: the first route in declaration order that accepted wins. Between two systems
///    arbitration could only ever be registration order; inside one body it is the order the
///    author wrote.
/// 3. **One chain**: the winner's exit → action → enter, then one `NextState` write. There is no
///    second chain to run, which is the structural half of the fix — the pre-merge shape ran a
///    full chain per accepting route and let the last `NextState` write decide the state.
///
/// A guard is evaluated behind `!__sc_hit` so a guard with side effects is not re-run against the
/// events being discarded — and a LOSING route's guard is still evaluated, because its lane must
/// be drained and the pre-merge shape evaluated it too.
fn leaf_fn(chart: &Chart<'_>, li: usize, leaf: usize) -> syn::Result<Option<TokenStream>> {
    let routes = &chart.routes[li];
    if routes.is_empty() {
        return Ok(None);
    }
    let cname = &chart.input.name;
    let sys = sys();
    let st = state_mod();

    // Every route's params, plus the params of every exit/enter body any route can run, fuse into
    // ONE signature. This is wider than the pre-merge per-route signature by construction, so a
    // name reused at two types on one leaf is refused here where it previously was not.
    let mut all: Vec<&Param> = Vec::new();
    let mut chains: Vec<(Vec<usize>, Vec<usize>)> = Vec::with_capacity(routes.len());
    for r in routes {
        let (exit_nodes, enter_nodes) = lca_chains(chart, leaf, r.target);
        all.extend(r.def.params.iter());
        for &i in &exit_nodes {
            if let Some(h) = &chart.nodes[i].state.exit {
                all.extend(h.params.iter());
            }
        }
        for &i in &enter_nodes {
            if let Some(h) = &chart.nodes[i].state.enter {
                all.extend(h.params.iter());
            }
        }
        chains.push((exit_nodes, enter_nodes));
    }
    let merged = super::model::merge_params(&all, "this leaf's merged routes")?;
    let params = merged.iter().map(param_tokens);

    // Phase 1 + 2: one drain per lane, then a single selection.
    let mut drains: Vec<TokenStream> = Vec::with_capacity(routes.len());
    let mut readers: Vec<TokenStream> = Vec::with_capacity(routes.len());
    for (i, r) in routes.iter().enumerate() {
        let reader = quote::format_ident!("__sc_ev_{}", i);
        let event = &r.def.event;
        readers.push(quote!(mut #reader: #sys::EventReader<#event>));
        let accept = match r.def.guard.as_ref() {
            Some(g) => quote! { if !__sc_hit && (#g) { __sc_hit = true; } },
            None => quote! { __sc_hit = true; },
        };
        let pick = u32::try_from(i + 1).expect("a chart with >4 billion routes on one leaf");
        drains.push(quote! {
            {
                let mut __sc_hit = false;
                for _ in #reader.read() { #accept }
                if __sc_hit && __sc_route == 0 { __sc_route = #pick; }
            }
        });
    }

    // Phase 3: the winner's ONE chain.
    let arms = routes.iter().zip(&chains).enumerate().map(|(i, (r, (exit_nodes, enter_nodes)))| {
        let pick = u32::try_from(i + 1).expect("a chart with >4 billion routes on one leaf");
        let exits = exit_nodes.iter().filter_map(|&n| {
            chart.nodes[n].state.exit.as_ref().map(|h| {
                let b = &h.body;
                quote! { { #b } }
            })
        });
        let action = r.def.action.as_ref().map(|a| quote! { { #a } });
        let enters = enter_nodes.iter().filter_map(|&n| {
            chart.nodes[n].state.enter.as_ref().map(|h| {
                let b = &h.body;
                quote! { { #b } }
            })
        });
        let target_variant = chart.variant(r.target);
        quote! {
            #pick => {
                #( #exits )*
                #action
                #( #enters )*
                *__sc_next = #st::NextState::Pending(#cname::#target_variant);
            }
        }
    });

    let fn_name = leaf_fn_ident(cname, &chart.nodes[leaf].cat);
    let allow = arity_allow();
    Ok(Some(quote! {
        #allow
        fn #fn_name(
            #( #readers, )*
            mut __sc_next: #sys::ResMut<#st::NextState<#cname>>,
            #( #params ),*
        ) {
            // 0 = no route accepted; otherwise the 1-based index of the FIRST-DECLARED route that
            // did. One selection, one chain.
            let mut __sc_route: u32 = 0;
            #( #drains )*
            match __sc_route {
                #( #arms )*
                _ => {}
            }
        }
    }))
}

/// Source-side states BELOW the LCA innermost-first (exits), and target-side states below the LCA
/// outermost-first (enters). Two leaves under one composite exclude that composite from both
/// chains, which is what keeps a `Running ↔ Paused` toggle from re-entering `Playing`.
fn lca_chains(chart: &Chart<'_>, leaf: usize, target: usize) -> (Vec<usize>, Vec<usize>) {
    let src = chart.lineage(leaf);
    let dst = chart.lineage(target);
    let mut depth = 0;
    while depth < src.len() && depth < dst.len() && src[depth] == dst[depth] {
        depth += 1;
    }
    (src[depth..].iter().rev().copied().collect(), dst[depth..].to_vec())
}

/// `insert_state(initial leaf)` plus the initial-enter startup system, as ONE call a plugin makes.
/// Keeping the flattening's knowledge behind a generated fn is what lets a front-end (or a
/// hand-written plugin) register a chart without re-deriving which leaf is initial.
fn install_fn(chart: &Chart<'_>) -> TokenStream {
    let cname = &chart.input.name;
    let init_variant = chart.variant(chart.initial_leaf);
    let startup = chart.has_initial_enter().then(|| {
        let f = initial_enter_fn_ident(cname);
        quote! { app.add_startup_system(#f); }
    });
    let name = install_fn_ident(cname);
    let doc = format!(" Seeds `{cname}`'s initial leaf and its entry chain. Generated.");
    quote! {
        #[doc = #doc]
        #[doc(hidden)]
        pub fn #name(app: &mut ::boyko_ecs::App) {
            app.insert_state(#cname::#init_variant);
            #startup
        }
    }
}

/// One `run_if(in_state(leaf))` registration per LEAF — not per route, which is the registration
/// half of the merge.
fn systems_fn(chart: &Chart<'_>) -> TokenStream {
    let cname = &chart.input.name;
    let cond = quote!(::boyko_ecs::ecs::core::schedule::common_conditions::in_state);
    let stmts = chart.leaves.iter().enumerate().filter_map(|(li, &leaf)| {
        if chart.routes[li].is_empty() {
            return None;
        }
        let f = leaf_fn_ident(cname, &chart.nodes[leaf].cat);
        let v = chart.variant(leaf);
        Some(quote! { b.add_system(#f).run_if(#cond(#cname::#v)); })
    });
    let name = systems_fn_ident(cname);
    let doc = format!(" Registers `{cname}`'s per-leaf transition systems. Generated.");
    quote! {
        #[doc = #doc]
        #[doc(hidden)]
        pub fn #name(b: &mut ::boyko_ecs::ecs::core::schedule::schedule_builder::ScheduleBuilder) {
            #( #stmts )*
        }
    }
}

/// The name-resolving stubs a failed chart still emits, so ONE bad chart costs one error instead
/// of that error plus "cannot find function" at every registration site. The same recovery
/// discipline Aether applies to a construct that did not parse.
pub fn failure_stubs(name: Option<&Ident>) -> TokenStream {
    let Some(cname) = name else {
        return TokenStream::new();
    };
    let install = install_fn_ident(cname);
    let systems = systems_fn_ident(cname);
    quote! {
        #[doc(hidden)]
        #[allow(dead_code, non_snake_case)]
        pub fn #install(_app: &mut ::boyko_ecs::App) {}
        #[doc(hidden)]
        #[allow(dead_code, non_snake_case)]
        pub fn #systems(
            _b: &mut ::boyko_ecs::ecs::core::schedule::schedule_builder::ScheduleBuilder,
        ) {}
    }
}
