//! The expander — Decision A3: emit the CANONICAL hand-written surface and let `boyko_macros`
//! do the codegen. One expansion authority, zero drift, minimal expansion volume (§8 R1); every
//! engine path below is a TOKEN resolved downstream, never a dependency of this crate.

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::Ident;

use crate::ast::{
    AetherBlock, BundleDef, ComponentDef, Construct, EvField, EventDef, OrderKind, PluginDef,
    Schedule, SysParam, SysParamTy, SystemDef, TagDef,
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
    let sys = quote!(::boyko_ecs::ecs::core::system);
    let (inferred_mut, ty) = match &p.ty {
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
    };
    let mut_kw = (p.explicit_mut || inferred_mut).then(|| quote!(mut));
    quote!(#mut_kw #name: #ty)
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

    let main_stmts = bucket_stmts(&systems, &resolved, &needs_key, Schedule::Update)?;
    let fixed_stmts = bucket_stmts(&systems, &resolved, &needs_key, Schedule::Fixed)?;
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
        fails_with(quote! { machine G {} }, "lands at rung A3");
        fails_with(quote! { material gold {} }, "lands at rung A5");
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
