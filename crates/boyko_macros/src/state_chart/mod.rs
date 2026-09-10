//! `state_chart!` — the engine's ONE hierarchical-state-machine codegen.
//!
//! A chart is authored as nested `state` blocks with `enter` / `exit` actions and
//! `on EVENT => TARGET` routes; it compiles to a **flat** enum of leaves, one system per leaf, and
//! `run_if(in_state(leaf))` registrations. The nesting exists only at compile time — no tree walk,
//! no parallel data structure, nothing that costs anything at run time.
//!
//! ```ignore
//! boyko_macros::state_chart! {
//!     chart GameFlow;
//!     initial Boot;
//!
//!     state Boot { on AssetsReady => Playing; }
//!     state Playing {
//!         initial Running;
//!         enter(mut cmds: Commands) { cmds.spawn(HudRoot); }
//!         exit(mut probe: ResMut<Probe>) { probe.left += 1; }
//!         state Running { on PausePressed => Playing.Paused; }
//!         state Paused  { on PausePressed => Playing.Running; }
//!         on PlayerDied(score: Res<Score>) if score.lives == 0 => GameOver;
//!     }
//!     state GameOver { on RestartPressed => Boot; }
//! }
//! ```
//!
//! Registration goes through the two generated fns:
//! `__state_chart_install_game_flow(app)` seeds the initial leaf and its entry chain, and
//! `__state_chart_systems_game_flow(b)` adds the per-leaf systems to a `ScheduleBuilder`.
//!
//! # Semantics
//!
//! * **Innermost wins.** A handler declared on a composite is inherited by every descendant leaf
//!   that does not declare its own for the same event.
//! * **LCA chains.** A transition exits the source-side states below the least common ancestor
//!   (innermost-first) and enters the target-side ones (outermost-first). Two leaves under one
//!   composite never re-enter it.
//! * **One transition per frame, and exactly one chain.** A leaf's routes are merged into one
//!   system; every lane is drained, the first-declared accepting route wins, and its chain is the
//!   only one that runs.
//! * **Unreachable states are a compile error** — see [`model::Chart::check_reachability`] for the
//!   severity rationale and the one known gap.

mod ast;
mod emit;
mod model;

use proc_macro2::TokenStream;
use syn::parse::Parser;
use syn::{Ident, Token};

use ast::ChartInput;
use model::Chart;

/// Expand one `state_chart! { … }` invocation.
///
/// On failure the expansion is the error PLUS name-resolving stubs for the two registration fns,
/// so a chart that does not compile costs one diagnostic instead of that diagnostic followed by an
/// "cannot find function" at every call site a plugin wrote.
pub fn expand(input: TokenStream) -> TokenStream {
    let name = peek_chart_name(input.clone());
    match syn::parse2::<ChartInput>(input).and_then(|c| Chart::build(&c).and_then(|m| emit::chart_items(&m)))
    {
        Ok(ts) => ts,
        Err(e) => {
            let mut out = e.to_compile_error();
            out.extend(emit::failure_stubs(name.as_ref()));
            out
        }
    }
}

/// Recover just the `chart <Name>;` head from a stream that may not parse as a whole — the stubs
/// above need the name and nothing else.
fn peek_chart_name(input: TokenStream) -> Option<Ident> {
    let parser = |s: syn::parse::ParseStream<'_>| -> syn::Result<Ident> {
        let kw: Ident = s.parse()?;
        if kw != "chart" {
            return Err(syn::Error::new(kw.span(), "not a chart head"));
        }
        let name: Ident = s.parse()?;
        s.parse::<Token![;]>()?;
        // `Parser::parse2` refuses a stream it did not fully consume, and the whole point here is
        // that the REST did not parse. Swallow the remainder so the head alone decides.
        let _rest: TokenStream = s.parse()?;
        Ok(name)
    };
    parser.parse2(input).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    /// Build a chart and return the diagnostic text, or `Ok(())` when it builds clean. Every test
    /// below drives the REAL pipeline (`parse` → `Chart::build`), so a message these tests pin is
    /// the message a user sees.
    fn build(input: TokenStream) -> Result<(), String> {
        let parsed: ChartInput = syn::parse2(input).map_err(|e| e.to_string())?;
        Chart::build(&parsed).map(|_| ()).map_err(|e| {
            e.into_iter().map(|d| d.to_string()).collect::<Vec<_>>().join(" | ")
        })
    }

    /// The whole expansion, as a normalized token string — used to pin the merge's SHAPE.
    fn expansion(input: TokenStream) -> String {
        let parsed: ChartInput = syn::parse2(input).expect("fixture parses");
        let chart = Chart::build(&parsed).expect("fixture builds");
        emit::chart_items(&chart).expect("fixture emits").to_string()
    }

    // -------------------------------------------------------------- reachability / dead states

    #[test]
    fn a_state_no_transition_targets_is_a_dead_state_error() {
        let e = build(quote! {
            chart M;
            initial A;
            state A { on E => B; }
            state B { on E => A; }
            state Orphan { on E => A; }
        })
        .expect_err("`Orphan` has no incoming edge");
        assert!(e.contains("state `Orphan` is unreachable"), "{e}");
        assert!(e.contains("not the chart's `initial` state"), "{e}");
    }

    /// A dead COMPOSITE reports once, on the composite — not once per leaf underneath it. A chart
    /// with an eight-leaf unused branch should read as one finding, not eight.
    #[test]
    fn a_dead_composite_reports_once_at_its_root() {
        let e = build(quote! {
            chart M;
            initial A;
            state A { on E => A; }
            state Dead {
                initial X;
                state X { on E => Dead.Y; }
                state Y { on E => Dead.X; }
            }
        })
        .expect_err("the whole `Dead` subtree is unreachable");
        assert!(
            e.contains("state `Dead` and everything nested in it are unreachable"),
            "{e}"
        );
        assert!(!e.contains("`Dead.X`"), "the leaves must not each get their own error: {e}");
    }

    /// Reachability follows INHERITED handlers: one root-level route into `Over` must make it
    /// reachable from every leaf that inherits it, not just from the state that declared it.
    #[test]
    fn an_inherited_handler_makes_its_target_reachable() {
        build(quote! {
            chart M;
            initial Play;
            state Play {
                initial Run;
                on Died => Over;
                state Run { on P => Play.Idle; }
                state Idle { on P => Play.Run; }
            }
            state Over {}
        })
        .expect("`Over` is reached through the inherited root handler");
    }

    /// A terminal state — reachable, with no way out — is legal and must NOT be diagnosed. The
    /// analysis is about states nothing can ENTER, not about states nothing leaves.
    #[test]
    fn a_terminal_state_is_not_a_dead_state() {
        build(quote! {
            chart M;
            initial A;
            state A { on E => Done; }
            state Done {}
        })
        .expect("a reachable terminal state is legal");
    }

    /// The initial leaf's ANCESTORS are reachable through it — a composite whose only role is to
    /// hold the initial leaf must not be reported dead.
    #[test]
    fn the_initial_leafs_ancestors_are_reachable_through_it() {
        build(quote! {
            chart M;
            initial Outer;
            state Outer {
                initial Inner;
                state Inner { on E => Outer.Inner; }
            }
        })
        .expect("the initial chain is reachable by definition");
    }

    /// Two dead subtrees produce TWO diagnostics in one build, not one-at-a-time whack-a-mole.
    #[test]
    fn every_dead_subtree_is_reported_in_one_pass() {
        let e = build(quote! {
            chart M;
            initial A;
            state A { on E => A; }
            state Ghost {}
            state Wraith {}
        })
        .expect_err("two unreachable states");
        assert!(e.contains("`Ghost`"), "{e}");
        assert!(e.contains("`Wraith`"), "{e}");
    }

    // ------------------------------------------------------------------------- the route merge

    /// The structural half of the fix: two events on ONE leaf produce ONE system, with one
    /// selection variable and one `NextState` write per arm — never two independent bodies.
    #[test]
    fn two_events_on_one_leaf_produce_one_system_with_one_selection() {
        let ts = expansion(quote! {
            chart M;
            initial A;
            state A {
                on Alpha => B;
                on Beta => C;
            }
            state B { on Back => A; }
            state C { on Back => A; }
        });
        assert_eq!(ts.matches("fn __state_chart_m__a").count(), 1, "ONE fn for leaf A: {ts}");
        assert!(ts.contains("__sc_ev_0"), "route 0 has its own reader: {ts}");
        assert!(ts.contains("__sc_ev_1"), "route 1 has its own reader: {ts}");
        assert_eq!(
            ts.matches("let mut __sc_route").count(),
            3,
            "one selection variable per leaf system (A, B, C): {ts}"
        );
    }

    /// The arbitration half: routes are emitted in DECLARATION order, and the match arms carry
    /// that order, so the first-declared accepting route is the one whose arm runs.
    #[test]
    fn routes_arbitrate_in_declaration_order_not_inheritance_order() {
        let ts = expansion(quote! {
            chart M;
            initial P;
            state P {
                initial L;
                on Outer => Q;             // declared FIRST, inherited by L
                state L { on Inner => Q; } // declared SECOND, on the leaf itself
            }
            state Q { on Back => P; }
        });
        let a = ts.find("EventReader < Outer >").expect("outer route present");
        let b = ts.find("EventReader < Inner >").expect("inner route present");
        assert!(
            a < b,
            "the inheritance walk is innermost-first, but arbitration order is DECLARATION order: {ts}"
        );
    }

    /// The `else if` cascade must not exist as two independent `if`s — a second arm running after
    /// the first is exactly the defect the merge removes.
    #[test]
    fn the_winner_is_selected_by_one_match_not_by_independent_branches() {
        let ts = expansion(quote! {
            chart M;
            initial A;
            state A { on X => B; on Y => B; }
            state B { on Z => A; }
        });
        assert_eq!(
            ts.matches("match __sc_route").count(),
            2,
            "one arbitration point per leaf system: {ts}"
        );
    }

    /// A leaf with no routes emits no system and no registration — an empty gated system would be
    /// pure expansion volume.
    #[test]
    fn a_routeless_leaf_emits_no_system() {
        let ts = expansion(quote! {
            chart M;
            initial A;
            state A { on E => Done; }
            state Done {}
        });
        assert!(!ts.contains("fn __state_chart_m__done"), "{ts}");
        assert_eq!(ts.matches("run_if").count(), 1, "only leaf A registers: {ts}");
    }

    // --------------------------------------------------------------------- ported chart faults

    #[test]
    fn the_ported_chart_faults_still_fire() {
        let dup = build(quote! {
            chart M; initial A;
            state A { on E => A; on E => A; }
        })
        .expect_err("duplicate handler");
        assert!(dup.contains("duplicate handler for `E` in state `A`"), "{dup}");

        let flat = build(quote! {
            chart M; initial A;
            state A { initial BC; state BC {} }
            state AB { initial C; state C {} }
        })
        .expect_err("flattened-name collision");
        assert!(flat.contains("both flatten to `ABC`"), "{flat}");

        let miss = build(quote! {
            chart M; initial Playing;
            state Playing { initial Runing; state Running {} state Paused {} }
        })
        .expect_err("unknown initial");
        assert!(miss.contains("did you mean `Running`?"), "{miss}");

        let composite = build(quote! {
            chart M; initial Idle;
            state Idle { on E => Playing; }
            state Playing { state Running {} }
        })
        .expect_err("composite target without initial");
        assert!(composite.contains("is a composite state with no `initial`"), "{composite}");

        // The degenerate case of the flatten-collision check reads as what it is.
        let sibling = build(quote! { chart M; initial A; state A {} state A {} })
            .expect_err("duplicate sibling");
        assert!(
            sibling.contains("duplicate state `A` — sibling states need distinct names"),
            "{sibling}"
        );

        // `initial` on a childless state can never retarget anything; ignoring it silently would
        // leave the author believing a nested chart exists.
        let leaf_initial = build(quote! { chart M; initial A; state A { initial B; } })
            .expect_err("initial on a leaf");
        assert!(
            leaf_initial.contains("`A` has no nested states, so `initial` has nothing to name"),
            "{leaf_initial}"
        );

        // Reachability must not decide whether a NAME is checked: `Lonely` is never targeted, so
        // the lazy `resolve_to_leaf` path would never look at its typo. (The name check runs
        // before the dead-state analysis, which is why this reports the typo and not the death.)
        let lonely = build(quote! {
            chart M; initial A;
            state A { on Go => A; }
            state Lonely { initial Runing; state Running {} }
        })
        .expect_err("unreferenced composite's initial is still checked");
        assert!(lonely.contains("no state `Runing` in `Lonely`"), "{lonely}");

        // The composite form of the lossy collapse lands on the PREDICATE rather than the fn.
        let pred = build(quote! {
            chart M; initial AB;
            state AB { initial X; state X {} }
            state Ab { initial Y; state Y {} }
        })
        .expect_err("two composites collapsing to one predicate");
        assert!(pred.contains("which both collapse to the predicate `in_ab`"), "{pred}");
    }

    /// The pair the pre-A7 collapse rule joined must now build cleanly. A rule change that only
    /// ever ADDS collisions would satisfy every collision assertion above while fixing nothing.
    #[test]
    fn the_pair_the_old_collapse_rule_joined_now_builds_clean() {
        build(quote! {
            chart M; initial AB;
            state AB { on E => A_b; }
            state A_b { on E => AB; }
        })
        .expect("`AB` → `ab` and `A_b` → `a_b` are distinct under the capital-run rule");
    }

    /// A handler an inner state SHADOWS is inherited by no leaf, so the per-leaf walk never
    /// resolves its target — but a target naming nothing is still a broken chart, and whether a
    /// name is checked must not depend on whether some leaf happens to reach it.
    #[test]
    fn a_shadowed_handlers_target_is_still_resolved() {
        let e = build(quote! {
            chart M; initial P0;
            state P0 {
                initial A;
                on E => Nowhere;
                state A { on E => Top; }
            }
            state Top {}
        })
        .expect_err("`Nowhere` names nothing");
        assert!(e.contains("no state `Nowhere` in `M`"), "{e}");
    }

    /// The merged-param conflict reaches the initial-enter chain too, and names ITS site rather
    /// than the leaf-routes one.
    #[test]
    fn the_initial_enter_chain_has_its_own_merged_param_site() {
        let parsed: ChartInput = syn::parse2(quote! {
            chart M; initial A;
            state A {
                initial B;
                enter (mut x: ResMut<T>) { let _ = &mut x; }
                state B { enter (x: Res<T>) { let _ = &x; } on E => A.B; }
            }
        })
        .expect("parses");
        let chart = Chart::build(&parsed).expect("builds");
        let msg = emit::chart_items(&chart).expect_err("conflicting param types").to_string();
        assert!(
            msg.contains("conflicting types across the initial state's merged `enter` chain"),
            "{msg}"
        );
    }

    /// The same conflict rule spans a leaf's EXIT handler and a route's own params — the merge
    /// fuses both into one signature, so a name reused at two types there is refused as well.
    #[test]
    fn an_exit_handler_and_a_route_param_share_one_merged_signature() {
        let parsed: ChartInput = syn::parse2(quote! {
            chart M; initial A;
            state A {
                exit (mut cmds: Commands) { let _ = &mut cmds; }
                on E (cmds: Res<Thing>) => B;
            }
            state B { on Back => A; }
        })
        .expect("parses");
        let chart = Chart::build(&parsed).expect("builds");
        let msg = emit::chart_items(&chart).expect_err("conflicting param types").to_string();
        assert!(msg.contains("conflicting types across this leaf's merged routes"), "{msg}");
    }

    /// An initial chain with no `enter` anywhere emits NO startup system — the expansion-volume
    /// rule applied to the one construct that could otherwise ship a dead empty fn per chart.
    #[test]
    fn an_enterless_initial_chain_emits_no_startup_system() {
        let ts = expansion(quote! {
            chart M;
            initial A;
            state A { initial B; state B {} }
        });
        assert!(!ts.contains("__state_chart_m__initial_enter"), "{ts}");
        assert!(!ts.contains("add_startup_system"), "{ts}");
        assert!(ts.contains("insert_state (M :: AB)"), "the seed is still emitted: {ts}");
    }

    /// The LCA bound, at the shape a token pin can see: an intra-`Field` hop re-enters the LEAF
    /// it lands on and neither of the two composites both leaves share.
    #[test]
    fn an_intra_composite_hop_replays_only_the_leafs_own_enter() {
        let ts = expansion(quote! {
            chart Sim;
            initial World;
            state World {
                initial Field;
                enter (mut cmds: Commands) { cmds.spawn(Ground); }
                state Field {
                    initial Idle;
                    enter (mut log: ResMut<Probe>) { log.field += 1; }
                    state Idle {
                        enter (mut log: ResMut<Probe>) { log.idle += 1; }
                        on Go => World.Field.Busy;
                    }
                    state Busy { on Stop => World.Field.Idle; }
                }
            }
        });
        // `Busy -Stop-> Idle`: the arm runs `Idle`'s enter once and NOTHING from `World`/`Field`.
        let busy = ts
            .split("fn __state_chart_sim__world_field_busy")
            .nth(1)
            .expect("the Busy leaf has a system");
        assert!(busy.contains("log . idle += 1"), "the target leaf's own enter runs: {busy}");
        assert!(!busy.contains("log . field += 1"), "the shared composite is NOT re-entered: {busy}");
        assert!(!busy.contains("cmds . spawn (Ground)"), "…nor the outer one: {busy}");
        // …while the startup chain runs all three, outermost-first.
        let boot = ts
            .split("fn __state_chart_sim__initial_enter")
            .nth(1)
            .expect("the initial-enter chain exists");
        let g = boot.find("cmds . spawn (Ground)").expect("outermost");
        let f = boot.find("log . field += 1").expect("middle");
        let i = boot.find("log . idle += 1").expect("leaf");
        assert!(g < f && f < i, "the chain runs outermost-first: {boot}");
    }

    /// The state-half generated-name collision survives the merge: the fn name still ends in the
    /// leaf's snake collapse, so two leaves that collapse alike still mint one name.
    #[test]
    fn two_leaves_whose_snake_collapse_matches_still_collide() {
        let e = build(quote! {
            chart M; initial AB;
            state AB { on E => Ab; }
            state Ab { on E => AB; }
        })
        .expect_err("`AB` and `Ab` both collapse to `ab`");
        assert!(e.contains("both generate the system `__state_chart_m__ab`"), "{e}");
    }

    /// The EVENT-half of that collision is gone with its ground: the generated name no longer
    /// carries an event segment, so two distinct event paths on one leaf are two indexed readers.
    #[test]
    fn two_event_paths_on_one_leaf_are_no_longer_a_name_collision() {
        build(quote! {
            chart M; initial A;
            state A { on a::E => B; on b::E => B; }
            state B { on Back => A; }
        })
        .expect("distinct event paths on one leaf are legal after the merge");
    }

    #[test]
    fn a_param_reused_at_two_types_across_one_leafs_routes_is_refused() {
        let e = build(quote! {
            chart M; initial A;
            state A { on X (p: Res<Foo>) => B; on Y (p: Res<Bar>) => B; }
            state B { on Z => A; }
        });
        // `Chart::build` does not merge params — the emitter does — so drive the emitter.
        assert!(e.is_ok(), "the model itself builds; the refusal is at emission");
        let parsed: ChartInput = syn::parse2(quote! {
            chart M; initial A;
            state A { on X (p: Res<Foo>) => B; on Y (p: Res<Bar>) => B; }
            state B { on Z => A; }
        })
        .expect("parses");
        let chart = Chart::build(&parsed).expect("builds");
        let msg = emit::chart_items(&chart).expect_err("conflicting param types").to_string();
        assert!(msg.contains("conflicting types across this leaf's merged routes"), "{msg}");
    }

    // ------------------------------------------------------------------------------ name rules

    /// `model::snake` is the THIRD implementation of the one snake_case rule — the other two are
    /// `aether_lang::expand::snake` (generated names) and `aether_lang::parse::snake_case` (the
    /// rename SUGGESTION in a diagnostic), pinned against each other by
    /// `both_snake_case_implementations_agree_on_the_same_rule` in `aether_lang/src/expand.rs`.
    ///
    /// A proc-macro crate exports nothing but macros, so this one CANNOT be compared to those two
    /// from a test in either crate. The corpus below is therefore the same ten cases that parity
    /// test uses, asserted here as an ORACLE: a divergence in any implementation reds one of the
    /// two tests, which is the property the cross-crate comparison would have given. Keep the two
    /// lists identical — the four cases this test carried before were a strict subset, so seven of
    /// the ten distinguishing cases (`GOLD`, `Gold`, `GameFlow`, `AB`, `Ab`, `A_b`, `x`) were
    /// unpinned on this side while the test's neighbours read as if the rule were covered.
    ///
    /// The rule the corpus distinguishes: a RUN of capitals is one word, not one word per letter.
    #[test]
    fn snake_treats_a_capital_run_as_one_word() {
        for (input, want) in [
            ("GOLD", "gold"),
            ("Gold", "gold"),
            ("GameFlow", "game_flow"),
            ("UIState", "ui_state"),
            ("HTTPProbe", "http_probe"),
            ("PlayingRunning", "playing_running"),
            ("A_b", "a_b"),
            ("AB", "ab"),
            ("Ab", "ab"),
            ("x", "x"),
        ] {
            assert_eq!(model::snake(input), want, "state_chart snake({input})");
        }
        // Not in the shared corpus: an ALREADY-spelled break must not double up.
        assert_eq!(model::snake("A_B"), "a_b");
    }

    /// A chart that fails still resolves the two registration names, so one bad chart is one
    /// error rather than that error plus a "cannot find function" per call site.
    #[test]
    fn a_failed_chart_still_emits_its_registration_stubs() {
        let out = expand(quote! {
            chart M; initial Nope;
            state A { on E => A; }
        })
        .to_string();
        assert!(out.contains("compile_error"), "{out}");
        assert!(out.contains("__state_chart_install_m"), "{out}");
        assert!(out.contains("__state_chart_systems_m"), "{out}");
    }
}
