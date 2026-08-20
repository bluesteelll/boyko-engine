//! The expander — Decision A3: emit the CANONICAL hand-written surface and let `boyko_macros`
//! do the codegen. One expansion authority, zero drift, minimal expansion volume (§8 R1); every
//! engine path below is a TOKEN resolved downstream, never a dependency of this crate.

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::Ident;

use crate::ast::{
    AetherBlock, BundleDef, ComponentDef, Construct, EvField, EventDef, MachineDef, OrderKind,
    PluginDef, Schedule, StateDef, SysParam, SysParamTy, SystemDef, TagDef, TransitionDef,
};
use crate::diag;

/// Expand a parsed block to the flat item list, in source order (deterministic output is what
/// the unit tests pin token-for-token). Block-level validation failures (§3.3's cross-construct
/// rules) become `compile_error!` exactly like parse failures.
pub fn expand(block: &AetherBlock) -> TokenStream {
    match expand_inner(block) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error(),
    }
}

fn expand_inner(block: &AetherBlock) -> syn::Result<TokenStream> {
    validate_block(block)?;
    let mut out = TokenStream::new();
    for c in &block.constructs {
        match c {
            Construct::Component(def) => out.extend(component(def)),
            Construct::Tag(def) => out.extend(tag(def)),
            Construct::Bundle(def) => out.extend(bundle(def)),
            Construct::Event(def) => out.extend(event(def)),
            Construct::System(def) => out.extend(system_fn(def)),
            Construct::Plugin(def) => out.extend(plugin_impl(def, block)?),
            Construct::Machine(def) => out.extend(machine_items(def)?),
        }
    }
    Ok(out)
}

/// The §3.3 cross-construct rules that need the WHOLE block (the reason per-construct macros
/// were rejected): one plugin per block, and scheduling clauses require the plugin header.
fn validate_block(block: &AetherBlock) -> syn::Result<()> {
    let mut first_plugin: Option<&PluginDef> = None;
    for c in &block.constructs {
        if let Construct::Plugin(p) = c {
            if let Some(first) = first_plugin {
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
            first_plugin = Some(p);
        }
    }
    if first_plugin.is_none() {
        for c in &block.constructs {
            if let Construct::System(s) = c
                && s.has_clauses()
            {
                return Err(diag::err(
                    s.name.span(),
                    "scheduling clauses (`on`, `after`, `when`, …) need a `plugin <Name>;` declaration in this block to hold the generated registration",
                ));
            }
            if let Construct::Machine(m) = c {
                return Err(diag::err(
                    m.name.span(),
                    "a `machine` needs a `plugin <Name>;` declaration in this block to hold its `insert_state` and transition registrations",
                ));
            }
        }
    }
    Ok(())
}

/// §3.1: `component` → `#[derive(::boyko_macros::Component)]` struct with the derive's own
/// attribute surface (`#[require(...)]`, `#[component(on_* = path, no_bundle)]`), fields `pub`.
fn component(def: &ComponentDef) -> TokenStream {
    let name = &def.name;
    let requires = (!def.requires.is_empty()).then(|| {
        let paths = &def.requires;
        quote! { #[require( #( #paths ),* )] }
    });
    let component_attr = {
        let mut keys: Vec<TokenStream> = Vec::new();
        for (kind, path) in &def.hooks {
            let key = Ident::new(kind.key(), proc_macro2::Span::call_site());
            keys.push(quote! { #key = #path });
        }
        if def.no_bundle {
            keys.push(quote! { no_bundle });
        }
        (!keys.is_empty()).then(|| quote! { #[component( #( #keys ),* )] })
    };
    let fields = def.fields.iter().map(|(fname, ty)| quote! { pub #fname: #ty });
    quote! {
        #[derive(::boyko_macros::Component)]
        #requires
        #component_attr
        pub struct #name {
            #( #fields ),*
        }
    }
}

/// §3.1: `tag` → a ZST component (the derive's auto-tag detection does the rest); `(bitset)`
/// adds `#[component(storage = "bitset")]` — the EnableTag backend. The parser already enforced
/// the "bitset ⇒ fieldless" rule by grammar (tags cannot carry fields at all), and the derive's
/// own check remains the authority.
fn tag(def: &TagDef) -> TokenStream {
    let name = &def.name;
    let storage = def.bitset.then(|| quote! { #[component(storage = "bitset")] });
    quote! {
        #[derive(::boyko_macros::Component)]
        #storage
        pub struct #name;
    }
}

/// §3.2: `bundle` → `#[derive(::boyko_macros::Bundle)]` — nothing more; the derive owns arity,
/// the named-struct rule, and the static-cache codegen.
fn bundle(def: &BundleDef) -> TokenStream {
    let name = &def.name;
    let fields = def.fields.iter().map(|(fname, ty)| quote! { pub #fname: #ty });
    quote! {
        #[derive(::boyko_macros::Bundle)]
        pub struct #name {
            #( #fields ),*
        }
    }
}

/// §3.4: `event` → `#[::boyko_macros::event]` with the two-band field markers. The Entity path
/// is the REAL nested one — `boyko_ecs` has no root re-export, and a token that resolves is the
/// whole tokens-not-deps contract.
fn event(def: &EventDef) -> TokenStream {
    let name = &def.name;
    let fields = def.fields.iter().map(|f| match f {
        EvField::Participant { name, components } => {
            let ctx = components
                .iter()
                .map(|p| quote!(#p).to_string().replace(' ', ""))
                .collect::<Vec<_>>()
                .join(", ");
            quote! {
                #[participant(components = #ctx)]
                pub #name: ::boyko_ecs::ecs::core::entity::entity::Entity
            }
        }
        EvField::Parameter { name, ty } => quote! {
            #[parameter]
            pub #name: #ty
        },
    });
    quote! {
        #[::boyko_macros::event]
        pub struct #name {
            #( #fields ),*
        }
    }
}

/// §3.3: `system` → a plain `pub fn` with the sugared signature and the UNTOUCHED verbatim
/// body. Every engine path is the REAL nested one (tokens-not-deps; the root re-exports only
/// App/Plugin/…, so the plan's idealized `::boyko_ecs::Res` is emitted as
/// `::boyko_ecs::ecs::core::system::Res` — the A1 Entity precedent).
fn system_fn(def: &SystemDef) -> TokenStream {
    let name = &def.name;
    let params = def.params.iter().map(sys_param_tokens);
    let body = &def.body;
    quote! {
        pub fn #name( #(#params),* ) { #body }
    }
}

/// One param: the §3.3 sugar table plus MUTABILITY INFERENCE — a param whose expansion needs
/// `&mut self` access gets a `mut` binding automatically. Recorded deviation from the plan's
/// inference list: `events<E>` is INCLUDED (this engine's `EventReader::read` takes `&mut
/// self`, so a non-mut reader binding could never be read).
fn sys_param_tokens(p: &SysParam) -> TokenStream {
    let name = &p.name;
    let (inferred_mut, ty) = param_ty_and_mut(&p.ty);
    let mut_kw = (p.explicit_mut || inferred_mut).then(|| quote!(mut));
    quote!(#mut_kw #name: #ty)
}

/// The sugar table's (needs-`mut`, emitted-type) pair — shared by `system` params and the
/// `machine` merged-param path (which needs the type ALONE for its dedup identity).
fn param_ty_and_mut(ty: &SysParamTy) -> (bool, TokenStream) {
    let sys = quote!(::boyko_ecs::ecs::core::system);
    match ty {
        SysParamTy::Query { data, filters } => {
            (type_mentions_mut(data), query_type(data, filters))
        }
        SysParamTy::Res(t) => (false, quote!(#sys::Res<#t>)),
        SysParamTy::ResMut(t) => (true, quote!(#sys::ResMut<#t>)),
        SysParamTy::Local(t) => (false, quote!(#sys::Local<#t>)),
        SysParamTy::Commands => (true, quote!(#sys::Commands)),
        SysParamTy::Events(t) => (true, quote!(#sys::EventReader<#t>)),
        SysParamTy::Emit(t) => (true, quote!(#sys::EventWriter<#t>)),
        SysParamTy::Verbatim(t) => (false, quote!(#t)),
    }
}

/// `query<D, filters>` → `Query<D, F>`: one filter stays bare (the kernel implements
/// `QueryFilter` for bare `With<C>` — verified against d4 tests), two-plus become a tuple,
/// zero omits `F` (the kernel's `()` default).
fn query_type(data: &syn::Type, filters: &[(crate::ast::FilterKind, syn::Path)]) -> TokenStream {
    let q = quote!(::boyko_ecs::ecs::core::iters::query);
    let fs: Vec<TokenStream> = filters
        .iter()
        .map(|(kind, path)| {
            let kn = Ident::new(kind.type_name(), Span::call_site());
            quote!(#q::#kn<#path>)
        })
        .collect();
    match fs.len() {
        0 => quote!(#q::Query<#data>),
        1 => {
            let f = &fs[0];
            quote!(#q::Query<#data, #f>)
        }
        _ => quote!(#q::Query<#data, (#(#fs),*)>),
    }
}

/// Token-level `&mut` / `Mut<` detection for query-data mutability inference. Ident-exact —
/// a type named `Mutation` or a path segment `permutation` never false-positives (a plain
/// substring scan on the printed type would).
fn type_mentions_mut(t: &syn::Type) -> bool {
    stream_mentions_mut(quote!(#t))
}

fn stream_mentions_mut(ts: TokenStream) -> bool {
    ts.into_iter().any(|tt| match tt {
        TokenTree::Ident(i) => i == "mut" || i == "Mut",
        TokenTree::Group(g) => stream_mentions_mut(g.stream()),
        _ => false,
    })
}

/// How a `before`/`after` target resolved against the block's siblings (§3.3).
enum ResolvedOrder<'a> {
    /// A sibling aether system — ordering goes through its captured `SystemKey`.
    Sibling { kind: OrderKind, target: usize },
    /// Anything else — a `SystemSet` type, emitted as `before_set`/`after_set` verbatim.
    Set { kind: OrderKind, path: &'a syn::Path },
}

/// The registration bucket a system lands in (`on` clause; `None` → Main).
fn bucket(s: &SystemDef) -> Schedule {
    s.schedule.unwrap_or(Schedule::Update)
}

/// §3.3: `plugin` → `pub struct NAME; impl Plugin for NAME { build, name }`. Registration is
/// grouped per schedule (startup one-shots, then Main `add_systems_cfg`, then Fixed
/// `add_systems_cfg_in`), and inside a schedule the emission order is TOPOLOGICALLY sorted
/// over sibling `before`/`after` edges so every needed `SystemKey` exists before use.
///
/// One recorded decision: with a `plugin` present, EVERY sibling system is registered —
/// clause-free ones land on Main unordered (the plugin "collects sibling systems"; the plan's
/// "register by hand" story is for plugin-FREE blocks).
///
/// One recorded deviation: for a bare unknown ident within Levenshtein ≤ 2 of a sibling
/// system, the plan wanted a pass-through plus a note attached to rustc's unresolved-name
/// error. Stable proc-macros cannot attach notes to downstream rustc errors, so that close
/// call becomes an AETHER error carrying the note's text; a real `SystemSet` type that close
/// in name is referenced by a qualified path to pass through.
fn plugin_impl(def: &PluginDef, block: &AetherBlock) -> syn::Result<TokenStream> {
    let systems: Vec<&SystemDef> = block
        .constructs
        .iter()
        .filter_map(|c| match c {
            Construct::System(s) => Some(s),
            _ => None,
        })
        .collect();

    // Resolve every ordering clause against the sibling table.
    let mut resolved: Vec<Vec<ResolvedOrder<'_>>> = Vec::with_capacity(systems.len());
    let mut needs_key = vec![false; systems.len()];
    for s in &systems {
        let mut rs = Vec::with_capacity(s.orders.len());
        for (kind, path, _) in &s.orders {
            rs.push(resolve_order(*kind, path, s, &systems, &mut needs_key)?);
        }
        resolved.push(rs);
    }

    // Startup one-shots keep source order (parse already rejected their other clauses).
    let startup_calls = systems
        .iter()
        .filter(|s| bucket(s) == Schedule::Startup)
        .map(|s| {
            let n = &s.name;
            quote!(app.add_startup_system(#n);)
        });

    let mut main_stmts = bucket_stmts(&systems, &resolved, &needs_key, Schedule::Update)?;
    let fixed_stmts = bucket_stmts(&systems, &resolved, &needs_key, Schedule::Fixed)?;

    // Sibling machines (§3.5): the plugin holds their `insert_state` and their transition
    // systems' Main registrations, after the systems' own (deterministic output order).
    let mut inserts: Vec<TokenStream> = Vec::new();
    for c in &block.constructs {
        if let Construct::Machine(m) = c {
            let (insert, stmts) = machine_registrations(m)?;
            inserts.push(insert);
            main_stmts.extend(stmts);
        }
    }

    let main_block = (!main_stmts.is_empty()).then(|| {
        quote! { app.add_systems_cfg(|b| { #(#main_stmts)* }); }
    });
    let fixed_block = (!fixed_stmts.is_empty()).then(|| {
        quote! {
            app.add_systems_cfg_in(::boyko_ecs::ecs::core::app::CoreSchedule::Fixed, |b| { #(#fixed_stmts)* });
        }
    });

    let pname = &def.name;
    let pstr = pname.to_string();
    Ok(quote! {
        pub struct #pname;
        impl ::boyko_ecs::Plugin for #pname {
            fn build(&self, app: &mut ::boyko_ecs::App) {
                #(#inserts)*
                #(#startup_calls)*
                #main_block
                #fixed_block
            }
            fn name(&self) -> &'static str { #pstr }
        }
    })
}

/// Resolve one `before`/`after` target (see [`plugin_impl`]'s deviation note).
fn resolve_order<'a>(
    kind: OrderKind,
    path: &'a syn::Path,
    from: &SystemDef,
    systems: &[&SystemDef],
    needs_key: &mut [bool],
) -> syn::Result<ResolvedOrder<'a>> {
    let bare = (path.leading_colon.is_none()
        && path.segments.len() == 1
        && path.segments[0].arguments.is_none())
    .then(|| path.segments[0].ident.to_string());
    if let Some(name) = bare {
        if let Some(target) = systems.iter().position(|s| s.name == name) {
            if bucket(systems[target]) == Schedule::Startup {
                return Err(diag::err(
                    path.segments[0].ident.span(),
                    format!("ordering references `{name}`, a startup system — startup systems run once, pre-loop, and cannot be ordered against"),
                ));
            }
            if bucket(systems[target]) != bucket(from) {
                return Err(diag::err(
                    path.segments[0].ident.span(),
                    format!("sibling system `{name}` runs on a different schedule — cross-schedule ordering is not expressible"),
                ));
            }
            needs_key[target] = true;
            return Ok(ResolvedOrder::Sibling { kind, target });
        }
        let sibling_names: Vec<String> = systems.iter().map(|s| s.name.to_string()).collect();
        let refs: Vec<&str> = sibling_names.iter().map(String::as_str).collect();
        if let Some(sugg) = diag::did_you_mean(&name, &refs) {
            return Err(diag::err(
                path.segments[0].ident.span(),
                format!("`{name}` is not a sibling aether system; a sibling `{sugg}` exists — system-to-system ordering uses the bare system name (a real SystemSet type this close in name must be referenced by a qualified path)"),
            ));
        }
    }
    Ok(ResolvedOrder::Set { kind, path })
}

/// Registration statements for one schedule bucket, topologically sorted over sibling edges
/// (stable Kahn: lowest source index first, so output is deterministic). A cycle is a compile
/// error naming every member's span — Aether says it earlier and closer to source than the
/// engine's own `ScheduleBuildError::OrderingCycle` at `build()`.
fn bucket_stmts(
    systems: &[&SystemDef],
    resolved: &[Vec<ResolvedOrder<'_>>],
    needs_key: &[bool],
    which: Schedule,
) -> syn::Result<Vec<TokenStream>> {
    let members: Vec<usize> = (0..systems.len()).filter(|&i| bucket(systems[i]) == which).collect();
    if members.is_empty() {
        return Ok(Vec::new());
    }

    // indegree over sibling edges target→member (the target's key must exist first).
    let mut indeg = vec![0usize; systems.len()];
    for &i in &members {
        for r in &resolved[i] {
            if let ResolvedOrder::Sibling { .. } = r {
                indeg[i] += 1;
            }
        }
    }
    let mut emitted = vec![false; systems.len()];
    let mut order: Vec<usize> = Vec::with_capacity(members.len());
    while order.len() < members.len() {
        let Some(&next) = members.iter().find(|&&i| !emitted[i] && indeg[i] == 0) else {
            // Cycle: every un-emitted member with a nonzero indegree participates.
            let cyclic: Vec<usize> =
                members.iter().copied().filter(|&i| !emitted[i]).collect();
            let names =
                cyclic.iter().map(|&i| format!("`{}`", systems[i].name)).collect::<Vec<_>>().join(", ");
            let mut e = diag::err(
                systems[cyclic[0]].name.span(),
                format!("system ordering cycle among {names} — break one `before`/`after` edge"),
            );
            for &i in &cyclic[1..] {
                e.combine(diag::err(systems[i].name.span(), "…cycle member"));
            }
            return Err(e);
        };
        emitted[next] = true;
        order.push(next);
        // Relax: members ordering against `next` lose one indegree.
        for &i in &members {
            if !emitted[i] {
                for r in &resolved[i] {
                    if let ResolvedOrder::Sibling { target, .. } = r
                        && *target == next
                    {
                        indeg[i] -= 1;
                    }
                }
            }
        }
    }

    Ok(order
        .into_iter()
        .map(|i| {
            let s = systems[i];
            let n = &s.name;
            let mut call = quote!(b.add_system(#n));
            for (p, _) in &s.in_sets {
                call = quote!(#call.in_set(#p));
            }
            for r in &resolved[i] {
                call = match r {
                    ResolvedOrder::Sibling { kind, target } => {
                        let k = key_ident(&systems[*target].name);
                        match kind {
                            OrderKind::Before => quote!(#call.before(#k)),
                            OrderKind::After => quote!(#call.after(#k)),
                        }
                    }
                    ResolvedOrder::Set { kind, path } => match kind {
                        OrderKind::Before => quote!(#call.before_set(#path)),
                        OrderKind::After => quote!(#call.after_set(#path)),
                    },
                };
            }
            for (e, _) in &s.whens {
                call = quote!(#call.run_if(#e));
            }
            if needs_key[i] {
                let k = key_ident(&s.name);
                quote!(let #k = #call.key();)
            } else {
                quote!(#call;)
            }
        })
        .collect())
}

/// The captured-`SystemKey` local for a sibling-ordered system (the plan's exact spelling).
fn key_ident(name: &Ident) -> Ident {
    format_ident!("__aether_k_{}", name)
}

// ---------------------------------------------------------------------------------- rung A3

/// One flattened state node. The hierarchy exists ONLY here, inside the transpiler (§3.5):
/// the runtime sees a flat enum and per-(leaf, event) systems.
struct MNode<'a> {
    state: &'a StateDef,
    /// Arena index of the parent; `None` for a top-level state.
    parent: Option<usize>,
    /// Concatenated path names (`PlayingRunning`) — the leaf's enum variant spelling.
    cat: String,
    /// Dotted path names (`Playing.Running`) — for diagnostics.
    dotted: String,
}

/// The flattened machine: an arena of nodes (preorder) + resolved leaves.
struct MachineModel<'a> {
    def: &'a MachineDef,
    nodes: Vec<MNode<'a>>,
    /// Arena indices of the LEAVES, in preorder — the enum variant order.
    leaves: Vec<usize>,
    /// The machine-level `initial`, resolved through composite `initial` chains to a leaf.
    initial_leaf: usize,
    /// Per-leaf inherited transitions, innermost-wins: `(owner_node, transition, target_leaf)`.
    routes: Vec<Vec<(usize, &'a TransitionDef, usize)>>,
}

impl<'a> MachineModel<'a> {
    /// Build + validate: every §3.5 hard fault is diagnosed here, on the user's tokens.
    fn build(def: &'a MachineDef) -> syn::Result<MachineModel<'a>> {
        let mut nodes: Vec<MNode<'a>> = Vec::new();
        fn walk<'a>(
            nodes: &mut Vec<MNode<'a>>,
            state: &'a StateDef,
            parent: Option<usize>,
        ) -> usize {
            let (cat, dotted) = match parent {
                Some(p) => (
                    format!("{}{}", nodes[p].cat, state.name),
                    format!("{}.{}", nodes[p].dotted, state.name),
                ),
                None => (state.name.to_string(), state.name.to_string()),
            };
            let idx = nodes.len();
            nodes.push(MNode { state, parent, cat, dotted });
            for c in &state.children {
                walk(nodes, c, Some(idx));
            }
            idx
        }
        for s in &def.states {
            walk(&mut nodes, s, None);
        }
        if nodes.is_empty() {
            return Err(diag::err(def.name.span(), "a machine declares at least one state"));
        }
        let leaves: Vec<usize> =
            (0..nodes.len()).filter(|&i| nodes[i].state.children.is_empty()).collect();

        // §5.4 flattening is name CONCATENATION, so two distinct chart positions can collapse
        // onto one generated name (`A.BC` and `AB.C` both spell `ABC`; duplicate siblings spell
        // themselves twice). Left to rustc it surfaces as "defined multiple times" on the
        // generated enum/impl with no word about flattening — Aether owns the better message
        // and both spans, so it pre-checks (§7.1).
        for i in 0..nodes.len() {
            let Some(first) = (0..i).find(|&j| nodes[j].cat == nodes[i].cat) else {
                continue;
            };
            let msg = if nodes[first].dotted == nodes[i].dotted {
                format!(
                    "duplicate state `{}` — sibling states need distinct names",
                    nodes[i].dotted
                )
            } else {
                format!(
                    "states `{}` and `{}` both flatten to `{}` — flattening concatenates the state path, so they would emit one name; rename one",
                    nodes[first].dotted, nodes[i].dotted, nodes[i].cat
                )
            };
            let mut e = diag::err(nodes[i].state.name.span(), msg);
            e.combine(diag::err(
                nodes[first].state.name.span(),
                "the first state flattening to this name is here",
            ));
            return Err(e);
        }

        // The `in_<group>` predicates are keyed on the flattened name's snake_case COLLAPSE, and
        // that collapse is lossy: `AB` and `A_b` flatten to two distinct variants yet generate
        // one `in_a_b`. The raw-name check above cannot see it — distinct strings, one emitted
        // method — so the collapsed identity gets its own comparison, on the composites that
        // actually emit a predicate.
        let composites: Vec<usize> =
            (0..nodes.len()).filter(|&i| !nodes[i].state.children.is_empty()).collect();
        for (k, &i) in composites.iter().enumerate() {
            let snake_i = snake(&nodes[i].cat);
            let Some(&first) = composites[..k].iter().find(|&&j| snake(&nodes[j].cat) == snake_i)
            else {
                continue;
            };
            let mut e = diag::err(
                nodes[i].state.name.span(),
                format!(
                    "composite states `{}` and `{}` flatten to `{}` and `{}`, which both collapse to the predicate `in_{snake_i}` — rename one",
                    nodes[first].dotted, nodes[i].dotted, nodes[first].cat, nodes[i].cat
                ),
            );
            e.combine(diag::err(
                nodes[first].state.name.span(),
                "the first composite generating this predicate is here",
            ));
            return Err(e);
        }

        // Duplicate handler for one event in one state — error on the SECOND `on`, with a
        // note at the first (the §3.5 diagnostic).
        for n in &nodes {
            for (i, t) in n.state.transitions.iter().enumerate() {
                let key = path_key(&t.event);
                if let Some(first) =
                    n.state.transitions[..i].iter().find(|p| path_key(&p.event) == key)
                {
                    let mut e = diag::err(
                        t.kw_span,
                        format!("duplicate handler for `{key}` in state `{}`", n.dotted),
                    );
                    e.combine(diag::err(first.kw_span, "the first handler is here"));
                    return Err(e);
                }
            }
        }

        // Every declared `initial` is validated HERE, not only the ones a transition happens to
        // reach: `resolve_to_leaf` runs lazily on targets, so an unreferenced composite's typo
        // (or an `initial` on a childless state, which retargeting can never use) would expand
        // silently. Reachability must not decide whether a name is checked.
        for i in 0..nodes.len() {
            let Some(init) = &nodes[i].state.initial else {
                continue;
            };
            if nodes[i].state.children.is_empty() {
                return Err(diag::err(
                    init.span(),
                    format!(
                        "`{0}` has no nested states, so `initial` has nothing to name — drop it, or nest `state {init} {{ … }}` inside `{0}`",
                        nodes[i].dotted
                    ),
                ));
            }
            let scope = nodes[i].dotted.clone();
            resolve_child(&nodes, Some(i), init, &scope)?;
        }

        // Every transition's TARGET is resolved here for the same reason. The per-leaf walk
        // below resolves only the handlers it actually inherits, and a handler an inner state
        // SHADOWS for the same event is never walked — so `state P { on E => Nowhere; state A {
        // on E => Top; } }` would expand silently, with `Nowhere` never looked up. Whether a
        // name is checked must not depend on whether some leaf happens to reach it.
        for n in &nodes {
            for t in &n.state.transitions {
                resolve_target(&nodes, &def.name.to_string(), &t.target)?;
            }
        }

        let model_initial = resolve_child(&nodes, None, &def.initial, &def.name.to_string())?;
        let initial_leaf = resolve_to_leaf(&nodes, model_initial, def.initial.span())?;

        // Per-leaf inherited transitions: walk the leaf's ancestor chain innermost-first;
        // the innermost handler for an event wins (§3.5 "superstate handlers were copied
        // into each leaf that lacks its own handler").
        let mut routes: Vec<Vec<(usize, &'a TransitionDef, usize)>> =
            Vec::with_capacity(leaves.len());
        for &leaf in &leaves {
            let mut seen: Vec<String> = Vec::new();
            let mut list: Vec<(usize, &'a TransitionDef, usize)> = Vec::new();
            let mut cur = Some(leaf);
            while let Some(i) = cur {
                for t in &nodes[i].state.transitions {
                    let key = path_key(&t.event);
                    if !seen.contains(&key) {
                        seen.push(key);
                        let target = resolve_target(&nodes, &def.name.to_string(), &t.target)?;
                        list.push((i, t, target));
                    }
                }
                cur = nodes[i].parent;
            }
            // The walk above is innermost-FIRST because that is what resolves inheritance
            // (§3.5's innermost-wins), but it is not the order the routes must REGISTER in.
            // §5.1 makes two same-frame transitions deterministic by "ordering the generated
            // systems in declaration order", and the walk puts every inherited handler after
            // the leaf's own no matter where it was written. Re-sort on the parser's source
            // index, which is the only exact record of that order.
            list.sort_by_key(|(_, t, _)| t.decl_index);
            routes.push(list);
        }

        // One generated fn per (leaf, inherited event), named from the snake_case collapse of
        // the flattened leaf path and the event path's LAST segment — and BOTH halves of that
        // name are lossy. `on a::E` + `on b::E` on one leaf collapse the event half; sibling
        // leaves `AB` and `A_b` collapse the state half. Either way rustc reports a duplicate
        // definition on GENERATED tokens; the check belongs where the name is actually minted,
        // across every fn the machine will emit, so both causes are caught by one comparison.
        let mut minted: Vec<(String, usize, &'a TransitionDef)> = Vec::new();
        for (li, &leaf) in leaves.iter().enumerate() {
            for (_, t, _) in &routes[li] {
                let name = transition_fn_ident(&def.name, &nodes[leaf].cat, &t.event).to_string();
                let Some(&(_, first_leaf, first_t)) = minted.iter().find(|(n, _, _)| *n == name)
                else {
                    minted.push((name, leaf, t));
                    continue;
                };
                let msg = if first_leaf == leaf {
                    format!(
                        "events `{}` and `{}` both generate the system `{name}` for leaf `{}` — the generated name keys on the event's last path segment; import one under an alias (`use … as …`)",
                        path_key(&first_t.event),
                        path_key(&t.event),
                        nodes[leaf].dotted
                    )
                } else {
                    format!(
                        "states `{}` and `{}` both generate the system `{name}` — generated names are the snake_case collapse of the flattened state path, and `{}` and `{}` collapse alike; rename one",
                        nodes[first_leaf].dotted,
                        nodes[leaf].dotted,
                        nodes[first_leaf].cat,
                        nodes[leaf].cat
                    )
                };
                let mut e = diag::err(t.kw_span, msg);
                e.combine(diag::err(
                    first_t.kw_span,
                    "the first handler generating this name is here",
                ));
                return Err(e);
            }
        }

        Ok(MachineModel { def, nodes, leaves, initial_leaf, routes })
    }

    /// The leaf's enum variant ident, spanned at the leaf's own name.
    fn variant(&self, leaf: usize) -> Ident {
        Ident::new(&self.nodes[leaf].cat, self.nodes[leaf].state.name.span())
    }

    /// Root-first ancestor chain including `idx` itself.
    fn lineage(&self, idx: usize) -> Vec<usize> {
        let mut v = Vec::new();
        let mut cur = Some(idx);
        while let Some(i) = cur {
            v.push(i);
            cur = self.nodes[i].parent;
        }
        v.reverse();
        v
    }
}

/// A path's dedup identity for handler-inheritance (token spelling, whitespace-free).
fn path_key(p: &syn::Path) -> String {
    quote!(#p).to_string().replace(' ', "")
}

/// Find `name` among the children of `parent` (or the top level), with the §3.5 message:
/// the declared list + did-you-mean.
fn resolve_child(
    nodes: &[MNode<'_>],
    parent: Option<usize>,
    name: &Ident,
    scope: &str,
) -> syn::Result<usize> {
    let candidates: Vec<usize> = (0..nodes.len())
        .filter(|&i| nodes[i].parent == parent)
        .collect();
    if let Some(&found) =
        candidates.iter().find(|&&i| nodes[i].state.name == *name)
    {
        return Ok(found);
    }
    let declared: Vec<String> =
        candidates.iter().map(|&i| nodes[i].state.name.to_string()).collect();
    let refs: Vec<&str> = declared.iter().map(String::as_str).collect();
    let mut msg = format!(
        "no state `{name}` in `{scope}`; states declared here: {}",
        declared.iter().map(|d| format!("`{d}`")).collect::<Vec<_>>().join(", ")
    );
    if let Some(sugg) = diag::did_you_mean(&name.to_string(), &refs) {
        msg.push_str(&format!(" (did you mean `{sugg}`?)"));
    }
    Err(diag::err(name.span(), msg))
}

/// Follow `initial` chains from a (possibly composite) state down to a leaf. A composite
/// with no `initial` is the §3.5 hard fault, suggested with its first child by name.
fn resolve_to_leaf(nodes: &[MNode<'_>], mut idx: usize, err_span: Span) -> syn::Result<usize> {
    loop {
        if nodes[idx].state.children.is_empty() {
            return Ok(idx);
        }
        let Some(init) = &nodes[idx].state.initial else {
            let first_child = (0..nodes.len())
                .find(|&i| nodes[i].parent == Some(idx))
                .map(|i| nodes[i].state.name.to_string())
                .unwrap_or_default();
            return Err(diag::err(
                err_span,
                format!(
                    "target `{}` is a composite state with no `initial` — add `initial <leaf>;` or target a leaf (`{}.{first_child}`)",
                    nodes[idx].dotted, nodes[idx].dotted
                ),
            ));
        };
        idx = resolve_child(nodes, Some(idx), init, &nodes[idx].dotted)?;
    }
}

/// Resolve a ROOT-ANCHORED transition target path (`Playing.Paused`) to a leaf.
fn resolve_target(
    nodes: &[MNode<'_>],
    machine: &str,
    segments: &[Ident],
) -> syn::Result<usize> {
    let mut parent: Option<usize> = None;
    let mut scope = machine.to_string();
    let mut idx = 0usize;
    for seg in segments {
        idx = resolve_child(nodes, parent, seg, &scope)?;
        scope = nodes[idx].dotted.clone();
        parent = Some(idx);
    }
    resolve_to_leaf(nodes, idx, segments.last().expect("grammar: non-empty path").span())
}

/// The snake_case spelling for generated fn names (`PlayingRunning` → `playing_running`).
fn snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Merge the params of several inlined handler bodies into one system signature (§5.3):
/// dedup by NAME, and refuse a name reused at a DIFFERENT type on the later occurrence, with
/// the earlier binding's span attached — §5.3 asks for both spans, because the reader needs
/// the pair to see which two handlers disagree.
fn merge_params<'a>(all: &[&'a SysParam], site: &str) -> syn::Result<Vec<&'a SysParam>> {
    let mut merged: Vec<&'a SysParam> = Vec::with_capacity(all.len());
    for &p in all {
        if let Some(prev) = merged.iter().find(|m| m.name == p.name) {
            let (_, a) = param_ty_and_mut(&prev.ty);
            let (_, b) = param_ty_and_mut(&p.ty);
            if a.to_string() != b.to_string() {
                let mut e = diag::err(
                    p.name.span(),
                    format!(
                        "param `{}` is declared with conflicting types across {site}",
                        p.name
                    ),
                );
                e.combine(diag::err(prev.name.span(), "the first binding of this name is here"));
                return Err(e);
            }
            continue;
        }
        merged.push(p);
    }
    Ok(merged)
}

/// The generated transition system's fn name (the plan's exact shape):
/// `__aether_<machine>__<leaf>__<event>`.
fn transition_fn_ident(machine: &Ident, leaf_cat: &str, event: &syn::Path) -> Ident {
    let ev = event.segments.last().expect("grammar: non-empty path").ident.to_string();
    format_ident!(
        "__aether_{}__{}__{}",
        snake(&machine.to_string()),
        snake(leaf_cat),
        snake(&ev)
    )
}

/// The machine's initial-enter-chain startup system (§5.3).
fn initial_enter_fn_ident(machine: &Ident) -> Ident {
    format_ident!("__aether_{}__initial_enter", snake(&machine.to_string()))
}

/// `true` iff the initial leaf's ancestor path declares at least one `enter` — the gate on
/// emitting (and registering) the startup system at all.
fn has_initial_enter(model: &MachineModel<'_>) -> bool {
    model
        .lineage(model.initial_leaf)
        .iter()
        .any(|&i| model.nodes[i].state.enter.is_some())
}

/// §5.3: the machine's **initial enter chain**. `insert_state` seeds the VALUE, but nothing in
/// the kernel runs an entry action for a state nobody transitioned into — so the `enter` bodies
/// along the initial leaf's ancestor path are emitted as ONE startup system, outermost-first
/// (the same order the LCA rule uses on a transition's enter side). Emitted only when that
/// chain has a body: an empty startup system would be pure expansion volume (§8 R1).
fn initial_enter_fn(model: &MachineModel<'_>) -> syn::Result<Option<TokenStream>> {
    if !has_initial_enter(model) {
        return Ok(None);
    }
    let chain = model.lineage(model.initial_leaf);
    let all: Vec<&SysParam> = chain
        .iter()
        .filter_map(|&i| model.nodes[i].state.enter.as_ref())
        .flat_map(|h| h.params.iter())
        .collect();
    let merged = merge_params(&all, "the initial state's merged `enter` chain")?;
    let params = merged.iter().map(|p| sys_param_tokens(p));
    let bodies = chain.iter().filter_map(|&i| {
        model.nodes[i].state.enter.as_ref().map(|h| {
            let b = &h.body;
            quote! { { #b } }
        })
    });
    let fn_name = initial_enter_fn_ident(&model.def.name);
    Ok(Some(quote! {
        fn #fn_name( #( #params ),* ) {
            #( #bodies )*
        }
    }))
}

/// §3.5 items: the flat enum, the `States` impl, the composite-group predicates, the
/// initial-enter chain, and one transition fn per (leaf, inherited event). Registration lives
/// in the sibling plugin.
fn machine_items(def: &MachineDef) -> syn::Result<TokenStream> {
    let model = MachineModel::build(def)?;
    let mname = &def.name;
    let variants: Vec<Ident> = model.leaves.iter().map(|&l| model.variant(l)).collect();

    // Composite predicates: `in_playing(self)` = membership in the composite's leaf set.
    let mut predicates: Vec<TokenStream> = Vec::new();
    for (i, n) in model.nodes.iter().enumerate() {
        if n.state.children.is_empty() {
            continue;
        }
        let members: Vec<Ident> = model
            .leaves
            .iter()
            .filter(|&&l| model.lineage(l).contains(&i))
            .map(|&l| model.variant(l))
            .collect();
        let pred = format_ident!("in_{}", snake(&n.cat));
        predicates.push(quote! {
            /// Zero-cost superstate predicate (compile-time group membership).
            #[inline]
            pub const fn #pred(self) -> bool {
                matches!(self, #( Self::#members )|*)
            }
        });
    }
    let predicate_impl = (!predicates.is_empty()).then(|| {
        quote! { impl #mname { #( #predicates )* } }
    });

    let initial_enter = initial_enter_fn(&model)?;

    let mut fns: Vec<TokenStream> = Vec::new();
    for (li, &leaf) in model.leaves.iter().enumerate() {
        for (owner, t, target) in &model.routes[li] {
            fns.push(transition_fn(&model, leaf, *owner, t, *target)?);
        }
    }

    Ok(quote! {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum #mname {
            #( #variants ),*
        }
        impl ::boyko_ecs::ecs::core::state::States for #mname {}
        #predicate_impl
        #initial_enter
        #( #fns )*
    })
}

/// One generated (leaf, event) transition system (§3.5's After block, real paths).
fn transition_fn(
    model: &MachineModel<'_>,
    leaf: usize,
    owner: usize,
    t: &TransitionDef,
    target: usize,
) -> syn::Result<TokenStream> {
    let _ = owner;
    let mname = &model.def.name;
    let fn_name = transition_fn_ident(mname, &model.nodes[leaf].cat, &t.event);
    let event = &t.event;
    let sys = quote!(::boyko_ecs::ecs::core::system);
    let state = quote!(::boyko_ecs::ecs::core::state);

    // LCA of source leaf and target leaf: longest common lineage prefix.
    let src_line = model.lineage(leaf);
    let dst_line = model.lineage(target);
    let mut lca_depth = 0;
    while lca_depth < src_line.len()
        && lca_depth < dst_line.len()
        && src_line[lca_depth] == dst_line[lca_depth]
    {
        lca_depth += 1;
    }
    // Exit actions: source-side states BELOW the LCA, innermost-first.
    let exit_nodes: Vec<usize> = src_line[lca_depth..].iter().rev().copied().collect();
    // Enter actions: target-side states below the LCA, outermost-first.
    let enter_nodes: Vec<usize> = dst_line[lca_depth..].to_vec();

    // Merged params: transition params + exit/enter handler params (§5.3).
    let mut all: Vec<&SysParam> = t.params.iter().collect();
    for &i in &exit_nodes {
        if let Some(h) = &model.nodes[i].state.exit {
            all.extend(h.params.iter());
        }
    }
    for &i in &enter_nodes {
        if let Some(h) = &model.nodes[i].state.enter {
            all.extend(h.params.iter());
        }
    }
    let merged = merge_params(&all, "this transition's merged enter/exit/action handlers")?;
    let params = merged.iter().map(|p| sys_param_tokens(p));

    // §5.1's "one transition per machine per frame", spelled DRAIN-THEN-ACT.
    //
    // The obvious body — act on the first accepted event and `return` — leaks: the kernel's
    // `EventIter` advances the cursor only past the events it YIELDED, so breaking out mid-drain
    // leaves the rest of THIS frame's events unread, and the next frame re-reads them and fires
    // a second transition. §5.1 requires the remainder to be "observed and discarded", so the
    // loop always runs to completion and merely REMEMBERS that one event was accepted; the
    // exit/action/enter chain and the `NextState` write happen once, after the drain.
    //
    // (§3.5's illustrative template shows the `return`-in-loop form; §5.1 is the semantic
    // authority and the two disagree — recorded here because the emission follows §5.1.)
    let accept = match t.guard.as_ref() {
        // Short-circuit on `!__aether_fire` so a guard with side effects is not evaluated
        // against the events that are being discarded.
        Some(g) => quote! { if !__aether_fire && (#g) { __aether_fire = true; } },
        None => quote! { __aether_fire = true; },
    };
    let exits = exit_nodes.iter().filter_map(|&i| {
        model.nodes[i].state.exit.as_ref().map(|h| {
            let b = &h.body;
            quote! { { #b } }
        })
    });
    let action = t.action.as_ref().map(|a| quote! { { #a } });
    let enters = enter_nodes.iter().filter_map(|&i| {
        model.nodes[i].state.enter.as_ref().map(|h| {
            let b = &h.body;
            quote! { { #b } }
        })
    });
    let target_variant = model.variant(target);

    Ok(quote! {
        fn #fn_name(
            mut __aether_ev: #sys::EventReader<#event>,
            mut __aether_next: #sys::ResMut<#state::NextState<#mname>>,
            #( #params ),*
        ) {
            let mut __aether_fire = false;
            for _ in __aether_ev.read() {
                #accept
            }
            if __aether_fire {
                #( #exits )*
                #action
                #( #enters )*
                *__aether_next = #state::NextState::Pending(#mname::#target_variant);
            }
        }
    })
}

/// The plugin-side registrations for one machine: `insert_state(initial-leaf)`, the §5.3
/// initial-enter chain as a startup system **after** the seed, and one
/// `run_if(in_state(leaf))` Main registration per generated transition system.
fn machine_registrations(
    def: &MachineDef,
) -> syn::Result<(TokenStream, Vec<TokenStream>)> {
    let model = MachineModel::build(def)?;
    let mname = &def.name;
    let init_variant = model.variant(model.initial_leaf);
    let initial_enter = has_initial_enter(&model).then(|| {
        let f = initial_enter_fn_ident(mname);
        quote! { app.add_startup_system(#f); }
    });
    let insert = quote! {
        app.insert_state(#mname::#init_variant);
        #initial_enter
    };
    let cond = quote!(::boyko_ecs::ecs::core::schedule::common_conditions::in_state);
    let mut stmts = Vec::new();
    for (li, &leaf) in model.leaves.iter().enumerate() {
        let leaf_variant = model.variant(leaf);
        for (_, t, _) in &model.routes[li] {
            let fn_name = transition_fn_ident(mname, &model.nodes[leaf].cat, &t.event);
            stmts.push(quote! {
                b.add_system(#fn_name).run_if(#cond(#mname::#leaf_variant));
            });
        }
    }
    Ok((insert, stmts))
}

#[cfg(test)]
mod tests {
    //! The A0 snapshot channel (see the crate doc's macrotest note): `expand_block` is a plain
    //! function, and these tests pin its output token-for-token — parse and expansion in one
    //! assertion, against the §3.1 before/after pair VERBATIM.

    use quote::quote;

    /// Normalized (whitespace-insensitive) token equality: `TokenStream::to_string` is already
    /// canonical for identical streams, so a plain string compare IS token equality.
    fn expands_to(input: proc_macro2::TokenStream, expected: proc_macro2::TokenStream) {
        let got = crate::expand_block(input).to_string();
        let want = expected.to_string();
        assert_eq!(got, want, "expansion drifted from the pinned §3.1 surface");
    }

    #[test]
    fn the_section_3_1_before_after_pair_holds_verbatim() {
        expands_to(
            quote! {
                component Health {
                    current: f32,
                    max: f32,
                    requires Regen,
                    on_add = heal_full,
                }

                tag Player;
                tag Stunned(bitset);
            },
            quote! {
                #[derive(::boyko_macros::Component)]
                #[require(Regen)]
                #[component(on_add = heal_full)]
                pub struct Health {
                    pub current: f32,
                    pub max: f32
                }
                #[derive(::boyko_macros::Component)]
                pub struct Player;
                #[derive(::boyko_macros::Component)]
                #[component(storage = "bitset")]
                pub struct Stunned;
            },
        );
    }

    #[test]
    fn no_bundle_and_multi_requires_and_every_hook_key_forward() {
        expands_to(
            quote! {
                component Rig {
                    bone: u32,
                    requires A, b::C,
                    on_insert = f::g,
                    on_remove = h,
                    no_bundle,
                }
            },
            quote! {
                #[derive(::boyko_macros::Component)]
                #[require(A, b::C)]
                #[component(on_insert = f::g, on_remove = h, no_bundle)]
                pub struct Rig {
                    pub bone: u32
                }
            },
        );
    }

    #[test]
    fn a_fieldless_component_is_a_plain_zst() {
        expands_to(
            quote! { component Marker {} },
            quote! {
                #[derive(::boyko_macros::Component)]
                pub struct Marker {}
            },
        );
    }

    /// Every diagnostic below asserts the MESSAGE (the contract a user reads), not the span —
    /// span pinning is rung A7's column-exact sweep.
    fn fails_with(input: proc_macro2::TokenStream, needle: &str) {
        let out = crate::expand_block(input).to_string();
        assert!(
            out.contains("compile_error") && out.contains(needle),
            "expected a compile_error containing {needle:?}, got: {out}"
        );
    }

    #[test]
    fn the_section_3_2_and_3_4_pairs_hold_verbatim() {
        expands_to(
            quote! {
                bundle Projectile {
                    pos: Position,
                    vel: Velocity,
                }
            },
            quote! {
                #[derive(::boyko_macros::Bundle)]
                pub struct Projectile {
                    pub pos: Position,
                    pub vel: Velocity
                }
            },
        );
        expands_to(
            quote! {
                event Damage {
                    victim: entity(Position, Health),
                    amount: f32,
                }
            },
            quote! {
                #[::boyko_macros::event]
                pub struct Damage {
                    #[participant(components = "Position, Health")]
                    pub victim: ::boyko_ecs::ecs::core::entity::entity::Entity,
                    #[parameter]
                    pub amount: f32
                }
            },
        );
    }

    #[test]
    fn a1_diagnostics_fire_where_the_plan_says() {
        // The 17th field carries the arity error's span-friendly message.
        let mut fields = proc_macro2::TokenStream::new();
        for i in 0..17 {
            let f = proc_macro2::Ident::new(&format!("f{i}"), proc_macro2::Span::call_site());
            fields.extend(quote! { #f: u32, });
        }
        fails_with(quote! { bundle Fat { #fields } }, "bundle arity is capped at 16");
        // A participant without its component context is refused, never defaulted.
        fails_with(
            quote! { event E { victim: entity, } },
            "participant fields name their component context",
        );
        // `entity` stays contextual: a qualified path is an ordinary parameter type.
        expands_to(
            quote! { event E { thing: my::entity, } },
            quote! {
                #[::boyko_macros::event]
                pub struct E {
                    #[parameter]
                    pub thing: my::entity
                }
            },
        );
    }

    #[test]
    fn unknown_construct_lists_the_registry_and_suggests() {
        fails_with(
            quote! { compnent Health {} },
            "unknown construct `compnent`",
        );
        fails_with(quote! { compnent Health {} }, "did you mean `component`?");
    }

    #[test]
    fn planned_constructs_name_their_rung_instead_of_pretending_unknown() {
        fails_with(quote! { material gold {} }, "lands at rung A5");
        fails_with(quote! { scene lab {} }, "lands at rung A6");
    }

    // -------------------------------------------------------- night-review fixes (A0/A1 scope)

    /// The review's MAJOR: a participant context the derive's comma-split ident channel cannot
    /// carry must be refused HERE, on the user's tokens — never forwarded into a downstream
    /// proc-macro panic with no span.
    #[test]
    fn participant_context_rejects_paths_and_generics_on_the_users_span() {
        fails_with(
            quote! { event E { hit: entity(foo::Bar), } },
            "bare component idents",
        );
        fails_with(
            quote! { event E { hit: entity(Slot<A, B>), } },
            "bare component idents",
        );
        // The plain form still passes untouched.
        expands_to(
            quote! { event E { hit: entity(Health), } },
            quote! {
                #[::boyko_macros::event]
                pub struct E {
                    #[participant(components = "Health")]
                    pub hit: ::boyko_ecs::ecs::core::entity::entity::Entity
                }
            },
        );
    }

    /// The review's position-dependence: a path whose FIRST segment spells a keyword parses
    /// the same at every list position (`ident ::` continues a path, bare keywords open items).
    #[test]
    fn requires_list_accepts_keyword_headed_paths_at_every_position() {
        expands_to(
            quote! { component X { requires A, no_bundle::C, on_add = f } },
            quote! {
                #[derive(::boyko_macros::Component)]
                #[require(A, no_bundle::C)]
                #[component(on_add = f)]
                pub struct X {}
            },
        );
    }

    /// The review's Unicode finding: a name the ASCII probe cannot classify must not produce a
    /// self-identical rename suggestion; `char::is_uppercase` accepts any titled spelling.
    #[test]
    fn unicode_names_pass_the_case_gate_or_fail_without_a_useless_rename() {
        // A Cyrillic-titled component is UpperCamelCase in its own script — accepted.
        expands_to(
            quote! { component Здоровье { hp: f32 } },
            quote! {
                #[derive(::boyko_macros::Component)]
                pub struct Здоровье {
                    pub hp: f32
                }
            },
        );
        // A lowercase Cyrillic name still fails, WITH a real (different) suggestion.
        fails_with(quote! { component здоровье { hp: f32 } }, "rename `здоровье` to `Здоровье`");
    }

    // ------------------------------------------------------------------ rung A2: system+plugin

    /// The §3.3 before/after pair, verbatim — with the REAL nested engine paths substituted for
    /// the plan's idealized `::boyko_ecs::Res` (the root re-exports only App/Plugin/…; tokens
    /// must RESOLVE — the A1 Entity precedent), and the plan's `…` bodies made concrete.
    #[test]
    fn the_section_3_3_before_after_pair_holds_verbatim() {
        expands_to(
            quote! {
                plugin Movement;

                system read_input(actions: res<ActionState>, mut cmds: commands)
                    on update in InputSet
                {
                    let _ = (&actions, &mut cmds);
                }

                system apply_velocity(q: query<(&mut Transform, &Velocity), with Player, without Frozen>,
                                      time: res<Time>)
                    on update
                    after read_input
                    when in_state(GameFlow::Playing)
                {
                    for (t, v) in &mut q {
                        t.translation += v.linear * time.delta_secs();
                    }
                }
            },
            quote! {
                pub struct Movement;
                impl ::boyko_ecs::Plugin for Movement {
                    fn build(&self, app: &mut ::boyko_ecs::App) {
                        app.add_systems_cfg(|b| {
                            let __aether_k_read_input = b.add_system(read_input).in_set(InputSet).key();
                            b.add_system(apply_velocity).after(__aether_k_read_input).run_if(in_state(GameFlow::Playing));
                        });
                    }
                    fn name(&self) -> &'static str { "Movement" }
                }
                pub fn read_input(
                    actions: ::boyko_ecs::ecs::core::system::Res<ActionState>,
                    mut cmds: ::boyko_ecs::ecs::core::system::Commands
                ) {
                    let _ = (&actions, &mut cmds);
                }
                pub fn apply_velocity(
                    mut q: ::boyko_ecs::ecs::core::iters::query::Query<
                        (&mut Transform, &Velocity),
                        (::boyko_ecs::ecs::core::iters::query::With<Player>,
                         ::boyko_ecs::ecs::core::iters::query::Without<Frozen>)
                    >,
                    time: ::boyko_ecs::ecs::core::system::Res<Time>
                ) {
                    for (t, v) in &mut q {
                        t.translation += v.linear * time.delta_secs();
                    }
                }
            },
        );
    }

    /// A clause-free system needs no plugin and expands to a bare fn — nothing else.
    #[test]
    fn a_clause_free_system_is_a_plain_fn_without_a_plugin() {
        expands_to(
            quote! { system tick(n: mut res<Counter>) { n.0 += 1; } },
            quote! {
                pub fn tick(mut n: ::boyko_ecs::ecs::core::system::ResMut<Counter>) { n.0 += 1; }
            },
        );
    }

    /// The mutability-inference table: query-with-&mut, mut res, commands, emit and events get
    /// `mut` bindings; plain res / read-only query / verbatim escapes do not. `Mutation` as a
    /// type name must NOT false-positive (the token-level scan, not a substring scan).
    #[test]
    fn mutability_inference_follows_the_param_table() {
        expands_to(
            quote! {
                system s(a: query<&T>, b: query<&mut T>, c: res<R>, d: events<E>, e: emit<E>,
                         f: query<&Mutation>, g: local<u32>) {}
            },
            quote! {
                pub fn s(
                    a: ::boyko_ecs::ecs::core::iters::query::Query<&T>,
                    mut b: ::boyko_ecs::ecs::core::iters::query::Query<&mut T>,
                    c: ::boyko_ecs::ecs::core::system::Res<R>,
                    mut d: ::boyko_ecs::ecs::core::system::EventReader<E>,
                    mut e: ::boyko_ecs::ecs::core::system::EventWriter<E>,
                    f: ::boyko_ecs::ecs::core::iters::query::Query<&Mutation>,
                    g: ::boyko_ecs::ecs::core::system::Local<u32>
                ) {}
            },
        );
    }

    /// Schedule routing: startup one-shots lead, Fixed lands in `add_systems_cfg_in`, a single
    /// filter stays bare (no 1-tuple), a non-sibling path becomes `after_set`, and the verbatim
    /// escape hatch passes any real SystemParam through untouched.
    #[test]
    fn schedules_sets_and_the_escape_hatch_route_correctly() {
        expands_to(
            quote! {
                plugin Sim;
                system boot(mut cmds: commands) on startup { let _ = &mut cmds; }
                system step(q: query<&mut Body, with Alive>) on fixed after PhysicsSet { let _ = &mut q; }
                system draw(dev: NonSendRes<Gpu>) on update { let _ = &dev; }
            },
            quote! {
                pub struct Sim;
                impl ::boyko_ecs::Plugin for Sim {
                    fn build(&self, app: &mut ::boyko_ecs::App) {
                        app.add_startup_system(boot);
                        app.add_systems_cfg(|b| {
                            b.add_system(draw);
                        });
                        app.add_systems_cfg_in(::boyko_ecs::ecs::core::app::CoreSchedule::Fixed, |b| {
                            b.add_system(step).after_set(PhysicsSet);
                        });
                    }
                    fn name(&self) -> &'static str { "Sim" }
                }
                pub fn boot(mut cmds: ::boyko_ecs::ecs::core::system::Commands) { let _ = &mut cmds; }
                pub fn step(
                    mut q: ::boyko_ecs::ecs::core::iters::query::Query<
                        &mut Body,
                        ::boyko_ecs::ecs::core::iters::query::With<Alive>
                    >
                ) { let _ = &mut q; }
                pub fn draw(dev: NonSendRes<Gpu>) { let _ = &dev; }
            },
        );
    }

    /// A sibling `before` edge still emits the TARGET first (its key must exist), with
    /// `.before(key)` on the referrer.
    #[test]
    fn sibling_before_captures_the_targets_key() {
        expands_to(
            quote! {
                plugin P;
                system a() on update before z {}
                system z() on update {}
            },
            quote! {
                pub struct P;
                impl ::boyko_ecs::Plugin for P {
                    fn build(&self, app: &mut ::boyko_ecs::App) {
                        app.add_systems_cfg(|b| {
                            let __aether_k_z = b.add_system(z).key();
                            b.add_system(a).before(__aether_k_z);
                        });
                    }
                    fn name(&self) -> &'static str { "P" }
                }
                pub fn a() {}
                pub fn z() {}
            },
        );
    }

    // ------------------------------------------------------------------ rung A3: machine

    /// The §3.5 before/after pair, verbatim — the plan's GameFlow chart with its `…` bodies
    /// made concrete, against the REAL nested engine paths. Pins: flattening (4 leaves), the
    /// superstate predicate, innermost-wins inheritance (PlayerDied exists for BOTH Playing
    /// leaves), LCA exit/enter inlining, guard placement, first-accepted-event-wins, and the
    /// plugin's insert_state + run_if(in_state(leaf)) registrations.
    #[test]
    fn the_section_3_5_before_after_pair_holds_verbatim() {
        expands_to(
            quote! {
                plugin Flow;

                machine GameFlow {
                    initial Boot;

                    state Boot {
                        on AssetsReady => Playing;
                    }

                    state Playing {
                        initial Running;
                        enter (mut cmds: commands) { cmds.spawn(Hud); }
                        exit (mut cmds: commands) { cmds.despawn_hud(); }

                        state Running {
                            on PausePressed => Playing.Paused;
                        }
                        state Paused {
                            on PausePressed => Playing.Running;
                        }

                        on PlayerDied (score: res<Score>) if score.lives == 0 => GameOver {
                        }
                    }

                    state GameOver {
                        on RestartPressed => Boot;
                    }
                }
            },
            quote! {
                pub struct Flow;
                impl ::boyko_ecs::Plugin for Flow {
                    fn build(&self, app: &mut ::boyko_ecs::App) {
                        app.insert_state(GameFlow::Boot);
                        app.add_systems_cfg(|b| {
                            b.add_system(__aether_game_flow__boot__assets_ready)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(GameFlow::Boot));
                            b.add_system(__aether_game_flow__playing_running__pause_pressed)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(GameFlow::PlayingRunning));
                            b.add_system(__aether_game_flow__playing_running__player_died)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(GameFlow::PlayingRunning));
                            b.add_system(__aether_game_flow__playing_paused__pause_pressed)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(GameFlow::PlayingPaused));
                            b.add_system(__aether_game_flow__playing_paused__player_died)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(GameFlow::PlayingPaused));
                            b.add_system(__aether_game_flow__game_over__restart_pressed)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(GameFlow::GameOver));
                        });
                    }
                    fn name(&self) -> &'static str { "Flow" }
                }
                #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
                pub enum GameFlow {
                    Boot,
                    PlayingRunning,
                    PlayingPaused,
                    GameOver
                }
                impl ::boyko_ecs::ecs::core::state::States for GameFlow {}
                impl GameFlow {
                    /// Zero-cost superstate predicate (compile-time group membership).
                    #[inline]
                    pub const fn in_playing(self) -> bool {
                        matches!(self, Self::PlayingRunning | Self::PlayingPaused)
                    }
                }
                fn __aether_game_flow__boot__assets_ready(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<AssetsReady>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<GameFlow>>,
                    mut cmds: ::boyko_ecs::ecs::core::system::Commands
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        __aether_fire = true;
                    }
                    if __aether_fire {
                        { cmds.spawn(Hud); }
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(GameFlow::PlayingRunning);
                    }
                }
                fn __aether_game_flow__playing_running__pause_pressed(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<PausePressed>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<GameFlow>>,
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        __aether_fire = true;
                    }
                    if __aether_fire {
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(GameFlow::PlayingPaused);
                    }
                }
                fn __aether_game_flow__playing_running__player_died(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<PlayerDied>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<GameFlow>>,
                    score: ::boyko_ecs::ecs::core::system::Res<Score>,
                    mut cmds: ::boyko_ecs::ecs::core::system::Commands
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        if !__aether_fire && (score.lives == 0) { __aether_fire = true; }
                    }
                    if __aether_fire {
                        { cmds.despawn_hud(); }
                        { }
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(GameFlow::GameOver);
                    }
                }
                fn __aether_game_flow__playing_paused__pause_pressed(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<PausePressed>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<GameFlow>>,
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        __aether_fire = true;
                    }
                    if __aether_fire {
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(GameFlow::PlayingRunning);
                    }
                }
                fn __aether_game_flow__playing_paused__player_died(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<PlayerDied>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<GameFlow>>,
                    score: ::boyko_ecs::ecs::core::system::Res<Score>,
                    mut cmds: ::boyko_ecs::ecs::core::system::Commands
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        if !__aether_fire && (score.lives == 0) { __aether_fire = true; }
                    }
                    if __aether_fire {
                        { cmds.despawn_hud(); }
                        { }
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(GameFlow::GameOver);
                    }
                }
                fn __aether_game_flow__game_over__restart_pressed(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<RestartPressed>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<GameFlow>>,
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        __aether_fire = true;
                    }
                    if __aether_fire {
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(GameFlow::Boot);
                    }
                }
            },
        );
    }

    #[test]
    fn a3_diagnostics_fire_where_the_plan_says() {
        // Unknown `initial` target, with the declared list + did-you-mean (§3.5 verbatim).
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial Playing;
                    state Playing { initial Runing; state Running {} state Paused {} }
                }
            },
            "no state `Runing` in `Playing`; states declared here: `Running`, `Paused` (did you mean `Running`?)",
        );
        // A transition targeting a composite with no `initial` (§3.5 verbatim).
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial Boot;
                    state Boot { on Go => Playing; }
                    state Playing { state Running {} }
                }
            },
            "target `Playing` is a composite state with no `initial`",
        );
        // Two handlers for one event in one state — the second `on` errs.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A { on E => A; on E => A; }
                }
            },
            "duplicate handler for `E` in state `A`",
        );
        // A machine needs the plugin header.
        fails_with(
            quote! { machine M { initial A; state A {} } },
            "a `machine` needs a `plugin <Name>;` declaration",
        );
        // A merged-param NAME reused with a different type across handlers is refused.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A {
                        exit (mut cmds: commands) { let _ = &mut cmds; }
                        on E (cmds: res<Thing>) => B;
                    }
                    state B {}
                }
            },
            "conflicting types",
        );
    }

    // ------------------------------------------------------- rung A4: machine hierarchy depth

    /// §5.3's initial-enter chain and the LCA rule's *upper* bound, in one three-level chart.
    ///
    /// Pins, all of them A4 contract:
    /// * the initial resolves through TWO composite `initial` hops (`World` → `Field` → `Idle`);
    /// * `insert_state` is followed by the chain's ONE startup system — `enter` bodies of the
    ///   initial leaf's whole ancestor path, outermost-first, params merged across the three
    ///   handlers (`cmds` declared twice at one type collapses to one binding);
    /// * a transition INSIDE `Field` runs only the enter/exit below the LCA — `Busy → Idle`
    ///   replays `Idle`'s enter and NOT `World`'s or `Field`'s, which the naive "enter the
    ///   whole target lineage" reading would have emitted;
    /// * one `in_<group>` predicate per composite, at both depths.
    #[test]
    fn the_section_5_3_initial_enter_chain_and_the_lca_bound_hold_verbatim() {
        expands_to(
            quote! {
                plugin Boot;

                machine Sim {
                    initial World;

                    state World {
                        initial Field;
                        enter (mut cmds: commands) { cmds.spawn(Ground); }

                        state Field {
                            initial Idle;
                            enter (mut cmds: commands, log: mut res<Probe>) { log.field += 1; }

                            state Idle {
                                enter (log: mut res<Probe>) { log.idle += 1; }
                                on Go => World.Field.Busy;
                            }
                            state Busy {
                                on Stop => World.Field.Idle;
                            }
                        }
                    }
                }
            },
            quote! {
                pub struct Boot;
                impl ::boyko_ecs::Plugin for Boot {
                    fn build(&self, app: &mut ::boyko_ecs::App) {
                        app.insert_state(Sim::WorldFieldIdle);
                        app.add_startup_system(__aether_sim__initial_enter);
                        app.add_systems_cfg(|b| {
                            b.add_system(__aether_sim__world_field_idle__go)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(Sim::WorldFieldIdle));
                            b.add_system(__aether_sim__world_field_busy__stop)
                                .run_if(::boyko_ecs::ecs::core::schedule::common_conditions::in_state(Sim::WorldFieldBusy));
                        });
                    }
                    fn name(&self) -> &'static str { "Boot" }
                }
                #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
                pub enum Sim {
                    WorldFieldIdle,
                    WorldFieldBusy
                }
                impl ::boyko_ecs::ecs::core::state::States for Sim {}
                impl Sim {
                    /// Zero-cost superstate predicate (compile-time group membership).
                    #[inline]
                    pub const fn in_world(self) -> bool {
                        matches!(self, Self::WorldFieldIdle | Self::WorldFieldBusy)
                    }
                    /// Zero-cost superstate predicate (compile-time group membership).
                    #[inline]
                    pub const fn in_world_field(self) -> bool {
                        matches!(self, Self::WorldFieldIdle | Self::WorldFieldBusy)
                    }
                }
                fn __aether_sim__initial_enter(
                    mut cmds: ::boyko_ecs::ecs::core::system::Commands,
                    mut log: ::boyko_ecs::ecs::core::system::ResMut<Probe>
                ) {
                    { cmds.spawn(Ground); }
                    { log.field += 1; }
                    { log.idle += 1; }
                }
                fn __aether_sim__world_field_idle__go(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<Go>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<Sim>>,
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        __aether_fire = true;
                    }
                    if __aether_fire {
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(Sim::WorldFieldBusy);
                    }
                }
                fn __aether_sim__world_field_busy__stop(
                    mut __aether_ev: ::boyko_ecs::ecs::core::system::EventReader<Stop>,
                    mut __aether_next: ::boyko_ecs::ecs::core::system::ResMut<::boyko_ecs::ecs::core::state::NextState<Sim>>,
                    mut log: ::boyko_ecs::ecs::core::system::ResMut<Probe>
                ) {
                    let mut __aether_fire = false;
                    for _ in __aether_ev.read() {
                        __aether_fire = true;
                    }
                    if __aether_fire {
                        { log.idle += 1; }
                        *__aether_next = ::boyko_ecs::ecs::core::state::NextState::Pending(Sim::WorldFieldIdle);
                    }
                }
            },
        );
    }

    /// An initial chain with no `enter` anywhere emits NO startup system and NO registration —
    /// the §8 R1 expansion-volume rule applied to the one construct that could have shipped a
    /// dead empty fn per machine.
    #[test]
    fn an_enterless_initial_chain_emits_no_startup_system() {
        expands_to(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A { initial B; state B {} }
                }
            },
            quote! {
                pub struct P;
                impl ::boyko_ecs::Plugin for P {
                    fn build(&self, app: &mut ::boyko_ecs::App) {
                        app.insert_state(M::AB);
                    }
                    fn name(&self) -> &'static str { "P" }
                }
                #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
                pub enum M {
                    AB
                }
                impl ::boyko_ecs::ecs::core::state::States for M {}
                impl M {
                    /// Zero-cost superstate predicate (compile-time group membership).
                    #[inline]
                    pub const fn in_a(self) -> bool {
                        matches!(self, Self::AB)
                    }
                }
            },
        );
    }

    /// Assert `first` is emitted before `second` — for contracts about ORDER, where a full
    /// token pin would bury the one claim being made.
    fn emits_in_order(input: proc_macro2::TokenStream, first: &str, second: &str) {
        let out = crate::expand_block(input).to_string();
        let (i, j) = (out.find(first), out.find(second));
        assert!(
            matches!((i, j), (Some(a), Some(b)) if a < b),
            "expected {first:?} before {second:?}, got: {out}"
        );
    }

    /// §5.1 makes two same-frame transitions deterministic by registering "in declaration
    /// order". Inheritance walks each leaf innermost-first, so a superstate handler declared
    /// BEFORE the leaf's own would otherwise register after it — order decided by the tree,
    /// not by the source.
    #[test]
    fn registration_follows_declaration_order_not_the_inheritance_walk() {
        emits_in_order(
            quote! {
                plugin P;
                machine M {
                    initial P0;
                    state P0 {
                        initial A;
                        on E1 => X;
                        state A { on E2 => X; }
                    }
                    state X {}
                }
            },
            "__aether_m__p0_a__e1",
            "__aether_m__p0_a__e2",
        );
        // …and the reverse source order registers the other way round, so the ordering tracks
        // the SOURCE rather than some fixed leaf-vs-ancestor rule.
        emits_in_order(
            quote! {
                plugin P;
                machine M {
                    initial P0;
                    state P0 {
                        initial A;
                        state A { on E2 => X; }
                        on E1 => X;
                    }
                    state X {}
                }
            },
            "__aether_m__p0_a__e2",
            "__aether_m__p0_a__e1",
        );
    }

    /// `Expr::Let` is a real expression node, legal only as an `if`/`while` scrutinee. Aether
    /// splices guards into `if !(…)` and conditions into `.run_if(…)`, where it is not valid
    /// Rust — so it is refused on the user's own `let`.
    #[test]
    fn let_bindings_are_refused_as_guards_and_run_conditions() {
        fails_with(
            quote! { plugin P; system s() on update when let Some(x) = f() {} },
            "`let` bindings are not usable as a run condition",
        );
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A { on E if let Some(_) = q => A; }
                }
            },
            "`let` bindings are not usable as a transition guard",
        );
    }

    /// A raw ident prints WITH its `r#` escape, so the case gate must classify the escaped
    /// spelling — otherwise `r#Foo` is refused for starting with `r`, and the suggested rename
    /// (`R#Foo`) is not a legal identifier at all.
    #[test]
    fn the_case_gate_reads_through_a_raw_ident_escape() {
        expands_to(
            quote! { component r#Foo { x: u32 } },
            quote! {
                #[derive(::boyko_macros::Component)]
                pub struct r#Foo {
                    pub x: u32
                }
            },
        );
        fails_with(quote! { component r#health { hp: f32 } }, "rename `r#health` to `Health`");
    }

    #[test]
    fn a4_hierarchy_diagnostics_fire_where_the_plan_says() {
        // Flattening is CONCATENATION, so two chart positions can collide on one generated
        // name. Aether names both positions and the name they share.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A { initial BC; state BC {} }
                    state AB { initial C; state C {} }
                }
            },
            "states `A.BC` and `AB.C` both flatten to `ABC`",
        );
        // The degenerate case of the same check reads as what it is.
        fails_with(
            quote! { plugin P; machine M { initial A; state A {} state A {} } },
            "duplicate state `A` — sibling states need distinct names",
        );
        // `initial` on a childless state can never retarget anything — silently ignoring it
        // would leave the author believing a nested chart exists.
        fails_with(
            quote! { plugin P; machine M { initial A; state A { initial B; } } },
            "`A` has no nested states, so `initial` has nothing to name",
        );
        // Reachability must not decide whether a name is checked: `Lonely` is never targeted,
        // so the lazy `resolve_to_leaf` path would never have looked at its typo.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A { on Go => A; }
                    state Lonely { initial Runing; state Running {} }
                }
            },
            "no state `Runing` in `Lonely`",
        );
        // Inheritance dedups on the event's FULL spelling, the generated fn name on its LAST
        // segment — the gap between the two would emit one fn twice.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A { on a::E => A; on b::E => A; }
                }
            },
            "events `a::E` and `b::E` both generate the system `__aether_m__a__e` for leaf `A`",
        );
        // The OTHER half of the same name is just as lossy: two leaves whose flattened names
        // differ but whose snake_case collapse does not.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial AB;
                    state AB { on E => AB; }
                    state A_b { on E => A_b; }
                }
            },
            "states `AB` and `A_b` both generate the system `__aether_m__a_b__e`",
        );
        // …and the composite form of that collapse, which lands on the predicate instead.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial AB;
                    state AB { initial X; state X {} }
                    state A_b { initial Y; state Y {} }
                }
            },
            "which both collapse to the predicate `in_a_b`",
        );
        // A handler an inner state SHADOWS is never inherited by any leaf, so the per-leaf walk
        // never resolves its target — but a target that names nothing is still a broken chart.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial P0;
                    state P0 {
                        initial A;
                        on E => Nowhere;
                        state A { on E => Top; }
                    }
                    state Top {}
                }
            },
            "no state `Nowhere` in `M`",
        );
        // The merged-param conflict rule reaches the initial-enter chain too, and names ITS site.
        fails_with(
            quote! {
                plugin P;
                machine M {
                    initial A;
                    state A {
                        initial B;
                        enter (x: mut res<T>) { let _ = &mut x; }
                        state B { enter (x: res<T>) { let _ = &x; } }
                    }
                }
            },
            "conflicting types across the initial state's merged `enter` chain",
        );
    }

    #[test]
    fn a2_diagnostics_fire_where_the_plan_says() {
        // §3.3: parens instead of angle brackets on query.
        fails_with(
            quote! { system s(q: query(&mut T)) {} },
            "query takes angle brackets",
        );
        // §3.3: duplicate `on`.
        fails_with(
            quote! { plugin P; system s() on update on fixed {} },
            "duplicate schedule clause; a system runs on exactly one schedule",
        );
        // §3.3: clauses need the plugin header.
        fails_with(
            quote! { system s() on update {} },
            "need a `plugin <Name>;` declaration",
        );
        // §3.3: startup rejects every other clause.
        fails_with(
            quote! { plugin P; system s() on startup in SomeSet {} },
            "rejected on startup systems",
        );
        // Cross-schedule sibling ordering is not expressible.
        fails_with(
            quote! { plugin P; system a() on fixed {} system c() on update after a {} },
            "runs on a different schedule",
        );
        // Ordering against a startup system is meaningless.
        fails_with(
            quote! { plugin P; system a() on startup {} system c() on update after a {} },
            "cannot be ordered against",
        );
        // A sibling ordering cycle is caught at expansion, before the engine's own check.
        fails_with(
            quote! { plugin P; system a() on update after c {} system c() on update after a {} },
            "system ordering cycle",
        );
        // The did-you-mean deviation: a near-miss bare ident errors with the note's text.
        fails_with(
            quote! { plugin P; system read_input() on update {} system s() on update after read_inpt {} },
            "a sibling `read_input` exists",
        );
        // Unknown filter with a suggestion.
        fails_with(
            quote! { system s(q: query<&T, wih P>) {} },
            "unknown query filter `wih`",
        );
        // Unknown clause with a suggestion.
        fails_with(
            quote! { plugin P; system s() afterr X {} },
            "unknown clause `afterr`",
        );
        // One plugin per block.
        fails_with(
            quote! { plugin A; plugin B; },
            "one `plugin` per aether block",
        );
        // Case conventions on both new constructs.
        fails_with(quote! { plugin movement; }, "rename `movement` to `Movement`");
        fails_with(quote! { system Foo() {} }, "system names are snake_case");
    }

    #[test]
    fn case_convention_diagnosed_with_a_rename() {
        fails_with(quote! { component health { hp: f32 } }, "rename `health` to `Health`");
        fails_with(quote! { tag player; }, "rename `player` to `Player`");
    }

    #[test]
    fn duplicate_hooks_and_bad_tag_modifiers_are_refused() {
        fails_with(
            quote! { component A { on_add = f, on_add = g } },
            "duplicate hook `on_add`",
        );
        fails_with(quote! { tag T(dense); }, "unknown tag modifier `dense`");
    }
}
