//! **Four censuses over `src/animation.rs`, taken from its AST.** One pins the
//! BRANCH SET of the systems rung A1 ships; one pins the CALL SET and the ordered
//! CALL SITES; one pins the EXECUTION EDGES THAT HAVE NO SYNTAX AT THE CALL SITE —
//! drop glue and hook registration — which is what the eighth adversarial pass
//! found the first three blind to; the fourth pins the TERMINATION CONDITION those
//! systems and the authoring door agree on. None asserts a value; all assert that a
//! SET is exactly what a written-down table says it is, so a new member fails the
//! build by existing rather than by being noticed.
//!
//! **What is measured is narrower than what a reader wants, and the gap is written
//! down.** These censuses walk nine named bodies in one file, plus the drop and
//! hook edges into them. That is not the same as "every branch A1 executes", and
//! the difference is enumerated in *THE RESIDUE* below rather than left to be
//! rediscovered — which is how the previous eight passes each found their defect.
//!
//! # Why a census, and why it PARSES rather than scans
//!
//! Six consecutive adversarial passes over rung A1 found the same class of defect
//! — live production code that no gate executes, or a disclosure no gate can
//! falsify — at a NEW SITE each time. The seventh moved the defects out of the
//! fixture and into this file, and its three hardest findings were one finding:
//! **a regex-and-keyword scanner cannot see Rust.** All three were live code that
//! left the whole crate at EXIT=0:
//!
//! | evasion | why the scanner was blind |
//! |---|---|
//! | `impl UiTweenScratch { fn advance(&mut self) {…} }` + `done.advance();` | the scanner keyed calls by BARE IDENTIFIER, so `advance(…)` and `done.advance()` were the same key — one already on the list |
//! | `macro_rules! zz_never` expanding to an allocating `if` | a keyword list cannot see a branch that is not spelled with a keyword, and the call scan explicitly skipped any identifier followed by `!` |
//! | `let _ = dt_real > 1.0e30 && { done.done.push(…); true };` | `&&` is a control-flow construct with NO keyword, and the scanner's own disclosed residue said a line without a control-flow token was straight-line code |
//!
//! Each of those is an ordinary node of the syntax tree. So this file no longer
//! lexes; it calls [`syn::parse_file`] and walks. The three keys the walk uses are
//! the three the scanner could not form:
//!
//! * a call is keyed by its **resolved shape** — `TweenTint::component_id` (an
//!   `ExprCall` with a path) and `.advance` (an `ExprMethodCall`) are different
//!   keys, and neither can be spelled to look like the other;
//! * a **macro invocation is a node**, with its path, in both expression and
//!   statement position — and when its body parses as a comma-separated expression
//!   list the walk descends INTO it, so an `&&` inside a `debug_assert!` is a node
//!   too;
//! * `ExprBinary` with `&&` or `||` is a node, because a short-circuit is a path.
//!
//! # Census 1 — the branch set ([`SITES`])
//!
//! Every control-flow NODE written syntactically inside the [`WALKED`] bodies —
//! `ui_clock_tick`, `ui_visual_sink_on_add`, `ui_visual_tick`, `ui_tween_reap` and
//! the functions they call INSIDE `animation.rs` — in syntactic order, each with
//! its executable paths, each path either naming the witness in
//! `tests/ui_a1_zero_alloc.rs` that defends it, or naming a `#[test]` in another
//! binary of this crate, or stating why it is not covered. Three assertions hold
//! it up:
//!
//! 1. the walked sequence of `(function, kind, key)` equals [`SITES`] element for
//!    element — an added node, a deleted node, a MOVED node and an EDITED node all
//!    red, and the message names the index;
//! 2. every function the walked bodies call that is DEFINED IN THIS FILE is itself
//!    in the scanned set — checked by QUALIFIED name, so a new
//!    `UiTweenScratch::advance` does not hide behind the free `advance`;
//! 3. every function they call that is NOT defined in this file is enumerated in
//!    [`CALLS`] with a note on whether its INTERNAL control flow bears on the
//!    coverage claim — the row for `.set_if_neq` is why this list exists, because
//!    that verb's equality short-circuit is production-reachable, invisible to a
//!    walk of this crate, and was executed zero times by the fixture until
//!    2026-08-28.
//!
//! **"Written syntactically inside" now means what it says.** Until 2026-08-28 a
//! nested item was recorded as ONE opaque node and its contents were not walked, so
//! a `struct` plus an `impl` moved inside `ui_visual_tick` — the `impl` carrying a
//! fourth termination condition — cost two rows and hid an `if` that this sentence
//! claimed to enumerate. [`Walk::visit_item`] descends now; the walked region
//! contains zero nested items today, so the descent closed the class at no cost.
//!
//! # Census 1b — the ordered CALL SITES ([`CALL_SITES`])
//!
//! [`CALLS`] is a SET, so a SECOND call to an already-enumerated callee does not
//! red it — the sixth pass disclosed that residue in prose and the seventh walked
//! straight through it with `let _ = … && { done.done.push(…); true };`, whose two
//! callees were both already listed. So the calls are ALSO pinned as an ordered
//! sequence. A new `done.done.push(…)` anywhere in either system is a new element
//! at a new index, whatever it is nested inside and whether or not it carries a
//! branch.
//!
//! # Census 2 — the termination condition ([`ADVANCE_PIN`], [`START_PIN`])
//!
//! `invalid_tween_duration`'s doc discloses a property of the SYSTEM: *every*
//! accepted `duration_ms` above the `elapsed` saturation ceiling yields a row that
//! never completes, is never reaped, and bumps `set_if_neq` on every frame.
//! `ui_a1_tween.rs`'s gate for it samples three durations. Three points cannot
//! close an open class, so this pins the CONDITION: the printed token text of
//! `advance` and of the `tween_helpers!` body must be exactly [`ADVANCE_PIN`] and
//! [`START_PIN`]. Any added cap, clamp, sub-range rewrite or extra conjunct reds.
//! [`DISCLOSURE`] closes the other direction: deleting the disclosure while
//! leaving the predicate alone reds too.
//!
//! # What the WALK can see
//!
//! As nodes of the parsed tree:
//!
//! * `if`, `if let`, `else`, `else if`, `match` and every `Arm` (with its guard),
//!   `while`, `while let`, `for`, `loop`, `break`, `continue`, `return`, `?`,
//!   `let … else`;
//! * **`&&` and `||`** — every short-circuit, at every nesting depth. `a && b && c`
//!   is THREE nodes, because it is three paths;
//! * **every macro invocation**, in expression, statement and item position, keyed
//!   by its path. When the body parses as a comma-separated expression list the
//!   walk descends into it and the nodes inside are nodes here;
//! * **closures** — an `ExprClosure` is a node AND the walk descends into its body,
//!   so a branch inside a closure is enumerated. (Closures are `|args| body`; the
//!   `=>` the old scanner looked for is not part of one.);
//! * **calls, by resolved shape** — `ExprCall` keyed by its full path, so
//!   `TweenTint::component_id` and `TweenScale::component_id` are distinct;
//!   `ExprMethodCall` keyed `.method`, with any turbofish dropped;
//! * **a nested item** — a `fn`, `impl` or `mod` written INSIDE a walked body is a
//!   node, AND the walk descends into it. The descent arrived with the tenth
//!   adversarial pass: recording the item and returning left everything inside it
//!   unwalked, and the comment that used to excuse that — *"its contents are
//!   censused by the definition table and the closure assertion"* — was
//!   call-gated, so an operator `impl` moved inside a walked body reddened exactly
//!   one test and was then bought off with two opaque rows. See [`Walk::visit_item`];
//! * **operator expressions** — every `Expr::Index`, `Expr::Unary`, `Expr::Assign`
//!   and every `Expr::Binary` other than `&&` / `||`, in syntactic order. Not
//!   control flow, so they are their own list: [`OPERATOR_SITES`].
//!
//! # Execution edges with NO SYNTAX AT THE CALL SITE (censuses 1c, 1d)
//!
//! The first three censuses all key on **syntax at a call site**, and the eighth
//! adversarial pass found the class that has none — then the tenth found a third
//! member of it, and the eleventh a fourth. Every one of these was live in the
//! shipped tree and left the whole crate green:
//!
//! | edge | why every census was blind | closed by |
//! |---|---|---|
//! | `impl Drop for ZzDropCap` + `let _zz = ZzDropCap { … };` in the opacity arm — a FOURTH termination condition, same threshold as the `macro_rules!` form the census DOES catch | drop glue is called by no syntax. `Expr::Struct` matched no branch arm and no call arm; `walk_region` visits ten bodies, so a top-level `impl` is never reached; `collect_defs` records the method as bare `drop`, and nothing calls `.drop()` | [`every_drop_impl_on_a_type_the_walked_region_constructs_is_walked`] |
//! | `ui_visual_sink_on_add`, registered at `components.rs` on every `Tween*` channel and already carrying an unenumerated `if` | the KERNEL calls it, from an attribute one module over, written INSIDE `macro_rules! tween_channel` where a parsed tree has no attribute at all | [`every_hook_registered_on_the_walked_file_is_walked`] |
//! | **an OVERLOADED OPERATOR** — `impl std::ops::Mul<(&mut f32, f32)> for ZzMulCap`, again a fourth termination condition, dispatched from the opacity arm; and `impl std::ops::Index<usize> for ZzIdxCap`, a structurally different node, same result | `Expr::Index`, `Expr::Unary`, `Expr::Assign` and non-logical `Expr::Binary` matched NO arm, so the body was neither a branch, nor a call, nor a callee, nor a call site — and with zero `impl Drop` in the crate the drop scan did not fire either. **It cost ZERO rows**, where a cross-module callee costs two | [`every_operator_expression_of_the_walked_region_is_pinned`] and [`every_operator_impl_in_the_crate_is_dispositioned`] |
//! | **DESUGARING** — `impl Iterator for ZzIter` carrying a fourth termination condition (`*elapsed` capped at `3_600.0`), driven by `for _zz in (ZzIter { … }) {}` in the opacity arm | `for x in it` IS a walked node, but `Iterator::next` is reached from no syntax at all: write the one `SITES` row the `for` costs and the body behind it is in NO census. MEASURED 2026-08-28 on the shipped tree: **EXIT=0, 53 targets, 354 passed** with the cap live; dropping it to `0.001` reddened four behavioural tests, so the body demonstrably ran. Same shape reaches `From` through `?`, `Display`/`Debug` through a format macro, `fmt::Write` through `write!` | [`every_desugared_trait_impl_in_the_crate_is_dispositioned`] |
//!
//! All widen [`WALKED`] or add a list rather than widen a claim. The drop scan, the
//! operator-`impl` scan and the desugaring scan read every `.rs` under `src/` —
//! through ONE function, [`trait_impls_in_crate`], so widening either trait list is
//! a list edit and never a second scanner to keep in step. The hook scan reads the
//! same corpus as TOKENS, and also asserts this crate registers no observers (an
//! observer runner is reached by registration too, and today there are zero).
//!
//! **The operator-`impl` scan found three on its first run, where the pass that
//! prescribed it predicted zero** — and one of them, `impl PartialEq for UiVisual`,
//! is the body `sink.set_if_neq(composed)` short-circuits on. See
//! [`OPERATOR_IMPLS`].
//!
//! # THIS CLASS IS A LIST, NOT A CLOSURE
//!
//! Four members, found across four adversarial passes — the eighth found two, the
//! tenth one, the eleventh one — and **every one was found by a PROBE, never by
//! construction.** Nothing here derives the class from the language; each row is a
//! thing somebody smuggled past a green suite and then wrote down. The costs say
//! the same: drop glue and operator dispatch each cost **ZERO rows**, hook
//! registration cost a `WALKED` entry, and desugaring cost **ZERO rows for the
//! body** (its `for` node costs one `SITES` row, and buying that one row off is all
//! it takes to hide the `next` behind it).
//!
//! So the honest description of this instrument is: **the censuses above are
//! closed; this table is open.** A fifth member is not excluded by anything written
//! here, and the way it will be found is the way the first four were — a probe,
//! reported. Read a green run of this file as *"none of the four known no-syntax
//! edges is unaccounted for"*, never as *"there is no no-syntax edge"*.
//!
//! # THE RESIDUE — what the instrument does not measure
//!
//! Stating this is not politeness. A scanner that silently skips a construct is
//! this campaign's signature defect, and the previous spelling of this section was
//! itself wrong three times: it claimed `=>` covered "a match arm — or a closure"
//! (Rust closures carry no `=>`); it declared a residue that a `&&` walked straight
//! through; and it ended with *"this is now the WHOLE residue"* — a sentence the
//! two rows of the table above falsify. It is a list now, not a claim of closure.
//!
//! Each item names its reason and, where one was taken, its measurement.
//!
//! 1. **Branches inside a callee in another module or crate.** `Mut::set_if_neq`'s
//!    equality test and `Vec::push`'s reallocation are real paths of the armed
//!    window. [`CALLS`] enumerates the callees, which makes their branches
//!    *enumerable* — not *visible*. MEASURED 2026-08-28: `pub(crate) fn zz_cap` in
//!    `components.rs`, called from the opacity arm, reds
//!    [`every_callee_of_the_walked_region_is_enumerated`] and
//!    [`every_call_site_of_the_walked_region_is_pinned`] — and adding the two rows
//!    those failures ask for gives EXIT=0 **with the cap still live**, because the
//!    `if` inside `zz_cap` is invisible and neither `Call::local` nor `Call::note`
//!    is checked against its body. The answer to a new cross-module callee is a
//!    place to write a sentence.
//! 2. **Trait impls selected by type.** `impl_type_name` reads the `Self` path's
//!    last segment; nothing here resolves a receiver's type or a blanket impl. The
//!    drop scan is deliberately over-approximate in the fail-closed direction
//!    (`T::assoc(…)` counts `T` as constructed) precisely because it cannot
//!    resolve.
//! 3. **Macro EXPANSIONS.** The invocation is always a node, and the walk descends
//!    when the body parses as a comma-separated expression list — but the branches
//!    a `macro_rules!` body *expands to* are not nodes here.
//!    [`no_macro_in_the_walked_region_is_opaque`] makes an unparseable body a RED
//!    rather than a silent skip, which is the fail-closed half; it is not the same
//!    as seeing inside one.
//! 4. **`#[cfg]`-dead code is COUNTED.** All censuses read ONE text, so a branch
//!    under `#[cfg(feature = "never_enabled")]` reds this census and demands a
//!    [`SITES`] row with a coverage claim — for code that compiles in NO profile.
//!    Fail-closed for hiding, which is the right direction, at the price of a row
//!    whose `why` cannot be falsified. `debug_assert!` is the same asymmetry in its
//!    mild form: a node here, absent from the release binary
//!    `ui_a1_zero_alloc.rs` measures, and recorded on the row itself.
//! 5. **A `Drop` impl the scan cannot reach** — one in another crate (`Vec`'s; see
//!    item 1), one written inside a `macro_rules!` body (item 3), or one on a type
//!    the region obtains from a function whose name says nothing about what it
//!    returns (`let g = make_guard();`). The scan sees `Expr::Struct` and
//!    `T::assoc(…)`, and nothing else.
//! 6. **A coverage row pins an OBSERVABLE, not an EXECUTION COUNT — and for eight
//!    rows of twelve it does not pin the execution either.** [`Cover::By`] asserts
//!    its witness is REACHABLE. It cannot assert the path still executes, and the
//!    reachability itself is mechanical only so far: a witness reaches the graph
//!    through a call, or through a [`FN_VALUES`] row that pins it as an ARGUMENT at
//!    a named index of a named callee. Where that callee is a fixture function, the
//!    census checks the callee CALLS its parameter — four rows today. Where it is a
//!    kernel verb (`add_system`, `run_system`), `boyko_ecs` is not parsed here and
//!    the edge rests on the row's prose — eight rows today, and the count is
//!    printed. **This item used to assert the weaker half held absolutely, and it
//!    did not:** the tenth adversarial pass replaced a witness call with
//!    `let _ = (witness, …)` and bought "defends 7 path(s)" back with ONE row, at
//!    census EXIT=0 and crate 352 passed. `via` and `arg` closed that specific
//!    purchase; the kernel-verb half is a limit, not a closure. It cannot assert the
//!    path still executes either. MEASURED
//!    2026-08-28: `lerp_rgba8`'s `let mut shift = 0;` → `= 32;` drives the `while`
//!    body to zero iterations while the node key `shift < 32` does not move — a
//!    `let` is not a node — and both this census and the allocation gate stay
//!    green, because the tint_only cohort's composed `tint_mul` is a constant `0`
//!    either way. One unrelated test elsewhere in the crate does catch that
//!    particular neuter, so it is a coverage-column defect and not an escape. The
//!    exposure is concentrated: **`witness_steady_cohort_keeps_every_channel_live`
//!    is the sole defence of SEVENTEEN paths and observes four liveness counts** —
//!    every `Some(row) = …` taken arm, every `Some(t)` arm, all four `match
//!    advance(…)` discriminants, `advance`'s `t < 1.0` running side and its
//!    `debug_assert!`, the tick's `for` body, and two paths of the all-`None`
//!    conjunction. This test prints the tally for every witness, so the shape of
//!    that exposure is output rather than lore.
//! 7. **[`Cover::Elsewhere`] is weaker than [`Cover::By`].** It asserts the named
//!    `#[test]` exists in the named file; it does not assert that gate would fail
//!    if the path stopped executing. Both rows that use it record a measurement of
//!    exactly that, taken by hand — which is the difference between a checked claim
//!    and an automated one.
//! 8. **The PATH decomposition of a node.** The walk sees that `if let Some(row) =
//!    tint` is one node; that it has a taken and a not-taken path is authored in
//!    [`Site::paths`]. What is mechanical is that no control-flow node may exist
//!    without a row, and no row without a node.
//! 9. **Straight-line code that is neither a call nor an operator.** The reason
//!    this item used to give was *"it introduces no path and no callee"*, and an
//!    OVERLOADED OPERATOR refuted it: `a * b` introduces both while looking like
//!    straight-line code at the site, and it cost zero rows in every census here
//!    until [`OPERATOR_SITES`] landed. So the line is drawn differently now. A
//!    straight-line CALL is pinned by [`CALL_SITES`]; a straight-line OPERATOR — the
//!    `*elapsed += dt` this item used to cite, and every `[i]`, `*x`, `=` and
//!    non-logical binary — is pinned by [`OPERATOR_SITES`]. What is left invisible
//!    is a statement that is neither: a field read, a `let` of a plain path, a
//!    method-free reborrow. Those introduce no execution edge of any kind, which is
//!    now a claim about the CONSTRUCTS rather than about how the line looks.
//! 10. **`SITES` order.** The comparison is positional, so two structurally
//!     identical nodes (the four `match advance(…)`) are told apart ONLY by
//!     position. Swapping two channels' blocks reds this census, which is correct —
//!     their coverage rows differ by frame index — but the message will say
//!     "edited", not "reordered".
//! 11. **String CONTENT.** Every string literal is emptied to `""` before
//!     comparison, so a message that has become FALSE does not red these. That is a
//!     real limit and it has already bitten once — see the note on [`ADVANCE_PIN`].
//! 12. **`macro_rules!` bodies, as trees.** A `macro_rules!` body is a token tree
//!     with `$` metavariables and is not parseable Rust until expansion.
//!     [`START_PIN`] pins it as printed TOKENS — whitespace-, comment- and
//!     line-ending-immune, which raw bytes would not be — and not as a tree. The
//!     hook scan reads the same bodies as tokens for the same reason.
//! 13. **Anchor currency outside this crate.** The repository's `GATED_DOCS`
//!     anchor gate covers four documents and checks **735 anchors, 0 stale**
//!     (`ARCHITECTURE.md` 6 + `FEATURE_MAP.md` 222 +
//!     `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` 177 + `SYSTEMS.md` 330 — MEASURED
//!     2026-08-28). The number of line-numbered citations from any of them into any
//!     of this landing's ten moved source files is **zero**, ten times zero, also
//!     measured. The gate is not vacuous: swapping two live line numbers in
//!     `SYSTEMS.md` gives EXIT=101, "330 anchor(s) checked, 2 stale" — re-measured
//!     for the record 2026-08-28, then restored and `cmp`-proved. It is simply
//!     about OTHER FILES; its green says nothing about the files this landing moved.
//! 14. **A no-syntax `impl` that is in ANOTHER CRATE.** [`OPERATOR_SITES`] pins
//!     every operator EXPRESSION of the walked region, so a new or edited one reds
//!     wherever its body lives; [`OPERATOR_IMPLS`] pins every user operator `impl`
//!     in `boyko_ui/src`, so a new body reds even when no expression moved. Neither
//!     sees an `impl` in another crate whose type flows into an operator expression
//!     whose TOKENS do not change — `lerp1(from: f32, …)` becoming
//!     `lerp1(from: Px, …)` leaves `from + (to - from) * t` printed identically
//!     while `+` starts calling `Px`'s `Add`. That is residue item 1's class
//!     reached through an operator instead of through a call, and it has the same
//!     answer: [`CALLS`] is where a cross-crate body gets a sentence.
//!
//!     **[`DESUGARED_IMPLS`] has the SAME crate scope and therefore the same
//!     edge, and there the cross-crate population is not hypothetical.**
//!     [`trait_impls_in_crate`] reads `CARGO_MANIFEST_DIR/src`, so its
//!     *"1 user impl over 8 desugared traits"* is a statement about `boyko_ui/src`
//!     and nothing else. MEASURED 2026-08-28: the walked region drives **two**
//!     `Iterator` bodies that live elsewhere —
//!     `for (entity, …) in q.iter_entities_mut()` (`boyko_ecs`'s query iterator)
//!     and `for &(entity, component_id) in &done` (`core::slice::Iter`, through
//!     `impl IntoIterator for &Vec<T>`). Both are reached today, on every armed
//!     frame, by no syntax at all; both are outside every scan in this file. Their
//!     `for` nodes have `SITES` rows and their bodies have nothing, which is
//!     exactly the escape the `ZzIter` probe demonstrated — the difference is only
//!     that these two are std/kernel code rather than something a future edit
//!     writes.
//!
//!     **MEASURED 2026-08-28, and NOT the number that was predicted.** The pass
//!     that prescribed this scan asserted `boyko_ui/src` held zero user operator
//!     `impl`s. It holds **three** — `PartialEq for UiTextBuffer`,
//!     `PartialEq for UiVisual`, `PartialOrd for UiName` — and the middle one is
//!     reachable from the walked region's last line through `set_if_neq`, carrying
//!     five `&&` and four `Index` operations that no census in this file walks.
//!
//!     **RE-MEASURED 2026-08-28: this item's own guarantee was FALSE, and it is
//!     WITHDRAWN.** The sentence that stood here said the cross-crate population
//!     reached by the walked region was *"zero measured, because the region's
//!     operands are `f32`, `u32`, `u8` and `[f32; 2]`, all primitives — which is
//!     what makes a first non-primitive one red [`OPERATOR_SITES`]"*. The operand
//!     list is wrong and the population is **at least three**, all three reached on
//!     every armed frame:
//!
//!     | site (`src/animation.rs`) | operand type | `impl` reached | operator syntax at the site? |
//!     |---|---|---|---|
//!     | `let mut composed = *sink;` (`:720`) | `Mut<'w, UiVisual>` — the query item type, `boyko_ecs` `query/data/mut_.rs:211` | `impl<'w, T: Component> std::ops::Deref for Mut<'w, T>`, `mut_.rs:123` | YES — the `("ui_visual_tick", "unary", "* sink")` row of [`OPERATOR_SITES`] |
//!     | `clock.dt_real()` / `clock.dt_virtual()` (`:709`, `:710`) | `Res<'_, UiClock>` | `impl<R: Resource> Deref for Res<'_, R>`, `boyko_ecs` `system/params/res.rs:42` | **NO** — method resolution auto-derefs; there is no expression to pin |
//!     | `done.done.push(…)` (four arms) | `ResMut<'_, UiTweenScratch>` | `impl<R: Resource> DerefMut for ResMut<'_, R>`, `system/params/resmut.rs:53` | **NO** — field access auto-derefs; likewise no expression |
//!
//!     Two of the three carry no operator syntax at all, so no widening of
//!     [`OPERATOR_SITES`] could ever have listed them — the withdrawn sentence was
//!     not merely off by a count, it named the wrong mechanism. And the third
//!     refutes the guarantee directly: `* sink` is a non-primitive operand with a
//!     cross-crate `impl`, reached today, and it did NOT red anything, because it
//!     has had a row since [`OPERATOR_SITES`] landed.
//!
//!     **What still holds, stated as the property and not as a promise:**
//!     [`OPERATOR_SITES`] reds when the printed TOKENS of any operator expression
//!     of the walked region change — that and nothing more. It does not red when an
//!     operand's type changes under unchanged tokens, and it cannot see an
//!     auto-deref, which has no tokens of its own. The auto-deref sites are
//!     nevertheless all enumerated, one census over: `.dt_real`, `.dt_virtual` and
//!     `.push` each carry a [`CALLS`] row and a [`CALL_SITES`] row, so the
//!     RECEIVERS whose `Deref` is at stake are on a list even though the `Deref` is
//!     not. That is the checked property. It is a disclosure of where the bodies
//!     live, not a guarantee that a new one reds.
//! 15. **The observer scan's SCOPE is this crate only.** [`observer_registrations`]
//!     reads `CARGO_MANIFEST_DIR/src`, so its *"0 observer registrations"* is a
//!     statement about `boyko_ui/src` and nothing else. An observer registered from
//!     a SIBLING CRATE onto a function of `animation.rs` would leave that assertion
//!     true and irrelevant, and the runner would be reached exactly as
//!     `ui_visual_sink_on_add` is — by registration, never by a call site. The hook
//!     scan reads the same corpus and has the same edge, though it covers all four
//!     hook kinds within it.
//!
//!     **MEASURED 2026-08-28: the population is zero, and that is why this is
//!     latent rather than live.** No crate in the workspace outside `boyko_ecs`
//!     calls any of [`OBSERVER_VERBS`] at all, and every cross-crate
//!     `#[component(on_*)]` attribute in the tree sits in `boyko_ecs`'s own test
//!     fixtures or `aether_lang`'s expander tests — none names a `boyko_ui` path. A
//!     zero population is what makes a first instance red somewhere; it is also what
//!     makes this scan unable to see that instance if it is written next door.

// A census over source text: it reads two files from `CARGO_MANIFEST_DIR` and
// asserts over their contents. Nothing here is engine code, and the whole file is
// compiled out of every shipping build.
#![cfg(not(miri))]

use std::collections::BTreeSet;
use std::path::PathBuf;

use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{Attribute, Block, Expr, File, ImplItem, Item, ItemFn, Signature, Stmt, Token};

// ───────────────────────── the walked set ──────────────────────────────────

/// The functions this census walks: rung A0's `ui_clock_tick`, rung A1's
/// `ui_visual_tick` and `ui_tween_reap`, and the transitive closure of the
/// functions they call INSIDE `animation.rs`.
///
/// Names are QUALIFIED — `UiClock::dt_real`, not `dt_real`. That is not
/// decoration: the seventh adversarial pass hid a whole allocating function from
/// the previous census by naming it `advance` inside an `impl`, where a bare-name
/// key found the free `advance` already in scope and reported the set closed.
/// [`the_walked_set_is_closed_under_intra_file_calls`] compares qualified names,
/// so both would have to be listed and both would need rows.
///
/// The closure is not taken on faith — that same test recomputes it from the
/// walked bodies and reds if a locally-defined function is called from inside the
/// region and is not listed here.
///
/// Each entry must resolve to EXACTLY ONE definition in the parsed file
/// ([`every_walked_name_resolves_to_exactly_one_definition`]). The previous
/// spelling took spans by finding an anchor SUBSTRING and matching braces; a
/// parsed file has no such ambiguity to guard against.
const WALKED: &[&str] = &[
    "UiClock::dt_real",
    "UiClock::dt_virtual",
    "ui_clock_tick",
    "ui_visual_sink_on_add",
    "advance",
    "ease",
    "lerp1",
    "lerp_rgba8",
    "ui_visual_tick",
    "ui_tween_reap",
];

// ───────────────────────── the coverage table ──────────────────────────────

/// Whether the armed window of `tests/ui_a1_zero_alloc.rs` executes a path — and,
/// when it does, WHAT IN THAT FILE FAILS if it stops.
#[derive(Debug)]
enum Cover {
    /// Driven inside the armed window, and defended by an EXECUTABLE witness.
    ///
    /// The previous spelling was `By(&str)` asserting `fixture.contains(driver)`,
    /// i.e. a substring search. The seventh adversarial pass neutered the
    /// virtual-clock cohort by changing ONE ARGUMENT of its starter call
    /// (`TWEEN_FLAG_VIRTUAL_CLOCK` → `0`) while leaving the `let virtual_clock:
    /// Vec<Entity>` binding — the exact `Cover::By` text — untouched. A/B on the
    /// same tree: cohort armed ⇒ the allocation gate RED, cohort neutered ⇒ GREEN
    /// 3/3, this census EXIT=0 in both. The column kept reporting the lane covered
    /// in a tree where it demonstrably was not.
    ///
    /// So the link is two claims, neither of them a substring search:
    ///
    /// * `cohort` — a NAME BOUND in the fixture (a `let` binding, or a `fn` /
    ///   `const` / `static` item), found in the fixture's own AST. A renamed or
    ///   deleted cohort reds.
    /// * `witness` — a `fn` in the fixture that ASSERTS the path is being driven,
    ///   and that is REACHABLE from a `#[test]` function by the fixture's own call
    ///   graph. Deleting the witness reds; deleting its CALL reds; replacing the
    ///   call with a bare mention reds. Neutering the cohort then reds the witness,
    ///   at runtime, in the binary that measures.
    ///
    ///   **Where that stops being mechanical is written down rather than rounded
    ///   off.** A witness reachable only because it is handed to a KERNEL verb —
    ///   `add_system`, `run_system` — rests on the [`FnValue`] row's prose, because
    ///   `boyko_ecs` is not parsed here; today eight of the twelve rows are in that
    ///   position and [`every_function_value_reference_in_the_fixture_is_pinned`]
    ///   prints the split. See [`reachable_from_tests`] for the three strengths and
    ///   [`FN_VALUES`] for the laundering that made this paragraph necessary.
    By { cohort: &'static str, witness: &'static str },
    /// Not executed inside the armed window, but driven and ASSERTED by a named
    /// `#[test]` in another test binary of this crate.
    ///
    /// Added by the eighth adversarial pass, for the `on_add` hook. A hook fires at
    /// SEED time — the allocation fixture's armed window contains no component
    /// insert at all — so neither of its paths can ever be a [`Cover::By`], and
    /// spelling them [`Cover::Not`] would file two live production branches, one of
    /// which is the whole defence against AD12's "the panel that slid in jumps
    /// home", under "nobody looks". This is the third answer: *somebody looks, and
    /// here is who*. The named test must exist and carry `#[test]` in the named
    /// file, so a rename or a deletion reds.
    ///
    /// It is deliberately WEAKER than [`Cover::By`] and says so: it pins a gate's
    /// existence, not that the gate would fail if the path stopped executing. Where
    /// that stronger claim has been measured, the row's `why` records the
    /// measurement.
    Elsewhere { file: &'static str, test: &'static str },
    /// Not executed inside the armed window. The reason is the whole content of
    /// the row: "we did not get to it" and "it cannot be reached" are different
    /// claims and must not be spelled the same way.
    Not,
}

/// One executable path of one control-flow node.
#[derive(Debug)]
struct Path {
    /// The path, in the language of the source.
    what: &'static str,
    /// Covered by whom, or not covered.
    cover: Cover,
    /// Covered: on which armed frames, and with what signal. Not covered: why,
    /// and whether the path is reachable in production at all.
    why: &'static str,
}

/// One control-flow node of the walked region, with every path it introduces.
#[derive(Debug)]
struct Site {
    /// The [`WALKED`] entry it belongs to.
    func: &'static str,
    /// The node's kind, as [`Walk`] classifies it.
    kind: &'static str,
    /// The node's key: its discriminating tokens, printed, whitespace-collapsed,
    /// with every string literal emptied to `""`.
    key: &'static str,
    /// Its executable paths. AUTHORED — the walk sees nodes, not paths.
    paths: &'static [Path],
}

/// Every control-flow node of the walked region, in syntactic order.
///
/// This is the coverage table that used to be prose in `ui_a1_zero_alloc.rs`'s
/// gate doc. It is data now, and it is compared against a walk of the tree.
const SITES: &[Site] = &[
    // ── ui_clock_tick ──────────────────────────────────────────────────────
    //
    // Rung A0's third member of `UiAnimationSet`. It was in NEITHER census until
    // 2026-08-28 — `grep -n ui_clock_tick tests/ui_a1_source_census.rs` returned
    // zero hits — while running on every armed frame and carrying a
    // `debug_assert!` of exactly the shape enumerated for `advance`. It is in
    // BOTH arms of the A1 gate's differential fixture, so its cost cancels BY
    // CONSTRUCTION there and no widening of that fixture could ever cover it. It
    // has a single-arm gate of its own now
    // (`ui_clock_tick_allocates_zero_over_a_same_shape_baseline`), whose baseline
    // is `noop_clock` — the same `SystemParam` signature with an empty body.
    Site {
        func: "ui_clock_tick",
        kind: "macro",
        key: "debug_assert!",
        paths: &[
            Path {
                what: "the invariant holds",
                cover: Cover::By {
                    cohort: "build_clock_pair",
                    witness: "ui_clock_tick_allocates_zero_over_a_same_shape_baseline",
                },
                why: "every armed frame of the clock gate, in a DEBUG build. The release binary \
                      that gate measures has no such statement at all, so this node's cost is \
                      structurally zero there and its coverage is a debug-profile claim only",
            },
            Path {
                what: "the invariant fails",
                cover: Cover::Not,
                why: "not covered and deliberately not coverable: `max_delta` is finite and \
                      positive by construction (`UiClock::default`) and by validation \
                      (`UiClock::set_max_delta`, which panics through \
                      `invalid_ui_max_delta_panic`). A fixture that violated it would be \
                      asserting a panic rather than an allocation",
            },
        ],
    },
    Site {
        func: "ui_clock_tick",
        kind: "&&",
        key: "max . is_finite () && max > 0.0",
        paths: &[
            Path {
                what: "left true — the right operand is evaluated",
                cover: Cover::By {
                    cohort: "build_clock_pair",
                    witness: "ui_clock_tick_allocates_zero_over_a_same_shape_baseline",
                },
                why: "every armed frame of the clock gate, in a DEBUG build only — this node is \
                      INSIDE the `debug_assert!` above and does not exist in the release binary. \
                      A short-circuit is a path, and this is the first census over this crate that \
                      can see one at all: MEASURED 2026-08-28, `let _ = dt_real > 1.0e30 && \
                      { done.done.push(…); true };` was invisible to the keyword scanner that \
                      preceded this walk",
            },
            Path {
                what: "left false — `max > 0.0` is never evaluated",
                cover: Cover::Not,
                why: "not covered, for the same reason as the enclosing assertion's failing path: \
                      reaching it requires a non-finite `max_delta`, which the setter refuses and \
                      the default cannot produce",
            },
        ],
    },
    // ── ui_visual_sink_on_add ──────────────────────────────────────────────
    //
    // The `on_add` hook every `Tween*` channel carries. It was in NO census until
    // 2026-08-28 — `grep -n ui_visual_sink_on_add tests/ui_a1_source_census.rs`
    // returned zero hits: no WALKED entry, no SITES row, no CALLS row — while
    // running on every `Tween*` insert in the crate and already carrying the live,
    // unenumerated `if` below. It is reached ONLY by registration
    // (`#[component(… on_add = crate::animation::ui_visual_sink_on_add)]`, written
    // inside `macro_rules! tween_channel` in components.rs), so no call graph over
    // this file can find it and no walk of the three systems visits it.
    //
    // Both of its paths are `Cover::Elsewhere`, and that is structural rather than
    // a gap in the fixture: a hook fires on a component INSERT, the allocation
    // gate's armed window contains none, and no widening of that window could
    // change it — the same shape as `ui_clock_tick`'s cancellation, one rung over.
    Site {
        func: "ui_visual_sink_on_add",
        kind: "if",
        key: "world . get_component :: < UiVisual > (ctx . entity) . is_some ()",
        paths: &[
            Path {
                what: "a sink is already there — the hook does nothing",
                cover: Cover::Elsewhere {
                    file: "ui_a1_tween.rs",
                    test: "the_tick_composes_from_the_sink",
                },
                why: "that gate finishes a TweenOffset at -400 px, lets the reap remove it, then \
                      starts a TweenTint on the SAME node — a NEW add on an entity that already \
                      carries a sink, which is the only way to reach this path. It then asserts \
                      offset_px[0] is still -400 on three further frames. MEASURED 2026-08-28 by \
                      deleting this guard: the hook enqueues UiVisual::IDENTITY over the finished \
                      sink and that gate reds at ui_a1_tween.rs:615 with left 0.0, right -400.0 — \
                      'the finished offset is CARRIED, not reset'. So this row is a gate whose \
                      FAILURE is measured, not merely a gate that exists: it is AD12's 'the panel \
                      that slid in jumps home', arriving through the back door",
            },
            Path {
                what: "no sink yet — the identity insert is enqueued",
                cover: Cover::Elsewhere {
                    file: "ui_a1_tween.rs",
                    test: "presence_is_running_and_the_reap_ends_it",
                },
                why: "that gate starts a TweenTint on a bare node and asserts `sink_of(&world, e) \
                      == UiVisual::IDENTITY` immediately afterwards — 'a channel whose sink is \
                      missing is a tween that ticks into nothing'. MEASURED 2026-08-28 by deleting \
                      the insert: it reds at ui_a1_tween.rs:196, 'the node carries a UiVisual \
                      sink'. It is also the path every row of the allocation fixture's five \
                      cohorts takes on the FIRST iteration of seeded_world's warm burst, at seed \
                      time and outside the armed window",
            },
        ],
    },
    Site {
        func: "ui_visual_sink_on_add",
        kind: "return",
        key: "",
        paths: &[Path {
            what: "taken",
            cover: Cover::Elsewhere {
                file: "ui_a1_tween.rs",
                test: "the_tick_composes_from_the_sink",
            },
            why: "the same edge as the `if`'s taken path and by the same measurement — deleting \
                  the early return is deleting the guard. It is enumerated separately because the \
                  walk sees a `return` as its own node, and because a body that fell THROUGH here \
                  would stomp the sink while the `if` still read as present",
        }],
    },
    // ── advance ────────────────────────────────────────────────────────────
    Site {
        func: "advance",
        kind: "macro",
        key: "debug_assert!",
        paths: &[
            Path {
                what: "the invariant holds",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every row of every cohort, every frame — in a DEBUG build. The release \
                      binary the allocation gate measures has no such statement at all, so this \
                      node's cost is structurally zero there and its coverage is a debug-profile \
                      claim only",
            },
            Path {
                what: "the invariant fails",
                cover: Cover::Not,
                why: "not covered and deliberately not coverable: it is an invariant, and a \
                      fixture that violated it would be asserting a panic rather than an \
                      allocation. `elapsed` is initialized to 0.0 by the door and only ever has a \
                      clamped, non-negative UiClock delta added to it",
            },
        ],
    },
    Site {
        func: "advance",
        kind: "if/else",
        key: "flags & TWEEN_FLAG_VIRTUAL_CLOCK != 0",
        paths: &[
            Path {
                what: "virtual lane — bit 0 set",
                cover: Cover::By {
                    cohort: "virtual_clock",
                    witness: "witness_virtual_lane_reads_the_other_delta",
                },
                why: "every armed frame, 4 channels x NODES rows. Before that cohort existed \
                      every fixture row passed flags = 0 and this lane had ZERO executions in the \
                      window — MEASURED 2026-08-28, a with_capacity(64) here was GREEN 10/10; with \
                      the cohort it is RED 10/10, signal +128 on every index. The witness is what \
                      makes the cohort's presence FALSIFIABLE rather than textual: the fixture \
                      runs at relative_speed 0.5, so the two lanes carry DIFFERENT deltas, and \
                      this cohort's elapsed must diverge from the steady cohort's. Passing 0 for \
                      its flags — the seventh pass's neuter, one argument — makes them equal and \
                      reds the witness",
            },
            Path {
                what: "real lane — bit 0 clear",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_virtual_lane_reads_the_other_delta",
                },
                why: "every armed frame; the steady, completing and tint_only cohorts all pass \
                      flags = 0. The same witness proves it, from the other side of the same \
                      divergence",
            },
        ],
    },
    Site {
        func: "advance",
        kind: "if/else",
        key: "t < 1.0",
        paths: &[
            Path {
                what: "still running",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame — STEADY_MS is 600 s and nothing in this window \
                      approaches it",
            },
            Path {
                what: "completes",
                cover: Cover::By {
                    cohort: "completing",
                    witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
                },
                why: "armed frame i for channels()[i], i in 0..=3, NODES rows each",
            },
        ],
    },
    // ── lerp_rgba8 ─────────────────────────────────────────────────────────
    Site {
        func: "lerp_rgba8",
        kind: "while",
        key: "shift < 32",
        paths: &[
            Path {
                what: "body iterates",
                cover: Cover::By {
                    cohort: "tint_only",
                    witness: "witness_tint_only_sink_never_moves",
                },
                why: "four iterations per call, on every armed frame, from every live tint \
                      Some(t) arm — the steady, virtual_clock and tint_only cohorts",
            },
            Path {
                what: "loop exits",
                cover: Cover::By {
                    cohort: "tint_only",
                    witness: "witness_tint_only_sink_never_moves",
                },
                why: "every call. The bound is the constant 32 and the step the constant 8, so \
                      the zero-iteration case is unreachable by construction rather than \
                      uncovered",
            },
        ],
    },
    // ── ui_visual_tick ─────────────────────────────────────────────────────
    Site {
        func: "ui_visual_tick",
        kind: "for",
        key: "(entity , (mut sink , (tint , opacity , offset , scale))) in q . iter_entities_mut ()",
        paths: &[
            Path {
                what: "body executes",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame, 5 x NODES rows — the query yields the rested cohort too, \
                      which is the point of `witness_rested_cohort_takes_the_all_none_arm`",
            },
            Path {
                what: "zero rows yielded",
                cover: Cover::Not,
                why: "NOT covered. The gate's subtraction compares two arms over ONE world, and \
                      an empty query would need a second one. Reachable in production (a UI with \
                      no animated node), and recorded rather than closed because the loop body is \
                      where every cost this gate hunts lives: a cost on the empty-query path would \
                      have to be written OUTSIDE the loop, where it fires on every frame of every \
                      cohort and is caught by the tick-body rows above",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "if",
        key: "tint . is_none () && opacity . is_none () && offset . is_none () && scale . is_none ()",
        paths: &[
            Path {
                what: "all four arms None — the rested row",
                cover: Cover::By {
                    cohort: "rested",
                    witness: "witness_rested_cohort_takes_the_all_none_arm",
                },
                why: "every armed frame, NODES rows. An AnyOf of all-DENSE arms does not skip a \
                      row that has none of them, so this is the path every at-rest animated node \
                      takes forever — the majority path of any real UI",
            },
            Path {
                what: "at least one arm Some",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame, 4 x NODES rows",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "&&",
        key: "tint . is_none () && opacity . is_none () && offset . is_none () && scale . is_none ()",
        paths: &[
            Path {
                what: "left false — `scale.is_none()` is never evaluated",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame, 4 x NODES rows: any live channel among the first three \
                      short-circuits here. The outermost of three `&&` nodes the previous census \
                      could not see AT ALL — a keyword list has no keyword for a short-circuit, \
                      which is the hole the seventh pass drove a spurious reap entry through",
            },
            Path {
                what: "left true — the fourth operand is evaluated",
                cover: Cover::By {
                    cohort: "rested",
                    witness: "witness_rested_cohort_takes_the_all_none_arm",
                },
                why: "every armed frame, NODES rows — a rested row is the only one that reaches \
                      the last conjunct, because reaching it means the first three were all None",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "&&",
        key: "tint . is_none () && opacity . is_none () && offset . is_none ()",
        paths: &[
            Path {
                what: "left false — `offset.is_none()` is never evaluated",
                cover: Cover::By {
                    cohort: "tint_only",
                    witness: "witness_tint_only_sink_never_moves",
                },
                why: "every armed frame: tint_only carries TINT and nothing else, so \
                      `tint.is_none()` is false and this conjunction short-circuits before the \
                      offset arm is ever asked",
            },
            Path {
                what: "left true — the offset operand is evaluated",
                cover: Cover::By {
                    cohort: "rested",
                    witness: "witness_rested_cohort_takes_the_all_none_arm",
                },
                why: "every armed frame, NODES rows",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "&&",
        key: "tint . is_none () && opacity . is_none ()",
        paths: &[
            Path {
                what: "left false — `opacity.is_none()` is never evaluated",
                cover: Cover::By {
                    cohort: "tint_only",
                    witness: "witness_tint_only_sink_never_moves",
                },
                why: "every armed frame — the innermost short-circuit, taken by every row that \
                      carries a live tint",
            },
            Path {
                what: "left true — the opacity operand is evaluated",
                cover: Cover::By {
                    cohort: "rested",
                    witness: "witness_rested_cohort_takes_the_all_none_arm",
                },
                why: "every armed frame, NODES rows. Also the completing cohort from armed frame \
                      1 on, whose tint has been reaped",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "continue",
        key: "",
        paths: &[Path {
            what: "taken",
            cover: Cover::By {
                cohort: "rested",
                witness: "witness_rested_cohort_takes_the_all_none_arm",
            },
            why: "every armed frame, NODES rows. MEASURED 2026-08-27 against the fixture without \
                  a rested cohort: a with_capacity(64) here was GREEN and a panic! here left the \
                  gate PASSING while hanging three tests in ui_a1_tween",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "if-let",
        key: "Some (row) = tint",
        paths: &[
            Path {
                what: "Some — the channel is live",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame",
            },
            Path {
                what: "None — this channel absent, another live",
                cover: Cover::By {
                    cohort: "completing",
                    witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
                },
                why: "armed frames 1..=4: its tint completed on frame 0 and was reaped, while its \
                      other three channels keep the row past the all-None continue",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "match",
        key: "advance (& mut row . elapsed , row . inv_duration , row . flags , dt_real , dt_virtual)",
        paths: &[Path {
            what: "the discriminant is evaluated",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame — tint",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "Some (t)",
        paths: &[Path {
            what: "arm taken — tint interpolates",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame, 3 x NODES rows (steady, virtual_clock, tint_only)",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "None",
        paths: &[Path {
            what: "arm taken — tint completes",
            cover: Cover::By {
                cohort: "completing",
                witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
            },
            why: "armed frame 0 ONLY, NODES rows. Each None arm having a frame index of its own \
                  is what lets the per-index floor see it; a duration edit that moves two \
                  completions onto one frame silently deletes an arm from coverage",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "if-let",
        key: "Some (row) = opacity",
        paths: &[
            Path {
                what: "Some — the channel is live",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame",
            },
            Path {
                what: "None — this channel absent, another live",
                cover: Cover::By {
                    cohort: "tint_only",
                    witness: "witness_tint_only_sink_never_moves",
                },
                why: "every armed frame, NODES rows — that cohort carries TweenTint and nothing \
                      else. Also the completing cohort on armed frames 2..=4",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "match",
        key: "advance (& mut row . elapsed , row . inv_duration , row . flags , dt_real , dt_virtual)",
        paths: &[Path {
            what: "the discriminant is evaluated",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame — opacity",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "Some (t)",
        paths: &[Path {
            what: "arm taken — opacity interpolates",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame, 2 x NODES rows (steady, virtual_clock)",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "None",
        paths: &[Path {
            what: "arm taken — opacity completes",
            cover: Cover::By {
                cohort: "completing",
                witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
            },
            why: "armed frame 1 ONLY, NODES rows",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "if-let",
        key: "Some (row) = offset",
        paths: &[
            Path {
                what: "Some — the channel is live",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame",
            },
            Path {
                what: "None — this channel absent, another live",
                cover: Cover::By {
                    cohort: "tint_only",
                    witness: "witness_tint_only_sink_never_moves",
                },
                why: "every armed frame, NODES rows. Also the completing cohort on armed frames \
                      3..=4",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "match",
        key: "advance (& mut row . elapsed , row . inv_duration , row . flags , dt_real , dt_virtual)",
        paths: &[Path {
            what: "the discriminant is evaluated",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame — offset",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "Some (t)",
        paths: &[Path {
            what: "arm taken — offset interpolates",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame, 2 x NODES rows (steady, virtual_clock) — offset",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "None",
        paths: &[Path {
            what: "arm taken — offset completes",
            cover: Cover::By {
                cohort: "completing",
                witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
            },
            why: "armed frame 2 ONLY, NODES rows",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "if-let",
        key: "Some (row) = scale",
        paths: &[
            Path {
                what: "Some — the channel is live",
                cover: Cover::By {
                    cohort: "steady",
                    witness: "witness_steady_cohort_keeps_every_channel_live",
                },
                why: "every armed frame",
            },
            Path {
                what: "None — this channel absent, another live",
                cover: Cover::By {
                    cohort: "tint_only",
                    witness: "witness_tint_only_sink_never_moves",
                },
                why: "every armed frame, NODES rows. Also the completing cohort on armed frame 4",
            },
        ],
    },
    Site {
        func: "ui_visual_tick",
        kind: "match",
        key: "advance (& mut row . elapsed , row . inv_duration , row . flags , dt_real , dt_virtual)",
        paths: &[Path {
            what: "the discriminant is evaluated",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame — scale",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "Some (t)",
        paths: &[Path {
            what: "arm taken — scale interpolates",
            cover: Cover::By {
                cohort: "steady",
                witness: "witness_steady_cohort_keeps_every_channel_live",
            },
            why: "every armed frame, 2 x NODES rows (steady, virtual_clock) — scale",
        }],
    },
    Site {
        func: "ui_visual_tick",
        kind: "arm",
        key: "None",
        paths: &[Path {
            what: "arm taken — scale completes",
            cover: Cover::By {
                cohort: "completing",
                witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
            },
            why: "armed frame 3 ONLY, NODES rows",
        }],
    },
    // ── ui_tween_reap ──────────────────────────────────────────────────────
    Site {
        func: "ui_tween_reap",
        kind: "for",
        key: "& (entity , component_id) in & done",
        paths: &[
            Path {
                what: "body executes",
                cover: Cover::By {
                    cohort: "completing",
                    witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
                },
                why: "armed frames 0..=3, NODES iterations each",
            },
            Path {
                what: "zero iterations — nothing completed this frame",
                cover: Cover::By {
                    cohort: "EMPTY_REAP_FRAME",
                    witness: "witness_empty_reap_frame_completes_nothing",
                },
                why: "armed frame 4, and it is the ORDINARY frame of any real UI. The 2026-08-27 \
                      widening gave every armed frame a completion, which is the exact complement \
                      of reality: MEASURED 2026-08-28 a with_capacity(64) on this path was GREEN \
                      10/10, and with the fifth frame it is RED 10/10 at pair [6,6,6,6,7] against \
                      base [6,6,6,6,6]. The signal is +1, not +NODES, because the reap runs once \
                      per frame — which is exactly the resolution a per-index FLOOR has and a \
                      .max() does not",
            },
        ],
    },
    Site {
        func: "ui_tween_reap",
        kind: "if-let",
        key: "Some (store) = world . dense_registry_mut () . store_existing_mut (component_id)",
        paths: &[
            Path {
                what: "Some — the channel's dense store exists",
                cover: Cover::By {
                    cohort: "completing",
                    witness: "witness_completing_cohort_completes_one_channel_per_armed_frame",
                },
                why: "armed frames 0..=3, NODES iterations each",
            },
            Path {
                what: "None — no dense store for that ComponentId",
                cover: Cover::Not,
                why: "NOT covered, and unreachable from the shipped schedule rather than merely \
                      unvisited: the only writer of `done` is ui_visual_tick, which pushes \
                      (entity, C::component_id()) only from INSIDE `if let Some(row) = <C's \
                      channel>` — so a dense store for C demonstrably existed one system earlier \
                      in the same frame, and nothing between the tick and the reap drops a store. \
                      That claim is now mechanical rather than argued: CALL_SITES pins the four \
                      pushes in order, so a FIFTH push — anywhere, including inside a `&&`, which \
                      is how the seventh adversarial pass smuggled one in — reds before it can \
                      fabricate an entry the tick cannot produce. It is defensive depth against a \
                      future writer. MEASURED 2026-08-28, a with_capacity(64) in this else is \
                      GREEN 10/10, and that green is this row's evidence rather than its \
                      embarrassment",
            },
        ],
    },
];

// ───────────────────────── the callee tables ───────────────────────────────

/// A call the walked bodies make, keyed by RESOLVED SHAPE.
#[derive(Debug)]
struct Call {
    /// The callee's key: a full path for an `ExprCall` (`TweenTint::component_id`),
    /// or `.method` for an `ExprMethodCall` (`.set_if_neq`). The two namespaces
    /// cannot collide, which is the whole reason this is not a bare identifier.
    name: &'static str,
    /// `true` when the callee is one of [`WALKED`] — its branches are censused
    /// above. `false` when it is outside this file, and then [`Call::note`]
    /// carries the disposition of whatever control flow it hides.
    local: bool,
    /// What it is, and — for an external callee — whether its internal branches
    /// bear on the armed window's coverage claim.
    note: &'static str,
}

/// Every call the walked bodies make, as a SET, with its disposition.
///
/// This list exists because of exactly one row — `.set_if_neq`. The walk cannot
/// see inside another crate, so a branch that is production-reachable, is taken on
/// every frame by an ordinary UI node, and was executed ZERO times by this gate's
/// fixture is invisible to census 1 by construction. Enumerating the calls does
/// not make those branches visible; it makes them **enumerable**, so the
/// disposition is written down at a site that reds when a new callee appears.
const CALLS: &[Call] = &[
    Call {
        name: ".as_secs_f32",
        local: false,
        note: "Duration::as_secs_f32 on the raw frame delta. An integer-to-float conversion; no \
               branch that bears on the window",
    },
    Call {
        name: ".clear",
        local: false,
        note: "Vec::clear on a buffer of Copy pairs — a length store, no branch that bears on the \
               window",
    },
    Call {
        name: ".commands",
        local: false,
        note: "DeferredEcsMaster::commands — the hook's only route to a structural change. It \
               hands back the WORLD-RESIDENT deferred queue, so the insert lands at the outermost \
               apply and not inside the window that added the channel; that ordering is the reason \
               four channels added together each enqueue their own identical insert",
    },
    Call {
        name: ".delta_secs",
        local: false,
        note: "Time's VIRTUAL delta. Its own pause / relative_speed branching happened in \
               `advance_with` one frame earlier, outside the armed window; this is a field read",
    },
    Call {
        name: ".dense_registry_mut",
        local: false,
        note: "kernel accessor for the dense store table; the only immediate removal route an \
               exclusive system outside boyko_ecs has",
    },
    Call { name: ".dt_real", local: true, note: "censused above — a field read, no control flow" },
    Call {
        name: ".dt_virtual",
        local: true,
        note: "censused above — a field read, no control flow",
    },
    Call {
        name: ".entity",
        local: false,
        note: "Commands::entity — the builder that scopes the enqueued insert to the hook's own \
               ctx.entity. No branch; it is a handle, and the id it carries is the kernel's",
    },
    Call {
        name: ".get_component",
        local: false,
        note: "DeferredEcsMaster::get_component — the read that makes the hook route possible at \
               all. Its Option is exactly the `if` censused above, and BOTH of that `if`'s paths \
               have rows. The accessor's own internals (archetype lookup, absent-column path) are \
               the kernel's and are charged to the seed phase, never to the armed window",
    },
    Call {
        name: ".insert",
        local: false,
        note: "EntityCommands::insert of UiVisual::IDENTITY. Deferred: it appends to the \
               world-resident queue and the outermost drain applies it. Its own growth path is the \
               queue's, at seed time, outside every armed window in this crate",
    },
    Call {
        name: ".is_finite",
        local: false,
        note: "f32::is_finite inside ui_clock_tick's debug_assert!. The `&&` it feeds IS censused \
               above — the walk descends into a macro body that parses — and both of that \
               short-circuit's paths have rows",
    },
    Call {
        name: ".is_none",
        local: false,
        note: "Option discriminant test. The three `&&` nodes it feeds are censused above, one \
               row per short-circuit, which is the resolution the previous keyword scanner did \
               not have",
    },
    Call {
        name: ".is_some",
        local: false,
        note: "Option discriminant test, in the on_add hook. The `if` it feeds is censused above \
               and both of its paths carry a named gate in tests/ui_a1_tween.rs",
    },
    Call {
        name: ".iter_entities_mut",
        local: false,
        note: "the query iterator. Its internal control flow is the kernel's and is charged to \
               the PAIR arm only (the baseline's noop_normal iterates nothing), so it lands in the \
               measured floor and reads zero",
    },
    Call {
        name: ".min",
        local: false,
        note: "f32::min, twice — the UI clock's own clamp of both deltas to `max_delta` (AD1). It \
               is a branchless select on this target and neither input can be NaN (both come from \
               a Duration, and max_delta is setter-validated)",
    },
    Call {
        name: ".push",
        local: false,
        note: "Vec::push on UiTweenScratch's retained buffer. Its REALLOCATION path is a real \
               branch and is deliberately NOT covered: seeded_world's warm burst drives the buffer \
               to a real high-water mark before the window precisely so the grow path does not \
               fire inside it. That is what makes a `0` here mean 'no growth' rather than 'never \
               used'. WHERE the pushes are is pinned by CALL_SITES, not by this row",
    },
    Call {
        name: ".real_delta",
        local: false,
        note: "Time's RAW delta — unclamped, unscaled, pause-blind. A field read. It is what \
               makes the virtual lane observable: at relative_speed 0.5 this and `.delta_secs` \
               differ by construction",
    },
    Call {
        name: ".remove",
        local: false,
        note: "DenseStore::remove — takes a bare id with no liveness check, which is why \
               UiTweenScratch's generation-free key needs the three facts its doc records",
    },
    Call {
        name: ".resource_mut",
        local: false,
        note: "kernel resource accessor. It panics on an absent resource — a hidden branch, taken \
               on its non-panicking side on every frame; the panicking side is an invariant \
               violation, not a path a fixture should drive",
    },
    Call {
        name: ".set_if_neq",
        local: false,
        note: "THE reason this table exists. Mut::set_if_neq carries an equality short-circuit: \
               it writes, and bumps the changed tick, only when the new value DIFFERS. Both paths \
               are production-reachable and both are now driven inside the armed window — the \
               DIFFERENT path by the steady cohort (three moving f32 channels on every row) and \
               the EQUAL path by the tint_only cohort, whose composed tint_mul is a constant 0 \
               from the second warm frame on and which carries nothing else to move. MEASURED \
               2026-08-28: before that cohort, `if composed == *sink { with_capacity(64) }` was \
               GREEN 20/20; with it, RED 10/10 at pair [38,38,38,38,38] against base [6,6,6,6,6] — \
               exactly NODES on every index, i.e. the tint_only cohort and nobody else. \
               `witness_tint_only_sink_never_moves` is the executable half: it reads that cohort's \
               UiVisual on both sides of the armed window and asserts it did not move",
    },
    Call {
        name: ".store_existing_mut",
        local: false,
        note: "the dense-store lookup whose Option feeds the reap's `if let`; both of that `if \
               let`'s paths are censused above",
    },
    Call {
        name: "Some",
        local: false,
        note: "the Option tuple-variant constructor, in EXPRESSION position only — `advance`'s \
               `Some(t)`. The four `Some(row)` / `Some(t)` in pattern position are `Pat`s and the \
               walk does not confuse them with calls, which the previous byte scanner could not \
               avoid doing",
    },
    Call {
        name: "TweenOffset::component_id",
        local: false,
        note: "the derive's dense id accessor. It interns on FIRST call, so its cold path runs \
               during seeded_world's warm burst, long before the window opens, and only its warm \
               path runs inside it. The internals are the kernel's",
    },
    Call {
        name: "TweenOpacity::component_id",
        local: false,
        note: "as TweenOffset::component_id. A separate row because the key is the RESOLVED PATH: \
               the four are four callees, and collapsing them to one bare `component_id` is the \
               kind of key that let a whole function hide behind `advance`",
    },
    Call {
        name: "TweenScale::component_id",
        local: false,
        note: "as TweenOffset::component_id",
    },
    Call { name: "TweenTint::component_id", local: false, note: "as TweenOffset::component_id" },
    Call { name: "advance", local: true, note: "censused above — the free function" },
    Call { name: "ease", local: true, note: "censused above — the identity at rung A1" },
    Call { name: "lerp1", local: true, note: "censused above — straight-line arithmetic" },
    Call { name: "lerp_rgba8", local: true, note: "censused above — carries the `while` loop" },
    Call {
        name: "mem::take",
        local: false,
        note: "mem::take on the retained buffer — the DRAIN A1 gate 8 leg (i) is about. It moves \
               AND it constructs: `mem::take` calls `<T as Default>::default()` behind a generic \
               bound, which is a body no scan in this file keys. MEASURED 2026-08-28: `T` is \
               `Vec<(EntityId, ComponentId)>` (`animation.rs:472`), so the body constructed today \
               is std's, not a user impl. This route JOINS the no-syntax-edge class the header \
               enumerates, and the header's list does not yet carry it — see the bullet for \
               `Into`/`Default`/`Clone` beside DESUGARED_TRAITS, which describes it in full. \
               Unlike the four the header names, it was found by ARGUMENT, not by a probe",
    },
];

/// Every call the walked bodies make, as an ORDERED SEQUENCE of
/// `(function, callee key)`.
///
/// [`CALLS`] is a set and therefore blind to a SECOND call to a callee it already
/// lists. The sixth pass disclosed that residue in prose; the seventh walked
/// through it with a spurious `done.done.push(…)` smuggled into a `&&`
/// short-circuit, both of whose callees were already on the list. This closes it:
/// an added call is an added element at an index, no matter what it is nested in.
const CALL_SITES: &[(&str, &str)] = &[
    // ui_clock_tick — the outermost call of a chain is walked first, so
    // `time.real_delta().as_secs_f32().min(max)` reads .min, .as_secs_f32, .real_delta.
    ("ui_clock_tick", ".is_finite"),
    ("ui_clock_tick", ".min"),
    ("ui_clock_tick", ".as_secs_f32"),
    ("ui_clock_tick", ".real_delta"),
    ("ui_clock_tick", ".min"),
    ("ui_clock_tick", ".delta_secs"),
    // ui_visual_sink_on_add — the hook, reached by REGISTRATION and by nothing
    // else. The outermost call of a chain is walked first, so
    // `world.get_component::<UiVisual>(ctx.entity).is_some()` reads .is_some then
    // .get_component, and `world.commands().entity(…).insert(…)` reads .insert,
    // .entity, .commands.
    ("ui_visual_sink_on_add", ".is_some"),
    ("ui_visual_sink_on_add", ".get_component"),
    ("ui_visual_sink_on_add", ".insert"),
    ("ui_visual_sink_on_add", ".entity"),
    ("ui_visual_sink_on_add", ".commands"),
    // advance
    ("advance", "Some"),
    // lerp_rgba8
    ("lerp_rgba8", "lerp1"),
    // ui_visual_tick
    ("ui_visual_tick", ".dt_real"),
    ("ui_visual_tick", ".dt_virtual"),
    ("ui_visual_tick", ".iter_entities_mut"),
    ("ui_visual_tick", ".is_none"),
    ("ui_visual_tick", ".is_none"),
    ("ui_visual_tick", ".is_none"),
    ("ui_visual_tick", ".is_none"),
    // tint
    ("ui_visual_tick", "advance"),
    ("ui_visual_tick", "lerp_rgba8"),
    ("ui_visual_tick", "ease"),
    ("ui_visual_tick", ".push"),
    ("ui_visual_tick", "TweenTint::component_id"),
    // opacity
    ("ui_visual_tick", "advance"),
    ("ui_visual_tick", "lerp1"),
    ("ui_visual_tick", "ease"),
    ("ui_visual_tick", ".push"),
    ("ui_visual_tick", "TweenOpacity::component_id"),
    // offset
    ("ui_visual_tick", "advance"),
    ("ui_visual_tick", "ease"),
    ("ui_visual_tick", "lerp1"),
    ("ui_visual_tick", "lerp1"),
    ("ui_visual_tick", ".push"),
    ("ui_visual_tick", "TweenOffset::component_id"),
    // scale
    ("ui_visual_tick", "advance"),
    ("ui_visual_tick", "ease"),
    ("ui_visual_tick", "lerp1"),
    ("ui_visual_tick", "lerp1"),
    ("ui_visual_tick", ".push"),
    ("ui_visual_tick", "TweenScale::component_id"),
    // the ONE write, through the ONE verb
    ("ui_visual_tick", ".set_if_neq"),
    // ui_tween_reap
    ("ui_tween_reap", "mem::take"),
    ("ui_tween_reap", ".resource_mut"),
    ("ui_tween_reap", ".store_existing_mut"),
    ("ui_tween_reap", ".dense_registry_mut"),
    ("ui_tween_reap", ".remove"),
    ("ui_tween_reap", ".clear"),
    ("ui_tween_reap", ".resource_mut"),
];

// ───────────────────────── census 1d: the operator sites ───────────────────

/// Every OPERATOR EXPRESSION of the walked region, as an ORDERED SEQUENCE of
/// `(function, kind, key)`.
///
/// **The third execution edge with no syntax at the call site.** The eighth pass
/// found drop glue and hook registration; the tenth found this one, and it is the
/// cheapest of the three to smuggle. `Expr::Index`, `Expr::Unary`, `Expr::Assign`
/// and every `Expr::Binary` other than `&&` / `||` matched NO arm of [`Walk`], so
/// an overloaded operator called a user body that was neither a branch node, nor a
/// call node, nor a callee, nor a call site — and with no `impl Drop` in the crate
/// the drop scan did not fire either.
///
/// MEASURED 2026-08-28, both members of the class, on the shipped tree:
///
/// | probe | result |
/// |---|---|
/// | `impl std::ops::Mul<(&mut f32, f32)> for ZzMulCap` carrying a fourth termination condition, dispatched from the opacity arm | census **EXIT=0, 10 passed**, every headline count bit-identical — 32 nodes / 50 paths, 50 call sites, 31 callees, 0 `impl Drop` / 0 drop bodies; whole crate **EXIT=0, 53 targets, 352 passed** |
/// | `impl std::ops::Index<usize> for ZzIdxCap`, a structurally different node, same arm | identical: census **EXIT=0, 10 passed**, crate **EXIT=0, 352 passed** |
///
/// Both bodies demonstrably ran: moving the threshold reddened **four** tests each
/// (`an_over_ceiling_duration_is_accepted_and_never_completes`,
/// `m4b_the_clamp_is_visible_in_a_tweens_elapsed`,
/// `the_steady_animating_path_allocates_zero_over_baseline`,
/// `the_tick_bumps_the_sink_on_both_routes`).
///
/// **It cost ZERO rows**, which is what separates it from the escapes already on
/// record: a cross-module callee costs two rows
/// ([`every_callee_of_the_walked_region_is_enumerated`] plus
/// [`every_call_site_of_the_walked_region_is_pinned`]), and so does a factory
/// escape. This one cost none, so it also refutes residue item 9's stated reason —
/// that item excused straight-line code because *"it introduces no path and no
/// callee"*, and an operator overload introduces both while looking like
/// straight-line code at the site. Item 9 now says what is actually true of it.
///
/// # Why the WHOLE population and not a filtered one
///
/// MEASURED 2026-08-28: the walked region contains **43** operator expressions.
/// The alternative considered was to demand rows only for operators whose operand
/// type has a user `impl` in this crate.
///
/// **The reason this file used to give for rejecting that was FALSE, and it was
/// this program's own output that falsified it.** It said the narrow option
/// *"starts EMPTY, because the crate has none"* — while
/// [`every_operator_impl_in_the_crate_is_dispositioned`] PRINTS
/// `3 user operator impl(s) in src/` on every run, [`OPERATOR_IMPLS`] lists all
/// three by name, and this file says so in three other places. The narrow option
/// does start empty, and RE-MEASURED 2026-08-28 the reason is a different one: none
/// of the three types — `UiTextBuffer`, `UiVisual`, `UiName` — is the operand of
/// any operator expression in the walked region. The one that IS reachable,
/// `PartialEq for UiVisual`, is reached by the CALL `sink.set_if_neq(composed)`,
/// and the walked region contains no `==` at all.
///
/// Rejecting it on the corrected reason is if anything easier, because the narrow
/// option is weaker in TWO ways, not one:
///
/// * it is blind to an operator expression whose TOKENS do not move while its
///   operand type changes under it (`lerp1(from: f32, …)` becoming
///   `lerp1(from: Px, …)` leaves `from + (to - from) * t` printed identically);
/// * it filters on THIS crate's `impl`s, and the one non-primitive operand the
///   walked region actually has resolves to another crate's: `* sink` is
///   `Mut<'w, UiVisual>`, whose `Deref` is `boyko_ecs`
///   `src/ecs/core/iters/query/data/mut_.rs:123`. A this-crate filter drops the
///   single row that is live today. See residue item 14.
///
/// So both halves are here: this ordered pin, which reds on a new or edited
/// operator expression whatever crate its `impl` lives in, and
/// [`every_operator_impl_in_the_crate_is_dispositioned`], which reds on a new user
/// operator `impl` whether or not any expression moved.
///
/// The rows are arithmetic, and they are in a table OF THEIR OWN rather than in
/// [`CALL_SITES`], so the callee list stays 50 rows of dispositions and this stays
/// 43 rows of exactly what it says it is.
///
/// **`Expr::Assign` cannot dispatch anywhere** — Rust has no `Assign` trait, and
/// `a = b` is always a builtin move or copy. Its rows are here anyway, because a
/// filtered walk is a walk with a judgement call in it and this campaign's
/// signature defect is a scanner that silently skips a construct. They cost
/// nothing and they close, as a side effect, the half of residue item 9 that is
/// about a straight-line ASSIGNMENT.
#[rustfmt::skip]
const OPERATOR_SITES: &[(&str, &str, &str)] = &[
    // ui_clock_tick — the `debug_assert!`'s comparison, then the two writes.
    ("ui_clock_tick", "binary", "max > 0.0"),
    ("ui_clock_tick", "assign", "clock . dt_real = time . real_delta () . as_secs_f32 () . min (max)"),
    ("ui_clock_tick", "assign", "clock . dt_virtual = time . delta_secs () . min (max)"),
    // advance — the termination arithmetic. `* elapsed * inv_duration` and
    // `t < 1.0` ARE the termination condition, so a user `Mul` or `PartialOrd`
    // reached from here would be a second one; ADVANCE_PIN pins the same tokens
    // from the other side.
    ("advance", "binary", "* elapsed >= 0.0"),
    ("advance", "unary", "* elapsed"),
    ("advance", "binary", "flags & TWEEN_FLAG_VIRTUAL_CLOCK != 0"),
    ("advance", "binary", "flags & TWEEN_FLAG_VIRTUAL_CLOCK"),
    ("advance", "assign-op", "* elapsed += dt"),
    ("advance", "unary", "* elapsed"),
    ("advance", "binary", "* elapsed * inv_duration"),
    ("advance", "unary", "* elapsed"),
    ("advance", "binary", "t < 1.0"),
    // lerp1
    ("lerp1", "binary", "from + (to - from) * t"),
    ("lerp1", "binary", "(to - from) * t"),
    ("lerp1", "binary", "to - from"),
    // lerp_rgba8 — the `while` head is a node in SITES as well; here it is the
    // COMPARISON, which is a different edge from the loop.
    ("lerp_rgba8", "binary", "shift < 32"),
    ("lerp_rgba8", "binary", "(from >> shift) & 0xFF"),
    ("lerp_rgba8", "binary", "from >> shift"),
    ("lerp_rgba8", "binary", "(to >> shift) & 0xFF"),
    ("lerp_rgba8", "binary", "to >> shift"),
    ("lerp_rgba8", "binary", "lerp1 (a , b , t) + 0.5"),
    ("lerp_rgba8", "assign-op", "out |= (v & 0xFF) << shift"),
    ("lerp_rgba8", "binary", "(v & 0xFF) << shift"),
    ("lerp_rgba8", "binary", "v & 0xFF"),
    ("lerp_rgba8", "assign-op", "shift += 8"),
    // ui_visual_tick — the composition base, then the four channels' writes.
    ("ui_visual_tick", "unary", "* sink"),
    // tint
    ("ui_visual_tick", "assign", "composed . tint_mul = lerp_rgba8 (row . from , row . to , ease (t , row . easing))"),
    ("ui_visual_tick", "assign", "composed . tint_mul = row . to"),
    // opacity
    ("ui_visual_tick", "assign", "composed . opacity = lerp1 (row . from , row . to , ease (t , row . easing))"),
    ("ui_visual_tick", "assign", "composed . opacity = row . to"),
    // offset — the four `[0]` / `[1]` are `Index`, the node kind the tenth pass's
    // second probe used.
    ("ui_visual_tick", "assign", "composed . offset_px = [lerp1 (row . from [0] , row . to [0] , e) , lerp1 (row . from [1] , row . to [1] , e)]"),
    ("ui_visual_tick", "index", "row . from [0]"),
    ("ui_visual_tick", "index", "row . to [0]"),
    ("ui_visual_tick", "index", "row . from [1]"),
    ("ui_visual_tick", "index", "row . to [1]"),
    ("ui_visual_tick", "assign", "composed . offset_px = row . to"),
    // scale
    ("ui_visual_tick", "assign", "composed . scale = [lerp1 (row . from [0] , row . to [0] , e) , lerp1 (row . from [1] , row . to [1] , e)]"),
    ("ui_visual_tick", "index", "row . from [0]"),
    ("ui_visual_tick", "index", "row . to [0]"),
    ("ui_visual_tick", "index", "row . from [1]"),
    ("ui_visual_tick", "index", "row . to [1]"),
    ("ui_visual_tick", "assign", "composed . scale = row . to"),
    // ui_tween_reap — the drain's put-back.
    ("ui_tween_reap", "assign", "world . resource_mut :: < UiTweenScratch > () . done = done"),
];

// ───────────────────────── the termination pin ─────────────────────────────

/// `advance`, printed — attributes, signature and body, doc comments dropped and
/// string literals emptied.
///
/// This is the SYSTEM half of the termination condition: a row completes when
/// `t = elapsed * inv_duration` fails `< 1.0`, and there is no other way for it to
/// stop. The sixth adversarial pass added `&& *elapsed < 3_600.0` to the last line
/// — "no tween runs longer than an hour" — and the whole suite stayed at EXIT=0
/// across 1427 tests while `invalid_tween_duration`'s disclosure had been
/// falsified verbatim. The point-sampled gates could not see it because they only
/// ever look at what the DOOR stored.
///
/// It is printed TOKENS, not source lines, so a reflow of the body does not red it
/// and a change of one operator does. `#[inline]` is inside the pin (a non-doc
/// attribute); the `///` prose is not.
///
/// Note the `""`: the `debug_assert!`'s message is emptied before comparison, so
/// this pin is blind to message prose. That limit has already bitten — the
/// over-ceiling gate's live-row message still described a termination the added
/// cap had made false. A message is not a mechanism; the mechanism is these tokens.
const ADVANCE_PIN: &[&str] = &[
    "# [inline]",
    "fn advance (elapsed : & mut f32 , inv_duration : f32 , flags : u8 , dt_real : f32 ,",
    "dt_virtual : f32) -> Option < f32 > {",
    "debug_assert ! (* elapsed >= 0.0 , \"\") ;",
    "let dt = if flags & TWEEN_FLAG_VIRTUAL_CLOCK != 0 { dt_virtual } else { dt_real } ;",
    "* elapsed += dt ;",
    "let t = * elapsed * inv_duration ;",
    "if t < 1.0 { Some (t) } else { None }",
    "}",
];

/// The `tween_helpers!` macro body, printed — the DOOR half.
///
/// The disclosure says the guard tests finiteness and the sign bit and NOTHING
/// ELSE, so every over-ceiling duration is accepted verbatim and the row it builds
/// stores `1000.0 / duration_ms` computed from the value PASSED IN. The evasions
/// live exactly between the guard and the insert: a global `.min(1e4)`, a sub-range
/// rewrite that preserves the three sampled points bit-exactly, an extra conjunct
/// `&& duration_ms < 1e20`.
///
/// The sub-range rewrite is the one that matters. MEASURED 2026-08-28,
/// `if duration_ms > 5.0e8 && duration_ms <= 1.0e9 { 1.0e4 } else { duration_ms }`
/// immediately after the guard left all three arms of the datum assertion
/// bit-exact and the whole crate green, while every accepted duration in
/// `(5.24288e8, 1e9]` — squarely inside the disclosed class — completed in ten
/// seconds. A three-point sample cannot close an open class. This can, because it
/// is not a sample: it is the predicate.
///
/// **Why this one is TOKENS and not a tree.** A `macro_rules!` body is not Rust
/// until it expands — `$start`, `$payload` and `$bundle` are metavariables, and
/// `syn` has nothing to walk. Printing the token stream is the strongest pin
/// available at that granularity, and it is strictly stronger than the normalized
/// LINES this used to be: comments are already gone, whitespace is already
/// canonical, and the checkout's line-ending configuration cannot reach it. (A
/// gate over raw bytes hashes that configuration instead of the content — a defect
/// this repository has shipped once.) It covers `$stop` too, which the line pin
/// did not.
const START_PIN: &[&str] = &[
    "($ start : ident , $ stop : ident , $ channel : ident , $ bundle : ident , $ payload : ty ,",
    "$ finite : expr , $ doc_what : literal) => {",
    // ── the door ───────────────────────────────────────────────────────────
    "pub fn $ start (cmds : & mut Commands , entity : Entity , from : $ payload , to : $ payload ,",
    "duration_ms : f32 , easing : EasingId , flags : u8 ,) {",
    // The guard, and the whole of it: finiteness and the sign bit, nothing else.
    "if ! (duration_ms . is_finite () && duration_ms . is_sign_positive ()) {",
    "invalid_tween_duration (duration_ms) ;",
    "return ;",
    "}",
    "let finite : fn ($ payload) -> bool = $ finite ;",
    "debug_assert ! (finite (from) && finite (to) , \"\") ;",
    // The reciprocal, computed from the value PASSED IN — no clamp between the
    // guard and this line, which is where every measured evasion lived.
    "let inv_duration = 1000.0 / duration_ms ;",
    "cmds . entity (entity) . insert ($ bundle { tween : $ channel { from , to , elapsed : 0.0 ,",
    "inv_duration , easing , flags , _pad : [0 ; 2] , } , }) ;",
    "}",
    // ── the stop half, which the LINE pin this replaces never covered ──────
    "pub fn $ stop (cmds : & mut Commands , entity : Entity) {",
    "cmds . entity (entity) . remove ::<$ channel > () ;",
    "}",
    "} ;",
];

/// Fragments of the DISCLOSURE the two pins above protect, each of which must
/// occur in `animation.rs`'s comment text.
///
/// The pins red when the predicate changes and the disclosure does not. These red
/// when the disclosure goes and the predicate stays — a doc deletion that would
/// otherwise leave two green pins guarding a claim nobody makes any more.
///
/// Matched against [`doc_text`], i.e. against the WORDS with comment markers and
/// line breaks flattened away, so re-wrapping a paragraph does not red this and
/// deleting the sentence does.
const DISCLOSURE: &[&str] = &[
    // The class the guard does NOT close.
    "yields a row that never completes, is never reaped, and bumps `set_if_neq` on EVERY frame",
    // The ceiling, and that the boundary itself completes.
    "**The operator is `>`, not `>=`, and the boundary value itself COMPLETES.**",
    // The predicate's exact spelling, and why it is not `> 0.0`.
    "`is_finite() && is_sign_positive()`",
    // `advance`'s completion test, and the reason for its spelling.
    "The completion test is spelled `t < 1.0`, not `!(t >= 1.0)`",
];

// ───────────────────────── printing and normalizing ────────────────────────

/// Whitespace runs collapsed to one space, ends trimmed.
fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every string literal in a PRINTED TOKEN STRING replaced by `""`.
///
/// Safe here in a way it would not be over raw source: the input has already been
/// tokenized by `proc-macro2`, so comments are gone, raw strings are already
/// re-printed, and a `"` can only be a literal's delimiter. The stripper this
/// replaces had to disclaim char literals and raw strings because it ran over
/// bytes the lexer had not seen.
fn empty_strings(printed: &str) -> String {
    let b: Vec<char> = printed.chars().collect();
    let mut out = String::with_capacity(printed.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != '"' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        out.push_str("\"\"");
        i += 1;
        while i < b.len() {
            if b[i] == '\\' {
                i += 2;
                continue;
            }
            if b[i] == '"' {
                i += 1;
                break;
            }
            i += 1;
        }
    }
    out
}

/// Every `#[doc = …]` attribute deleted from a printed token string.
///
/// Bracket matching over a PRINTED stream is exact — the printer emits balanced
/// delimiters by construction — which is what lets the two pins carry code without
/// carrying prose. `#[inline]` and every other attribute survive.
fn strip_doc_attrs(printed: &str) -> String {
    let b: Vec<char> = printed.chars().collect();
    let mut out = String::with_capacity(printed.len());
    let mut i = 0usize;
    while i < b.len() {
        // `# [doc ...]`, optionally `# ! [doc ...]` for an inner attribute.
        if b[i] == '#' {
            let mut j = i + 1;
            while j < b.len() && (b[j] == ' ' || b[j] == '!') {
                j += 1;
            }
            if j < b.len() && b[j] == '[' {
                let mut k = j + 1;
                while k < b.len() && b[k] == ' ' {
                    k += 1;
                }
                // Compared CHAR by CHAR. An earlier spelling indexed the &str by
                // `k`, which is a char offset — the doc prose here is full of
                // em-dashes, so the two diverged and every attribute survived
                // while its string content was emptied. The pin then carried a
                // row of `#[doc = ""]` and reddened on a reflow.
                if b[k..].starts_with(&['d', 'o', 'c']) {
                    let mut depth = 0i32;
                    let mut e = j;
                    while e < b.len() {
                        if b[e] == '[' {
                            depth += 1;
                        } else if b[e] == ']' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        e += 1;
                    }
                    i = e + 1;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// The canonical key of any syntax node: printed, doc-attribute-free,
/// string-emptied, whitespace-collapsed.
fn key_of<T: ToTokens>(node: &T) -> String {
    squeeze(&empty_strings(&strip_doc_attrs(&node.to_token_stream().to_string())))
}

/// The COMMENT text of a Rust source, flattened: each line's leading whitespace
/// and `///` / `//!` / `//` marker removed, then every whitespace run collapsed to
/// a single space.
///
/// Fragments of prose are matched against this rather than against the raw bytes,
/// so a re-wrapped paragraph does not red the disclosure check and a CRLF checkout
/// does not either.
fn doc_text(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim_start();
        let t = t.strip_prefix("///").or_else(|| t.strip_prefix("//!")).unwrap_or(t);
        out.push_str(t);
        out.push(' ');
    }
    squeeze(&out)
}

// ───────────────────────── the definition table ────────────────────────────

/// One function defined in the parsed file, under its qualified name.
struct Def<'a> {
    /// `advance`, `UiClock::dt_real`, `UiTweenScratch::advance`, …
    qualified: String,
    /// The last path segment, for the "which definition did this call mean"
    /// over-approximation. Never used as an identity — that is the whole point.
    bare: String,
    attrs: &'a [Attribute],
    sig: &'a Signature,
    block: &'a Block,
}

impl Def<'_> {
    /// The printed pin of a whole function: non-doc attributes, signature, body.
    fn pin(&self) -> String {
        let attrs = self
            .attrs
            .iter()
            .filter(|a| !a.path().is_ident("doc"))
            .map(|a| a.to_token_stream().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let printed = format!(
            "{attrs} {} {}",
            self.sig.to_token_stream(),
            self.block.to_token_stream()
        );
        squeeze(&empty_strings(&strip_doc_attrs(&printed)))
    }
}

/// The `Self` type of an `impl`, as a name — the last segment of its path.
fn impl_type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => {
            p.path.segments.last().map_or_else(|| "?".to_string(), |s| s.ident.to_string())
        }
        other => key_of(other),
    }
}

/// Every `Item` written INSIDE a block, at any depth — a nested `fn`, `impl` or
/// `mod`. Collected so a definition cannot escape the census by being nested.
struct ItemHunt<'a> {
    found: Vec<&'a Item>,
}

impl<'ast> Visit<'ast> for ItemHunt<'ast> {
    fn visit_item(&mut self, i: &'ast Item) {
        self.found.push(i);
        visit::visit_item(self, i);
    }
}

fn nested_items(block: &Block) -> Vec<&Item> {
    let mut h = ItemHunt { found: Vec::new() };
    h.visit_block(block);
    h.found
}

/// Every function the file defines, qualified, including the ones nested inside
/// other bodies and the ones inside `#[cfg(test)] mod tests`.
fn collect_defs<'a, I: IntoIterator<Item = &'a Item>>(
    items: I,
    prefix: &str,
    out: &mut Vec<Def<'a>>,
) {
    let qualify = |name: &str| {
        if prefix.is_empty() { name.to_string() } else { format!("{prefix}::{name}") }
    };
    for it in items {
        match it {
            Item::Fn(f) => {
                let q = qualify(&f.sig.ident.to_string());
                out.push(Def {
                    qualified: q.clone(),
                    bare: f.sig.ident.to_string(),
                    attrs: &f.attrs,
                    sig: &f.sig,
                    block: &f.block,
                });
                collect_defs(nested_items(&f.block), &q, out);
            }
            Item::Impl(im) => {
                let ty = impl_type_name(&im.self_ty);
                for ii in &im.items {
                    if let ImplItem::Fn(f) = ii {
                        let q = qualify(&format!("{ty}::{}", f.sig.ident));
                        out.push(Def {
                            qualified: q.clone(),
                            bare: f.sig.ident.to_string(),
                            attrs: &f.attrs,
                            sig: &f.sig,
                            block: &f.block,
                        });
                        collect_defs(nested_items(&f.block), &q, out);
                    }
                }
            }
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_defs(inner, prefix, out);
                }
            }
            Item::Trait(t) => {
                for ti in &t.items {
                    if let syn::TraitItem::Fn(f) = ti
                        && let Some(b) = &f.default
                    {
                        let q = qualify(&format!("{}::{}", t.ident, f.sig.ident));
                        out.push(Def {
                            qualified: q.clone(),
                            bare: f.sig.ident.to_string(),
                            attrs: &f.attrs,
                            sig: &f.sig,
                            block: b,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

// ───────────────────────── the walk ────────────────────────────────────────

/// One control-flow node: the function it is in, its kind, its key.
type Branch = (String, &'static str, String);

/// Whether a `syn` 2 `BinOp` is a COMPOUND ASSIGNMENT (`+=`, `|=`, …), which
/// dispatches to `AddAssign` / `BitOrAssign` / … rather than to `Add` / `BitOr`.
///
/// `syn` 2 folded `ExprAssignOp` into `ExprBinary`, so the two families arrive at
/// the same match arm and this is what separates them.
fn is_assign_op(op: syn::BinOp) -> bool {
    use syn::BinOp;
    matches!(
        op,
        BinOp::AddAssign(_)
            | BinOp::SubAssign(_)
            | BinOp::MulAssign(_)
            | BinOp::DivAssign(_)
            | BinOp::RemAssign(_)
            | BinOp::BitXorAssign(_)
            | BinOp::BitAndAssign(_)
            | BinOp::BitOrAssign(_)
            | BinOp::ShlAssign(_)
            | BinOp::ShrAssign(_)
    )
}

/// The AST walk.
///
/// Deliberately NOT parameterized by the visited lifetime, so the same walker can
/// descend into a macro body it just parsed — an owned tree whose lifetime is
/// local. That is how `debug_assert!(a && b, "")`'s `&&` becomes a node here.
struct Walk {
    func: String,
    branches: Vec<Branch>,
    calls: Vec<(String, String)>,
    /// Macro invocations whose body did not parse as a comma-separated expression
    /// list. Asserted EMPTY, so an opaque body is a red and not a silent skip.
    opaque: Vec<(String, String)>,
    /// Type names the region may CONSTRUCT. Not control flow and not a call, so it
    /// is neither a [`Branch`] nor a [`Walk::call`] — it exists to answer one
    /// question that neither of those can:
    /// [`every_drop_impl_on_a_type_the_walked_region_constructs_is_walked`].
    constructs: BTreeSet<String>,
    /// Operator expressions, in syntactic order — see [`OPERATOR_SITES`]. THE
    /// THIRD EXECUTION EDGE WITH NO SYNTAX AT THE CALL SITE.
    ops: Vec<(String, &'static str, String)>,
}

impl Walk {
    fn new(func: &str) -> Self {
        Walk {
            func: func.to_string(),
            branches: Vec::new(),
            calls: Vec::new(),
            opaque: Vec::new(),
            constructs: BTreeSet::new(),
            ops: Vec::new(),
        }
    }

    fn branch(&mut self, kind: &'static str, key: String) {
        self.branches.push((self.func.clone(), kind, key));
    }

    /// Records an operator expression: an `Expr::Binary` that is not `&&` / `||`,
    /// an `Expr::Unary`, an `Expr::Index`, an `Expr::Assign` or an
    /// `Expr::AssignOp`.
    ///
    /// Every one of these MAY dispatch to a user `impl` — `Mul`, `Neg`, `Deref`,
    /// `Index`, `AddAssign`, `PartialOrd` — and that dispatch is an execution edge
    /// with NO SYNTAX AT THE CALL SITE, the same class as drop glue and a hook
    /// registration. It is not a branch, not a call and not a construction, so
    /// before this existed it changed no count in any census.
    fn op(&mut self, kind: &'static str, key: String) {
        self.ops.push((self.func.clone(), kind, key));
    }

    fn call(&mut self, key: String) {
        self.calls.push((self.func.clone(), key));
    }

    /// Records a type the region may construct.
    ///
    /// Deliberately an OVER-approximation, and in the fail-closed direction: an
    /// `Expr::Struct` is a construction for certain, and any `T::assoc(…)` might
    /// be one. A type recorded here that is never actually constructed costs a
    /// `DROP_GLUE` row nobody needed; a type MISSED here costs a `Drop` body that
    /// runs in the walked region and is in no census — which is the defect this
    /// exists for.
    fn constructs(&mut self, ty: String) {
        self.constructs.insert(ty);
    }

    fn absorb(&mut self, other: Walk) {
        self.branches.extend(other.branches);
        self.calls.extend(other.calls);
        self.opaque.extend(other.opaque);
        self.constructs.extend(other.constructs);
        self.ops.extend(other.ops);
    }

    /// A macro invocation: always a node; its body walked when it parses.
    fn macro_node(&mut self, mac: &syn::Macro) {
        let path = key_of(&mac.path);
        match mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            Ok(args) => {
                self.branch("macro", format!("{path}!"));
                let mut sub = Walk::new(&self.func);
                for a in &args {
                    sub.visit_expr(a);
                }
                self.absorb(sub);
            }
            Err(_) => {
                self.branch("macro", format!("{path}! <opaque>"));
                self.opaque.push((self.func.clone(), path));
            }
        }
    }
}

impl<'ast> Visit<'ast> for Walk {
    fn visit_expr(&mut self, e: &'ast Expr) {
        match e {
            Expr::If(x) => {
                let is_let = matches!(&*x.cond, Expr::Let(_));
                let kind = match (is_let, x.else_branch.as_ref().map(|(_, b)| &**b)) {
                    (false, None) => "if",
                    (false, Some(Expr::If(_))) => "if/else-if",
                    (false, Some(_)) => "if/else",
                    (true, None) => "if-let",
                    (true, Some(Expr::If(_))) => "if-let/else-if",
                    (true, Some(_)) => "if-let/else",
                };
                let key = match &*x.cond {
                    Expr::Let(l) => format!("{} = {}", key_of(&l.pat), key_of(&l.expr)),
                    other => key_of(other),
                };
                self.branch(kind, key);
            }
            Expr::Match(x) => self.branch("match", key_of(&x.expr)),
            Expr::While(x) => {
                let is_let = matches!(&*x.cond, Expr::Let(_));
                let key = match &*x.cond {
                    Expr::Let(l) => format!("{} = {}", key_of(&l.pat), key_of(&l.expr)),
                    other => key_of(other),
                };
                self.branch(if is_let { "while-let" } else { "while" }, key);
            }
            Expr::ForLoop(x) => {
                self.branch("for", format!("{} in {}", key_of(&x.pat), key_of(&x.expr)));
            }
            Expr::Loop(_) => self.branch("loop", String::new()),
            Expr::Binary(b) => match b.op {
                syn::BinOp::And(_) => self.branch("&&", key_of(b)),
                syn::BinOp::Or(_) => self.branch("||", key_of(b)),
                // Every other binary operator is a possible dispatch to a user
                // `impl` — a body reached with no syntax at the call site. In
                // `syn` 2 a compound assignment is an `ExprBinary` too
                // (`BinOp::AddAssign`, …), not the `ExprAssignOp` that `syn` 1
                // had, so both land here and are told apart by kind.
                op => self.op(if is_assign_op(op) { "assign-op" } else { "binary" }, key_of(b)),
            },
            Expr::Unary(u) => self.op("unary", key_of(u)),
            Expr::Index(x) => self.op("index", key_of(x)),
            Expr::Assign(a) => self.op("assign", key_of(a)),
            Expr::Try(x) => self.branch("?", key_of(&x.expr)),
            Expr::Return(x) => {
                self.branch("return", x.expr.as_ref().map_or_else(String::new, key_of));
            }
            Expr::Break(x) => {
                self.branch("break", x.expr.as_ref().map_or_else(String::new, key_of));
            }
            Expr::Continue(_) => self.branch("continue", String::new()),
            Expr::Closure(c) => {
                let inputs =
                    c.inputs.iter().map(key_of).collect::<Vec<_>>().join(", ");
                self.branch("closure", format!("|{inputs}|"));
            }
            Expr::Await(_) => self.branch("await", String::new()),
            Expr::Async(_) => self.branch("async", String::new()),
            Expr::Yield(_) => self.branch("yield", String::new()),
            Expr::Macro(m) => {
                self.macro_node(&m.mac);
                return;
            }
            Expr::Call(c) => {
                let key = match &*c.func {
                    Expr::Path(p) => {
                        let segs: Vec<String> =
                            p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                        // `T::assoc(…)` MIGHT be `T`'s constructor. Recorded as a
                        // construction candidate; see `Walk::constructs`.
                        if segs.len() >= 2 {
                            self.constructs(segs[segs.len() - 2].clone());
                        }
                        segs.join("::")
                    }
                    other => format!("<indirect> {}", key_of(other)),
                };
                self.call(key);
            }
            Expr::MethodCall(m) => self.call(format!(".{}", m.method)),
            // A struct literal is a construction for certain. It is NOT a branch
            // and NOT a call, so it changes no census-1 or census-1b count — the
            // eighth adversarial pass's `ZzDropCap { elapsed: &mut row.elapsed }`
            // was invisible for exactly that reason, while its `impl Drop` carried
            // a fourth termination condition.
            Expr::Struct(s) => {
                if let Some(seg) = s.path.segments.last() {
                    self.constructs(seg.ident.to_string());
                }
            }
            _ => {}
        }
        visit::visit_expr(self, e);
    }

    fn visit_arm(&mut self, a: &'ast syn::Arm) {
        let key = match &a.guard {
            Some((_, g)) => format!("{} if {}", key_of(&a.pat), key_of(g)),
            None => key_of(&a.pat),
        };
        self.branch("arm", key);
        visit::visit_arm(self, a);
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        // `let … else` diverges: it is a branch, and it is the one `let` form
        // that is.
        if l.init.as_ref().and_then(|i| i.diverge.as_ref()).is_some() {
            self.branch("let-else", key_of(&l.pat));
        }
        visit::visit_local(self, l);
    }

    fn visit_stmt(&mut self, s: &'ast Stmt) {
        if let Stmt::Macro(m) = s {
            self.macro_node(&m.mac);
            return;
        }
        visit::visit_stmt(self, s);
    }

    /// A `fn`, `impl` or `mod` written INSIDE a walked body: a node in its own
    /// right, AND descended into.
    ///
    /// **The descent is the tenth adversarial pass's finding.** This used to
    /// record the item and return, on the stated ground that *"its own contents
    /// are censused by the definition table and the closure assertion"*. That
    /// compensation is CALL-GATED and there is not always a call:
    /// [`the_walked_set_is_closed_under_intra_file_calls`] only looks at
    /// definitions whose bare name the region CALLS, and an operator-trait `impl`
    /// is reached by an operator, never by a call. MEASURED 2026-08-28, before
    /// this line: a `struct` plus an `impl std::ops::Mul` moved inside
    /// `ui_visual_tick`'s body — the `impl` carrying a fourth termination
    /// condition — reddened EXACTLY ONE test, asking for two opaque
    /// `<nested item>` rows, while the closure assertion and the callee census
    /// both printed `ok`. Adding those two rows gave EXIT=0, 352 passed, with the
    /// cap live and the `if` inside the operator body — written syntactically
    /// inside `ui_visual_tick` — counted nowhere.
    ///
    /// With the descent, that `if` is a node of the walked region and needs a
    /// [`SITES`] row with a coverage claim, which is what census 1 says it does.
    /// The walked region contains **zero** nested items today (MEASURED
    /// 2026-08-28: no `kind: "nested item"` row in [`SITES`], and the comparison
    /// is ordered and element-wise, so the walk emits none either), so descending
    /// costs nothing now and closes the class permanently.
    fn visit_item(&mut self, i: &'ast Item) {
        if let Item::Macro(m) = i {
            self.macro_node(&m.mac);
            return;
        }
        let what = match i {
            Item::Fn(f) => format!("fn {}", f.sig.ident),
            Item::Impl(im) => format!("impl {}", impl_type_name(&im.self_ty)),
            Item::Mod(m) => format!("mod {}", m.ident),
            Item::Struct(s) => format!("struct {}", s.ident),
            Item::Enum(e) => format!("enum {}", e.ident),
            other => key_of(other),
        };
        self.branch("nested item", what);
        visit::visit_item(self, i);
    }
}

// ───────────────────────── the sources ─────────────────────────────────────

fn read(rel: &[&str]) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for p in rel {
        path.push(p);
    }
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn animation_src() -> String {
    read(&["src", "animation.rs"])
}

fn fixture_src() -> String {
    read(&["tests", "ui_a1_zero_alloc.rs"])
}

fn parse(src: &str, what: &str) -> File {
    syn::parse_file(src).unwrap_or_else(|e| {
        panic!(
            "{what} does not parse as Rust: {e}. This census reads the tree, not the bytes — a \
             parse failure is a red and never a skipped construct, which is the whole reason it \
             stopped lexing"
        )
    })
}

/// Every `.rs` file under this crate's `src/`, sorted, so the two crate-wide
/// scans below read one deterministic corpus.
fn crate_src_files() -> Vec<PathBuf> {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.push("src");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in rd {
            let p = entry.expect("invariant: a readable directory entry").path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    assert!(!out.is_empty(), "this crate's src/ contains no .rs file, so both crate-wide scans \
                              below are vacuous");
    out
}

/// The path of a file relative to the crate root, with `/` separators, for a
/// message that reads the same on both platforms.
fn rel_of(p: &std::path::Path) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.strip_prefix(&root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every `impl Drop for T` this crate's `src/` contains, at any nesting depth, as
/// `(T, file)`.
struct DropHunt {
    found: Vec<String>,
}

impl<'ast> Visit<'ast> for DropHunt {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if let Some((_, path, _)) = &i.trait_
            && path.segments.last().is_some_and(|s| s.ident == "Drop")
        {
            self.found.push(impl_type_name(&i.self_ty));
        }
        visit::visit_item_impl(self, i);
    }
}

fn drop_impls_in_crate() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in crate_src_files() {
        let src = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        let file = parse(&src, &rel_of(&p));
        let mut h = DropHunt { found: Vec::new() };
        h.visit_file(&file);
        for ty in h.found {
            out.push((ty, rel_of(&p)));
        }
    }
    out.sort();
    out
}

/// The `core::ops` / `core::cmp` traits an OPERATOR dispatches to.
///
/// An `impl` of any of these turns a `*`, a `[i]`, a `+=`, a `>` or a `*ptr` into a
/// call to a user body, at a site that carries no call syntax. `Drop` is not here:
/// it has its own scan, because drop glue is keyed by a CONSTRUCTION rather than by
/// an operator.
///
/// `PartialEq` and `PartialOrd` are on the list even though they are usually
/// DERIVED, and a `#[derive]` is not an `impl` item — so a derive is invisible here
/// by construction, and only a hand-written comparison body is caught. That is the
/// right direction: a derived comparison has no body to hide a branch in.
const OPERATOR_TRAITS: &[&str] = &[
    "Add",
    "Sub",
    "Mul",
    "Div",
    "Rem",
    "Neg",
    "Not",
    "BitAnd",
    "BitOr",
    "BitXor",
    "Shl",
    "Shr",
    "AddAssign",
    "SubAssign",
    "MulAssign",
    "DivAssign",
    "RemAssign",
    "BitAndAssign",
    "BitOrAssign",
    "BitXorAssign",
    "ShlAssign",
    "ShrAssign",
    "Index",
    "IndexMut",
    "Deref",
    "DerefMut",
    "PartialEq",
    "PartialOrd",
];

/// The traits a user body is reached through by DESUGARING — no operator, and no
/// call syntax either.
///
/// The eleventh adversarial pass found this, and it is the FOURTH member of the
/// no-syntax class. `for x in it` is a walked node, but `Iterator::next` is not
/// reached from any syntax the walk can see: the node costs ONE [`SITES`] row and
/// the body behind it costs nothing. MEASURED 2026-08-28 on the shipped tree —
/// `impl Iterator for ZzIter` carrying a fourth termination condition (`*elapsed`
/// capped at `3_600.0`), driven by `for _zz in (ZzIter { … }) {}` in the opacity
/// arm, left the whole crate at **EXIT=0, 53 targets, 354 passed** once its one
/// `SITES` row was written. The body demonstrably ran: dropping the cap to `0.001`
/// reddened four behavioural tests
/// (`an_over_ceiling_duration_is_accepted_and_never_completes`,
/// `the_anyof_fetch_spans_four_dense_stores`,
/// `the_steady_animating_path_allocates_zero_over_baseline`,
/// `the_tick_bumps_the_sink_on_both_routes`).
///
/// The same shape reaches `From` through `?`, `Display` and `Debug` through a
/// format macro's `{}` / `{:?}`, and `fmt::Write` through `write!` — which expands
/// to `.write_fmt(…)`, a method call the walk cannot see, because it descends into
/// a macro body as an EXPRESSION LIST and `write_fmt` is not one of the
/// expressions.
///
/// **Not on this list, deliberately:**
///
/// * `Deref` / `DerefMut` — they are on [`OPERATOR_TRAITS`] already, and they are
///   also the auto-deref carriers (residue item 14). Listing them twice would
///   double every row.
/// * `Drop` — keyed by a CONSTRUCTION, and it has its own scan.
/// * `Fn` / `FnMut` / `FnOnce` — `f(x)` IS call syntax, and the walk keys it.
/// * `Into`, `Default`, `Clone` — **not because every route to them is keyed.** It is
///   not: a GENERIC BOUND inside a callee reaches them with no syntax at the call
///   site, and the walked region takes exactly that route once — `mem::take`
///   (`animation.rs:808`), which calls `<T as Default>::default()`. What holds, and
///   all that holds, is MEASURED 2026-08-28: that one route resolves to `T =
///   Vec<(EntityId, ComponentId)>`, a std body, and the **22** hand-written
///   `impl Default` in `boyko_ui/src` — including `UiClock`'s at `animation.rs:317`,
///   in the WALKED FILE itself, though not in the walked REGION: its only caller is
///   `UiAnimationPlugin::build`, which is not a `WALKED` name — are none of them
///   reached from the walked region. **That zero is for `Default`/`Clone`/`Into`
///   only, NOT for the generic-bound route as a class**: `Mut::set_if_neq`'s
///   `where T: PartialEq` reaches the hand-written `impl PartialEq for UiVisual`
///   (`boyko_ui/src/components.rs:1153`) from `ui_visual_tick`'s LAST LINE, with no syntax at the
///   site. That one is dispositioned in `OPERATOR_IMPLS` and gated behaviourally by
///   `the_sinks_equality_is_idempotent_under_nan`. The zero
///   is what makes a FIRST one silent, exactly as residue item 15 says of observers.
///   `Clone` and `Into` have **zero** hand-written impls in the crate, so listing
///   them would cost zero rows; `Default` would cost 22. Neither is listed, and the
///   gap is a disclosure rather than a guarantee.
const DESUGARED_TRAITS: &[&str] = &[
    "Iterator",
    "IntoIterator",
    "From",
    "Try",
    "FromResidual",
    "Display",
    "Debug",
    "Write",
];

/// Every `impl <trait> for T` this crate's `src/` contains for a trait in `traits`,
/// at any nesting depth, as `(trait, T)`.
struct TraitImplHunt {
    traits: &'static [&'static str],
    found: Vec<(String, String)>,
}

impl<'ast> Visit<'ast> for TraitImplHunt {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if let Some((_, path, _)) = &i.trait_
            && let Some(seg) = path.segments.last()
            && self.traits.contains(&seg.ident.to_string().as_str())
        {
            self.found.push((seg.ident.to_string(), impl_type_name(&i.self_ty)));
        }
        visit::visit_item_impl(self, i);
    }
}

/// Scans `CARGO_MANIFEST_DIR/src` for `impl`s of any trait in `traits`.
///
/// One function for both no-syntax `impl` censuses, so widening either list is a
/// list edit and never a second scanner to keep in step.
fn trait_impls_in_crate(traits: &'static [&'static str]) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for p in crate_src_files() {
        let src = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        let file = parse(&src, &rel_of(&p));
        let mut h = TraitImplHunt { traits, found: Vec::new() };
        h.visit_file(&file);
        for (tr, ty) in h.found {
            out.push((tr, ty, rel_of(&p)));
        }
    }
    out.sort();
    out
}

fn operator_impls_in_crate() -> Vec<(String, String, String)> {
    trait_impls_in_crate(OPERATOR_TRAITS)
}

fn desugared_impls_in_crate() -> Vec<(String, String, String)> {
    trait_impls_in_crate(DESUGARED_TRAITS)
}

/// Every `#[component(…)]` hook target this crate's `src/` registers, as
/// `(hook, path, file)` — read as TOKENS, not as parsed attributes.
///
/// The token walk is not a preference. The registration that matters is written
/// inside `macro_rules! tween_channel` in `components.rs`, where a `macro_rules!`
/// body is a token tree with `$` metavariables that does not parse as Rust until
/// expansion — so `syn` reports NO attribute there, and an attribute scan over the
/// parsed tree finds only the one hand-written `#[component(on_add = …)]` in the
/// same file. MEASURED: `grep -n ui_visual_sink_on_add tests/ui_a1_source_census.rs`
/// returned zero hits before this landed, while that function ran on every
/// `Tween*` insert in the crate.
fn hook_registrations() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for p in crate_src_files() {
        let src = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        let ts: proc_macro2::TokenStream = src.parse().unwrap_or_else(|e| {
            panic!("{} does not tokenize: {e}", rel_of(&p))
        });
        let mut here = Vec::new();
        scan_component_attrs(ts, &mut here);
        for (hook, path) in here {
            out.push((hook, path, rel_of(&p)));
        }
    }
    out.sort();
    out
}

/// Walks a token stream for `component ( … )` and harvests its hook arguments.
fn scan_component_attrs(ts: proc_macro2::TokenStream, out: &mut Vec<(String, String)>) {
    use proc_macro2::{Delimiter, TokenTree};
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    for (i, t) in toks.iter().enumerate() {
        if let TokenTree::Ident(id) = t
            && *id == "component"
            && let Some(TokenTree::Group(g)) = toks.get(i + 1)
            && g.delimiter() == Delimiter::Parenthesis
        {
            harvest_hooks(g.stream(), out);
        }
        if let TokenTree::Group(g) = t {
            scan_component_attrs(g.stream(), out);
        }
    }
}

/// `on_add = a::b::c` inside a `#[component(…)]` argument list, per top-level
/// comma-separated chunk.
fn harvest_hooks(ts: proc_macro2::TokenStream, out: &mut Vec<(String, String)>) {
    use proc_macro2::{Spacing, TokenTree};
    const HOOK_KINDS: &[&str] = &["on_add", "on_insert", "on_replace", "on_remove"];
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    let mut chunk: Vec<&TokenTree> = Vec::new();
    let mut chunks: Vec<Vec<&TokenTree>> = Vec::new();
    for t in &toks {
        match t {
            TokenTree::Punct(p) if p.as_char() == ',' && p.spacing() == Spacing::Alone => {
                chunks.push(std::mem::take(&mut chunk));
            }
            other => chunk.push(other),
        }
    }
    chunks.push(chunk);

    for c in chunks {
        let Some(TokenTree::Ident(k)) = c.first() else { continue };
        let kind = k.to_string();
        if !HOOK_KINDS.contains(&kind.as_str()) {
            continue;
        }
        let Some(TokenTree::Punct(eq)) = c.get(1) else { continue };
        if eq.as_char() != '=' {
            continue;
        }
        let path: String = c[2..]
            .iter()
            .map(|t| match t {
                TokenTree::Ident(i) => i.to_string(),
                TokenTree::Punct(p) => p.as_char().to_string(),
                other => other.to_string(),
            })
            .collect();
        out.push((kind, path));
    }
}

/// The kernel verbs that register an OBSERVER, in the shapes a crate outside
/// `boyko_ecs` can reach them.
///
/// Enumerated from `boyko_ecs/src/ecs/core/ecs_master/observer_api.rs` and
/// `archetype/archetype_master.rs` (plus `EntityCommands::observe`).
const OBSERVER_VERBS: &[&str] = &[
    "add_observer",
    "observe",
    "observe_entity",
    "observe_entity_event",
    "observe_entity_on_despawn",
    "observe_on_add",
    "observe_on_insert",
    "observe_on_link",
    "observe_on_remove",
    "observe_on_replace",
    "observe_on_unlink",
];

/// Every call in this crate's `src/` to one of [`OBSERVER_VERBS`], as
/// `(verb, file)`.
struct ObserverHunt {
    found: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for ObserverHunt {
    fn visit_expr(&mut self, e: &'ast Expr) {
        match e {
            Expr::MethodCall(m) => {
                let name = m.method.to_string();
                if OBSERVER_VERBS.contains(&name.as_str()) {
                    self.found.insert(name);
                }
            }
            Expr::Call(c) => {
                if let Expr::Path(p) = &*c.func
                    && let Some(seg) = p.path.segments.last()
                    && OBSERVER_VERBS.contains(&seg.ident.to_string().as_str())
                {
                    self.found.insert(seg.ident.to_string());
                }
            }
            _ => {}
        }
        visit::visit_expr(self, e);
    }
}

fn observer_registrations() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in crate_src_files() {
        let src = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        let file = parse(&src, &rel_of(&p));
        let mut h = ObserverHunt { found: BTreeSet::new() };
        h.visit_file(&file);
        for v in h.found {
            out.push((v, rel_of(&p)));
        }
    }
    out.sort();
    out
}

/// The walked region: every [`WALKED`] body, in listed order.
fn walk_region(file: &File) -> Walk {
    let mut defs = Vec::new();
    collect_defs(&file.items, "", &mut defs);
    let mut all = Walk::new("");
    for name in WALKED {
        let d = defs
            .iter()
            .find(|d| d.qualified == *name)
            .unwrap_or_else(|| panic!("WALKED names {name}, which animation.rs does not define"));
        let mut w = Walk::new(name);
        w.visit_block(d.block);
        all.absorb(w);
    }
    all
}

// ───────────────────────── the fixture's own tree ──────────────────────────

/// Every name the fixture BINDS: `let` idents, and `fn` / `const` / `static` /
/// `struct` / `enum` item idents.
struct Binds {
    names: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for Binds {
    fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
        self.names.insert(p.ident.to_string());
        visit::visit_pat_ident(self, p);
    }
    fn visit_item(&mut self, i: &'ast Item) {
        match i {
            Item::Fn(f) => {
                self.names.insert(f.sig.ident.to_string());
            }
            Item::Const(c) => {
                self.names.insert(c.ident.to_string());
            }
            Item::Static(s) => {
                self.names.insert(s.ident.to_string());
            }
            Item::Struct(s) => {
                self.names.insert(s.ident.to_string());
            }
            Item::Enum(e) => {
                self.names.insert(e.ident.to_string());
            }
            _ => {}
        }
        visit::visit_item(self, i);
    }
}

/// Every bare name one body puts in CALLEE position, and — kept strictly apart —
/// every one it puts in VALUE position.
///
/// The separation is the finding of the eighth adversarial pass. The previous
/// spelling folded both into one set through a blanket `Expr::Path` arm, which made
/// [`reachable_from_tests`] a **mention set rather than a call graph**: MEASURED
/// 2026-08-28, replacing the call
/// `witness_tint_only_sink_never_moves(&mut world, &cohorts, sink_before, r);` with
/// `let _ = witness_tint_only_sink_never_moves;` left this census at 7 passed and
/// the whole crate at EXIT=0, while the witness defending FIVE [`SITES`] paths ran
/// zero times. Deleting the same line outright reds — so the neuter was one token
/// wide, the same width as the argument edit that motivated the `Cover::By`
/// rewrite in the first place.
///
/// A call is now `Expr::Call` or `Expr::MethodCall` and nothing else. A path in
/// value position lands in [`Callees::valued`], joins the call graph only through
/// the pinned [`FN_VALUES`] allowlist, and any value reference that is NOT on that
/// list reds [`every_function_value_reference_in_the_fixture_is_pinned`].
/// One path in VALUE position: the name, the callee whose ARGUMENT it is, and
/// which argument.
///
/// The receiver and the index are the tenth adversarial pass's finding. Before
/// them a value reference was recorded as a bare name from anywhere in the body,
/// which made a [`FnValue`] row satisfiable by a MENTION — see [`FN_VALUES`].
type ValueRef = (String, String, usize);

/// The `via` of a value reference that sits in no call's argument list at all.
///
/// A [`FN_VALUES`] row may never carry it
/// ([`every_function_value_reference_in_the_fixture_is_pinned`] asserts so), which
/// is what makes `let _ = witness;` unbuyable.
const NOT_AN_ARGUMENT: &str = "<not a call argument>";

struct Callees {
    /// `Expr::Call`'s last path segment, or an `Expr::MethodCall`'s method name.
    called: BTreeSet<String>,
    /// A path in value position — `armed_floor(build_pair, …)`'s `build_pair`,
    /// with the callee it is an argument OF and its index in that argument list.
    valued: BTreeSet<ValueRef>,
}

impl Callees {
    fn new() -> Self {
        Callees { called: BTreeSet::new(), valued: BTreeSet::new() }
    }

    /// Walks one call's argument list, attributing every BARE PATH argument to the
    /// callee it is passed to and to its index there. Anything else is an ordinary
    /// expression and is walked normally.
    fn call_args(&mut self, via: &str, args: &Punctuated<Expr, Token![,]>) {
        for (i, a) in args.iter().enumerate() {
            if let Expr::Path(p) = a
                && let Some(seg) = p.path.segments.last()
            {
                self.valued.insert((seg.ident.to_string(), via.to_string(), i));
                continue;
            }
            self.visit_expr(a);
        }
    }
}

impl<'ast> Visit<'ast> for Callees {
    fn visit_expr(&mut self, e: &'ast Expr) {
        match e {
            Expr::Call(c) => {
                let via = match &*c.func {
                    // The callee position is a CALL, never a value reference —
                    // descending into it here is what made every call also look
                    // like a mention.
                    Expr::Path(p) => {
                        let n = p
                            .path
                            .segments
                            .last()
                            .map_or_else(String::new, |s| s.ident.to_string());
                        if !n.is_empty() {
                            self.called.insert(n.clone());
                        }
                        n
                    }
                    other => {
                        self.visit_expr(other);
                        "<indirect>".to_string()
                    }
                };
                self.call_args(&via, &c.args);
                return;
            }
            Expr::MethodCall(m) => {
                let via = m.method.to_string();
                self.called.insert(via.clone());
                self.visit_expr(&m.receiver);
                self.call_args(&via, &m.args);
                return;
            }
            Expr::Path(p) => {
                if let Some(seg) = p.path.segments.last() {
                    self.valued.insert((
                        seg.ident.to_string(),
                        NOT_AN_ARGUMENT.to_string(),
                        0,
                    ));
                }
            }
            _ => {}
        }
        visit::visit_expr(self, e);
    }
}

/// One place the fixture names a function WITHOUT calling it at that site.
#[derive(Debug)]
struct FnValue {
    /// The bare name of the body the reference is written in.
    from: &'static str,
    /// The bare name of the function it names.
    to: &'static str,
    /// The callee this value is passed to, as an ARGUMENT — never
    /// [`NOT_AN_ARGUMENT`]. A reference that is not an argument to anything is not
    /// an execution edge and cannot be pinned as one.
    via: &'static str,
    /// Its index in `via`'s argument list. Load-bearing: it is what lets
    /// [`every_function_value_reference_in_the_fixture_is_pinned`] name `via`'s
    /// parameter and check that `via` CALLS it, whenever `via` is a function the
    /// fixture defines.
    arg: usize,
    /// Why the reference is an execution edge in spite of not being a call — i.e.
    /// what does call it.
    why: &'static str,
}

/// The function-VALUE references of `tests/ui_a1_zero_alloc.rs`, in full.
///
/// This is the allowlist that replaced a blanket `Expr::Path` arm. The fixture
/// really does pass builders as `fn` values — `armed_floor(build_pair, …)`,
/// `clock_floor(build_clock_baseline)` — and those are real execution edges: the
/// callee's parameter is a `fn` pointer it immediately calls. Two or three such
/// sites are a list; a blanket arm over every path in the file is not, because it
/// also swallows every ordinary call's own callee path and turns "reachable" into
/// "mentioned".
///
/// [`every_function_value_reference_in_the_fixture_is_pinned`] asserts this list is
/// EXACTLY the set of value references the fixture makes to functions it defines,
/// so a new one is a red with a place to write down why it is an edge — never a
/// silent widening.
///
/// # A row buys back an ARGUMENT, and for four of the twelve it buys back a CALL
///
/// **The tenth adversarial pass laundered a witness through this list.** The row
/// used to be `(from, to, why)`, the reference was satisfied by the `from` body
/// MENTIONING `to` anywhere in value position, and `why` was checked only for
/// non-emptiness. MEASURED 2026-08-28, on the shipped tree: replacing
/// `witness_tint_only_sink_never_moves(&mut world, &cohorts, sink_before, r);`
/// with `let _ = (witness_tint_only_sink_never_moves, sink_before, r);` plus ONE
/// row here printed *"13 pinned value references"* and *"witness
/// witness_tint_only_sink_never_moves defends 7 path(s)"*, census **EXIT=0, 10
/// passed**, whole crate **EXIT=0, 352 passed** — while that witness ran zero
/// times and seven [`SITES`] paths silently lost their only observation.
///
/// Two things changed, and they are not the same strength:
///
/// 1. **`via` and `arg` are mechanical for every row.** The reference must sit in
///    the argument list of a call to the named callee, at the named index. A row
///    may not carry [`NOT_AN_ARGUMENT`], so `let _ = witness;` and
///    `let _ = (witness, …)` are no longer buyable AT ALL — there is no row that
///    describes them.
/// 2. **`via` CALLING its parameter is mechanical only when `via` is a function
///    the fixture defines.** Four rows are: `armed_floor` and `clock_floor` both
///    take `build: fn(&mut EcsMaster) -> Schedule` and both call it. The other
///    eight name a KERNEL verb — `add_system`, `run_system` — whose body is in
///    `boyko_ecs` and is not parsed here, so for those the edge rests on the row's
///    `why` and nothing checks it. The test prints the split rather than implying
///    it is uniform.
///
/// That is why the reachability sentence this list supports is qualified where it
/// used to be absolute — see [`reachable_from_tests`].
const FN_VALUES: &[FnValue] = &[
    FnValue {
        from: "build_baseline",
        to: "noop_exclusive",
        via: "add_system",
        arg: 0,
        why: "passed to ScheduleBuilder::add_system, which stores it and runs it on every \
              Schedule::run of the baseline arm",
    },
    FnValue {
        from: "build_baseline",
        to: "noop_normal",
        via: "add_system",
        arg: 0,
        why: "as noop_exclusive — the baseline arm's second system",
    },
    FnValue {
        from: "build_clock_baseline",
        to: "noop_clock",
        via: "add_system",
        arg: 0,
        why: "the clock gate's whole subtrahend: ui_clock_tick's exact SystemParam signature with \
              an empty body, added to the schedule and run every frame of the baseline arm",
    },
    FnValue {
        from: "build_clock_baseline",
        to: "noop_exclusive",
        via: "add_system",
        arg: 0,
        why: "as in build_baseline — the shape-preserving third system",
    },
    FnValue {
        from: "build_clock_baseline",
        to: "noop_normal",
        via: "add_system",
        arg: 0,
        why: "as in build_baseline — the shape-preserving second system",
    },
    FnValue {
        from: "build_clock_pair",
        to: "noop_exclusive",
        via: "add_system",
        arg: 0,
        why: "as in build_baseline; the clock gate's two arms differ ONLY in the first system",
    },
    FnValue {
        from: "build_clock_pair",
        to: "noop_normal",
        via: "add_system",
        arg: 0,
        why: "as in build_baseline; the clock gate's two arms differ ONLY in the first system",
    },
    FnValue {
        from: "rested_rows",
        to: "count",
        via: "run_system",
        arg: 0,
        why: "a NESTED fn passed to EcsMaster::run_system, which builds it into a one-shot system \
              and runs it immediately. It is the rested cohort's counter, so \
              `witness_rested_cohort_takes_the_all_none_arm` observes nothing without this edge",
    },
    FnValue {
        from: "the_steady_animating_path_allocates_zero_over_baseline",
        to: "build_baseline",
        via: "armed_floor",
        arg: 0,
        why: "passed to `armed_floor(build: fn(&mut EcsMaster) -> Schedule, …)`, whose first \
              statement inside the REPS loop is `build(&mut world)`",
    },
    FnValue {
        from: "the_steady_animating_path_allocates_zero_over_baseline",
        to: "build_pair",
        via: "armed_floor",
        arg: 0,
        why: "as build_baseline — the same `armed_floor` parameter, the arm under measurement",
    },
    FnValue {
        from: "ui_clock_tick_allocates_zero_over_a_same_shape_baseline",
        to: "build_clock_baseline",
        via: "clock_floor",
        arg: 0,
        why: "passed to `clock_floor(build: fn(&mut EcsMaster) -> Schedule)`, which calls \
              `build(&mut world)` once per repetition",
    },
    FnValue {
        from: "ui_clock_tick_allocates_zero_over_a_same_shape_baseline",
        to: "build_clock_pair",
        via: "clock_floor",
        arg: 0,
        why: "as build_clock_baseline. It is ALSO called directly at the end of that test, for the \
              non-vacuity check that the tick actually wrote the clock — so this row is the edge \
              the value reference adds, not the only one",
    },
];

/// The set of fixture functions REACHABLE from a `#[test]` function, by the
/// fixture's own CALL GRAPH — calls, plus the pinned function-value edges.
///
/// # What "reachable" pins, exactly
///
/// This used to carry the sentence *"a witness that exists and is never called is
/// not reachable, and reds exactly as loudly as one that was deleted."* **That was
/// a positive assertion and it was false**, because a [`FN_VALUES`] row was
/// satisfied by a MENTION — see that list for the laundering the tenth adversarial
/// pass measured, which bought back "defends 7 path(s)" for a witness that ran zero
/// times.
///
/// What the edge set actually pins, in three strengths:
///
/// * **A CALL is a call.** `witness_x(…)` in a reachable body is an execution edge,
///   mechanically, and deleting it reds.
/// * **A value reference is an edge only as a pinned ARGUMENT.** It joins this set
///   through a [`FN_VALUES`] row that names the callee it is passed to and the
///   index it sits at; a reference that is not an argument to anything carries
///   [`NOT_AN_ARGUMENT`] and no row may describe it. `let _ = witness;` and
///   `let _ = (witness, …)` are therefore not buyable.
/// * **Whether the receiver CALLS what it was handed is checked for four rows of
///   twelve** — the ones whose `via` the fixture defines. For the eight that name a
///   kernel verb (`add_system`, `run_system`) the edge rests on the row's `why`,
///   which is prose. So a witness handed to a fixture-local receiver that ignores
///   it reds; a witness handed to a kernel verb that ignores it would not, and no
///   sentence here says otherwise.
fn reachable_from_tests(file: &File) -> BTreeSet<String> {
    let mut defs = Vec::new();
    collect_defs(&file.items, "", &mut defs);

    let edges: Vec<(String, BTreeSet<String>)> = defs
        .iter()
        .map(|d| {
            let mut c = Callees::new();
            c.visit_block(d.block);
            let mut out = c.called;
            for v in FN_VALUES {
                if v.from == d.bare
                    && c.valued.contains(&(v.to.to_string(), v.via.to_string(), v.arg))
                {
                    out.insert(v.to.to_string());
                }
            }
            (d.bare.clone(), out)
        })
        .collect();

    let mut frontier: Vec<String> = defs
        .iter()
        .filter(|d| d.attrs.iter().any(|a| a.path().is_ident("test")))
        .map(|d| d.bare.clone())
        .collect();
    let mut seen: BTreeSet<String> = frontier.iter().cloned().collect();

    while let Some(f) = frontier.pop() {
        for (name, callees) in &edges {
            if *name != f {
                continue;
            }
            for c in callees {
                if seen.insert(c.clone()) {
                    frontier.push(c.clone());
                }
            }
        }
    }
    seen
}

/// **Every function-value reference the fixture makes is pinned, with the reason
/// it is an execution edge.**
///
/// The allowlist that replaced the blanket `Expr::Path` arm must not become a
/// blanket by growing silently. This walks the fixture, collects every path in
/// VALUE position whose bare name is a function the fixture DEFINES — WITH the
/// callee it is an argument of and its index there — and compares the set element
/// for element against [`FN_VALUES`].
///
/// Then, for every row whose `via` is a function the fixture itself defines, it
/// names `via`'s parameter at that index and asserts `via`'s body CALLS it. That
/// is the half that is mechanical; the count of rows for which it is not is
/// printed, never rounded away.
#[test]
fn every_function_value_reference_in_the_fixture_is_pinned() {
    let file = parse(&fixture_src(), "tests/ui_a1_zero_alloc.rs");
    let mut defs = Vec::new();
    collect_defs(&file.items, "", &mut defs);
    let fn_names: BTreeSet<String> = defs.iter().map(|d| d.bare.clone()).collect();

    let mut found: Vec<(String, String, String, usize)> = Vec::new();
    for d in &defs {
        let mut c = Callees::new();
        c.visit_block(d.block);
        for (to, via, arg) in &c.valued {
            if fn_names.contains(to) {
                found.push((d.bare.clone(), to.clone(), via.clone(), *arg));
            }
        }
    }
    found.sort();
    found.dedup();

    let mut pinned: Vec<(String, String, String, usize)> = FN_VALUES
        .iter()
        .map(|v| (v.from.to_string(), v.to.to_string(), v.via.to_string(), v.arg))
        .collect();
    pinned.sort();

    let render = |v: &[(String, String, String, usize)]| {
        v.iter()
            .map(|(f, t, via, a)| format!("  [{f}] -> {t}  (argument {a} of `{via}`)"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        found,
        pinned,
        "the fixture's function-VALUE references are not what FN_VALUES pins.\n\nEach of these is a \
         path naming a function the fixture defines, written somewhere other than a call site. \
         `reachable_from_tests` counts calls only, so a value reference is an execution edge ONLY \
         when something calls the value — name the callee it is HANDED TO in `via`, its index in \
         `arg`, and what calls it in `why`. A reference that is an argument to nothing carries \
         `{NOT_AN_ARGUMENT}` and cannot be pinned: it is a MENTION, and a witness must not be \
         reachable through it.\n\nMEASURED 2026-08-28, twice, on the shipped tree. With the \
         blanket `Expr::Path` arm, `let _ = witness_tint_only_sink_never_moves;` kept that witness \
         'reachable' while it ran zero times. With the arm removed but `via` not yet here, \
         `let _ = (witness_tint_only_sink_never_moves, sink_before, r);` PLUS ONE ROW printed '13 \
         pinned value references' and 'defends 7 path(s)', census EXIT=0, crate EXIT=0 / 352 \
         passed — same escape, one row of purchase price.\n\nwalked:\n{}\n\npinned:\n{}",
        render(&found),
        render(&pinned)
    );

    let mut mechanical = 0usize;
    let mut on_prose = 0usize;
    for v in FN_VALUES {
        assert!(!v.why.is_empty(), "FN_VALUES row [{}] -> {} has no reason", v.from, v.to);
        assert_ne!(
            v.via, NOT_AN_ARGUMENT,
            "FN_VALUES row [{}] -> {} claims the reference is an argument to nothing. That is not \
             an execution edge, it is a mention, and the whole point of `via` is that no row can \
             say it",
            v.from, v.to
        );

        // Is the receiver a function this fixture defines? Then the edge is
        // checkable end to end: name its parameter and see whether its body calls
        // it. A receiver with a `self` argument is skipped — the call-site index
        // and the signature index are off by one there, and this census does not
        // guess.
        let Some(recv) = defs.iter().find(|d| d.bare == v.via) else {
            on_prose += 1;
            continue;
        };
        if recv.sig.inputs.iter().any(|a| matches!(a, syn::FnArg::Receiver(_))) {
            on_prose += 1;
            continue;
        }
        let param = recv.sig.inputs.iter().nth(v.arg).unwrap_or_else(|| {
            panic!(
                "FN_VALUES row [{}] -> {} says argument {} of `{}`, which takes {} parameter(s)",
                v.from,
                v.to,
                v.arg,
                v.via,
                recv.sig.inputs.len()
            )
        });
        let syn::FnArg::Typed(pt) = param else { unreachable!("receiver already excluded") };
        let syn::Pat::Ident(id) = &*pt.pat else {
            on_prose += 1;
            continue;
        };
        let name = id.ident.to_string();
        let mut c = Callees::new();
        c.visit_block(recv.block);
        assert!(
            c.called.contains(&name),
            "FN_VALUES row [{}] -> {} hands that function to `{}` as argument {}, and `{}` never \
             CALLS its own parameter `{name}`. The row therefore buys back 'reachable' without \
             buying back execution, which is exactly the laundering this field exists to stop",
            v.from,
            v.to,
            v.via,
            v.arg,
            v.via
        );
        mechanical += 1;
    }

    println!(
        "A1 function-value census: {} pinned value references — {mechanical} whose receiver is a \
         fixture function proved to call its parameter, {on_prose} resting on the row's prose \
         because the receiver is a kernel verb this crate does not parse",
        FN_VALUES.len()
    );
}

// ───────────────────────── census 1: the branch set ────────────────────────

fn render_branches(v: &[Branch]) -> String {
    v.iter()
        .enumerate()
        .map(|(i, (f, k, key))| format!("  {i:>3} [{f}] <{k}> {key}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// **Every control-flow node written syntactically inside the [`WALKED`] bodies is
/// enumerated in [`SITES`], with its coverage.**
///
/// Read that sentence narrowly, because it used to be written wider than the
/// instrument. Until 2026-08-28 it claimed *"every executable branch of the A1/A0
/// systems and their intra-file callees"* — a claim about a RUNTIME REACHABILITY
/// SET, checked by a SYNTACTIC WALK OF NINE BODIES IN ONE FILE. The eighth
/// adversarial pass produced two live branches inside that gap and neither reds:
/// an `impl Drop` on a type the region constructs, and the `on_add` hook the
/// channels register. Both are now walked — by
/// [`every_drop_impl_on_a_type_the_walked_region_constructs_is_walked`] and
/// [`every_hook_registered_on_the_walked_file_is_walked`], which widen [`WALKED`]
/// rather than widen this sentence — and what is left over is written down as a
/// list in this file's module header, under *"the residue"*.
///
/// The comparison is ORDERED and element-wise, so an addition, a deletion, an edit
/// and a MOVE each red, and the message names the index. It is deliberately
/// stricter than a count: a count is satisfied by any permutation, and a branch
/// that moved between two systems is a branch whose coverage claim must be
/// re-argued.
///
/// **A coverage row pins an OBSERVABLE, not an execution count.** [`Cover::By`]
/// asserts its witness runs; it cannot assert the path still executes. MEASURED
/// 2026-08-28: `lerp_rgba8`'s `let mut shift = 0;` → `= 32;` drives the `while`
/// body to ZERO iterations while the node key `shift < 32` does not move (a `let`
/// is not a node), and both this census and the allocation gate stay green — the
/// tint_only cohort's composed `tint_mul` is a constant `0` either way, so its
/// witness observes nothing different. That is a defect of the COVERAGE COLUMN,
/// not an escape: one unrelated test elsewhere in the crate does catch that
/// particular neuter. The per-witness tally this test prints is where the
/// exposure is: a witness defending seventeen paths with four liveness counts is
/// the weakest column in the table, and it is named in the module header.
#[test]
fn the_branch_set_of_the_animation_systems_is_pinned() {
    let src = animation_src();
    let file = parse(&src, "src/animation.rs");
    let walk = walk_region(&file);

    let pinned: Vec<Branch> =
        SITES.iter().map(|s| (s.func.to_string(), s.kind, s.key.to_string())).collect();

    for i in 0..walk.branches.len().min(pinned.len()) {
        assert_eq!(
            walk.branches[i], pinned[i],
            "control-flow node {i} of the walked region is not the one SITES[{i}] pins.\n\
             walked: {:?}\npinned: {:?}\n\n\
             A branch was added, deleted, edited or MOVED inside ui_clock_tick / ui_visual_tick / \
             ui_tween_reap / their intra-file callees. Add or amend the SITES row, and give every \
             path of it a witness in tests/ui_a1_zero_alloc.rs or a written reason for not having \
             one.\n\n\
             IF THE NEW NODE IS A MACRO INVOCATION, READ THIS BEFORE ADDING A ROW. A macro body \
             this walk could not parse is arbitrary code at a site the coverage table says nothing \
             about, and rung A1's whole disclosure is a claim about code at exactly these sites: \
             `invalid_tween_duration` states that EVERY accepted duration_ms above the elapsed \
             ceiling 'yields a row that never completes, is never reaped, and bumps set_if_neq on \
             EVERY frame'. MEASURED 2026-08-28 by the seventh adversarial pass: a `macro_rules! \
             zz_cap` before ui_visual_tick plus ONE line `zz_cap!(opacity, done, entity);` after \
             `let mut composed = *sink;` reaped every opacity tween past 5 s and left the crate at \
             346 passed, EXIT=0 — the disclosure falsified, ADVANCE_PIN and START_PIN both green, \
             because the second termination condition was in neither of them. If your macro can \
             end a tween, the disclosure is now false and must be rewritten in this same edit, \
             together with `an_over_ceiling_duration_is_accepted_and_never_completes` in \
             tests/ui_a1_tween.rs.\n\n\
             full walk:\n{}\n\nfull pin:\n{}",
            walk.branches[i],
            pinned[i],
            render_branches(&walk.branches),
            render_branches(&pinned)
        );
    }
    assert_eq!(
        walk.branches.len(),
        pinned.len(),
        "the walked region has {} control-flow nodes and SITES pins {}. The prefix matched, so the \
         difference is at the end.\n\nfull walk:\n{}\n\nfull pin:\n{}",
        walk.branches.len(),
        pinned.len(),
        render_branches(&walk.branches),
        render_branches(&pinned)
    );

    // Every row must actually SAY something, every cohort must be a name the
    // fixture binds, and every witness must be a fixture function that RUNS.
    let fixture = parse(&fixture_src(), "tests/ui_a1_zero_alloc.rs");
    let mut binds = Binds { names: BTreeSet::new() };
    binds.visit_file(&fixture);
    let live = reachable_from_tests(&fixture);

    let mut covered = 0usize;
    let mut uncovered = 0usize;
    let mut elsewhere = 0usize;
    for site in SITES {
        assert!(
            !site.paths.is_empty(),
            "SITES row [{}] <{}> {} declares no path. A control-flow node has at least one",
            site.func,
            site.kind,
            site.key
        );
        for p in site.paths {
            assert!(
                !p.what.is_empty() && !p.why.is_empty(),
                "SITES row [{}] <{}> {} has a path with an empty field",
                site.func,
                site.kind,
                site.key
            );
            match p.cover {
                Cover::By { cohort, witness } => {
                    covered += 1;
                    assert!(
                        binds.names.contains(cohort),
                        "SITES row [{}] {} claims its path {:?} is driven by the cohort \
                         {cohort:?}, and tests/ui_a1_zero_alloc.rs BINDS no such name. A coverage \
                         claim pointing at a cohort that was renamed or deleted is worse than no \
                         claim: it reads as coverage and is not",
                        site.func,
                        site.key,
                        p.what
                    );
                    assert!(
                        live.contains(witness),
                        "SITES row [{}] {} names {witness:?} as the EXECUTABLE witness for its \
                         path {:?}, and that function is not reachable from any #[test] in \
                         tests/ui_a1_zero_alloc.rs. Either it does not exist, or it exists and \
                         nothing calls it — and a witness nothing calls is exactly the shape this \
                         column replaced: the previous spelling asserted only that a STRING \
                         occurred in the fixture, and the seventh adversarial pass neutered a \
                         cohort by editing ONE ARGUMENT of its starter call while that string sat \
                         untouched. If you are reaching for a FN_VALUES row to make this green, \
                         read that list first: a row buys back an ARGUMENT at a named index, not a \
                         mention, and for eight of the twelve it does not buy back a call",
                        site.func,
                        site.key,
                        p.what
                    );
                }
                Cover::Elsewhere { file, test } => {
                    elsewhere += 1;
                    let other = parse(&read(&["tests", file]), file);
                    let mut od = Vec::new();
                    collect_defs(&other.items, "", &mut od);
                    assert!(
                        od.iter().any(|d| d.bare == test
                            && d.attrs.iter().any(|a| a.path().is_ident("test"))),
                        "SITES row [{}] {} names tests/{file}::{test} as the gate for its path \
                         {:?}, and that file carries no #[test] by that name. A pointer at a gate \
                         that was renamed or deleted reads as coverage and is not",
                        site.func,
                        site.key,
                        p.what
                    );
                }
                Cover::Not => uncovered += 1,
            }
        }
    }

    let witnesses: BTreeSet<&str> = SITES
        .iter()
        .flat_map(|s| s.paths)
        .filter_map(|p| match p.cover {
            Cover::By { witness, .. } => Some(witness),
            Cover::Elsewhere { .. } | Cover::Not => None,
        })
        .collect();
    println!(
        "A1 branch census: {} control-flow nodes, {} paths — {covered} covered by {} executable \
         witnesses, {elsewhere} by a named gate in another binary, {uncovered} explicitly not",
        SITES.len(),
        covered + uncovered + elsewhere,
        witnesses.len()
    );

    // How LOAD-BEARING each witness is. Printed rather than asserted, because
    // there is no defensible threshold — but a column defending seventeen paths
    // with four liveness counts is a fact about this table that no reader should
    // have to reconstruct by hand, and it is where P5's "a covered path driven to
    // zero executions by an edit that changes no node key" lands hardest.
    for w in &witnesses {
        let n = SITES
            .iter()
            .flat_map(|s| s.paths)
            .filter(|p| matches!(p.cover, Cover::By { witness, .. } if witness == *w))
            .count();
        println!("  witness {w} defends {n} path(s)");
    }
}

/// **No branch escapes census 1 by moving into a new local helper — and none
/// escapes by COLLIDING with a name already in scope.**
///
/// Every function `animation.rs` defines whose BARE name a walked body calls must
/// itself be in [`WALKED`], BY QUALIFIED NAME.
///
/// The qualification is the finding. MEASURED 2026-08-28: adding
/// `impl UiTweenScratch { fn advance(&mut self) { … Vec::with_capacity(64) … } }`
/// plus `done.advance();` inside the tick loop left the previous census at EXIT=0,
/// 4 passed — a whole function with an unexecuted allocating branch, called from
/// the tick, enumerated nowhere — because both the call scan and the local-function
/// scan were keyed by bare identifier and both found the free `advance` already in
/// scope. Under qualified names the set of definitions named `advance` is
/// `{advance, UiTweenScratch::advance}` and the second is not in [`WALKED`].
#[test]
fn the_walked_set_is_closed_under_intra_file_calls() {
    let file = parse(&animation_src(), "src/animation.rs");
    let mut defs = Vec::new();
    collect_defs(&file.items, "", &mut defs);
    let walk = walk_region(&file);

    let walked: BTreeSet<&str> = WALKED.iter().copied().collect();
    let called_bare: BTreeSet<String> = walk
        .calls
        .iter()
        .map(|(_, k)| k.trim_start_matches('.').rsplit("::").next().unwrap().to_string())
        .collect();

    for d in &defs {
        if !called_bare.contains(&d.bare) {
            continue;
        }
        assert!(
            walked.contains(d.qualified.as_str()),
            "the walked bodies call something named {:?}, and animation.rs defines {:?}, which is \
             NOT in WALKED. Its branches are therefore invisible to the branch census. Add it to \
             WALKED and add its control-flow nodes to SITES — or, if it is genuinely a different \
             function that merely shares a name, say so by giving it a name that does not collide, \
             because this census cannot resolve a receiver's type and will not pretend to. \
             MEASURED 2026-08-28: `UiTweenScratch::advance` called as `done.advance()` was \
             enumerated NOWHERE while the previous census reported 4 passed",
            d.bare,
            d.qualified
        );
    }
    println!(
        "A1 closure census: {} definitions in animation.rs, {} distinct bare names called from the \
         walked region",
        defs.len(),
        called_bare.len()
    );
}

/// **Every [`WALKED`] name resolves to exactly one definition.**
///
/// The previous census took each function's span by finding an anchor SUBSTRING in
/// stripped source and matching braces, and had to assert the anchor occurred
/// exactly once because a second match made every span ambiguous. A parsed file
/// has no such ambiguity — but a DUPLICATE definition (two `impl` blocks, a
/// `#[cfg]` pair) still would, so it is checked rather than assumed.
#[test]
fn every_walked_name_resolves_to_exactly_one_definition() {
    let file = parse(&animation_src(), "src/animation.rs");
    let mut defs = Vec::new();
    collect_defs(&file.items, "", &mut defs);
    for name in WALKED {
        let n = defs.iter().filter(|d| d.qualified == *name).count();
        assert_eq!(
            n, 1,
            "WALKED names {name}, which animation.rs defines {n} times. Zero means it was renamed \
             and the coverage rows beneath it are about code that no longer exists; two means \
             every claim this census makes about it is ambiguous"
        );
    }
}

/// **No macro invocation in the walked region is opaque to the walk.**
///
/// A macro is always a NODE — that is what makes `zz_cap!(…)` red census 1. But a
/// macro whose body does not parse as a comma-separated expression list hides its
/// contents, so a branch inside it would be invisible even though the invocation
/// is not. Today every macro in the region is `debug_assert!`, whose body parses.
/// If that stops being true this reds, and the choice is explicit: teach the walk
/// that macro's grammar, or give the invocation a SITES row that states what its
/// expansion may and may not do.
#[test]
fn no_macro_in_the_walked_region_is_opaque() {
    let file = parse(&animation_src(), "src/animation.rs");
    let walk = walk_region(&file);
    assert!(
        walk.opaque.is_empty(),
        "these macro invocations in the walked region did not parse as an expression list, so \
         their bodies are invisible to this census: {:?}. The invocation itself is still a node \
         and still reds when it appears — but a branch INSIDE it is not, and the seventh \
         adversarial pass shipped exactly that: `macro_rules! zz_never` expanding to an allocating \
         `if`, census EXIT=0 both before and after the branch was made to execute",
        walk.opaque
    );
}

// ───────────────────────── census 1c: the edges with no syntax ─────────────

/// The `Drop` bodies that join the walked region, qualified — `T::drop` for each
/// `impl Drop for T` in this crate's `src/` whose `T` the walked region
/// constructs.
///
/// EMPTY today, and that is a measurement rather than an omission: this crate's
/// `src/` contains **zero** `impl Drop` (the only one anywhere in the crate is
/// `ArmGuard` in `tests/text_emit_zero_alloc.rs`, which is fixture code and is
/// constructed nowhere near this region), while the walked region's construction
/// set is the handful of types listed by
/// [`every_drop_impl_on_a_type_the_walked_region_constructs_is_walked`]'s own
/// output — none of which owns a destructor.
///
/// The list exists because **drop glue is an execution edge with NO SYNTAX AT THE
/// CALL SITE**, and all three of the censuses above key on syntax at the call
/// site. MEASURED 2026-08-28 by the eighth adversarial pass, before this landed:
///
/// ```ignore
/// struct ZzDropCap<'a> { elapsed: &'a mut f32 }
/// impl Drop for ZzDropCap<'_> {
///     fn drop(&mut self) { if *self.elapsed > 5.0 { *self.elapsed = f32::INFINITY; } }
/// }
/// // in ui_visual_tick's opacity arm:
/// let _zz = ZzDropCap { elapsed: &mut row.elapsed };
/// ```
///
/// — a FOURTH termination condition, at the same site and with the same threshold
/// as the `macro_rules!` form the census already caught, left the crate at EXIT=0,
/// 53 targets, 349 passed, and this census at 7 passed. The macro form reds three
/// times. Same cap, same place, different syntax.
const DROP_GLUE: &[&str] = &[];

/// **Every `Drop` body that runs because the walked region constructs its type is
/// itself walked.**
///
/// The walk collects the types the region constructs — `Expr::Struct` for certain,
/// `T::assoc(…)` as a candidate — and this scans every `.rs` file under `src/` for
/// `impl Drop for T`. The intersection is the drop glue the region executes, and it
/// must be exactly [`DROP_GLUE`], every member of which must also be in [`WALKED`]
/// so its branches get [`SITES`] rows.
///
/// **Its limits, stated rather than discovered.** It cannot see an `impl Drop`
/// written in ANOTHER CRATE (`Vec`'s is the obvious one, and is dispositioned by
/// [`CALLS`] instead), nor one written inside a `macro_rules!` body, nor a type the
/// region obtains from a function whose name says nothing about the type it returns
/// — `let g = make_guard();`. Those are named in the module header's residue list.
#[test]
fn every_drop_impl_on_a_type_the_walked_region_constructs_is_walked() {
    let file = parse(&animation_src(), "src/animation.rs");
    let walk = walk_region(&file);
    let impls = drop_impls_in_crate();

    let required: Vec<String> = impls
        .iter()
        .filter(|(ty, _)| walk.constructs.contains(ty))
        .map(|(ty, _)| format!("{ty}::drop"))
        .collect();
    let pinned: Vec<String> = DROP_GLUE.iter().map(|s| (*s).to_string()).collect();

    assert_eq!(
        required,
        pinned,
        "the walked region constructs a type that owns a `Drop`, and DROP_GLUE does not list its \
         body.\n\nDrop glue is an execution edge with NO SYNTAX AT THE CALL SITE: it is not a \
         branch, not a call and not a macro, so census 1, census 1b and census 2 are all blind to \
         it BY CONSTRUCTION. MEASURED 2026-08-28: an `impl Drop` capping `elapsed` at 5 s, \
         constructed in ui_visual_tick's opacity arm, left the crate at 349 passed and EXIT=0 while \
         falsifying `invalid_tween_duration`'s over-ceiling disclosure — the same cap the \
         `macro_rules!` form reds three times for.\n\nAdd the `drop` body to DROP_GLUE and to \
         WALKED, and give its control-flow nodes SITES rows. If it is defined OUTSIDE \
         src/animation.rs, WALKED cannot name it and the walked region itself has to \
         widen.\n\nconstructed by the region:\n  {}\n\nimpl Drop in this crate's src/:\n{}",
        walk.constructs.iter().cloned().collect::<Vec<_>>().join(", "),
        if impls.is_empty() {
            "  (none)".to_string()
        } else {
            impls.iter().map(|(t, f)| format!("  {t}  [{f}]")).collect::<Vec<_>>().join("\n")
        }
    );

    for g in DROP_GLUE {
        assert!(
            WALKED.contains(g),
            "DROP_GLUE names {g:?}, which is not in WALKED — so its body runs in the region and \
             its branches are in no census"
        );
    }
    println!(
        "A1 drop-glue census: {} impl Drop in src/, {} type(s) constructed by the walked region, \
         {} drop body/bodies walked",
        impls.len(),
        walk.constructs.len(),
        DROP_GLUE.len()
    );
}

/// Every component-hook function this crate registers that is DEFINED IN THE
/// WALKED FILE, and therefore joins the walked region.
///
/// A hook is the other execution edge with no syntax at the call site: nothing in
/// `animation.rs` calls [`ui_visual_sink_on_add`], and nothing ever will — the
/// kernel calls it, from a registration written as an ATTRIBUTE one module over,
/// inside a `macro_rules!` body. MEASURED 2026-08-28, before this landed:
/// `grep -n "ui_visual_sink_on_add" tests/ui_a1_source_census.rs` returned ZERO
/// hits — no `WALKED` entry, no `SITES` row, no `CALLS` row — while the function
/// ran on every `Tween*` insert in the crate and already carried a live,
/// unenumerated `if`. Planting a second `if` plus a heap allocation in that body
/// left the crate at EXIT=0, 53 targets, 349 passed.
///
/// [`ui_visual_sink_on_add`]: crate-internal; see `src/animation.rs`.
const HOOKS: &[&str] = &["ui_visual_sink_on_add"];

/// **Every component hook registered on a function of the walked file is walked,
/// and this crate registers no observers at all.**
///
/// The registration scan reads TOKENS, because the registration that matters is
/// written inside `macro_rules! tween_channel` where a parsed tree has no attribute
/// at all — see [`hook_registrations`].
///
/// The observer half is asserted EMPTY rather than enumerated: `boyko_ui/src`
/// contains zero calls to any of [`OBSERVER_VERBS`] (MEASURED 2026-08-28). If one
/// appears, this reds and its target has to be dispositioned the way a hook's is,
/// because an observer runner is reached exactly the same way — by registration,
/// never by a call site.
#[test]
fn every_hook_registered_on_the_walked_file_is_walked() {
    let anim = parse(&animation_src(), "src/animation.rs");
    let mut defs = Vec::new();
    collect_defs(&anim.items, "", &mut defs);
    let anim_fns: BTreeSet<String> = defs.iter().map(|d| d.bare.clone()).collect();

    let regs = hook_registrations();
    assert!(
        !regs.is_empty(),
        "this crate registers no component hooks at all, which contradicts \
         `#[component(on_add = …)]` in src/components.rs — the token scan is broken, and a broken \
         scan of this shape reports the walked region closed"
    );

    // A registration belongs to the walked file when its target's bare name is
    // defined there AND the path either names the `animation` module or carries no
    // module qualifier. Two functions in two modules may share a bare name, and
    // this census refuses to guess which one an unqualified path meant.
    let mut mine: Vec<String> = Vec::new();
    for (hook, path, file) in &regs {
        let segs: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
        let Some(bare) = segs.last() else { continue };
        if !anim_fns.contains(*bare) {
            continue;
        }
        let qualified_elsewhere = segs.len() >= 2 && segs[segs.len() - 2] != "animation";
        assert!(
            !qualified_elsewhere,
            "the {hook} hook in {file} names {path:?}, whose bare name is ALSO a function \
             src/animation.rs defines. This census cannot resolve which one the path means and \
             will not pretend to — rename one of them"
        );
        mine.push((*bare).to_string());
    }
    mine.sort();
    mine.dedup();

    let pinned: Vec<String> = HOOKS.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        mine,
        pinned,
        "the component hooks registered on functions of src/animation.rs are not what HOOKS \
         pins.\n\nA hook is an execution edge with NO SYNTAX AT THE CALL SITE — the kernel calls \
         it, from an attribute one module over — so census 1b's call graph cannot reach it and \
         census 1's walk never visits it. MEASURED 2026-08-28: `ui_visual_sink_on_add` appeared \
         NOWHERE in this file while running on every Tween* insert, and a second `if` plus a heap \
         allocation planted in its body left the crate at 349 passed, EXIT=0.\n\nAdd it to HOOKS \
         and to WALKED, and enumerate its branches in SITES.\n\nevery #[component(…)] hook \
         registration in this crate:\n{}",
        regs.iter().map(|(h, p, f)| format!("  {h} = {p}  [{f}]")).collect::<Vec<_>>().join("\n")
    );

    for h in HOOKS {
        assert!(
            WALKED.contains(h),
            "HOOKS names {h:?}, which is not in WALKED — so it runs on every insert of the \
             component that registers it, and its branches are in no census"
        );
    }

    let observers = observer_registrations();
    assert!(
        observers.is_empty(),
        "this crate now registers observers: {observers:?}. An observer runner is reached by \
         REGISTRATION, exactly like a hook, so its target needs the same treatment — if it names a \
         function of src/animation.rs, that function joins WALKED and its branches get SITES rows. \
         MEASURED 2026-08-28: boyko_ui/src contained zero calls to any of OBSERVER_VERBS, which is \
         why this is an emptiness assertion rather than a table"
    );

    println!(
        "A1 hook census: {} #[component(…)] hook registration(s) in this crate, {} on the walked \
         file, {} observer registration(s)",
        regs.len(),
        HOOKS.len(),
        observers.len()
    );
}

/// The user operator `impl`s of this crate's `src/`, each with the reason the
/// walked region can or cannot reach it — `(trait, type, why)`.
///
/// **NOT empty, and that was the surprise.** The tenth adversarial pass predicted
/// this list would start empty — *"the crate has zero such impls today, so the set
/// starts empty and a first one reds"* — and the first run of the scan MEASURED
/// **three** (2026-08-28). One of them is live on the walked region's own last
/// line. A prediction of zero is exactly the kind of claim this file exists to
/// check rather than repeat.
const OPERATOR_IMPLS: &[(&str, &str, &str)] = &[
    (
        "PartialEq",
        "UiTextBuffer",
        "src/binding/components.rs, the P4 binding lane's inline string. Not reachable from the \
         walked region: no `UiTextBuffer` value enters `animation.rs` at all — the four channel \
         payloads are `u32` and `f32`, and `UiVisual` has no text field. Its own `&&` and its two \
         slice `Index`es are outside this census by the same rule as any other module's control \
         flow (residue item 1)",
    ),
    (
        "PartialEq",
        "UiVisual",
        "src/components.rs — AD11's BITWISE sink equality, and it IS reachable from the walked \
         region: `sink.set_if_neq(composed)` is the last line of `ui_visual_tick`, and \
         `Mut::set_if_neq`'s equality short-circuit calls exactly this body. The path is a CALL \
         (residue item 1), not an operator expression — the walked region contains no `==` — so \
         `OPERATOR_SITES` is not the list that would have found it and the `.set_if_neq` row in \
         CALLS is. What that row could not say, and this one does: the equality it short-circuits \
         on is written in THIS crate, one module over, and carries FIVE `&&` and FOUR `Index` \
         operations that no census in this file walks, because the walk reads one file. Its \
         behavioural gate is `the_sinks_equality_is_idempotent_under_nan` \
         (tests/ui_a1_tween.rs), which is what makes this a disposition rather than a hole",
    ),
    (
        "PartialOrd",
        "UiName",
        "src/components.rs, a one-line `Some(self.cmp(other))` forwarder for the widget-name \
         ordering. Not reachable from the walked region: no `UiName` value enters `animation.rs`. \
         It carries no branch of its own — the ordering lives in the derived `Ord`",
    ),
];

/// **This crate defines no user operator `impl` that the walked region could
/// dispatch to without anyone noticing.**
///
/// [`OPERATOR_SITES`] pins the operator EXPRESSIONS; this pins the BODIES they can
/// reach. The two are not the same check and each sees what the other cannot:
///
/// * a NEW operator expression reds `OPERATOR_SITES` even when its `impl` lives in
///   another crate, which this scan cannot see;
/// * a NEW `impl` reds THIS even when no expression moved — an existing
///   `from + (to - from) * t` prints identically after its operand type changes
///   under it, so `OPERATOR_SITES` is blind to exactly that.
///
/// It is deliberately over-approximate in the fail-closed direction, like the drop
/// scan's `T::assoc(…)` rule: an `impl Add for UiRect` in `layout.rs` reds this
/// even though `animation.rs` never touches `UiRect`, because nothing here resolves
/// a receiver's type. The answer is a row saying so — the same cost as a
/// cross-module callee's [`CALLS`] row, and the same reason.
///
/// That over-approximation earned its keep on the first run. The pass that
/// prescribed this scan predicted the population would be zero; it is **three**,
/// and one of them — `impl PartialEq for UiVisual` — is the body
/// `sink.set_if_neq(composed)` short-circuits on, with five `&&` and four `Index`
/// operations that no census in this file walks. See [`OPERATOR_IMPLS`].
#[test]
fn every_operator_impl_in_the_crate_is_dispositioned() {
    let found = operator_impls_in_crate();
    let found_keys: Vec<(String, String)> =
        found.iter().map(|(tr, ty, _)| (tr.clone(), ty.clone())).collect();
    let pinned: Vec<(String, String)> = OPERATOR_IMPLS
        .iter()
        .map(|(tr, ty, _)| ((*tr).to_string(), (*ty).to_string()))
        .collect();

    assert_eq!(
        found_keys,
        pinned,
        "the user operator `impl`s in boyko_ui/src are not what OPERATOR_IMPLS pins.\n\nAn \
         operator-trait `impl` is an execution edge with NO SYNTAX AT THE CALL SITE — the third \
         of that class, after drop glue and hook registration. A `*`, a `[i]`, a `+=` or a `>` in \
         the walked region becomes a call to this body, and that body is in no census. MEASURED \
         2026-08-28, on the shipped tree: `impl std::ops::Mul<(&mut f32, f32)> for ZzMulCap` \
         carrying a FOURTH termination condition, dispatched from the opacity arm, left the whole \
         crate at EXIT=0, 53 targets, 352 passed with every headline count bit-identical — 32 \
         nodes / 50 paths, 50 call sites, 31 callees, 0 impl Drop. An `impl Index` gave the same \
         result.\n\nIf the walked region can reach it, put the type in WALKED and its \
         control-flow nodes in SITES, exactly as a hook or a `Drop` body would be. If it cannot, \
         add a row and say why — this scan cannot resolve a receiver's type and will not pretend \
         to.\n\nfound:\n{}\n\npinned:\n{}",
        found
            .iter()
            .map(|(tr, ty, f)| format!("  impl {tr} for {ty}  [{f}]"))
            .collect::<Vec<_>>()
            .join("\n"),
        pinned.iter().map(|(tr, ty)| format!("  impl {tr} for {ty}")).collect::<Vec<_>>().join("\n")
    );

    for (tr, ty, why) in OPERATOR_IMPLS {
        assert!(!why.is_empty(), "OPERATOR_IMPLS row `impl {tr} for {ty}` has no reason");
    }
    println!(
        "A1 operator-impl census: {} user operator impl(s) in src/ over {} operator traits",
        found.len(),
        OPERATOR_TRAITS.len()
    );
}

/// The user `impl`s of this crate's `src/` that DESUGARING reaches, each with the
/// reason the walked region can or cannot reach it — `(trait, type, why)`.
///
/// Sibling of [`OPERATOR_IMPLS`], in a table of its own for the same reason
/// [`OPERATOR_SITES`] is: a class gets its own list rather than being folded into
/// somebody else's, so no existing count moves and no existing sentence goes stale.
///
/// **MEASURED 2026-08-28, with this table empty: the population is ONE.**
/// `Iterator`, `IntoIterator`, `From`, `Try`, `FromResidual`, `Display` and `Debug`
/// are each **zero** in `boyko_ui/src` — which is why widening was taken over a
/// residue item. It cost one row, and that row is disposable on a fact rather than
/// on prose.
const DESUGARED_IMPLS: &[(&str, &str, &str)] = &[(
    "Write",
    "UiTextBuffer",
    "src/binding/components.rs:134 — `impl core::fmt::Write`, the P4 binding lane's inline \
     string. (The scan keys on the LAST path segment, so a `std::io::Write` would land in this \
     same row shape; both are reached by `write!` / `writeln!`, which expand to `.write_fmt(…)` \
     — a method call the walk cannot see, because it descends into a macro body as an EXPRESSION \
     LIST and `write_fmt` is not one of the expressions.)\n\nNot reachable from the walked \
     region, on two independent counts, both measured 2026-08-28: (1) no `UiTextBuffer` value \
     enters `animation.rs` at all — `grep -c UiTextBuffer src/animation.rs` is 0, the four \
     channel payloads are `u32` and `f32`, and `UiVisual` has no text field; (2) the walked \
     region contains no format macro of any kind. Its ONLY macro nodes are the two \
     `debug_assert!`s that SITES already pins (`ui_clock_tick`, `advance`), and both carry a \
     plain string with no interpolation — so no user `Display`, `Debug` or `Write` body is \
     reached from the walked region even on the panic path. A first `write!` or `{}` \
     interpolation there is a new macro node, which reds SITES, and this row is what says the \
     body behind it would not.",
)];

/// **This crate defines no `impl` that DESUGARING can reach from the walked region
/// without anyone noticing.**
///
/// The FOURTH execution edge with no syntax at the call site, after drop glue, hook
/// registration and operator dispatch. See [`DESUGARED_TRAITS`] for the measurement
/// and for the traits deliberately left off the list.
///
/// Over-approximate in the fail-closed direction, exactly like the operator scan
/// and the drop scan: nothing here resolves a receiver's type, so an `impl` on a
/// type `animation.rs` never touches still gets a row saying so. That is the same
/// cost as a cross-module callee's [`CALLS`] row, and the same reason.
#[test]
fn every_desugared_trait_impl_in_the_crate_is_dispositioned() {
    let found = desugared_impls_in_crate();
    let found_keys: Vec<(String, String)> =
        found.iter().map(|(tr, ty, _)| (tr.clone(), ty.clone())).collect();
    let pinned: Vec<(String, String)> = DESUGARED_IMPLS
        .iter()
        .map(|(tr, ty, _)| ((*tr).to_string(), (*ty).to_string()))
        .collect();

    assert_eq!(
        found_keys,
        pinned,
        "the desugaring-reachable `impl`s in boyko_ui/src are not what DESUGARED_IMPLS pins.\n\n\
         Desugaring is an execution edge with NO SYNTAX AT THE CALL SITE — the fourth of that \
         class. `for x in it` calls `Iterator::next`, `?` calls `From::from`, a format \
         macro's `{{}}` calls `Display::fmt`, `write!` calls `write_fmt`, and NONE of those \
         names appears at the \
         site.\n\nIf the walked region can reach it, put the type in WALKED and its control-flow \
         nodes in SITES, exactly as a hook, a `Drop` body or an operator `impl` would be. If it \
         cannot, add a row and say why.\n\nfound:\n{}\n\npinned:\n{}",
        found
            .iter()
            .map(|(tr, ty, f)| format!("  impl {tr} for {ty}  [{f}]"))
            .collect::<Vec<_>>()
            .join("\n"),
        pinned.iter().map(|(tr, ty)| format!("  impl {tr} for {ty}")).collect::<Vec<_>>().join("\n")
    );

    for (tr, ty, why) in DESUGARED_IMPLS {
        assert!(!why.is_empty(), "DESUGARED_IMPLS row `impl {tr} for {ty}` has no reason");
    }
    println!(
        "A1 desugaring-impl census: {} user impl(s) in src/ over {} desugared traits",
        found.len(),
        DESUGARED_TRAITS.len()
    );
}

// ───────────────────────── census 1b: the calls ────────────────────────────

/// **Every callee of the walked region is enumerated, so a branch hidden inside
/// one is dispositioned rather than discovered.**
///
/// The walk cannot look inside another crate. `Mut::set_if_neq`'s equality
/// short-circuit and `Vec::push`'s reallocation are real paths of the armed window
/// living behind a call, and enumerating the calls is what makes them enumerable.
/// A new callee reds this and its internal control flow gets a row.
///
/// Keys are RESOLVED SHAPES: `TweenTint::component_id` for a path call, `.push`
/// for a method call. `advance` and `.advance` are different rows and neither can
/// stand in for the other.
#[test]
fn every_callee_of_the_walked_region_is_enumerated() {
    let file = parse(&animation_src(), "src/animation.rs");
    let walk = walk_region(&file);

    let found: BTreeSet<String> = walk.calls.iter().map(|(_, k)| k.clone()).collect();
    let pinned: BTreeSet<String> = CALLS.iter().map(|c| c.name.to_string()).collect();

    let added: Vec<&String> = found.difference(&pinned).collect();
    let gone: Vec<&String> = pinned.difference(&found).collect();
    assert!(
        added.is_empty(),
        "the walked bodies call {added:?}, which CALLS does not enumerate. Add a row saying what \
         it is and — if it is outside this file — whether its INTERNAL branches bear on the armed \
         window's coverage. That is not pedantry: `.set_if_neq`'s equality short-circuit is \
         exactly such a branch, is taken every frame by an ordinary UI node, and was executed zero \
         times by this gate's fixture until 2026-08-28.\n\nfull call walk:\n{}",
        walk.calls
            .iter()
            .enumerate()
            .map(|(i, (f, k))| format!("  {i:>3} [{f}] {k}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        gone.is_empty(),
        "CALLS enumerates {gone:?}, which the walked bodies no longer call. A list that keeps a \
         row for a call that is gone reads as coverage it does not have — delete the row in the \
         same edit that deleted the call"
    );

    let walked_bare: BTreeSet<String> =
        WALKED.iter().map(|w| w.rsplit("::").next().unwrap().to_string()).collect();
    for c in CALLS {
        let bare = c.name.trim_start_matches('.').rsplit("::").next().unwrap();
        assert_eq!(
            c.local,
            walked_bare.contains(bare),
            "CALLS row {:?} marks local = {}, and the WALKED set says otherwise",
            c.name,
            c.local
        );
        assert!(!c.note.is_empty(), "CALLS row {:?} has no note", c.name);
    }
    println!(
        "A1 callee census: {} distinct callees — {} intra-file, {} external",
        CALLS.len(),
        CALLS.iter().filter(|c| c.local).count(),
        CALLS.iter().filter(|c| !c.local).count()
    );
}

/// **Every call SITE of the walked region is pinned, in order.**
///
/// [`CALLS`] is a set, and a set cannot see a SECOND call to a callee it already
/// lists. The sixth pass wrote that residue down as "a new straight-line call to a
/// callee already on the list"; the seventh drove through it —
/// `let _ = dt_real > 1.0e30 && { done.done.push((entity, TweenTint::component_id())); true };`
/// smuggles a spurious reap entry for a channel that did not finish, and BOTH of
/// its callees were already enumerated. This census reds on the added element
/// whether or not it carries a branch and whatever it is nested inside.
#[test]
fn every_call_site_of_the_walked_region_is_pinned() {
    let file = parse(&animation_src(), "src/animation.rs");
    let walk = walk_region(&file);

    let found: Vec<(String, String)> = walk.calls.clone();
    let pinned: Vec<(String, String)> =
        CALL_SITES.iter().map(|(f, k)| ((*f).to_string(), (*k).to_string())).collect();
    let render = |v: &[(String, String)]| {
        v.iter()
            .enumerate()
            .map(|(i, (f, k))| format!("  {i:>3} [{f}] {k}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    for i in 0..found.len().min(pinned.len()) {
        assert_eq!(
            found[i], pinned[i],
            "call site {i} of the walked region is not the one CALL_SITES[{i}] pins.\nwalked: \
             {:?}\npinned: {:?}\n\nA call was added, deleted, edited or MOVED. If it is a \
             `done.done.push(…)`, note that the four the tick ships are each inside a `None` arm \
             of a `match advance(…)` and each pushes the channel that just finished — a fifth push \
             anywhere else records a completion that did not happen.\n\nfull walk:\n{}\n\nfull \
             pin:\n{}",
            found[i],
            pinned[i],
            render(&found),
            render(&pinned)
        );
    }
    assert_eq!(
        found.len(),
        pinned.len(),
        "the walked region has {} call sites and CALL_SITES pins {}. The prefix matched, so the \
         difference is at the end.\n\nfull walk:\n{}\n\nfull pin:\n{}",
        found.len(),
        pinned.len(),
        render(&found),
        render(&pinned)
    );
    println!("A1 call-site census: {} call sites in the walked region", CALL_SITES.len());
}

/// **Every OPERATOR EXPRESSION of the walked region is pinned, in order.**
///
/// An overloaded operator is a call whose call site has no call syntax. See
/// [`OPERATOR_SITES`] for the two probes that walked through every other census in
/// this file at a cost of ZERO rows.
#[test]
fn every_operator_expression_of_the_walked_region_is_pinned() {
    let file = parse(&animation_src(), "src/animation.rs");
    let walk = walk_region(&file);

    let pinned: Vec<(String, &str, String)> = OPERATOR_SITES
        .iter()
        .map(|(f, k, key)| ((*f).to_string(), *k, (*key).to_string()))
        .collect();
    let render = |v: &[(String, &str, String)]| {
        v.iter()
            .enumerate()
            .map(|(i, (f, k, key))| format!("  {i:>3}     (\"{f}\", \"{k}\", \"{key}\"),"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let found: Vec<(String, &str, String)> = walk.ops.clone();

    for i in 0..found.len().min(pinned.len()) {
        assert_eq!(
            found[i], pinned[i],
            "operator expression {i} of the walked region is not the one \
             OPERATOR_SITES[{i}] pins.\nwalked: {:?}\npinned: {:?}\n\n\
             An operator expression was added, deleted, edited or MOVED. If the new one's operand \
             type has a user `impl`, it CALLS that impl's body and that body is in no census: add \
             the type to WALKED and its control-flow nodes to SITES, exactly as a hook or a `Drop` \
             body would be. MEASURED 2026-08-28: an `impl Mul` carrying a fourth termination \
             condition, dispatched from the opacity arm, left the whole crate at EXIT=0, 53 \
             targets, 352 passed with every other headline count bit-identical.\n\n\
             full walk:\n{}\n\nfull pin:\n{}",
            found[i],
            pinned[i],
            render(&found),
            render(&pinned)
        );
    }
    assert_eq!(
        found.len(),
        pinned.len(),
        "the walked region has {} operator expressions and OPERATOR_SITES pins {}. The prefix \
         matched, so the difference is at the end.\n\nfull walk:\n{}\n\nfull pin:\n{}",
        found.len(),
        pinned.len(),
        render(&found),
        render(&pinned)
    );
    println!(
        "A1 operator census: {} operator expressions in the walked region",
        OPERATOR_SITES.len()
    );
}

// ───────────────────────── census 2: the termination condition ─────────────

/// **The termination condition is exactly what `invalid_tween_duration`'s
/// disclosure describes, and nothing more.**
///
/// The disclosure is a claim about the SYSTEM — *every* accepted `duration_ms`
/// above the `elapsed` ceiling yields a row that never completes. A test that
/// samples three durations refutes only a bound that happens to fall between two
/// of them, and the sixth adversarial pass demonstrated both halves of that gap
/// with code that left 1427 tests at EXIT=0:
///
/// | evasion | where | why the point-sampled gates missed it |
/// |---|---|---|
/// | `&& *elapsed < 3_600.0` | `advance` | nothing downstream of the door is sampled |
/// | `if duration_ms > 5.0e8 && duration_ms <= 1.0e9 { 1.0e4 }` | after the guard | all three sampled arms preserved bit-exactly |
/// | `.min(1e4)` | after the guard | caught only by the datum assertion, which is the OTHER half |
/// | `&& duration_ms < 1e20` | in the guard | caught only by the `f32::MAX` arm |
///
/// So this asserts the predicate ITSELF, as printed tokens. All four evasions red
/// it, and so does a fifth nobody has thought of.
///
/// It does NOT replace `an_over_ceiling_duration_is_accepted_and_never_completes`
/// (`tests/ui_a1_tween.rs`). That gate proves the BEHAVIOUR the disclosure
/// describes actually happens; this proves no second termination condition was
/// added anywhere the behaviour gate does not look. A pin without a behavioural
/// gate would be satisfied by source that does not run.
///
/// **Neither pin can see a termination condition written OUTSIDE these two
/// spans** — the seventh pass put one in `ui_visual_tick` via a macro and left
/// both green. That one is census 1's, whose failure message says so.
#[test]
fn the_termination_condition_is_pinned_to_the_disclosure() {
    let raw = animation_src();
    let file = parse(&raw, "src/animation.rs");
    let mut defs = Vec::new();
    collect_defs(&file.items, "", &mut defs);

    let advance = defs
        .iter()
        .find(|d| d.qualified == "advance")
        .expect("invariant: animation.rs defines `advance`");
    assert_eq!(
        advance.pin(),
        squeeze(&ADVANCE_PIN.join(" ")),
        "`advance`'s printed tokens are not what ADVANCE_PIN pins.\n\nA tween stops when, and only \
         when, `t = elapsed * inv_duration` fails `< 1.0`, and there is no other way for it to \
         stop. If you added a cap, a clamp, a second conjunct or an early return, \
         `invalid_tween_duration`'s disclosure — 'every accepted duration_ms STRICTLY above that \
         ceiling yields a row that never completes' — is now FALSE, and it must be rewritten in \
         this same edit, together with \
         `an_over_ceiling_duration_is_accepted_and_never_completes` in tests/ui_a1_tween.rs. \
         MEASURED 2026-08-28: `&& *elapsed < 3_600.0` on the last line left 1427 tests at EXIT=0 \
         while falsifying that sentence verbatim"
    );

    let helpers = file
        .items
        .iter()
        .find_map(|i| match i {
            Item::Macro(m) if m.ident.as_ref().is_some_and(|id| id == "tween_helpers") => Some(m),
            _ => None,
        })
        .expect("invariant: animation.rs defines `macro_rules! tween_helpers`");
    let printed =
        squeeze(&empty_strings(&strip_doc_attrs(&helpers.mac.tokens.to_string())));
    assert_eq!(
        printed,
        squeeze(&START_PIN.join(" ")),
        "`tween_helpers!` is not what START_PIN pins.\n\nThe door tests finiteness and the sign \
         bit and NOTHING ELSE, and it stores `1000.0 / duration_ms` computed from the value PASSED \
         IN. A clamp, a sub-range rewrite or an extra conjunct between the guard and the insert \
         falsifies `invalid_tween_duration`'s disclosure. MEASURED 2026-08-28, the sub-range \
         rewrite `if duration_ms > 5.0e8 && duration_ms <= 1.0e9 {{ 1.0e4 }} else \
         {{ duration_ms }}` preserved all three arms of the datum assertion BIT-EXACTLY and left \
         the crate green while every accepted duration in (5.24288e8, 1e9] completed in ten \
         seconds"
    );

    let prose = doc_text(&raw);
    for fragment in DISCLOSURE {
        let want = squeeze(fragment);
        assert!(
            prose.contains(&want),
            "the disclosure fragment {fragment:?} is gone from animation.rs. The two pins above \
             red when the PREDICATE changes and the disclosure does not; this reds the other \
             direction — a doc deletion that leaves two green pins guarding a claim nobody makes \
             any more"
        );
    }
}

/// Silences the unused-import warning `ItemFn` would otherwise raise while the
/// definition table borrows its parts rather than the whole.
const _: Option<fn(&ItemFn)> = None;
