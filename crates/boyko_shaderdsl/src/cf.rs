//! The CONTROL-FLOW backend axis (`Cf`) + its `Eval` instantiation (`EvalCf`).
//!
//! Increment 1 of the "whole shader in Rust" path: the field/normal/decode/cubic
//! leaves ([`crate::field`] / [`crate::normal`] / [`crate::brick`]) are pure
//! straight-line expressions, so a single [`FieldScalar`](crate::FieldScalar) backend
//! axis sufficed. A MARCHER function (the first being
//! [`crate::brick::dist_to_brick_exit_body`]) additionally has runtime/unrolled
//! control flow — an `[unroll]` `for` loop with a data-dependent `continue` — which a
//! by-value scalar cannot express. `Cf` is the control-flow backend axis: a ZST marker
//! that supplies the control-flow COMBINATORS as associated functions AND fixes the value
//! type via [`Cf::Scalar`] (so the body is generic over `C: Cf` alone). Instantiated two
//! ways:
//!
//! - [`EvalCf`] (here, always compiled) — REAL host control flow: [`unroll_for`] is a
//!   real `for`, [`if_`] a real `if`, [`cont`] the loop-continue token. This is the
//!   CPU oracle the brick-exit eval sweep locks (it is a pure host `for`/`if` ZST; no
//!   physics-reachable code calls it).
//! - `EmitCf` ([`crate::emit`], `feature = "emit"`) — each combinator RECORDS a
//!   statement into the emit STMT IR; the printer walks it into the `[unroll]`/`for`/
//!   `continue` HLSL. The ENTIRE emit-recorder surface (`EmitCf` + the `Stmt`/`Block`
//!   IR + the recorder thread-local) is whole-module `#[cfg(feature = "emit")]`-gated,
//!   so a non-emit (physics) build cannot even NAME it (firewall option B).
//!
//! # The continue token — `core::ops::ControlFlow`
//!
//! A data-dependent `continue` is propagated out of the loop-body closure with the `?`
//! operator over [`Flow`] (`= core::ops::ControlFlow<LoopOp>`). The body writes
//! `C::if_(cond, || C::cont())?;`: when `cond` holds, `if_` returns the `cont` token
//! ([`ControlFlow::Break`]`(`[`LoopOp::Continue`]`)`) and `?` early-returns it from the
//! FnMut, so any LIVE TAIL mutation AFTER the continue point does NOT run — matching the
//! host `continue`. [`unroll_for`] maps that `Break(Continue)` to a real `continue`
//! (Eval) / a `Stmt::Continue` (Emit). `core::ops::ControlFlow` is `#[must_use]`,
//! `no_std`, and its `Try` impl is STABLE for use with `?` (implementing `Try` for a
//! custom type is nightly; reusing the std type is not), so the Eval path stays stable +
//! `no_std`/alloc-free.

use crate::scalar::FieldScalar;

/// Which loop control transfer a propagated [`Flow`] `Break` carries.
///
/// A 2-token scheme so the loop-control payload type is STABLE before Increment 4 adds
/// a real `break`: today only [`Continue`](LoopOp::Continue) is produced (by [`Cf::cont`]),
/// but the payload already names [`Break`](LoopOp::Break) so wiring the `break` token in
/// Inc 4 is a leaf addition (a `brk()` combinator + the `Break` arm) — no `Flow` type
/// change, hence no re-audit of every body's `?` propagation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoopOp {
    /// Skip the rest of THIS iteration (the loop `continue`) — the only token produced
    /// in Increment 1.
    Continue,
    /// Exit the loop (the loop `break`) — carried now for type stability; PRODUCED by
    /// Increment 4 (no combinator emits it yet).
    Break,
}

/// The control-flow propagation token threaded through a loop body by `?`.
///
/// `Continue(())` = FALL THROUGH to the next statement (the common case); `Break(op)` =
/// a loop control transfer carrying a [`LoopOp`] (today only [`LoopOp::Continue`], the
/// loop-continue token). The naming is the std `ControlFlow`'s, whose semantics are
/// INVERTED from a loop's `continue`/fall-through — the [`Cf`] combinators ([`Cf::cont`]
/// / `if_`) hide that, so a body never spells the std variants directly.
pub type Flow = core::ops::ControlFlow<LoopOp>;

/// The control-flow backend axis — a ZST marker supplying the unrolled/branching
/// combinators a marcher body folds, with its value type FIXED by [`Scalar`](Self::Scalar).
///
/// A marcher body is `fn body<C: Cf>(...) -> C::Scalar`: `C::Scalar` carries the
/// arithmetic, `C` carries the control flow. Each combinator is an associated function
/// (no `self`) because the backends are ZSTs — the dispatch is entirely at the type
/// level (zero runtime cost on Eval; pure recording on Emit).
///
/// Fixing the value type per backend (`Scalar = f32` on [`EvalCf`], `Scalar = Emit` on
/// `EmitCf`) is what lets the Emit combinators take/return the CONCRETE `Emit` handle
/// directly — `init.0` / `vec[i].0` are plain field accesses, no value-erasing transmute
/// bridge.
pub trait Cf {
    /// The value type the combinators thread — `f32` on [`EvalCf`] (the Eval oracle), the
    /// SSA-node handle `Emit` on `EmitCf` (the HLSL recorder). Fixing it per backend
    /// collapses the old `S: FieldScalar` value axis into `Cf`, so the body is generic
    /// over `C` ALONE.
    type Scalar: FieldScalar;

    /// A MUTABLE LOCAL of value type [`Scalar`](Self::Scalar) (the `exit` accumulator).
    /// Eval stores the scalar in a [`core::cell::Cell`] (interior mutability — the loop
    /// body holds `&var` and assigns through it without `&mut` aliasing); Emit records a
    /// `DeclVar`/`Assign` over a named HLSL local (a `u32` name handle). The body holds
    /// the var by value and passes `&var` to get/set.
    ///
    /// Today this holds a single [`Scalar`](Self::Scalar)-typed local (`exit`). Increment
    /// 3's `brick_cell_class` needs DIFFERENT-typed locals (a `uint` class + a `float3`
    /// cell_min); that is a separate typed-var extension Inc 3 adds (e.g. a second assoc
    /// type or a per-type handle), NOT built now.
    type Var;

    /// The unrolled-loop INDUCTION VARIABLE handle passed to the [`unroll_for`](Self::
    /// unroll_for) body. `usize` on Eval (the real `for` counter); an emit iv SSA node on
    /// `EmitCf` (so [`index`](Self::index) records `vec[a]` against the iv's printed
    /// name). Carried as an associated type so the body's per-axis index spelling is
    /// backend-routed.
    type Iv: Copy;

    /// Indexes a `float3` PARAMETER by the unroll iv — `vec[iv]`. Eval reads `vec[iv]`
    /// directly (a real array index); Emit records `Node::VecIndex` printed as
    /// `<name>[a]` (the parameter NAME carried by the seeded `[Emit; 3]`, the iv name by
    /// the loop). Routed through `Cf` (not a plain `vec[a]`) because the iv type differs
    /// per backend and the Emit spelling must be the dynamic `vec[a]`, not three distinct
    /// `vec_0`/`vec_1`/`vec_2` handles.
    fn index(vec: [Self::Scalar; 3], iv: Self::Iv) -> Self::Scalar;

    /// A NAMED float constant — `val` on Eval, the SYMBOL `sym` on Emit (so the emitted
    /// HLSL spells `BRICK_EXIT_EPS`, not `1.0e-4`). Eval returns `Scalar::lit(val)`; Emit
    /// records `Node::NamedLit { sym, val }` (registering `sym` in the printer's
    /// named-literal table).
    fn named_lit(sym: &'static str, val: f32) -> Self::Scalar;

    /// Forces `x` to MATERIALIZE as a named `float tN` temp in PROGRAM ORDER (Emit);
    /// IDENTITY on Eval (no temp — the value flows directly). The body wraps a
    /// sub-expression in `temp(...)` to pin the emitted shape to the committed HLSL's
    /// materialization choice (which subexpressions become `tN` locals vs inline) — that
    /// choice is the original author's, not derivable from use-count, so the body
    /// single-sources it EXPLICITLY. An UN-wrapped expression (e.g. `abs(t0)`, `p[a]`)
    /// stays inline. This is what makes the generator's text re-DXC byte-identical to the
    /// committed `.comp.spv` (the .spv id allocation follows the temp structure).
    fn temp(x: Self::Scalar) -> Self::Scalar;

    /// Declares a mutable local named `name` initialized to `init`, returning its
    /// [`Var`](Self::Var) handle. Eval boxes `init` into a `Cell` (the `name` is ignored —
    /// a `Cell` has no printed identity); Emit records `Stmt::DeclVar` (the `rhs` is
    /// `init`'s emit expression) and uses `name` as the HLSL local's name. Threading the
    /// name (vs a hardcoded `"exit"`) keeps two distinct locals printing distinct names
    /// once Inc 3 declares more than one.
    fn decl_var(name: &'static str, init: Self::Scalar) -> Self::Var;

    /// Reads the CURRENT value of a mutable local. Eval returns the `Cell`'s value; Emit
    /// returns an emit handle spelling the variable's NAME (`exit`, a named mutable local,
    /// not an SSA temp).
    fn get_var(v: &Self::Var) -> Self::Scalar;

    /// Assigns a mutable local. Eval `set`s the `Cell`; Emit records `Stmt::Assign`.
    fn set_var(v: &Self::Var, val: Self::Scalar);

    /// Runs an UNROLLED `for iv in 0..n` over `body`. `attr` is the loop attribute the
    /// HLSL emitter spells (`"[unroll]"`); Eval ignores it. `body` returns a [`Flow`]:
    /// a [`ControlFlow::Break`](core::ops::ControlFlow::Break)`(`[`LoopOp::Continue`]`)`
    /// from `body` (propagated by `?` over a [`Cf::cont`]) is the loop CONTINUE (Eval maps
    /// it to a real `continue`; Emit records `Stmt::Continue`), so a live tail after the
    /// continue is skipped. (Increment 4 adds the [`LoopOp::Break`] arm.)
    fn unroll_for<F: FnMut(Self::Iv) -> Flow>(attr: &'static str, n: usize, body: F);

    /// A `if cond { body() }` — `body` returns a [`Flow`] (typically a [`Cf::cont`]).
    /// When `cond` is false, FALLS THROUGH ([`ControlFlow::Continue`]); when true,
    /// returns whatever `body` yields (so a `C::if_(cond, || C::cont())?` early-returns
    /// the continue). Eval evaluates the real `if`; Emit records `Stmt::If`.
    ///
    /// `cond` is the [`Scalar`](Self::Scalar)'s own [`Mask`](FieldScalar::Mask) (e.g. the
    /// result of [`FieldScalar::le`]), so a body's `dir.abs().le(eps)` feeds `if_`
    /// directly with no separate `Cf::Mask` axis to keep in sync.
    fn if_<F: FnOnce() -> Flow>(cond: <Self::Scalar as FieldScalar>::Mask, body: F) -> Flow;

    /// The loop-CONTINUE token (a [`ControlFlow::Break`]`(`[`LoopOp::Continue`]`)`) —
    /// `?`-propagated out of the loop body to skip the rest of the iteration.
    fn cont() -> Flow;
}

/// The control-flow EVAL backend — REAL host `for`/`if`/`continue`, a unit ZST.
///
/// Always compiled (the brick-exit eval sweep instantiates it), but a pure host
/// control-flow ZST: NO physics-reachable code calls it (the host
/// `boyko_sdf_math::brick::dist_to_brick_exit` stays hand-written and does not
/// delegate — firewall option B), so it links nothing a physics build pulls.
#[derive(Clone, Copy)]
pub struct EvalCf;

// The ZST guarantee (the brief's mandatory assert): EvalCf carries no data, so the
// combinator dispatch is entirely type-level (zero runtime cost).
const _: () = assert!(size_of::<EvalCf>() == 0);

impl Cf for EvalCf {
    type Scalar = f32;
    // The mutable local IS its value, held in a `Cell` for interior mutability (the loop
    // body assigns through `&var` while the `unroll_for` closure borrows it). `Cell<f32>`
    // is not `Copy`, but the body never copies the var (it passes `&var`), so `Var` needs
    // no `Copy` bound.
    type Var = core::cell::Cell<f32>;

    #[inline]
    fn decl_var(_name: &'static str, init: f32) -> core::cell::Cell<f32> {
        // The name is an Emit-only printing concern (a `Cell` has no printed identity).
        core::cell::Cell::new(init)
    }

    #[inline]
    fn get_var(v: &core::cell::Cell<f32>) -> f32 {
        v.get()
    }

    #[inline]
    fn set_var(v: &core::cell::Cell<f32>, val: f32) {
        v.set(val);
    }

    type Iv = usize;

    #[inline]
    fn index(vec: [f32; 3], iv: usize) -> f32 {
        vec[iv]
    }

    #[inline]
    fn named_lit(_sym: &'static str, val: f32) -> f32 {
        f32::lit(val)
    }

    #[inline]
    fn temp(x: f32) -> f32 {
        // IDENTITY on Eval — materialization is an emit-only printing concern; the value
        // flows directly with no behavioral effect (the Eval result is unchanged).
        x
    }

    #[inline]
    fn unroll_for<F: FnMut(usize) -> Flow>(_attr: &'static str, n: usize, mut body: F) {
        for i in 0..n {
            // A `Break(Continue)` (the loop-continue token) maps to a real `continue`, so
            // the body's tail after the propagated continue does not run; the std
            // `Continue` fall-through runs the tail. `Break(Break)` is the Inc-4 loop break.
            match body(i) {
                core::ops::ControlFlow::Continue(()) => {}
                core::ops::ControlFlow::Break(LoopOp::Continue) => continue,
                core::ops::ControlFlow::Break(LoopOp::Break) => break,
            }
        }
    }

    #[inline]
    fn if_<F: FnOnce() -> Flow>(cond: bool, body: F) -> Flow {
        if cond {
            body()
        } else {
            core::ops::ControlFlow::Continue(())
        }
    }

    #[inline]
    fn cont() -> Flow {
        core::ops::ControlFlow::Break(LoopOp::Continue)
    }
}
