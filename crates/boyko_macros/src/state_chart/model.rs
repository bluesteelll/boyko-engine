//! The flattened chart: arena, leaves, per-leaf route merge, and the whole diagnostic set.
//!
//! The hierarchy exists ONLY here. The runtime sees a flat enum, one system per LEAF, and
//! `run_if(in_state(leaf))` — no tree walk, no parallel data structure, nothing that costs
//! anything at run time.
//!
//! # Why routes are merged PER LEAF and not per (leaf, event)
//!
//! The shape this replaces gave every route its own system, each gated only by
//! `run_if(in_state(leaf))`. `State<S>` is swapped once per frame, before the executor loop, so
//! within the accepting frame every one of a leaf's gates is open at the same time. Two events of
//! different types arriving for one leaf therefore ran BOTH exit/action/enter chains, and
//! `NextState` was decided by whichever system the scheduler happened to run last.
//!
//! Merging a leaf's routes into ONE system makes a second same-frame chain structurally
//! impossible — there is one selection and one chain — and it is also what makes **first declared
//! wins** expressible at all: arbitration between two systems can only be registration order,
//! while arbitration inside one body is a decision the author wrote.

use std::collections::BTreeSet;

use proc_macro2::Span;
use quote::ToTokens;
use syn::Ident;

use super::ast::{ChartInput, Param, StateNode, Transition, err};

/// One flattened node.
pub struct Node<'a> {
    pub state: &'a StateNode,
    /// Arena index of the parent; `None` for a top-level state.
    pub parent: Option<usize>,
    /// Concatenated path names (`PlayingRunning`) — the leaf's enum variant spelling.
    pub cat: String,
    /// Dotted path names (`Playing.Running`) — for diagnostics.
    pub dotted: String,
}

/// One inherited route on one leaf, with its target already resolved to a leaf.
///
/// The node that DECLARED the handler is deliberately NOT carried: the emitter derives the
/// exit/enter chains from the source leaf and the target leaf alone, so an owner index would be a
/// field nothing reads.
pub struct Route<'a> {
    pub def: &'a Transition,
    /// Arena index of the resolved target LEAF.
    pub target: usize,
}

/// The flattened chart.
pub struct Chart<'a> {
    pub input: &'a ChartInput,
    pub nodes: Vec<Node<'a>>,
    /// Arena indices of the LEAVES in preorder — the enum variant order.
    pub leaves: Vec<usize>,
    /// The chart's `initial`, resolved through composite `initial` chains to a leaf.
    pub initial_leaf: usize,
    /// Per-leaf routes: innermost-wins inheritance resolved, then re-sorted into DECLARATION
    /// order, which is the order arbitration reads.
    pub routes: Vec<Vec<Route<'a>>>,
}

impl<'a> Chart<'a> {
    /// Build + validate. Every hard fault is diagnosed here, on the user's own tokens.
    pub fn build(input: &'a ChartInput) -> syn::Result<Chart<'a>> {
        let mut nodes: Vec<Node<'a>> = Vec::new();
        walk(&mut nodes, &input.states, None);
        if nodes.is_empty() {
            return Err(err(input.name.span(), "a chart declares at least one state"));
        }
        let leaves: Vec<usize> =
            (0..nodes.len()).filter(|&i| nodes[i].state.children.is_empty()).collect();

        check_flattened_name_collisions(&nodes)?;
        check_predicate_collisions(&nodes)?;
        check_duplicate_handlers(&nodes)?;
        check_declared_initials(&nodes)?;

        // Every transition's TARGET is resolved here, not only the ones some leaf happens to
        // inherit: the per-leaf walk below resolves only handlers it actually reaches, and a
        // handler an inner state SHADOWS for the same event is never walked. Whether a name is
        // checked must not depend on whether some leaf happens to reach it.
        let chart_name = input.name.to_string();
        for n in &nodes {
            for t in &n.state.transitions {
                resolve_target(&nodes, &chart_name, &t.target)?;
            }
        }

        let root_initial = resolve_child(&nodes, None, &input.initial, &chart_name)?;
        let initial_leaf = resolve_to_leaf(&nodes, root_initial, input.initial.span())?;

        let routes = build_routes(&nodes, &leaves, &chart_name)?;
        check_generated_name_collisions(&nodes, &leaves, &routes, &input.name)?;

        let chart = Chart { input, nodes, leaves, initial_leaf, routes };
        chart.check_reachability()?;
        Ok(chart)
    }

    /// The leaf's enum variant ident, spanned at the leaf's own name.
    pub fn variant(&self, leaf: usize) -> Ident {
        Ident::new(&self.nodes[leaf].cat, self.nodes[leaf].state.name.span())
    }

    /// Root-first ancestor chain including `idx` itself.
    pub fn lineage(&self, idx: usize) -> Vec<usize> {
        lineage(&self.nodes, idx)
    }

    /// `true` iff the initial leaf's ancestor path declares at least one `enter`.
    pub fn has_initial_enter(&self) -> bool {
        self.lineage(self.initial_leaf).iter().any(|&i| self.nodes[i].state.enter.is_some())
    }

    /// **Reachability / dead-state analysis.**
    ///
    /// A leaf is reachable when the chart's own edges can put the machine there: the initial leaf
    /// is reachable, and any target of a route on a reachable leaf is reachable. Inherited
    /// handlers count, so one root-level `on Died => GameOver` makes `GameOver` reachable from
    /// every leaf. A composite is reachable when any descendant leaf is.
    ///
    /// **Severity: a hard ERROR.** The alternatives were weighed and only this one is
    /// implementable *and* falsifiable:
    ///
    /// * A **warning** is not on the table. Stable proc-macros have no diagnostic channel —
    ///   `proc_macro::Diagnostic` is nightly — so "warn" would mean emitting nothing, and an
    ///   analysis whose finding is invisible is the "gate that cannot fail" failure class this
    ///   repo names, not a softer gate.
    /// * **Silence** is what shipped, and it is why an unreachable state is currently a chart the
    ///   author reads as live and the machine can never enter.
    /// * Every sibling chart fault here — duplicate handler, unknown `initial`, composite without
    ///   `initial`, flattened-name collision — is already a hard error. A dead state is the same
    ///   class of author mistake in the same closed declarative artifact.
    ///
    /// The direction is also the safe one to be wrong in: relaxing a refusal later is additive,
    /// while tightening a permissive analysis is a breaking change.
    ///
    /// KNOWN GAP, recorded rather than papered over: `NextState<C>` is a public resource, so
    /// hand-written code CAN drive a chart from outside its own edges (`boyko_demo` does exactly
    /// this for its `Mode` state). A chart with a state entered only that way is a legal program
    /// this refusal rejects. The escape is one contextual keyword in a state body, and it is NOT
    /// added here because it would also have to be threaded through Aether's `machine` grammar,
    /// which belongs to the construct rewrite.
    fn check_reachability(&self) -> syn::Result<()> {
        let mut reachable: BTreeSet<usize> = BTreeSet::new();
        let mut work = vec![self.initial_leaf];
        reachable.insert(self.initial_leaf);
        while let Some(leaf) = work.pop() {
            let li = self
                .leaves
                .iter()
                .position(|&l| l == leaf)
                .expect("invariant: the worklist only ever holds leaf indices");
            for r in &self.routes[li] {
                if reachable.insert(r.target) {
                    work.push(r.target);
                }
            }
        }
        if reachable.len() == self.leaves.len() {
            return Ok(());
        }

        // A node is reachable iff any descendant leaf is. Report the MAXIMAL dead subtrees — one
        // error per dead composite, not one per leaf underneath it — so a chart with a whole
        // unused branch reads as one finding.
        let node_reachable: Vec<bool> = (0..self.nodes.len())
            .map(|i| {
                self.leaves
                    .iter()
                    .any(|&l| reachable.contains(&l) && self.lineage(l).contains(&i))
            })
            .collect();
        let dead_roots: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| {
                !node_reachable[i]
                    && self.nodes[i].parent.is_none_or(|p| node_reachable[p])
            })
            .collect();

        let mut combined: Option<syn::Error> = None;
        for &i in &dead_roots {
            let what = if self.nodes[i].state.children.is_empty() {
                format!("state `{}` is unreachable", self.nodes[i].dotted)
            } else {
                format!(
                    "state `{}` and everything nested in it are unreachable",
                    self.nodes[i].dotted
                )
            };
            let msg = format!(
                "{what}: no transition in `{chart}` targets it and it is not the chart's `initial` \
                 state, so the machine can never enter it — add a transition into it, or remove it",
                chart = self.input.name
            );
            let e = err(self.nodes[i].state.name.span(), msg);
            match &mut combined {
                Some(c) => c.combine(e),
                slot @ None => *slot = Some(e),
            }
        }
        match combined {
            Some(e) => Err(e),
            // `reachable.len() < leaves.len()` with no maximal dead root is unreachable by
            // construction (a dead leaf always has a dead-or-root ancestor chain), but the model
            // must not return `Ok` on a state it just proved dead.
            None => Ok(()),
        }
    }
}

fn walk<'a>(nodes: &mut Vec<Node<'a>>, states: &'a [StateNode], parent: Option<usize>) {
    for state in states {
        let (cat, dotted) = match parent {
            Some(p) => (
                format!("{}{}", nodes[p].cat, state.name),
                format!("{}.{}", nodes[p].dotted, state.name),
            ),
            None => (state.name.to_string(), state.name.to_string()),
        };
        let idx = nodes.len();
        nodes.push(Node { state, parent, cat, dotted });
        walk(nodes, &state.children, Some(idx));
    }
}

fn lineage(nodes: &[Node<'_>], idx: usize) -> Vec<usize> {
    let mut v = Vec::new();
    let mut cur = Some(idx);
    while let Some(i) = cur {
        v.push(i);
        cur = nodes[i].parent;
    }
    v.reverse();
    v
}

/// Flattening is name CONCATENATION, so two distinct chart positions can collapse onto one
/// generated name (`A.BC` and `AB.C` both spell `ABC`). Left to rustc it surfaces as "defined
/// multiple times" on a generated enum variant with no word about flattening.
fn check_flattened_name_collisions(nodes: &[Node<'_>]) -> syn::Result<()> {
    for i in 0..nodes.len() {
        let Some(first) = (0..i).find(|&j| nodes[j].cat == nodes[i].cat) else {
            continue;
        };
        let msg = if nodes[first].dotted == nodes[i].dotted {
            format!("duplicate state `{}` — sibling states need distinct names", nodes[i].dotted)
        } else {
            format!(
                "states `{}` and `{}` both flatten to `{}` — flattening concatenates the state path, so they would emit one name; rename one",
                nodes[first].dotted, nodes[i].dotted, nodes[i].cat
            )
        };
        let mut e = err(nodes[i].state.name.span(), msg);
        e.combine(err(
            nodes[first].state.name.span(),
            "the first state flattening to this name is here",
        ));
        return Err(e);
    }
    Ok(())
}

/// The `in_<group>` predicates key on the flattened name's snake_case COLLAPSE, and that collapse
/// is lossy: `AB` and `A_b` are two distinct variants that generate one `in_a_b`. The raw-name
/// check above cannot see it — distinct strings, one emitted method.
fn check_predicate_collisions(nodes: &[Node<'_>]) -> syn::Result<()> {
    let composites: Vec<usize> =
        (0..nodes.len()).filter(|&i| !nodes[i].state.children.is_empty()).collect();
    for (k, &i) in composites.iter().enumerate() {
        let snake_i = snake(&nodes[i].cat);
        let Some(&first) = composites[..k].iter().find(|&&j| snake(&nodes[j].cat) == snake_i)
        else {
            continue;
        };
        let mut e = err(
            nodes[i].state.name.span(),
            format!(
                "composite states `{}` and `{}` flatten to `{}` and `{}`, which both collapse to the predicate `in_{snake_i}` — rename one",
                nodes[first].dotted, nodes[i].dotted, nodes[first].cat, nodes[i].cat
            ),
        );
        e.combine(err(
            nodes[first].state.name.span(),
            "the first composite generating this predicate is here",
        ));
        return Err(e);
    }
    Ok(())
}

/// Two handlers for one event in one state — error on the SECOND `on`, note at the first.
fn check_duplicate_handlers(nodes: &[Node<'_>]) -> syn::Result<()> {
    for n in nodes {
        for (i, t) in n.state.transitions.iter().enumerate() {
            let key = path_key(&t.event);
            if let Some(first) = n.state.transitions[..i].iter().find(|p| path_key(&p.event) == key)
            {
                let mut e = err(
                    t.kw_span,
                    format!("duplicate handler for `{key}` in state `{}`", n.dotted),
                );
                e.combine(err(first.kw_span, "the first handler is here"));
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Every declared `initial` is validated, not only the ones a transition happens to reach:
/// `resolve_to_leaf` runs lazily on targets, so an unreferenced composite's typo would expand
/// silently. Reachability must not decide whether a name is checked.
fn check_declared_initials(nodes: &[Node<'_>]) -> syn::Result<()> {
    for i in 0..nodes.len() {
        let Some(init) = &nodes[i].state.initial else {
            continue;
        };
        if nodes[i].state.children.is_empty() {
            return Err(err(
                init.span(),
                format!(
                    "`{0}` has no nested states, so `initial` has nothing to name — drop it, or nest `state {init} {{ … }}` inside `{0}`",
                    nodes[i].dotted
                ),
            ));
        }
        let scope = nodes[i].dotted.clone();
        resolve_child(nodes, Some(i), init, &scope)?;
    }
    Ok(())
}

/// Per-leaf inherited routes: walk each leaf's ancestor chain innermost-first so the innermost
/// handler for an event wins, then re-sort on `decl_index`, because arbitration is declaration
/// order and the inheritance walk is not.
fn build_routes<'a>(
    nodes: &[Node<'a>],
    leaves: &[usize],
    chart_name: &str,
) -> syn::Result<Vec<Vec<Route<'a>>>> {
    let mut routes: Vec<Vec<Route<'a>>> = Vec::with_capacity(leaves.len());
    for &leaf in leaves {
        let mut seen: Vec<String> = Vec::new();
        let mut list: Vec<Route<'a>> = Vec::new();
        let mut cur = Some(leaf);
        while let Some(i) = cur {
            for t in &nodes[i].state.transitions {
                let key = path_key(&t.event);
                if !seen.contains(&key) {
                    seen.push(key);
                    let target = resolve_target(nodes, chart_name, &t.target)?;
                    list.push(Route { def: t, target });
                }
            }
            cur = nodes[i].parent;
        }
        list.sort_by_key(|r| r.def.decl_index);
        routes.push(list);
    }
    Ok(routes)
}

/// One generated fn per LEAF, named from the snake_case collapse of the flattened leaf path — and
/// that collapse is lossy (`AB` and `A_b` both spell `ab`). rustc would report the duplicate on
/// GENERATED tokens, so the check belongs where the name is minted.
///
/// Note what this check no longer needs to cover. Under one-system-per-route the generated name
/// also carried the EVENT's last path segment, so `on a::E` and `on b::E` on one leaf collapsed
/// onto one fn name and had to be refused. The route merge deletes that name component, so the
/// refusal's ground is gone with it: two distinct event paths on one leaf now become two
/// independently-indexed readers in one signature, arbitrated first-declared-wins like any other
/// pair.
fn check_generated_name_collisions(
    nodes: &[Node<'_>],
    leaves: &[usize],
    routes: &[Vec<Route<'_>>],
    chart: &Ident,
) -> syn::Result<()> {
    let mut minted: Vec<(String, usize)> = Vec::new();
    for (li, &leaf) in leaves.iter().enumerate() {
        if routes[li].is_empty() {
            continue; // a leaf with no routes emits no system, so it mints no name
        }
        let name = leaf_fn_ident(chart, &nodes[leaf].cat).to_string();
        let Some(&(_, first_leaf)) = minted.iter().find(|(n, _)| *n == name) else {
            minted.push((name, leaf));
            continue;
        };
        let mut e = err(
            nodes[leaf].state.name.span(),
            format!(
                "states `{}` and `{}` both generate the system `{name}` — generated names are the snake_case collapse of the flattened state path, and `{}` and `{}` collapse alike; rename one",
                nodes[first_leaf].dotted,
                nodes[leaf].dotted,
                nodes[first_leaf].cat,
                nodes[leaf].cat
            ),
        );
        e.combine(err(
            nodes[first_leaf].state.name.span(),
            "the first state generating this name is here",
        ));
        return Err(e);
    }
    Ok(())
}

/// A path's dedup identity for handler inheritance (token spelling, whitespace-free).
pub fn path_key(p: &syn::Path) -> String {
    p.to_token_stream().to_string().replace(' ', "")
}

/// Find `name` among the children of `parent` (or the top level), listing what IS declared and
/// suggesting a near-miss.
fn resolve_child(
    nodes: &[Node<'_>],
    parent: Option<usize>,
    name: &Ident,
    scope: &str,
) -> syn::Result<usize> {
    let candidates: Vec<usize> = (0..nodes.len()).filter(|&i| nodes[i].parent == parent).collect();
    if let Some(&found) = candidates.iter().find(|&&i| nodes[i].state.name == *name) {
        return Ok(found);
    }
    let declared: Vec<String> =
        candidates.iter().map(|&i| nodes[i].state.name.to_string()).collect();
    let refs: Vec<&str> = declared.iter().map(String::as_str).collect();
    let mut msg = format!(
        "no state `{name}` in `{scope}`; states declared here: {}",
        declared.iter().map(|d| format!("`{d}`")).collect::<Vec<_>>().join(", ")
    );
    if let Some(sugg) = did_you_mean(&name.to_string(), &refs) {
        msg.push_str(&format!(" (did you mean `{sugg}`?)"));
    }
    Err(err(name.span(), msg))
}

/// Follow `initial` chains from a (possibly composite) state down to a leaf.
fn resolve_to_leaf(nodes: &[Node<'_>], mut idx: usize, err_span: Span) -> syn::Result<usize> {
    loop {
        if nodes[idx].state.children.is_empty() {
            return Ok(idx);
        }
        let Some(init) = &nodes[idx].state.initial else {
            let first_child = (0..nodes.len())
                .find(|&i| nodes[i].parent == Some(idx))
                .map(|i| nodes[i].state.name.to_string())
                .unwrap_or_default();
            return Err(err(
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

/// Resolve a ROOT-ANCHORED target path (`Playing.Paused`) to a leaf.
fn resolve_target(nodes: &[Node<'_>], chart: &str, segments: &[Ident]) -> syn::Result<usize> {
    let mut parent: Option<usize> = None;
    let mut scope = chart.to_string();
    let mut idx = 0usize;
    for seg in segments {
        idx = resolve_child(nodes, parent, seg, &scope)?;
        scope = nodes[idx].dotted.clone();
        parent = Some(idx);
    }
    resolve_to_leaf(nodes, idx, segments.last().expect("grammar: non-empty path").span())
}

/// The generated per-leaf system's ident.
pub fn leaf_fn_ident(chart: &Ident, leaf_cat: &str) -> Ident {
    quote::format_ident!("__state_chart_{}__{}", snake(&chart.to_string()), snake(leaf_cat))
}

/// The generated initial-enter startup system's ident.
pub fn initial_enter_fn_ident(chart: &Ident) -> Ident {
    quote::format_ident!("__state_chart_{}__initial_enter", snake(&chart.to_string()))
}

/// The generated `insert_state` + startup installer's ident.
pub fn install_fn_ident(chart: &Ident) -> Ident {
    quote::format_ident!("__state_chart_install_{}", snake(&chart.to_string()))
}

/// The generated schedule-registration fn's ident.
pub fn systems_fn_ident(chart: &Ident) -> Ident {
    quote::format_ident!("__state_chart_systems_{}", snake(&chart.to_string()))
}

/// The snake_case spelling for generated names (`PlayingRunning` → `playing_running`).
///
/// A RUN of capitals is one word, not one word per letter: `UIState` → `ui_state`, `HTTPProbe` →
/// `http_probe`. The rule, applied at each uppercase char: open a new word when the previous char
/// was lowercase or a digit, or when the previous was uppercase and the NEXT is lowercase
/// (`UIState`: the `S` opens `state`).
///
/// Still lossy, deliberately: `AB` and `Ab` both collapse to `ab`. That is what
/// `check_generated_name_collisions` exists to catch, on the user's tokens, before rustc reports
/// a duplicate definition on generated ones.
pub fn snake(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() && i != 0 {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            // `!out.ends_with('_')`: a name that already spells the break (`A_B`) gets one
            // separator, not two.
            if (!prev.is_uppercase() || next_is_lower) && !out.ends_with('_') {
                out.push('_');
            }
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Merge the params of several handler bodies into one signature: dedup by NAME, refuse a name
/// reused at a DIFFERENT type on the later occurrence, and carry the earlier binding's span so
/// the reader sees which two handlers disagree.
///
/// `mut` merges by UNION — one site needing a mutable binding makes the merged binding mutable,
/// because after the merge there is one binding, not two.
pub fn merge_params<'a>(all: &[&'a Param], site: &str) -> syn::Result<Vec<MergedParam<'a>>> {
    let mut merged: Vec<MergedParam<'a>> = Vec::with_capacity(all.len());
    for &p in all {
        if let Some(prev) = merged.iter_mut().find(|m| *m.name == p.name) {
            if prev.ty.to_token_stream().to_string() != p.ty.to_token_stream().to_string() {
                let mut e = err(
                    p.name.span(),
                    format!("param `{}` is declared with conflicting types across {site}", p.name),
                );
                e.combine(err(prev.name.span(), "the first binding of this name is here"));
                return Err(e);
            }
            prev.mutable |= p.mutable;
            continue;
        }
        merged.push(MergedParam { mutable: p.mutable, name: &p.name, ty: &p.ty });
    }
    Ok(merged)
}

/// One entry of a merged signature.
pub struct MergedParam<'a> {
    pub mutable: bool,
    pub name: &'a Ident,
    pub ty: &'a syn::Type,
}

/// The nearest candidate within Levenshtein distance ≤ 2, ties to the first declared.
fn did_you_mean<'a>(found: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = levenshtein(found, c);
        if d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    best.map(|(c, _)| c)
}

/// Textbook two-row Levenshtein — the inputs are identifier-sized, so simplicity wins.
fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        core::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}
