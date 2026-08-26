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
//! - [`EvalCf`] (here, always compiled) — REAL host control flow: [`unroll_for`](Cf::unroll_for) is a
//!   real `for`, [`if_`](Cf::if_) a real `if`, [`cont`](Cf::cont) the loop-continue token. This is the
//!   CPU oracle the brick-exit eval sweep locks (it is a pure host `for`/`if` ZST; no
//!   physics-reachable code calls it).
//! - `EmitCf` (`crate::emit`, `feature = "emit"`) — each combinator RECORDS a
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
//! ([`ControlFlow::Break`](core::ops::ControlFlow::Break)`(`[`LoopOp::Continue`]`)`) and `?`
//! early-returns it from the FnMut, so any LIVE TAIL mutation AFTER the continue point does
//! NOT run — matching the host `continue`. [`unroll_for`](Cf::unroll_for) maps that
//! `Break(Continue)` to a real `continue`
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
    /// FUNCTION return (the early-return marcher) — produced by [`Cf::ret`] (Increment 3,
    /// `brick_cell_class`). The return VALUE travels OUT OF BAND (a body-local cell on
    /// Eval; the value node recorded into `Stmt::Return` on Emit), so the payload stays
    /// ZST and the [`Flow`] type is unchanged (no `?`-propagation re-audit). The token is
    /// consumed by the function-scope IIFE's `?` — `brick_cell_class` has no loop, so it
    /// never reaches [`Cf::unroll_for`]'s match (a `debug_assert!` guards that).
    Return,
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

    /// The unrolled-loop INDUCTION VARIABLE handle passed to the
    /// [`unroll_for`](Self::unroll_for) body. `usize` on Eval (the real `for` counter); an emit iv SSA node on
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

    /// A MUTABLE `bool` LOCAL (the B1 marcher's `hit` / `exhausted` flags). Eval stores the
    /// `bool` in a [`core::cell::Cell`] (the same interior-mutability shape [`Var`](Self::Var)
    /// uses for `float`); Emit records a `Stmt::DeclVar` whose `ty` is [`bool`], spelling
    /// `bool <name> = <init>;`. Distinct from [`Var`](Self::Var) (a `float` local) only in the
    /// declared type — Increment 4d, the first rung of the B1-marcher single-source ladder.
    type BoolVar;

    /// Declares a mutable `bool` local named `name` initialized to the literal `init`, returning
    /// its [`BoolVar`](Self::BoolVar) handle. The `bool` analogue of [`decl_var`](Self::decl_var)
    /// (which hardcodes a `float`). Eval boxes `init` into a `Cell<bool>` (the `name` is an
    /// Emit-only printing concern); Emit records a `Stmt::DeclVar` whose `ty` is the `bool`
    /// type token and whose `rhs` is a `false`/`true` literal node, spelling `bool <name> =
    /// <init>;`. The B1 marcher's `bool hit = false;` / `bool exhausted = true;` preamble decls.
    fn decl_bool_var(name: &'static str, init: bool) -> Self::BoolVar;

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
    /// When `cond` is false, FALLS THROUGH ([`ControlFlow::Continue`](core::ops::ControlFlow::Continue)); when true,
    /// returns whatever `body` yields (so a `C::if_(cond, || C::cont())?` early-returns
    /// the continue). Eval evaluates the real `if`; Emit records `Stmt::If`.
    ///
    /// `cond` is the [`Scalar`](Self::Scalar)'s own [`Mask`](FieldScalar::Mask) (e.g. the
    /// result of [`FieldScalar::le`]), so a body's `dir.abs().le(eps)` feeds `if_`
    /// directly with no separate `Cf::Mask` axis to keep in sync.
    fn if_<F: FnOnce() -> Flow>(cond: <Self::Scalar as FieldScalar>::Mask, body: F) -> Flow;

    /// The loop-CONTINUE token (a [`ControlFlow::Break`](core::ops::ControlFlow::Break)`(`[`LoopOp::Continue`]`)`) —
    /// `?`-propagated out of the loop body to skip the rest of the iteration.
    fn cont() -> Flow;

    /// The loop-BREAK token (a [`ControlFlow::Break`](core::ops::ControlFlow::Break)`(`[`LoopOp::Break`]`)`)
    /// — the Inc-4b PRODUCER of the break payload
    /// [`unroll_for`](Self::unroll_for)/[`runtime_for`](Self::runtime_for) already CONSUME
    /// (cf.rs `Break` arms, tested). Mirrors [`cont`](Self::cont):
    /// `?`-propagated out of the loop body (typically through [`if_`](Self::if_) as `C::if_(
    /// cond, C::brk)?`) to EXIT the loop. On Eval it returns `Break(LoopOp::Break)` (which
    /// [`runtime_for`](Self::runtime_for) maps to a real `break` then returns
    /// `Flow::Continue(())`, so the post-loop tail runs); on Emit it records a `Stmt::Break`
    /// then returns the same token (the recorder keeps recording the live tail — the break is
    /// captured structurally inside the enclosing `Stmt::If`).
    fn brk() -> Flow;

    // ---- Increment 3 typed facets (the `brick_cell_class` value model) -----------
    //
    // A `uint` class + a `float3` cell_min out-param + a `StructuredBuffer<uint>` load
    // — the second CF leaf's value types. Each is an associated TYPE instantiated as a
    // body-local / parameter (NOT stored on the ZST backend), so the
    // `size_of::<EvalCf>() == 0` guarantee still holds. Eval is native Rust (real
    // casts, real `||`, a `Cell` out-param); Emit records SSA nodes / statements.

    /// The `uint` scalar — `u32` on Eval, the SSA-node handle [`Scalar`](Self::Scalar)
    /// on Emit (a handle carrying `crate::emit`'s `uint` `EmitTy` per-node). The
    /// brick class + the linear cell index.
    type Uint: Copy;

    /// A `uint3` PARAMETER (`dims`) — `[u32; 3]` on Eval, an emit `uint3`-param handle on
    /// Emit. Swizzled component-wise by [`uint3_x`](Self::uint3_x) etc.
    type Uint3: Copy;

    /// A `float3` VALUE (`rel`, `cell_min`'s rhs) — `[f32; 3]` on Eval, an emit `float3`
    /// SSA handle on Emit. Distinct from the straight-line leaves' `[Self::Scalar; 3]`:
    /// `brick_cell_class` records `float3` as a FIRST-CLASS value (a `rel` temp, a `p -
    /// origin` subtract) so the emitted HLSL spells `float3 rel = ...;`, not three scalar
    /// temps.
    type Vec3f: Copy;

    /// The `float3` OUT-PARAMETER local state (`cell_min`) — `Cell<[f32; 3]>` on Eval
    /// (interior mutability; the body writes through `&o`), an emit out-param NAME handle
    /// on Emit (its writes print bare `cell_min = ...;`, NOT a `float3 cell_min = ...;`
    /// decl). Owned (no lifetime), passed by `&` to
    /// [`out_vec3_assign`](Self::out_vec3_assign) — the SAME `&Self::Var` idiom
    /// [`get_var`](Self::get_var) uses.
    type OutVec3;

    /// The RETURN-VALUE cell (`Cell<u32>` on Eval — the body-local cell the IIFE reads
    /// after an early return; a ZST on Emit, the value travels in the recorded
    /// `Stmt::Return`). Owned, passed by `&` to [`ret`](Self::ret) /
    /// [`if_ret`](Self::if_ret).
    type RetCell;

    /// A `StructuredBuffer<uint>` PARAMETER (`grid`) — a BORROW of the external grid data
    /// (`&'a [u32]` on Eval, an emit buffer-name handle on Emit), so it is a GENERIC
    /// associated type over the borrow lifetime (the grid is owned by the caller, not the
    /// body). `Copy` (a shared slice / a name handle). Read by
    /// [`buffer_load`](Self::buffer_load).
    type Buf<'a>: Copy;

    /// `(p - origin)` — component-wise `float3` subtraction.
    fn vec3_sub(a: Self::Vec3f, b: Self::Vec3f) -> Self::Vec3f;
    /// `(a + b)` — component-wise `float3` addition (`origin + offset`).
    fn vec3_add(a: Self::Vec3f, b: Self::Vec3f) -> Self::Vec3f;
    /// `(v / s)` — `float3` divided by a `float` scalar (`(p - origin) / bw`).
    fn vec3_div_scalar(v: Self::Vec3f, s: Self::Scalar) -> Self::Vec3f;
    /// `(v * s)` — `float3` times a `float` scalar (`float3(ix,iy,iz) * bw`).
    fn vec3_mul_scalar(v: Self::Vec3f, s: Self::Scalar) -> Self::Vec3f;
    /// `float3(x, y, z)` from THREE `uint`s — the HLSL ctor's implicit uint→float
    /// (`float3(ix, iy, iz)`). A NARROW node (asserts all-`uint` operands on Emit); the
    /// only cross-type construct in the leaf.
    fn vec3_from_uints(x: Self::Uint, y: Self::Uint, z: Self::Uint) -> Self::Vec3f;

    /// `v.x` / `v.y` / `v.z` — a `float3` swizzle to a `float` scalar (`rel.x`).
    fn vec3_x(v: Self::Vec3f) -> Self::Scalar;
    /// `v.y`.
    fn vec3_y(v: Self::Vec3f) -> Self::Scalar;
    /// `v.z`.
    fn vec3_z(v: Self::Vec3f) -> Self::Scalar;

    /// `d.x` / `d.y` / `d.z` — a `uint3` swizzle to a `uint` (`dims.x`).
    fn uint3_x(d: Self::Uint3) -> Self::Uint;
    /// `d.y`.
    fn uint3_y(d: Self::Uint3) -> Self::Uint;
    /// `d.z`.
    fn uint3_z(d: Self::Uint3) -> Self::Uint;

    /// `(uint)f` — the HLSL NUMERIC `float -> uint` truncating cast (`(uint)rel.x`), NOT
    /// `asuint` (a bit-reinterpret). On Eval `f as u32` (the same truncation HLSL's
    /// `OpConvertFToU` performs for in-range non-negative inputs — the negative-rel case
    /// is guarded OUT before any cast runs).
    fn float_to_uint(f: Self::Scalar) -> Self::Uint;

    /// `a + b` over two `uint`s (the index accumulation).
    fn uadd(a: Self::Uint, b: Self::Uint) -> Self::Uint;
    /// `a * b` over two `uint`s (the row/slice strides).
    fn umul(a: Self::Uint, b: Self::Uint) -> Self::Uint;

    /// A NAMED `uint` constant — `val` on Eval, the SYMBOL `sym` on Emit (so the
    /// emitted HLSL spells `BRICK_OUTSIDE_GRID`, not `4294967295u`).
    fn named_uint(sym: &'static str, val: u32) -> Self::Uint;

    /// `grid[idx]` — a `StructuredBuffer<uint>` load (→ `uint`).
    fn buffer_load(buf: Self::Buf<'_>, idx: Self::Uint) -> Self::Uint;

    /// `f < 0.0`-style: the `float` strict-less-than producing the leaf's guard
    /// [`Mask`](FieldScalar::Mask). (`rel.x < 0.0` reuses [`FieldScalar::lt`]; this is the
    /// `uint` `>=` analogue.) `a >= b` over two `uint`s — the bounds guard `ix >= dims.x`.
    fn uge(a: Self::Uint, b: Self::Uint) -> <Self::Scalar as FieldScalar>::Mask;

    /// `a || b` over two guard masks — the short-circuit OR. On Eval the masks are
    /// already-computed `bool`s (`a || b`); every comparand in `brick_cell_class` is
    /// side-effect-free (pure `<` / `>=` over locals), so the eager-mask form is
    /// RESULT-EQUIVALENT to the short-circuit (the tail-skip — the casts not running on a
    /// negative rel — is preserved by statement ORDER + the `ret`'s `?`, proven by the
    /// negative-rel sweep). On Emit it records the lazy `OpBranchConditional` chain DXC
    /// lowers `a||b||c` to (spike E2a: zero `OpLogicalOr`).
    fn or(
        a: <Self::Scalar as FieldScalar>::Mask,
        b: <Self::Scalar as FieldScalar>::Mask,
    ) -> <Self::Scalar as FieldScalar>::Mask;

    // ---- Increment 4f: the B1 sor-retreat condition leaves (`uint >`, logical `&&`, `0u`) ----
    //
    // The B1 over-relaxation SOR-FAIL-RETREAT guard is `if (it > 0u && sor_prev + d <
    // FIELD_LIPSCHITZ_L * sor_step_prev)` — a `uint` strict-`>` (`it > 0u`) joined to a `float`
    // `<` by a logical `&&`, with `0u` a bare `uint` literal. Each is a 2-line mirror of a proven
    // facet ([`uge`](Self::uge) for `ugt`, [`or`](Self::or) for `and2`, [`named_uint`](Self::
    // named_uint)/[`named_lit`](Self::named_lit) for `uint_lit`); ZERO new loop/return machinery.

    /// `a > b` over two [`Uint`](Self::Uint)s, producing a [`Mask`](FieldScalar::Mask) — the B1
    /// sor-retreat's `it > 0u` iteration guard. The `uint` strict-`>` analogue of
    /// [`uge`](Self::uge) (`a >= b`); a DISTINCT opcode (`OpUGreaterThan`) from a swapped `<`.
    /// On Eval `a > b` over the host `u32`s; on Emit a `crate::emit` `UGt` node printed inline
    /// (`it > 0u`).
    fn ugt(a: Self::Uint, b: Self::Uint) -> <Self::Scalar as FieldScalar>::Mask;

    /// `a && b` over two guard masks — the LOGICAL AND joining the sor-retreat's `it > 0u`
    /// guard to the Lipschitz `<` test. On Eval the masks are already-computed `bool`s (`a &&
    /// b`); BOTH comparands are pure side-effect-free reads (`it`, a `<` over locals), so the
    /// eager-mask form is RESULT-EQUIVALENT to the GPU short-circuit (the SAME equivalence
    /// [`or`](Self::or) carries). On Emit it records the lazy `OpBranchConditional` chain DXC
    /// lowers `&&` to (like `||`). SEPARATE from the bitwise `uint` `&` (overloading would
    /// mistype the result as `Uint` and print `&`).
    fn and2(
        a: <Self::Scalar as FieldScalar>::Mask,
        b: <Self::Scalar as FieldScalar>::Mask,
    ) -> <Self::Scalar as FieldScalar>::Mask;

    /// A `uint` LITERAL — `x` on both backends as the VALUE, but spelled `<x>u` on Emit (NOT a
    /// symbol). The B1 sor-retreat's `0u` (a bare literal, not the symbolic
    /// [`named_uint`](Self::named_uint) constant). On Eval returns `x`; on Emit records a
    /// `crate::emit` `UintLit`
    /// node (printed `<x>u`, already an inline leaf typed `Uint`).
    fn uint_lit(x: u32) -> Self::Uint;

    /// Declares a NAMED mutable `float3` temp (`float3 rel = <rhs>;`). Eval is identity
    /// (the value flows directly); Emit records a named `Stmt::DeclTemp` (a `float3`
    /// temp). Returns the temp handle so later swizzles spell `rel.x`.
    fn temp_vec3(name: &'static str, v: Self::Vec3f) -> Self::Vec3f;

    /// Declares a NAMED mutable `uint` temp (`uint ix = <rhs>;`). Eval is identity; Emit
    /// records a named `uint` `Stmt::DeclTemp`. Returns the temp handle.
    fn temp_uint(name: &'static str, u: Self::Uint) -> Self::Uint;

    /// Assigns the `float3` OUT-PARAMETER (`cell_min = <rhs>;`). Eval `set`s the
    /// `Cell<[f32; 3]>` through `&o`; Emit records a `Stmt::OutAssign` printing a bare
    /// `cell_min = <rhs>;` (NO decl — `cell_min` is an `out` parameter, not a local).
    fn out_vec3_assign(o: &Self::OutVec3, v: Self::Vec3f);

    /// `if (cond) { return value; }` — the early-return guard. On Eval: when `cond`, the
    /// `value` is deposited into the `ret` cell (via `&cell`) and a
    /// [`Break`](LoopOp::Return) token is returned (the body's `?` short-circuits the
    /// IIFE, skipping the live tail — the casts after guard 1); else FALL THROUGH. On
    /// Emit: records a `Stmt::If` whose then-block is EXACTLY ONE `Stmt::Return(value)`
    /// (no spurious assign), then falls through (the recorder keeps recording the tail
    /// structurally).
    fn if_ret(
        cell: &Self::RetCell,
        cond: <Self::Scalar as FieldScalar>::Mask,
        value: Self::Uint,
    ) -> Flow;

    /// The SOLE function-return mechanism (replaces the deleted dual set_var+ret). On
    /// Eval deposits `value` into the `cell` (via `&cell`) and returns
    /// [`Break`](LoopOp::Return); on Emit records a single `Stmt::Return(value)`. The
    /// body's tail `C::ret(&cell, tail)?` is the final return (`return grid[idx];`).
    fn ret(cell: &Self::RetCell, value: Self::Uint) -> Flow;

    // ---- Increment 4a: the runtime `[loop]` + the FLOAT return facet ----------------
    //
    // `m2_regula_falsi` (the smallest genuine runtime `[loop]`) returns a FLOAT
    // (`return mid;`) and carries five Phi vars across a const-bounded `[loop]` whose
    // header spells a BOUND SYMBOL (`M2_MARMITT_ITERS`, not a `<n>u` literal). The
    // facets below are ADDITIVE — the `uint` return path ([`ret`](Self::ret) /
    // [`if_ret`](Self::if_ret) / [`RetCell`](Self::RetCell)) used by `brick_cell_class`
    // is UNTOUCHED. `brk()`/`Stmt::Break` are DEFERRED to Inc 4b (`m2_regula_falsi` has
    // no plain break, so no untested emit surface is added now).

    /// The FLOAT RETURN-VALUE cell (`Cell<f32>` on Eval — the body-local cell the
    /// function-scope IIFE reads after an early in-loop return; a ZST on Emit, the value
    /// travels in the recorded `Stmt::Return`). The float analogue of
    /// [`RetCell`](Self::RetCell). Owned, passed by `&` to [`ret_f`](Self::ret_f) /
    /// [`if_ret_f`](Self::if_ret_f).
    type RetCellF;

    /// A `float4` PARAMETER (`c`, the cubic coefficients) — `[f32; 4]` on Eval (opaque:
    /// `c` is CALL-THROUGH-ONLY, never swizzled in `m2_regula_falsi`), a `Node::Vec4Param`
    /// name handle on Emit. Consumed solely by [`call2`](Self::call2) (`m2_cubic_eval(c,
    /// mid)`). `Copy` (a `[f32; 4]` / a name handle).
    type Vec4f: Copy;

    /// Seeds a SIGNATURE PARAMETER as a mutable local WITHOUT a declaration. The four
    /// regula-falsi carried params {lo, hi, f_lo, f_hi} are HLSL signature parameters
    /// (`float lo`, ...), so [`get_var`](Self::get_var)/[`set_var`](Self::set_var) must
    /// resolve their names (`hi`, `hi = ...;`) but the body must record NO
    /// `Stmt::DeclVar` (a `float lo = ...;` redecl would diverge the text). Eval boxes the
    /// param value into a `Cell` (identical to [`decl_var`](Self::decl_var) on Eval); Emit
    /// seeds a [`Var`](Self::Var) name entry but records NO statement — the SUPPRESSED-DECL
    /// path. (`mid` is a TRUE local → `decl_var("mid", get_var(&lo))` → `float mid = lo;`.)
    fn decl_param(name: &'static str, init: Self::Scalar) -> Self::Var;

    /// Forces `x` to MATERIALIZE as a NAMED `float <name>` local in program order — the
    /// `float` analogue of [`temp_vec3`](Self::temp_vec3) / [`temp_uint`](Self::temp_uint)
    /// (the brick-cell named locals). `m2_regula_falsi`'s `float denom = ...;` and `float
    /// f_mid = ...;` are NAMED (not anonymous `tN`), so the body uses `temp_float("denom",
    /// ...)`. Eval is identity (the value flows directly); Emit records a named `float`
    /// `Stmt::DeclTemp`.
    fn temp_float(name: &'static str, x: Self::Scalar) -> Self::Scalar;

    /// `cond ? t : e` — the value select over [`Scalar`](Self::Scalar). Eval is the eager
    /// `if cond { t } else { e }` (both arms pure; the discarded `denom ≈ 0` path's `inf`
    /// is never read — matching the GPU computing BOTH ternary arms); Emit records a
    /// `Node::Select` whose printer wraps BOTH arms unconditionally `({cond}) ? ({t}) :
    /// ({e})` (the committed `m2_regula_falsi` ternary form). Distinct from
    /// [`FieldScalar::select`] only in being routed through [`Cf`] so the marcher body
    /// reads `C::select(...)` uniformly with the other combinators.
    fn select(
        cond: <Self::Scalar as FieldScalar>::Mask,
        t: Self::Scalar,
        e: Self::Scalar,
    ) -> Self::Scalar;

    /// A call to a FROZEN hand-written shader function of two args — `m2_cubic_eval(c,
    /// mid)`. The leaf body (`m2_cubic_eval`) is already generated separately; here it is
    /// spelled as a CALL SITE (like [`crate::normal`] spells `sdf(...)`). `fn_sym` is the
    /// callee name; `a`/`b` the two argument values (heterogeneous types — `c` is a
    /// `float4`, `mid` a `float` — live in the node graph, so both are plain
    /// [`Scalar`](Self::Scalar)-or-[`Vec4f`](Self::Vec4f) handles). Returns a
    /// [`Scalar`](Self::Scalar). On Eval, the host evaluates the frozen function directly
    /// (the closure passed by the generic body); on Emit, records a `Node::Call2`.
    fn call2(fn_sym: &'static str, a: Self::Vec4f, b: Self::Scalar) -> Self::Scalar;

    /// A call to a FROZEN hand-written shader function of ONE `float3` arg returning a
    /// `float` — `field_distance(p + L * t)` (Inc 4b). The float3→float analogue of
    /// [`call2`](Self::call2) (whose `float4,float` signature is NOT reusable). `fn_sym` is
    /// the callee name (interned, so ANY callee — `field_distance` now,
    /// `brick_cell_class`/`select_level` later — not the hardcoded `sdf` of
    /// [`crate::field`]'s field-call leaf); `a` the single [`Vec3f`](Self::Vec3f) argument.
    /// On Eval the host evaluates the frozen function directly through the THREADED CLOSURE
    /// the generic body carries (the A1 field-call seam — like [`call2`](Self::call2)'s
    /// `m2_cubic_eval` closure), so this hook is UNREACHED on Eval (`unreachable!`, the
    /// honest-panic discipline — NOT a wrong value); on Emit it records a `Node::Call1`.
    fn call1(fn_sym: &'static str, a: Self::Vec3f) -> Self::Scalar;

    /// Runs a RUNTIME `for (uint <iv> = 0u; <iv> < <bound_sym>; ++<iv>)` over `body`. THE
    /// new control-flow construct (Inc 4a) — the FIRST genuine runtime `[loop]` (an
    /// `OpLoop`, vs the `[unroll]` of [`unroll_for`](Self::unroll_for)). `attr` is the loop
    /// attribute (`"[loop]"`); `iv` the induction-variable name (`"i"`); `bound_sym` the
    /// BOUND SYMBOL the header spells (`"M2_MARMITT_ITERS"`, NOT a `<n>u` literal — the key
    /// diff from `unroll_for`); `bound_val` the concrete trip count Eval iterates (the
    /// symbol's value, `8`). `body` returns a [`Flow`].
    ///
    /// EvalCf CONTROL TABLE (driven per-arm by a unit test):
    /// - body returns `Continue(())` → run the next iteration;
    /// - `Break(`[`LoopOp::Continue`]`)` → a real `continue` (skip this iteration's tail);
    /// - `Break(`[`LoopOp::Break`]`)` → a real `break`, THEN `return Flow::Continue(())`
    ///   (the loop consumed its own break; the function tail runs);
    /// - `Break(`[`LoopOp::Return`]`)` → `return Flow::Break(LoopOp::Return)` — FORWARD the
    ///   function return to the caller's `?` (the function-scope IIFE), so an in-loop
    ///   `ret_f` short-circuits the tail `ret_f` (the early `mid`, NOT the 8-iter `mid`);
    /// - natural completion → `return Flow::Continue(())`.
    ///
    /// So `runtime_for` CONSUMES its own loop control (Break/Continue) and FORWARDS the
    /// function return — hence it RETURNS [`Flow`] (unlike [`unroll_for`](Self::unroll_for),
    /// whose body has no `ret`). On Emit it ALWAYS returns `Flow::Continue(())` (records the
    /// body once; `?` never fires on Emit), and threads `iv`/`bound_sym` into the
    /// `Stmt::Loop` header (single-source — no hardcoded name).
    fn runtime_for<F: FnMut(Self::Iv) -> Flow>(
        attr: &'static str,
        iv: &'static str,
        bound_sym: &'static str,
        bound_val: usize,
        body: F,
    ) -> Flow;

    /// `if (cond) { then } else { els }` — the TWO-arm branch (the existing
    /// [`if_`](Self::if_) is single-arm). `m2_regula_falsi`'s `if (f_lo * f_mid <= 0.0) { hi = mid; f_hi =
    /// f_mid; } else { lo = mid; f_lo = f_mid; }`. Each arm is a `FnOnce() -> `[`Flow`]
    /// recording its block (here pure `set_var`s, returning `Flow::Continue(())`). Eval runs
    /// the real `if`/`else`; Emit records a `Stmt::IfElse` (push/record/pop each block) and
    /// FALLS THROUGH (`Flow::Continue(())`), so the recorder keeps recording the tail.
    fn if_else<T: FnOnce() -> Flow, E: FnOnce() -> Flow>(
        cond: <Self::Scalar as FieldScalar>::Mask,
        then: T,
        els: E,
    ) -> Flow;

    /// `if (cond) { return value; }` — the FLOAT early-return guard (the float analogue of
    /// [`if_ret`](Self::if_ret)). On Eval: when `cond`, `value` is deposited into the
    /// [`RetCellF`](Self::RetCellF) (via `&cell`) and a [`Break`](LoopOp::Return) is
    /// returned (the body's `?` forwards it through [`runtime_for`](Self::runtime_for) to
    /// the function-scope IIFE, skipping the tail); else FALL THROUGH. On Emit: records a
    /// `Stmt::If` whose then-block is EXACTLY ONE `Stmt::Return(value)`. `m2_regula_falsi`'s
    /// `if (abs(f_mid) <= ... || (hi - lo) <= ...) { return mid; }`.
    fn if_ret_f(
        cell: &Self::RetCellF,
        cond: <Self::Scalar as FieldScalar>::Mask,
        value: Self::Scalar,
    ) -> Flow;

    /// The FLOAT function-return (the float analogue of [`ret`](Self::ret)). On Eval
    /// deposits `value` into the [`RetCellF`](Self::RetCellF) and returns
    /// [`Break`](LoopOp::Return); on Emit records a single `Stmt::Return(value)`.
    /// `m2_regula_falsi`'s tail `return mid;`.
    fn ret_f(cell: &Self::RetCellF, value: Self::Scalar) -> Flow;

    // ---- Increment 4b.2: the BOOL return + OUT-FLOAT facets (the `m2_surface_hit` refine) ----
    //
    // `m2_surface_hit` returns a `bool` (`return true;` / `return false;`, lowering to a
    // genuine `OpTypeBool` function return with `OpConstantTrue`/`OpConstantFalse` — NOT
    // `uint` 0/1, the binary fact the spike read off the committed `.comp.spv`) and writes its
    // refined hit `t` through an `out float hit_t`. The facets below are 2-line mirrors of the
    // proven float-return facet ([`RetCellF`](Self::RetCellF) / [`ret_f`](Self::ret_f) /
    // [`if_ret_f`](Self::if_ret_f)) and the brick-cell out-vec3 facet ([`OutVec3`](Self::
    // OutVec3) / [`out_vec3_assign`](Self::out_vec3_assign)); ZERO new loop/integer machinery.

    /// The BOOL RETURN-VALUE cell (`Cell<bool>` on Eval — the body-local cell the
    /// function-scope IIFE reads after an early in-loop `return true`; a ZST on Emit, the
    /// `true`/`false` travels in the recorded `Stmt::Return` as a `Node::BoolLit`). The bool
    /// analogue of [`RetCellF`](Self::RetCellF). Owned, passed by `&` to [`ret_b`](Self::ret_b)
    /// / [`if_hit_ret_b`](Self::if_hit_ret_b).
    type RetCellB;

    /// The `out float` OUT-PARAMETER local state (`hit_t`) — `Cell<f32>` on Eval (interior
    /// mutability; the body writes through `&o`), an emit out-param NAME handle on Emit (its
    /// writes print bare `hit_t = ...;`, NOT a `float hit_t = ...;` decl). The `float` analogue
    /// of [`OutVec3`](Self::OutVec3). Owned, passed by `&` to
    /// [`out_float_assign`](Self::out_float_assign) / [`if_hit_ret_b`](Self::if_hit_ret_b).
    type OutFloat;

    /// The BOOL function-return (the bool analogue of [`ret_f`](Self::ret_f)). On Eval deposits
    /// `value` into the [`RetCellB`](Self::RetCellB) and returns [`Break`](LoopOp::Return); on
    /// Emit records a single `Stmt::Return` carrying a `Node::BoolLit` (printed `true`/`false`,
    /// NOT a `uint`). `m2_surface_hit`'s function-tail `return false;`.
    fn ret_b(cell: &Self::RetCellB, value: bool) -> Flow;

    /// Assigns the `out float` OUT-PARAMETER (`hit_t = <rhs>;`). Eval `set`s the `Cell<f32>`
    /// through `&o`; Emit records a bare `hit_t = <rhs>;` (NO decl — `hit_t` is an `out`
    /// parameter, not a local). The `float` analogue of
    /// [`out_vec3_assign`](Self::out_vec3_assign). `m2_surface_hit`'s in-loop `hit_t = rt;`.
    fn out_float_assign(o: &Self::OutFloat, v: Self::Scalar);

    /// The COMPOSITE in-loop hit — `if (cond) { hit_t = rt; return true; }`. Records BOTH
    /// statements in ONE [`if_`](Self::if_)-style then-block (NOT the single-statement
    /// [`if_ret_f`](Self::if_ret_f)): the out-float assign (`hit_t = rt;`) THEN the bool return
    /// (`return true;`), IN ORDER. On Eval this writes `hit_t` BEFORE the
    /// [`Break`](LoopOp::Return) short-circuits the IIFE, so the oracle reads the FRESH `rt`
    /// (not the stale entry
    /// default); on Emit the then-block is EXACTLY the two committed statements in order. The
    /// `?`-propagated `Break(Return)` forwards through [`runtime_for`](Self::runtime_for) to the
    /// function-scope IIFE (skipping the tail `ret_b(false)`). `m2_surface_hit`'s
    /// `if (abs(d) < EPS) { hit_t = rt; return true; }`.
    fn if_hit_ret_b(
        hit_out: &Self::OutFloat,
        ret_out: &Self::RetCellB,
        cond: <Self::Scalar as FieldScalar>::Mask,
        rt_val: Self::Scalar,
    ) -> Flow;

    // ---- Increment 5b: the COMPUTED-bool return facet (the `m2_brick_span` tail) -------
    //
    // `m2_brick_span` ends `return tmax > tmin;` — a COMPUTED bool (a [`Mask`](FieldScalar::Mask)),
    // unlike [`ret_b`](Self::ret_b)'s `bool` LITERAL (`return true;`/`return false;`). The mask is
    // the function's value, so the recorded `Stmt::Return`'s operand is the mask NODE (printed
    // `tmax > tmin`), not a [`Node::BoolLit`]. A 2-line mirror of [`ret_b`](Self::ret_b) with a
    // [`Mask`](FieldScalar::Mask) operand in place of the `bool`; ZERO new loop/integer machinery.

    /// The COMPUTED-bool function-return — `return <mask>;` (the `m2_brick_span` tail `return tmax >
    /// tmin;`). The Mask variant of [`ret_b`](Self::ret_b) (which takes a `bool` LITERAL). On Eval
    /// deposits the `value` mask (a host `bool`) into the [`RetCellB`](Self::RetCellB) and returns
    /// [`Break`](LoopOp::Return); on Emit records a single `Stmt::Return` carrying the MASK NODE (the
    /// `Stmt::Return` printer spells `return <mask-expr>;` — the same inline-expr printer that handles
    /// any non-`BoolLit` operand). DISTINCT from [`ret_b`](Self::ret_b) (a `BoolLit` operand).
    fn ret_b_expr(cell: &Self::RetCellB, value: <Self::Scalar as FieldScalar>::Mask) -> Flow;

    // ---- Increment 4e: the BOOL mutable-local facets (the B1 exhaustion re-march) ----
    //
    // The B1 budget-exhaustion re-march (`b1_exhaustion_remarch_body`) carries the `hit`
    // flag THROUGH the inner re-march `[loop]` — it WRITES `hit = true;` on a converged
    // accept (NOT a `return`: the marcher continues past the inner loop) and READS it back
    // for the Eval oracle's result tuple. The decls of `hit`/`t` live in the HAND-WRITTEN
    // re-march preamble (`t = t_seed; hit = false;`), so the span needs a SUPPRESSED-DECL
    // bool seam (the bool analogue of [`decl_param`](Self::decl_param)) plus a bool
    // set/get. Each is a 2-line mirror of the proven `float` [`Var`](Self::Var) facets
    // ([`decl_param`](Self::decl_param)/[`set_var`](Self::set_var)/[`get_var`](Self::
    // get_var)); ZERO new loop/return machinery.

    /// Declares a mutable `bool` local named `name` (init `init`) WITHOUT recording a
    /// declaration — the bool analogue of [`decl_param`](Self::decl_param) (which suppresses a
    /// `float` decl). The re-march's `hit`/`t` are declared by the HAND-WRITTEN preamble
    /// (`hit = false;`), so [`set_bool_var`](Self::set_bool_var)/
    /// [`get_bool_var`](Self::get_bool_var) must resolve their names but the span must record
    /// NO `Stmt::DeclVar` (a
    /// `bool hit = false;` redecl would diverge the committed text). Distinct from
    /// [`decl_bool_var`](Self::decl_bool_var) (which RECORDS the decl). Eval boxes `init` into a
    /// `Cell<bool>` (identical to [`decl_bool_var`](Self::decl_bool_var) on Eval); Emit seeds a
    /// [`BoolVar`](Self::BoolVar) name entry but records NO statement.
    fn decl_bool_param(name: &'static str, init: bool) -> Self::BoolVar;

    /// Reads the CURRENT value of a mutable `bool` local — the bool analogue of
    /// [`get_var`](Self::get_var). On Eval returns the `Cell<bool>`'s value (read back for the
    /// re-march oracle's `(hit, t)` result tuple). On Emit the generated span never EMITS a read of
    /// `hit` (no statement/node references the flag's VALUE — the span mutates `hit` by name); the
    /// body's tail constructs the `(hit, t)` tuple ONLY for the Eval oracle, and the Emit PRODUCER
    /// discards it. So the Emit side records NO statement and pushes NO node — it returns a
    /// byte-neutral placeholder whose value is irrelevant (discarded by the producer). NOT an
    /// `unreachable!` (unlike [`call1`](Self::call1), which the producer routes around with a
    /// closure): the tuple IS constructed on both backends, so the hook IS called on Emit.
    fn get_bool_var(v: &Self::BoolVar) -> bool;

    /// Assigns a mutable `bool` local to the literal `val` — the bool analogue of
    /// [`set_var`](Self::set_var). Eval `set`s the `Cell<bool>`; Emit records a `Stmt::Assign`
    /// whose `rhs` is a `Node::BoolLit` (`hit = true;`, reusing the proven `Stmt::Assign`
    /// printer + the bool-literal node). The re-march's in-loop `hit = true;` accept.
    fn set_bool_var(v: &Self::BoolVar, val: bool);

    // ---- Increment 5a: the SIGNED-INT subsystem + the M4Level access-text (`select_level`) ----
    //
    // `select_level` (`sdf_gbuffer_composite.hlsl:1221`) scans the nested clip-map levels for the
    // tightest enclosing LOD, returning a SIGNED `int` (`return (int)L;` inside, `return -1;` at
    // the tail). The facets below land the signed-int return path — distinct from the `uint` return
    // ([`ret`](Self::ret)) by spelling a SIGNED literal `-1` (NOT `<x>u`) + an `(int)L` cast — plus
    // the `all(p >= o) && all(p < hi)` bool3 reduction and the M4Level access-text reads. The
    // `[unroll]` loop reuses [`runtime_for`](Self::runtime_for) (attr `"[unroll]"`, the bound SYMBOL
    // `BRICK_LEVELS`); the level-field / pc reads are THREADED CLOSURES on Eval (the [`call1`](Self::
    // call1) discipline — these hooks are the EMIT recorders, `unreachable!` on Eval).

    /// The SIGNED-`int` value type the `select_level` return carries — `i32` on Eval (the host
    /// fixture's level index / the `-1` outside sentinel), the SSA-node handle
    /// [`Scalar`](Self::Scalar) on Emit (a handle carrying `crate::emit`'s `int`
    /// `crate::emit::EmitTy`). DISTINCT
    /// from [`Uint`](Self::Uint) so the return prints a SIGNED literal (`-1`, NOT `4294967295u`) and
    /// an `(int)L` cast.
    type Int: Copy;

    /// Reads the `[unroll]` loop INDUCTION VARIABLE as a `uint` VALUE — `select_level`'s `L` used in
    /// both `L >= pc.brick_levels` (a `uint` `>=`) and `(int)L` (the cast). The iv-as-value read
    /// (the `dist_to_brick_exit` `p[a]` discipline). On Eval `iv as u32` (the host `usize` counter
    /// narrowed — `L < BRICK_LEVELS = 3` always fits); on Emit the iv SSA node IS already typed
    /// `uint` (a `UintInput` printing `L`), so identity. DISTINCT from [`index`](Self::index) (which
    /// indexes a `float3` PARAMETER by the iv); this exposes the iv's own `uint` value.
    fn iv_uint(iv: Self::Iv) -> Self::Uint;

    /// The SIGNED-`int` RETURN-VALUE cell (`Cell<i32>` on Eval — the body-local cell the
    /// function-scope IIFE reads after an early in-loop `return (int)L`; a ZST on Emit, the value
    /// travels in the recorded `Stmt::Return`). The `int` analogue of [`RetCellF`](Self::RetCellF) /
    /// [`RetCellB`](Self::RetCellB). Owned, passed by `&` to [`ret_i`](Self::ret_i) /
    /// [`if_ret_i`](Self::if_ret_i).
    type RetCellI;

    /// A SIGNED-`int` LITERAL — `x` on both backends as the VALUE, but spelled BARE (`-1`, NOT a
    /// `<x>u` unsigned suffix) on Emit. `select_level`'s tail `return -1;`. On Eval returns `x`; on
    /// Emit records a `crate::emit` `IntLit` node (printed `-1`, an inline leaf typed
    /// [`Int`](Self::Int)). DISTINCT from [`uint_lit`](Self::uint_lit) (which spells `<x>u`).
    fn int_lit_signed(x: i32) -> Self::Int;

    /// `(int)<uint>` — the HLSL value-preserving `uint -> int` cast (`select_level`'s `(int)L`). On
    /// Eval `u as i32` (the in-range cast — `L < BRICK_LEVELS = 3` always fits an `i32`); on Emit a
    /// `crate::emit` `IntFromUint` node (printed `(int)L`, an inline leaf typed
    /// [`Int`](Self::Int)). The ONLY non-literal `int`-typed value surface.
    fn int_from_uint(u: Self::Uint) -> Self::Int;

    /// `all(p >= o)` — a component-wise `float3` `>=` (`p >= o`, a bool3) reduced by the HLSL `all`
    /// intrinsic to a single [`Mask`](FieldScalar::Mask). `select_level`'s lower-corner containment
    /// test. On Eval the three lanes are compared and ANDed (`p.x>=o.x && p.y>=o.y && p.z>=o.z`); on
    /// Emit an `All3(Bool3Ge(p, o))` node printed `all(p >= o)`.
    fn all3_ge(p: Self::Vec3f, o: Self::Vec3f) -> <Self::Scalar as FieldScalar>::Mask;

    /// `all(p < hi)` — the upper-corner analogue of [`all3_ge`](Self::all3_ge) (a component-wise
    /// `float3` `<` reduced by `all`). `select_level`'s exclusive upper bound (`p == hi` is EXCLUDED
    /// — the `<`, not `<=`, so the boundary belongs to the next cell). On Emit an `All3(Bool3Lt(p,
    /// hi))` node printed `all(p < hi)`.
    fn all3_lt(p: Self::Vec3f, hi: Self::Vec3f) -> <Self::Scalar as FieldScalar>::Mask;

    /// Reads a PUSH-CONSTANT `uint` FIELD by its bare text (`pc.brick_levels`) — `select_level`'s
    /// runtime level count, the `[unroll]` loop's early-out guard (`if (L >= pc.brick_levels) break;`).
    /// On Eval this hook is the EMIT recorder routed around by a threaded closure (the
    /// [`call1`](Self::call1) discipline), so it is UNREACHED (`unreachable!`); on Emit it records a `crate::emit`
    /// `PcUint` node printing the bare `field` text. `field` is the LITERAL HLSL text (`"pc.brick_levels"`).
    fn pc_uint(field: &'static str) -> Self::Uint;

    /// Reads a `float3` swizzle of an `M4Level` array element by ACCESS TEXT — `m2_levels[<L>].<field>`
    /// (`select_level`'s `m2_levels[L].origin_brick_world.xyz` / `m2_levels[L].dims_atlas_dim.xyz`).
    /// `field` carries the member + swizzle (`"origin_brick_world.xyz"`). The `M4Level` STRUCT LAYOUT
    /// is NOT modeled — only the access text. On Eval this hook is the EMIT recorder routed around by
    /// a threaded closure (UNREACHED, `unreachable!`); on Emit a `crate::emit` `LevelField` node
    /// printing `m2_levels[<L>].<field>`.
    fn level_field_vec3(l: Self::Iv, field: &'static str) -> Self::Vec3f;

    /// Reads a SCALAR `float` swizzle of an `M4Level` array element by ACCESS TEXT —
    /// `m2_levels[<L>].<field>` (`select_level`'s `m2_levels[L].origin_brick_world.w`). The scalar
    /// analogue of [`level_field_vec3`](Self::level_field_vec3) (a `.w` swizzle). On Eval this hook
    /// is the EMIT recorder routed around by a threaded closure (UNREACHED, `unreachable!`); on Emit
    /// a `crate::emit` `LevelField` node printing `m2_levels[<L>].<field>` (typed `float`).
    fn level_field_scalar(l: Self::Iv, field: &'static str) -> Self::Scalar;

    /// `if (cond) { return <int>; }` — the SIGNED-`int` early-return guard (the `int` analogue of
    /// [`if_ret_f`](Self::if_ret_f) / [`if_ret`](Self::if_ret)). On Eval: when `cond`, `value` is
    /// deposited into the [`RetCellI`](Self::RetCellI) (via `&cell`) and a [`Break`](LoopOp::Return)
    /// is returned (the body's `?` forwards it through [`runtime_for`](Self::runtime_for) to the
    /// function-scope IIFE, skipping the tail); else FALL THROUGH. On Emit: records a `Stmt::If` whose
    /// then-block is EXACTLY ONE `Stmt::Return(value)`. `select_level`'s `if (all && all) { return
    /// (int)L; }`.
    fn if_ret_i(
        cell: &Self::RetCellI,
        cond: <Self::Scalar as FieldScalar>::Mask,
        value: Self::Int,
    ) -> Flow;

    /// The SIGNED-`int` function-return (the `int` analogue of [`ret_f`](Self::ret_f) /
    /// [`ret`](Self::ret)). On Eval deposits `value` into the [`RetCellI`](Self::RetCellI) and
    /// returns [`Break`](LoopOp::Return); on Emit records a single `Stmt::Return(value)`.
    /// `select_level`'s tail `return -1;`.
    fn ret_i(cell: &Self::RetCellI, value: Self::Int) -> Flow;

    // ---- Increment 5c: the DDA marcher subsystem (`m2_brick_cubic_hit`) ----------------
    //
    // `m2_brick_cubic_hit` (`sdf_gbuffer_composite.hlsl:1014`) is the LARGEST + final brick called
    // body: a 3D-DDA that marches the ray through a brick's interior voxel cells, forms the JCGT
    // cubic at the first sign-bracketing cell, and solves it for the in-cell crossing. It needs
    // four FACET GROUPS none of the prior leaves have:
    //
    //   1. NAMED LOCAL ARRAYS — `int cell[3]`, `int step[3]`, `float t_next[3]`, `float t_delta[3]`
    //      + a per-cell `float s[8]` corner buffer (uninit decl, per-element get/set, `+=`).
    //   2. GENERALIZED CALL SITES — `m2_corner` (8 heterogeneous args incl. RESOURCE params),
    //      `m2_jcgt_cubic_coeffs(s, ...)` (a by-name ARRAY arg), `m2_marmitt_root` (a `float4` arg).
    //   3. INT CASTS/ARITH — `(uint)max(cell[0], 0)`, `(float)(c0 + 1)`, `W - 2u`, `step[axis] == 0`.
    //   4. MISC — the nested `uint` axis-select, the dynamic `rd_v[axis]` index, the `float3(...)`
    //      scalar ctor, a captured `uint W`.
    //
    // EMIT-ONLY CONTRACT: `m2_corner` calls `atlas.SampleLevel(...)` — a `Texture3D` the CPU cannot
    // run — so `m2_brick_cubic_hit_body::<EvalCf>` is NEVER instantiated. There is NO eval sweep; the
    // cmp-`.spv` is the SOLE gate (precedented by Inc 5a's `unreachable!`-on-Eval level/pc hooks).
    // EVERY EvalCf impl below is therefore `unreachable!` (the honest-panic discipline, like
    // [`call1`](Self::call1)).

    /// A NAMED LOCAL `int` ARRAY (`int cell[3]`) — an `IntArr` name handle on Emit, an unreachable
    /// ZST on Eval (the body is EMIT-ONLY). Declared by [`decl_array_int`](Self::decl_array_int),
    /// read/written per-element by [`arr_int_get`](Self::arr_int_get) /
    /// [`arr_int_set`](Self::arr_int_set) / [`arr_int_add_assign`](Self::arr_int_add_assign).
    type IntArr: Copy;

    /// A NAMED LOCAL `float` ARRAY (`float t_next[3]` / `float s[8]`) — the `float` analogue of
    /// [`IntArr`](Self::IntArr).
    type FloatArr: Copy;

    /// A RESOURCE PARAMETER (`atlas` — a `Texture3D<float>`, or `atlas_smp` — a `SamplerState`) — a
    /// `ResTok` name handle on Emit, an unreachable ZST on Eval. A CALL-THROUGH-ONLY operand
    /// (consumed by [`call_corner`](Self::call_corner)).
    type ResTok: Copy;

    // The `float4` VALUE `coeffs` (the cubic coefficients returned by `m2_jcgt_cubic_coeffs`) reuses
    // the SAME [`Vec4f`](Self::Vec4f) the regula-falsi `c` param uses (NO new associated type) — the
    // cubic-hit `coeffs` temp + the `m2_marmitt_root(coeffs, ...)` arg both thread it.

    // -- Group 1: the named-local-array combinators -----------------------------------

    /// Declares an UNINITIALIZED named-local `int` array (`int <name>[<len>];`). Returns the
    /// [`IntArr`](Self::IntArr) handle so later element ops spell `<name>[idx]`.
    fn decl_array_int(name: &'static str, len: u32) -> Self::IntArr;
    /// Declares an UNINITIALIZED named-local `float` array (`float <name>[<len>];`).
    fn decl_array_float(name: &'static str, len: u32) -> Self::FloatArr;

    /// `<name>[<idx>]` — an `int`-array element READ (the inline `cell[axis]` / `cell[0]`). The
    /// index is a [`Uint`](Self::Uint) (the iv `axis` or a `uint` literal).
    fn arr_int_get(a: Self::IntArr, idx: Self::Uint) -> Self::Int;
    /// `<name>[<idx>]` — a `float`-array element READ (`t_next[0]` / `s[k]`).
    fn arr_float_get(a: Self::FloatArr, idx: Self::Uint) -> Self::Scalar;

    /// `<name>[<idx>] = <v>;` — an `int`-array element STORE (`cell[axis] = c0;`).
    fn arr_int_set(a: Self::IntArr, idx: Self::Uint, v: Self::Int);
    /// `<name>[<idx>] = <v>;` — a `float`-array element STORE (`s[0] = <call>;`, `t_next[axis] =
    /// <expr>;`).
    fn arr_float_set(a: Self::FloatArr, idx: Self::Uint, v: Self::Scalar);

    /// `<name>[<idx>] += <v>;` — an `int`-array element COMPOUND-ADD (`cell[axis] += step[axis];`).
    /// The `+=` TOKEN (NOT `<name>[idx] = <name>[idx] + v`): the spike (R1) proved the `= +` form
    /// computes the access-chain TWICE at `-O0`, so it is NOT byte-identical.
    fn arr_int_add_assign(a: Self::IntArr, idx: Self::Uint, v: Self::Int);
    /// `<name>[<idx>] += <v>;` — a `float`-array element COMPOUND-ADD (`t_next[axis] +=
    /// t_delta[axis];`). Same `+=`-token R1 rationale as
    /// [`arr_int_add_assign`](Self::arr_int_add_assign).
    fn arr_float_add_assign(a: Self::FloatArr, idx: Self::Uint, v: Self::Scalar);

    // -- Group 2: the generalized call sites ------------------------------------------

    /// `m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half)` — the 8-arg
    /// resource-bearing corner fetch (`Texture3D.SampleLevel`). Returns a [`Scalar`](Self::Scalar)
    /// (the decoded corner distance). EMIT-ONLY (the CPU cannot run `SampleLevel`).
    #[allow(clippy::too_many_arguments)]
    fn call_corner(
        fn_sym: &'static str,
        atlas: Self::ResTok,
        smp: Self::ResTok,
        tile_org: Self::Vec3f,
        cx: Self::Uint,
        cy: Self::Uint,
        cz: Self::Uint,
        inv_atlas: Self::Scalar,
        band_half: Self::Scalar,
    ) -> Self::Scalar;

    /// `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)` — the cubic-coefficient fold over the by-NAME corner
    /// array `s`, the cell-local ray origin `lo_g`, and the ray direction `rd_v`. Returns a
    /// [`Vec4f`](Self::Vec4f) (the `[c0,c1,c2,c3]` coefficients).
    fn call_coeffs(
        fn_sym: &'static str,
        s: Self::FloatArr,
        lo_g: Self::Vec3f,
        rd_v: Self::Vec3f,
    ) -> Self::Vec4f;

    /// `m2_marmitt_root(coeffs, a, b)` — the Marmitt cubic root over `[a, b]` (`0.0`, `seg_hi -
    /// seg_lo`). Returns a [`Scalar`](Self::Scalar) (the in-cell crossing `local_t`, or `-1`).
    fn call_marmitt(
        fn_sym: &'static str,
        coeffs: Self::Vec4f,
        a: Self::Scalar,
        b: Self::Scalar,
    ) -> Self::Scalar;

    /// `(int)<callee>(<arg>)` — the `(int)m2_clamp_index(g_entry)` call+cast: a 1-arg `float ->
    /// uint` frozen call ([`call1`](Self::call1)-shape) immediately `(int)`-cast. Returns an
    /// [`Int`](Self::Int). The `float`-arg variant of [`call1`](Self::call1) (whose arg is a
    /// `float3`); folded with the cast so the `int c0 = (int)m2_clamp_index(g_entry);` materializes
    /// as one `int` temp.
    fn call_clamp_index_int(fn_sym: &'static str, g: Self::Scalar) -> Self::Int;

    // -- Group 3: the int casts / arithmetic ------------------------------------------

    /// `max(a, b)` over two SIGNED `int`s (`max(cell[0], 0)`). The signed-int analogue of
    /// [`FieldScalar::max`] (FLOAT).
    fn smax(a: Self::Int, b: Self::Int) -> Self::Int;
    /// `(uint)<int>` — the value-preserving `int -> uint` cast (`(uint)max(...)`).
    fn uint_from_int(a: Self::Int) -> Self::Uint;
    /// `a < b` over two SIGNED `int`s, producing a [`Mask`](FieldScalar::Mask) — the DDA-exit's
    /// `cell[axis] < 0`. The SIGNED `<` (a DISTINCT opcode from the FLOAT / `uint` `<`).
    fn slt(a: Self::Int, b: Self::Int) -> <Self::Scalar as FieldScalar>::Mask;
    /// `a + b` over two SIGNED `int`s (`c0 + 1`).
    fn sadd(a: Self::Int, b: Self::Int) -> Self::Int;
    /// `(float)<int>` — the value-preserving `int -> float` cast (`(float)c0` / `(float)(c0 + 1)`).
    fn float_from_int(a: Self::Int) -> Self::Scalar;
    /// `(float)<uint>` — the value-preserving `uint -> float` cast (`(float)cx`).
    fn float_from_uint(a: Self::Uint) -> Self::Scalar;
    /// `a - b` over two `uint`s (`W - 2u` / `W - 1u`).
    fn usub(a: Self::Uint, b: Self::Uint) -> Self::Uint;
    /// `min(a, b)` over two `uint`s (`min((uint)max(cell[0], 0), W - 2u)`). The `uint` analogue of
    /// [`FieldScalar::min`] (FLOAT) — needed because the `uint`-typed clamp's `min` cannot reuse the
    /// FLOAT `min` (its operands would mis-check `Float`).
    fn umin(a: Self::Uint, b: Self::Uint) -> Self::Uint;
    /// `a == b` over two SIGNED `int`s, producing a [`Mask`](FieldScalar::Mask) — the DDA-exit's
    /// `step[axis] == 0`. The SIGNED integer `==`.
    fn sint_eq(a: Self::Int, b: Self::Int) -> <Self::Scalar as FieldScalar>::Mask;
    /// Forces an [`Int`](Self::Int) to MATERIALIZE as a NAMED `int <name>` local — the `int`
    /// analogue of [`temp_float`](Self::temp_float) / [`temp_uint`](Self::temp_uint).
    /// `m2_brick_cubic_hit`'s `int c0 = (int)m2_clamp_index(g_entry);`.
    fn temp_int(name: &'static str, x: Self::Int) -> Self::Int;

    // -- Group 4: the misc facets -----------------------------------------------------

    /// A captured `uint` read by bare NAME (`W`) — the `const uint W = M2_BRICK_ALLOC;` the
    /// hand-written shader declares ABOVE the generated span. The `W - 2u` / `W - 1u` consumers read
    /// it. The plain-captured analogue of [`pc_uint`](Self::pc_uint) (a push-constant field).
    fn captured_uint(name: &'static str) -> Self::Uint;

    /// A nested `uint` axis-select `cond ? t : e` — the DDA's `axis = (...) ? 0u : ((...) ? 1u :
    /// 2u)`. The `uint`-arm analogue of [`select`](Self::select) (FLOAT arms). On Emit a
    /// `SelectParenU` node (the condition `(...)`-wrapped, the else arm self-wraps when a nested
    /// select).
    fn select_uint(
        cond: <Self::Scalar as FieldScalar>::Mask,
        t: Self::Uint,
        e: Self::Uint,
    ) -> Self::Uint;

    /// `<vec>[<idx>]` — a DYNAMIC index of a WHOLE `float3` PARAMETER (`rd_v[axis]` / `ro_v[0]`).
    /// DISTINCT from [`index`](Self::index) (which reads a SEEDED `[Scalar; 3]`): this indexes a
    /// whole [`Vec3f`](Self::Vec3f) by an arbitrary [`Uint`](Self::Uint), so a single seeded
    /// `Vec3Param` is BOTH passed whole (the `call_coeffs` `rd_v` arg) AND indexed `rd_v[axis]`.
    fn vec3_dyn_index(v: Self::Vec3f, idx: Self::Uint) -> Self::Scalar;

    /// `float3(<x>, <y>, <z>)` from THREE already-`float` SCALAR expressions (the `lo_g` ctor).
    /// DISTINCT from [`vec3_from_uints`](Self::vec3_from_uints) (three `uint`s, implicit uint→float):
    /// here all three are `float` arithmetic.
    fn vec3_from_scalars(x: Self::Scalar, y: Self::Scalar, z: Self::Scalar) -> Self::Vec3f;

    /// Forces `v` to MATERIALIZE as a NAMED `float4 <name>` local — the `float4` analogue of
    /// [`temp_vec3`](Self::temp_vec3) (`float3`). `m2_brick_cubic_hit`'s `float4 coeffs = ...;`.
    fn temp_vec4(name: &'static str, v: Self::Vec4f) -> Self::Vec4f;

    // ---- Track B Increment G1: the `float2` axis + the bitwise `uint` `&`/`>>` (`pack_material_id_ba`) ----
    //
    // `pack_material_id_ba` (`sdf_gbuffer_composite.hlsl:519`) is the G-buffer material-id packer: it
    // splits a 16-bit `uint id` into its low/high bytes (`id & 255u`, `id >> 8u & 255u`) and returns
    // each as a normalized `[0,1]` UNORM in a `float2` (`float2((float)lo / 255.0, (float)hi /
    // 255.0)`). The facets below land the MINIMAL `float2` axis (mirroring the `float3` facets) plus
    // the two DEAD bitwise nodes' methods (`crate::emit`'s `Node::And` / `Node::Shr`, whose printer
    // arms already exist). The named `lo`/`hi` `uint` temps reuse [`temp_uint`](Self::temp_uint); the
    // `255u`/`8u` literals reuse [`uint_lit`](Self::uint_lit); the `(float)lo` cast reuses
    // [`float_from_uint`](Self::float_from_uint); the `/ 255.0` divide is the scalar
    // [`crate::scalar::FieldScalar::div`].

    /// The `float2` VALUE type the `pack_material_id_ba` return carries — `[f32; 2]` on Eval (the
    /// `[lo/255, hi/255]` pair), the SSA-node handle [`Scalar`](Self::Scalar) on Emit (a
    /// `crate::emit::Node::Vec2FromScalars` typed `float2`). The `float2` analogue of
    /// [`Vec3f`](Self::Vec3f). `Copy` (a `[f32; 2]` / a node handle).
    type Vec2f: Copy;

    /// The `float2` RETURN-VALUE cell (`Cell<[f32; 2]>` on Eval — the body-local cell the producer
    /// reads after the body runs; a ZST on Emit, the `float2(...)` travels in the recorded
    /// `Stmt::Return`). The `float2` analogue of [`RetCellF`](Self::RetCellF) /
    /// [`RetCellI`](Self::RetCellI). Owned, passed by `&` to [`ret_vec2`](Self::ret_vec2).
    type RetCellV2;

    /// `a & b` over two [`Uint`](Self::Uint)s — the bitwise AND (`id & 255u`). ACTIVATES the
    /// `crate::emit::Node::And` (its `{} & {}` printer arm already exists). On Eval `a & b` over the
    /// host `u32`s; on Emit an `And` node (an UNPARENTHESIZED inline `id & 255u`). SEPARATE from the
    /// logical [`and2`](Self::and2) (which joins two Masks and prints `&&`): this is the bitwise `&`
    /// over two `uint` VALUES, result-typed [`Uint`](Self::Uint).
    fn and_u(a: Self::Uint, b: Self::Uint) -> Self::Uint;

    /// `a >> b` over two [`Uint`](Self::Uint)s — the logical right shift (`id >> 8u`). ACTIVATES the
    /// `crate::emit::Node::Shr` (its `{} >> {}` printer arm already exists). On Eval `a >> b` over
    /// the host `u32`s; on Emit a `Shr` node (an UNPARENTHESIZED inline `id >> 8u`). The
    /// `id >> 8u & 255u` precedence is correct UNPARENTHESIZED (`>>` binds tighter than `&`).
    fn shr_u(a: Self::Uint, b: Self::Uint) -> Self::Uint;

    /// `float2(<x>, <y>)` from TWO already-`float` SCALAR expressions — the `pack_material_id_ba`
    /// return ctor. The `float2` analogue of [`vec3_from_scalars`](Self::vec3_from_scalars) (three
    /// scalars). Asserts both operands `Float`; result [`Vec2f`](Self::Vec2f). On Eval `[x, y]`; on
    /// Emit a `crate::emit::Node::Vec2FromScalars`.
    fn vec2_from_scalars(x: Self::Scalar, y: Self::Scalar) -> Self::Vec2f;

    /// The `float2` function-return — `return <float2>;` (the `pack_material_id_ba` tail `return
    /// float2(...);`). The `float2` analogue of [`ret_f`](Self::ret_f) / [`ret_i`](Self::ret_i). On
    /// Eval deposits `value` into the [`RetCellV2`](Self::RetCellV2) and returns
    /// [`Break`](LoopOp::Return); on Emit records a single `Stmt::Return` carrying the
    /// [`Vec2f`](Self::Vec2f) node (the hand-written `float2 pack_material_id_ba` SIGNATURE supplies
    /// the return type — NO function-typer). Returns [`Flow`] so the body's `?` short-circuits the
    /// producer IIFE (Eval) / records structurally (Emit).
    fn ret_vec2(cell: &Self::RetCellV2, value: Self::Vec2f) -> Flow;

    // ---- Track B Increment G2: the `oct_encode` octahedral-normal encoder ----------------
    //
    // `oct_encode` (`sdf_gbuffer_composite.hlsl:507`) folds a unit normal into a `[0,1]^2` octahedral
    // pair. It is the LAST G-buffer leaf and the first with a MUTABLE `float3` PARAMETER reassigned in
    // place (`n /= ...`, modeled as the R1 whole-variable `n = n / ...`), a MUTABLE `float2` local
    // (`float2 e = n.xy;` reassigned inside an `if`), a REAL `if (n.z < 0.0)` fall-through branch, two
    // scalar sign-ternaries (`e.x >= 0.0 ? 1.0 : -1.0`), and a fused `e * 0.5 + 0.5` return. The facets
    // below land: the mutable `float3` suppressed-decl param ([`Vec3Var`](Self::Vec3Var)), the mutable
    // `float2` local ([`Vec2Var`](Self::Vec2Var)), the `float2` component ops (the `.xy`/`.yx` swizzles,
    // the `.x`/`.y` component reads, `abs`, `*`, `* s`, `+ s`, `s - v`). The `if_` reuses the proven
    // fall-through [`if_`](Self::if_); the sign-ternaries reuse [`select`](Self::select); the `n.z <
    // 0.0` guard reuses [`FieldScalar::lt`]; the `e.x >= 0.0` comparands reuse [`FieldScalar::ge`].

    /// A MUTABLE `float3` LOCAL holding the SUPPRESSED-DECL parameter `n` (the param reassigned in
    /// place by `n /= ...`). The `float3` analogue of [`decl_param`](Self::decl_param) (the scalar
    /// suppressed-decl carried param): Eval stores the `[f32; 3]` in a [`core::cell::Cell`] (interior
    /// mutability — the `if` body reads/assigns through `&var`); Emit seeds a `Var`
    /// name entry but records NO `Stmt::DeclVar` (a `float3 n = ...;` redecl would diverge the committed
    /// text — `n` is the HLSL signature parameter). Distinct from [`Var`](Self::Var) (a `float` local)
    /// only in the held type. Owned, passed by `&` to [`get_var_vec3`](Self::get_var_vec3) /
    /// [`set_var_vec3`](Self::set_var_vec3).
    type Vec3Var;

    /// Seeds the SIGNATURE PARAMETER `n` as a mutable `float3` local WITHOUT a declaration — the
    /// `float3` analogue of [`decl_param`](Self::decl_param) (which seeds a `float` param). The `init`
    /// is the param's symbolic seed ([`Vec3f`](Self::Vec3f)); Eval boxes its `[f32; 3]` into a `Cell`
    /// (identical to [`decl_param`](Self::decl_param) on Eval — the no-decl distinction is Emit-only),
    /// Emit seeds a `Var` name entry but records NO statement (the SUPPRESSED-DECL
    /// path). Returns the [`Vec3Var`](Self::Vec3Var) handle so the body's `n.x` / `n.xy` reads resolve
    /// the name `n`.
    fn decl_param_vec3(name: &'static str, init: Self::Vec3f) -> Self::Vec3Var;

    /// Reads the CURRENT `float3` value of a [`Vec3Var`](Self::Vec3Var). Eval returns the `Cell`'s
    /// `[f32; 3]`; Emit returns a [`Vec3f`](Self::Vec3f) handle spelling the variable's NAME (`n`).
    fn get_var_vec3(v: &Self::Vec3Var) -> Self::Vec3f;

    /// Assigns a [`Vec3Var`](Self::Vec3Var) (`n = <expr>;`). Eval `set`s the `Cell`; Emit records a
    /// `Stmt::Assign` whose rhs is the `float3` expression (a BARE `n = ...;`, NO `float3` decl).
    fn set_var_vec3(v: &Self::Vec3Var, val: Self::Vec3f);

    /// A MUTABLE `float2` LOCAL (the `float2 e = n.xy;` declared local, reassigned inside the `if`).
    /// The `float2` analogue of [`Var`](Self::Var) (a `float` local) / [`Vec3Var`](Self::Vec3Var):
    /// Eval stores the `[f32; 2]` in a [`core::cell::Cell`]; Emit records a `Stmt::DeclVar` whose `ty`
    /// is `crate::emit::EmitTy::Float2` (`float2 e = <init>;`). Owned, passed by `&` to
    /// [`get_var_vec2`](Self::get_var_vec2) / [`set_var_vec2`](Self::set_var_vec2).
    type Vec2Var;

    /// Declares a mutable `float2` local named `name` initialized to `init` (`float2 e = n.xy;`) — the
    /// `float2` analogue of [`decl_var`](Self::decl_var) (a `float` local). Eval boxes `init` into a
    /// `Cell<[f32; 2]>`; Emit records a `Stmt::DeclVar` (`ty = Float2`, `rhs = init`). Returns the
    /// [`Vec2Var`](Self::Vec2Var) handle.
    fn decl_var_vec2(name: &'static str, init: Self::Vec2f) -> Self::Vec2Var;

    /// Reads the CURRENT `float2` value of a [`Vec2Var`](Self::Vec2Var). Eval returns the `Cell`'s
    /// `[f32; 2]`; Emit returns a [`Vec2f`](Self::Vec2f) handle spelling the variable's NAME (`e`).
    fn get_var_vec2(v: &Self::Vec2Var) -> Self::Vec2f;

    /// Assigns a [`Vec2Var`](Self::Vec2Var) (`e = <expr>;`). Eval `set`s the `Cell`; Emit records a
    /// `Stmt::Assign` whose rhs is the `float2` expression.
    fn set_var_vec2(v: &Self::Vec2Var, val: Self::Vec2f);

    /// `v.xy` — a `float3` → `float2` swizzle (`n.xy`). Eval drops the `.z` lane (`[v[0], v[1]]`); Emit
    /// records a `crate::emit::Node::Vec2Swizzle` printing `<src>.xy`. Result [`Vec2f`](Self::Vec2f).
    fn vec3_xy(v: Self::Vec3f) -> Self::Vec2f;

    /// `v.yx` — a `float2` → `float2` lane SWAP (`e.yx`). Eval swaps (`[v[1], v[0]]`); Emit records a
    /// `crate::emit::Node::Vec2Swizzle` printing `<src>.yx`. Result [`Vec2f`](Self::Vec2f).
    fn vec2_yx(v: Self::Vec2f) -> Self::Vec2f;

    /// `v.x` — a `float2` → `float` component read (`e.x`). Eval reads lane 0; Emit records a
    /// `crate::emit::Node::Vec2Comp` printing `<src>.x`. Result [`Scalar`](Self::Scalar).
    fn vec2_x(v: Self::Vec2f) -> Self::Scalar;

    /// `v.y` — a `float2` → `float` component read (`e.y`). Eval reads lane 1; Emit records a
    /// `crate::emit::Node::Vec2Comp` printing `<src>.y`. Result [`Scalar`](Self::Scalar).
    fn vec2_y(v: Self::Vec2f) -> Self::Scalar;

    /// `abs(v)` — a component-wise `float2` absolute value (`abs(e.yx)`). Eval is `[|x|, |y|]`; Emit
    /// records a `crate::emit::Node::Vec2Abs` printing `abs(<v>)`. Result [`Vec2f`](Self::Vec2f).
    fn vec2_abs(v: Self::Vec2f) -> Self::Vec2f;

    /// `a * b` — a component-wise `float2` multiply (`(1.0 - abs(e.yx)) * float2(...)`). Eval is `[a0*b0,
    /// a1*b1]`; Emit records a `crate::emit::Node::Vec2Mul` (the `float2` analogue of
    /// [`vec3_mul_scalar`](Self::vec3_mul_scalar), but BOTH operands `float2`). Result
    /// [`Vec2f`](Self::Vec2f).
    fn vec2_mul(a: Self::Vec2f, b: Self::Vec2f) -> Self::Vec2f;

    /// `v * s` — a `float2` times a `float` scalar (`e * 0.5`). Eval is `[v0*s, v1*s]`; Emit records a
    /// `crate::emit::Node::Vec2MulScalar` (the `float2` analogue of
    /// [`vec3_mul_scalar`](Self::vec3_mul_scalar)). Result [`Vec2f`](Self::Vec2f).
    fn vec2_mul_scalar(v: Self::Vec2f, s: Self::Scalar) -> Self::Vec2f;

    /// `v + s` — a `float2` plus a `float` scalar broadcast (`... + 0.5`). Eval is `[v0+s, v1+s]`; Emit
    /// records a `crate::emit::Node::Vec2AddScalar`. Result [`Vec2f`](Self::Vec2f).
    fn vec2_add_scalar(v: Self::Vec2f, s: Self::Scalar) -> Self::Vec2f;

    /// `s - v` — a `float` scalar (broadcast) MINUS a `float2`, scalar on the LEFT (`1.0 - abs(e.yx)`).
    /// Eval is `[s-v0, s-v1]`; Emit records a `crate::emit::Node::Vec2RSubScalar` printing `<s> -
    /// <v>` (the scalar-LHS form, DISTINCT from a `float2 - float` which has no committed use here).
    /// Result [`Vec2f`](Self::Vec2f).
    fn vec2_rsub_scalar(s: Self::Scalar, v: Self::Vec2f) -> Self::Vec2f;

    /// `cond ? t : e` — the BARE scalar ternary, NO parentheses on the condition OR the arms (the
    /// committed `oct_encode` sign-ternary `e.x >= 0.0 ? 1.0 : -1.0`). DISTINCT from
    /// [`select`](Self::select) (which records the BOTH-arms-wrapped `SelectParen` form of the
    /// regula-falsi root-finder) and [`FieldScalar::select`] (the condition-wrapped `(cond) ? t : e`):
    /// `oct_encode` spells the un-parenthesized form (the comparand `e.x >= 0.0` + the literals
    /// `1.0`/`-1.0` are all leaves, so no precedence wrap is needed). Eval is the eager `if cond { t }
    /// else { e }` (both arms pure `±1.0` literals); Emit records a `crate::emit::Node::SelectBare`
    /// printing all three parts un-wrapped.
    fn select_bare(
        cond: <Self::Scalar as FieldScalar>::Mask,
        t: Self::Scalar,
        e: Self::Scalar,
    ) -> Self::Scalar;

    // ---- Rung E: the particle-leaf prerequisite facets (docs/PARTICLES-PLAN.md) ----------
    //
    // The seven particle leaves need four op families this axis could not express: the
    // bitwise/shift `uint` ops the PCG32 hash folds (E1), the bit-cast + half-precision
    // conversions the packed per-particle attributes decode/encode (E2), a real `dot` (E3),
    // and the two transcendentals the billboard corner spins with (E4).
    //
    // TWO of E1's five listed ops ALREADY EXIST and are deliberately NOT duplicated: the
    // bitwise AND is [`and_u`](Self::and_u) and the logical right shift is
    // [`shr_u`](Self::shr_u) (Track B Increment G1, the `pack_material_id_ba` byte split).
    // A second name pushing the SAME node and printing the SAME text would be two spellings
    // of one op, and the committed `.spv` is pinned to the existing pair.
    //
    // PRECEDENCE — the one composition rule to know. The bitwise/shift operators bind LOOSER
    // than `+ - * /`, so `crate::emit`'s printer WRAPS any bitwise/shift operand that sits
    // inside an infix parent (`(state << 13u) ^ state`, `(a ^ b) * c`). The frozen
    // [`and_u`](Self::and_u) / [`shr_u`](Self::shr_u) printer arms keep spelling THEIR operands
    // un-wrapped (byte-identity with the committed `pack_material_id_ba`), so a body that nests
    // a NEW bitwise node INSIDE `and_u`/`shr_u` must materialize it first
    // ([`temp_uint`](Self::temp_uint)); nesting the other way round wraps correctly.
    //
    // SHIFT AMOUNTS: `shr_u`'s Eval arm is the plain host `>>`, which PANICS in debug for an
    // amount >= 32 instead of masking it the way the GPU does (see [`ushl`](Self::ushl) for the
    // measured masking rule). Every committed shift amount is a small constant, so the two
    // agree on the whole reachable domain; a leaf that ever needs an unbounded dynamic right
    // shift must mask the amount itself.

    /// `a << b` over two [`Uint`](Self::Uint)s — the LEFT shift (the PCG32 hash's diffusion
    /// step). The `<<` analogue of the existing [`shr_u`](Self::shr_u) (`>>`).
    ///
    /// SHIFT-AMOUNT MASKING (measured, `dxc -T cs_6_0 -spirv`, Vulkan SDK 1.4.350): DXC lowers
    /// `a << b` to `OpBitwiseAnd %b %uint_31` followed by `OpShiftLeftLogical` — the HLSL/D3D
    /// rule that only the LOW 5 BITS of the shift amount are used, so shifting by 32 is
    /// shifting by 0, NOT a zero result. The Eval arm therefore uses `u32::wrapping_shl`, which
    /// masks by 31 identically; the two backends agree on the FULL `u32` amount domain, not
    /// merely on the in-range part.
    fn ushl(a: Self::Uint, b: Self::Uint) -> Self::Uint;

    /// `a ^ b` over two [`Uint`](Self::Uint)s — the bitwise XOR (`OpBitwiseXor`), the PCG32
    /// hash's mixing step. On Eval the host `^` over `u32`s (exact, no wrapping question); on
    /// Emit a `crate::emit` `Xor` node.
    fn uxor(a: Self::Uint, b: Self::Uint) -> Self::Uint;

    /// `a | b` over two [`Uint`](Self::Uint)s — the bitwise OR (`OpBitwiseOr`), the packed-pair
    /// assembly (`lo | (hi << 16u)`). SEPARATE from the logical [`or`](Self::or) (which joins
    /// two Masks and prints `||`): this is `|` over two `uint` VALUES, result-typed
    /// [`Uint`](Self::Uint).
    fn uor(a: Self::Uint, b: Self::Uint) -> Self::Uint;

    /// `asuint(x)` — the BIT-REINTERPRET `float -> uint` (`OpBitcast`), NOT the numeric
    /// truncating cast [`float_to_uint`](Self::float_to_uint) (`(uint)f`, `OpConvertFToU`).
    /// The two are a standing confusion and produce completely different values, so they are
    /// distinct nodes with distinct spellings. On Eval `f32::to_bits`.
    fn asuint(x: Self::Scalar) -> Self::Uint;

    /// `asfloat(u)` — the BIT-REINTERPRET `uint -> float` (`OpBitcast`), NOT the numeric
    /// widening cast [`float_from_uint`](Self::float_from_uint) (`(float)u`, `OpConvertUToF`).
    /// On Eval `f32::from_bits` (total: every `u32` bit pattern is a valid `f32`, including the
    /// NaN payloads).
    fn asfloat(u: Self::Uint) -> Self::Scalar;

    /// `f16tof32(u)` — widens the IEEE 754 binary16 in the LOW 16 bits of `u` to a `float`
    /// (the HIGH 16 bits are ignored, so a packed pair may be passed directly). MEASURED
    /// lowering: `OpExtInst GLSL.std.450 UnpackHalf2x16` + `OpCompositeExtract 0`. On Eval
    /// [`crate::half::f16_bits_to_f32`] — the IEEE conversion, subnormals and NaN payloads
    /// included.
    fn f16tof32(u: Self::Uint) -> Self::Scalar;

    /// `f32tof16(x)` — narrows `x` to IEEE 754 binary16 in the LOW 16 bits of a `uint` (the
    /// HIGH 16 bits are zero). MEASURED lowering: `OpExtInst GLSL.std.450 PackHalf2x16` over
    /// `float2(x, 0.0)`, i.e. round-to-nearest-EVEN with real subnormals, ±Inf on overflow and
    /// a truncated-payload NaN. On Eval [`crate::half::f32_to_f16_bits`].
    fn f32tof16(x: Self::Scalar) -> Self::Uint;

    /// `dot(a, b)` over two `float3`s — the HLSL `dot` INTRINSIC (`OpDot`), returning a
    /// [`Scalar`](Self::Scalar).
    ///
    /// DISTINCT from [`crate::scalar::v_dot`], which spells the EXPLICIT scalar fold
    /// `a.x*b.x + a.y*b.y + a.z*b.z` precisely BECAUSE the frozen field leaves must stay
    /// byte-identical to a host oracle and `OpDot` is free to contract into an FMA chain. This
    /// node is the opposite trade: a leaf that spells `dot(...)` accepts that its host oracle
    /// is a CLOSE, not bit-exact, mirror (the same standing carve-out division carries). Use
    /// [`crate::scalar::v_dot`] whenever a bit-exact contract is required.
    ///
    /// On Eval the left-associated fold `(a.x*b.x + a.y*b.y) + a.z*b.z`.
    fn vec3_dot(a: Self::Vec3f, b: Self::Vec3f) -> Self::Scalar;

    /// `sin(x)` — the HLSL `sin` intrinsic (`OpExtInst GLSL.std.450 Sin`).
    ///
    /// The transcendentals live HERE (on the control-flow axis, whose Eval instantiation is a
    /// codegen-only ZST no physics-reachable code calls) rather than on
    /// [`FieldScalar`](crate::scalar::FieldScalar), whose `f32` impl IS the physics leaf — the
    /// same firewall reasoning [`crate::interp::InterpBackend`] states, which is why trig has
    /// so far existed ONLY on that codegen-gated backend. Both spell the identical HLSL, so a
    /// leaf may be authored against either axis.
    ///
    /// The Eval arm is the `nightly`/`std` shim [`FieldScalar::sqrt`](crate::scalar::FieldScalar::sqrt)
    /// uses for the other op stable `core` lacks.
    fn sin(x: Self::Scalar) -> Self::Scalar;

    /// `cos(x)` — the HLSL `cos` intrinsic (`OpExtInst GLSL.std.450 Cos`). The companion of
    /// [`sin`](Self::sin); see it for the axis + Eval-shim rationale.
    fn cos(x: Self::Scalar) -> Self::Scalar;

    /// `rsqrt(x)` — the HLSL reciprocal-square-root intrinsic (measured lowering:
    /// `OpExtInst GLSL.std.450 InverseSqrt`), the op a unit-vector / rotation-pair
    /// renormalization is written with instead of a `sqrt` followed by a divide.
    ///
    /// NOT bit-exact: `InverseSqrt` is an APPROXIMATE instruction (the Vulkan precision table
    /// allows 2 ULP), so a host oracle mirroring it as `1.0 / sqrt(x)` — which is what the Eval
    /// arm does — agrees to a tolerance, not to the bit. The same standing carve-out
    /// [`vec3_dot`](Self::vec3_dot) and division carry: a leaf that spells `rsqrt` has opted
    /// out of a byte-identity contract for that value.
    ///
    /// The DIVIDE this replaces needs no new facet — the scalar `/` is
    /// [`FieldScalar::div`](crate::scalar::FieldScalar::div), already reachable from any
    /// `C: Cf` body through [`Scalar`](Self::Scalar)'s own trait bound (as
    /// [`crate::pack::pack_material_id_ba_body`] spells it).
    fn rsqrt(x: Self::Scalar) -> Self::Scalar;

    // ---- UI-ADVANCED S1: the `ui_rect` fragment-leaf facets (`docs/UI-PLAN-SPRITES.md`) ----
    //
    // The six UI leaves (`crate::ui`) are the eDSL's first `float2`/`float4` VALUE math: the
    // per-corner rounded-box SDF, the clip-AABB coverage, the MSDF median/range pair, the
    // premultiplied border-over-fill composite, and the RGBA8 unpack. Every facet below is a
    // 2-line mirror of a proven `float2`/`float3` facet; the two genuinely new op families are
    // the `float2` INTRINSICS (`smoothstep`/`length`/`dot`/`fwidth`) and the `float4`
    // arithmetic (`float4(...)` ctor, `* s`, `+`, `.a`). `fwidth` is the one facet with NO
    // host semantics — its Eval arm is an honest panic (the [`call1`](Self::call1) discipline),
    // so the leaf that spells it (`ui_screen_px_range_body`) is deliberately not oracle-swept.

    /// The `float4` RETURN-VALUE cell (`Cell<[f32; 4]>` on Eval — the body-local cell the
    /// producer reads after the body runs; a ZST on Emit, the expression travels in the
    /// recorded `Stmt::Return`). The `float4` analogue of [`RetCellV2`](Self::RetCellV2).
    /// Owned, passed by `&` to [`ret_vec4`](Self::ret_vec4).
    type RetCellV4;

    /// The `float4` function-return — `return <float4>;` (the `ui_unpack_rgba8` /
    /// `ui_premultiplied_over` tails). The `float4` analogue of [`ret_vec2`](Self::ret_vec2).
    /// On Eval deposits `value` into the [`RetCellV4`](Self::RetCellV4) and returns
    /// [`Break`](LoopOp::Return); on Emit records a single `Stmt::Return`.
    fn ret_vec4(cell: &Self::RetCellV4, value: Self::Vec4f) -> Flow;

    /// `a - b` over two `float2`s (`abs(p) - half_size`) — component-wise. The `float2`
    /// analogue of [`vec3_sub`](Self::vec3_sub). On Eval `[a0-b0, a1-b1]`; on Emit a
    /// `crate::emit` `Vec2Sub` node (additive — flat on the LEFT of a same-class parent).
    fn vec2_sub(a: Self::Vec2f, b: Self::Vec2f) -> Self::Vec2f;

    /// `v - s` — a `float2` MINUS a `float` scalar broadcast (`clip.xy - fw`), vector on the
    /// LEFT. The operand-order complement of [`vec2_rsub_scalar`](Self::vec2_rsub_scalar)
    /// (`s - v`). On Eval `[v0-s, v1-s]`; on Emit a `Vec2SubScalar` node.
    fn vec2_sub_scalar(v: Self::Vec2f, s: Self::Scalar) -> Self::Vec2f;

    /// `max(v, s)` — a component-wise `float2` max against a scalar broadcast (`max(q, 0.0)`,
    /// the rounded-box outside clamp; HLSL promotes the scalar arg). On Eval
    /// `[max(v0,s), max(v1,s)]`; on Emit a `Vec2MaxScalar` node printed as the intrinsic call.
    fn vec2_max_scalar(v: Self::Vec2f, s: Self::Scalar) -> Self::Vec2f;

    /// `length(v)` over a `float2` (`length(max(q, 0.0))`) — `OpExtInst GLSL.std.450 Length`.
    /// The Eval arm is `sqrt(x*x + y*y)` over the host f32s — `Length` carries the same
    /// sqrt-family precision as [`FieldScalar::sqrt`], so the rounded-box oracle sweep pins
    /// table points where the radicand is an exact square (the corner cases) rather than
    /// claiming bit-exactness of the general norm.
    fn vec2_length(v: Self::Vec2f) -> Self::Scalar;

    /// `dot(a, b)` over two `float2`s (`dot(unit_range, screen_tex_sz)`) — `OpDot`, free to
    /// contract into an FMA like [`vec3_dot`](Self::vec3_dot), so it carries no bit-exact
    /// oracle contract. On Eval the left-associated fold `a.x*b.x + a.y*b.y`.
    fn vec2_dot(a: Self::Vec2f, b: Self::Vec2f) -> Self::Scalar;

    /// `smoothstep(e0, e1, x)` over three `float2`s (the clip AA band) — `OpExtInst
    /// GLSL.std.450 SmoothStep`, component-wise. The Eval arm mirrors the spec polynomial
    /// `t*t*(3 - 2*t)` over `t = clamp((x-e0)/(e1-e0), 0, 1)` — it contains a DIVIDE, so per
    /// the house rule it is exact only at the saturated ends (`x <= e0` ⇒ 0, `x >= e1` ⇒ 1),
    /// which is where the clip-coverage oracle pins its table.
    fn vec2_smoothstep(e0: Self::Vec2f, e1: Self::Vec2f, x: Self::Vec2f) -> Self::Vec2f;

    /// `fwidth(v)` over a `float2` (`fwidth(uv)`) — the FRAGMENT-stage derivative `OpFwidth`.
    /// NO host semantics exist for a device derivative, so the Eval arm is an honest
    /// `unreachable!` (the [`call1`](Self::call1) discipline — a loud panic, never a wrong
    /// value); a leaf that spells it is not oracle-swept and says so in its own doc.
    fn vec2_fwidth(v: Self::Vec2f) -> Self::Vec2f;

    /// `s / v` — a `float` scalar (broadcast) DIVIDED by a `float2`, scalar on the LEFT
    /// (`g_atlas_ubo.px_range / g_atlas_ubo.atlas_size`, `1.0 / fwidth(uv)`). A DIVIDE —
    /// `OpFDiv` carries 2.5 ULP, so per the house rule it is never part of a bit-exact
    /// contract; the facet exists because the committed MSDF range math spells it. On Eval
    /// `[s/v0, s/v1]`; on Emit a `Vec2RDivScalar` node.
    fn vec2_rdiv_scalar(s: Self::Scalar, v: Self::Vec2f) -> Self::Vec2f;

    /// `cond ? t : e` over `float2` ARMS — the rounded-box radius-pair select
    /// (`(p.x > 0.0) ? r.yz : r.xw`). The `float2` analogue of [`FieldScalar::select`] (the
    /// cond-wrapped, arms-bare form). On Eval the eager `if cond { t } else { e }` (both arms
    /// are pure swizzles); on Emit a `SelectVec2` node.
    fn select_vec2(
        cond: <Self::Scalar as FieldScalar>::Mask,
        t: Self::Vec2f,
        e: Self::Vec2f,
    ) -> Self::Vec2f;

    /// `v.xy` — a `float4` → `float2` two-lane swizzle (`clip.xy`, the clip AABB min). On Eval
    /// `[v[0], v[1]]`; on Emit a `Vec4SwizzleV2` node (mask 0). DISTINCT from
    /// [`vec3_xy`](Self::vec3_xy) (a `float3` source).
    fn vec4_xy(v: Self::Vec4f) -> Self::Vec2f;

    /// `v.zw` — the clip AABB max half (`clip.zw`). On Eval `[v[2], v[3]]`; mask 1.
    fn vec4_zw(v: Self::Vec4f) -> Self::Vec2f;

    /// `v.yz` — the rounded-box RIGHT-side radius pair `(tr, br)` (`r.yz`). On Eval
    /// `[v[1], v[2]]`; mask 2.
    fn vec4_yz(v: Self::Vec4f) -> Self::Vec2f;

    /// `v.xw` — the rounded-box LEFT-side radius pair `(tl, bl)` (`r.xw`). On Eval
    /// `[v[0], v[3]]`; mask 3.
    fn vec4_xw(v: Self::Vec4f) -> Self::Vec2f;

    /// `v.a` — a `float4` → `float` ALPHA read (`src.a`, the premultiplied-over source alpha),
    /// spelled `.a` (the committed color spelling), not `.w`. On Eval `v[3]`; on Emit a
    /// `Vec4Alpha` node.
    fn vec4_alpha(v: Self::Vec4f) -> Self::Scalar;

    /// `float4(<x>, <y>, <z>, <w>)` from FOUR already-`float` scalar expressions (the
    /// `ui_unpack_rgba8` channel ctor). The `float4` analogue of
    /// [`vec2_from_scalars`](Self::vec2_from_scalars). On Eval `[x, y, z, w]`; on Emit a
    /// `Vec4FromScalars` node.
    fn vec4_from_scalars(
        x: Self::Scalar,
        y: Self::Scalar,
        z: Self::Scalar,
        w: Self::Scalar,
    ) -> Self::Vec4f;

    /// `v * s` — a `float4` times a `float` scalar (`bc * border_cov`, `float4(...) * (1.0 /
    /// 255.0)`). The `float4` analogue of [`vec2_mul_scalar`](Self::vec2_mul_scalar). On Eval
    /// `[v0*s, .., v3*s]`; on Emit a `Vec4MulScalar` node (whose printer wraps a NON-LEAF
    /// scalar operand — the committed `(1.0 / 255.0)` / `(1.0 - src.a)` parens).
    fn vec4_mul_scalar(v: Self::Vec4f, s: Self::Scalar) -> Self::Vec4f;

    /// `a + b` over two `float4`s (`src + dst * (1.0 - src.a)`, the premultiplied OVER) —
    /// component-wise. On Eval `[a0+b0, .., a3+b3]`; on Emit a `Vec4Add` node.
    fn vec4_add(a: Self::Vec4f, b: Self::Vec4f) -> Self::Vec4f;

    /// Declares a NAMED `float2` temp (`float2 rx = <rhs>;`) — the `float2` analogue of
    /// [`temp_vec3`](Self::temp_vec3) / [`temp_float`](Self::temp_float). Eval is identity
    /// (the value flows directly); Emit records a named `float2` `Stmt::DeclTemp`. Returns the
    /// temp handle so later reads spell `rx`.
    fn temp_vec2(name: &'static str, v: Self::Vec2f) -> Self::Vec2f;

    // ---- UI-ADVANCED S5: the two `float2` primitives `ui_tile_uv` needs -------------------
    //
    // MEASURED at the S5 build: neither existed. `frac` / `floor` / `fract` occurred NOWHERE
    // in this crate (S-D15 (4) says so), and NEITHER did a `float2` `lerp` — the only `lerp`
    // on the axis is the SCALAR [`crate::scalar::FieldScalar::lerp`]. S-D15 (4)'s cost line
    // named one primitive; it is two, and the second is the one that matters for byte
    // identity (see [`vec2_lerp`](Self::vec2_lerp)).

    /// A NAMED `uint` constant that types as `uint` on the Emit backend — `FLAG_TILED`,
    /// `UI_TILE_X_SHIFT`, `UI_TILE_MASK`.
    ///
    /// DISTINCT from [`named_uint`](Self::named_uint), whose Emit node is a `NamedLit`
    /// typed `Float`: that one was minted for a symbol whose only consumer is a bare
    /// `return` (no operand type check), and these three are operands of
    /// [`and_u`](Self::and_u) / [`shr_u`](Self::shr_u), which DO check their operands
    /// `Uint`. The alternative — spelling them as bare `uint` literals — would put a
    /// second copy of the S-D2 bit layout inside the leaf body, beside the copy
    /// `emit_hlsl_ui_flag_consts` generates from the layout; S-D10's rule is that no
    /// shader spells a number a host constant also spells.
    ///
    /// On Eval returns `val` (the real number, so the oracle sweeps the real decode); on
    /// Emit records the SYMBOL.
    fn named_uint_val(sym: &'static str, val: u32) -> Self::Uint;

    /// `frac(v)` over a `float2` — the component-wise fractional part, `OpExtInst
    /// GLSL.std.450 Fract`. The wrap that makes `Tile` tile (S-D15's
    /// `frac(local_uv * tiles)`).
    ///
    /// On Eval `x - x.floor()` per lane, which is HLSL's own definition and matches
    /// `Fract` on the whole finite domain including negatives (`frac(-0.25) == 0.75`).
    /// Exact: a subtract of two exactly-representable values, no rounding step — so unlike
    /// a divide it CAN carry a bit-exact oracle contract.
    fn vec2_frac(v: Self::Vec2f) -> Self::Vec2f;

    /// `lerp(a, b, t)` over three `float2`s — the linear blend, `OpExtInst GLSL.std.450
    /// FMix`. It is the UNTILED arm of `ui_tile_uv`, and it is spelled as the intrinsic
    /// rather than decomposed into `a + t * (b - a)` ON PURPOSE: `FMix`'s specified form is
    /// `x * (1 - t) + y * t`, the decomposition rounds differently, and the four S2 / one S3
    /// / one S4 image pins were blessed against the committed `lerp(inst.uv.xy, inst.uv.zw,
    /// input.local_uv)`. Keeping the intrinsic makes the untiled sprite's pixel IDENTICAL
    /// rather than merely equal to within a ULP — which is the difference between six image
    /// pins that hold by construction and six that hold by luck (`reference-golden-fp-
    /// resolution`: an 8-bit golden cannot SEE a 1-ULP shader edit, so it would not have
    /// caught the decomposition either way).
    ///
    /// On Eval `a * (1 - t) + b * t` per lane — the spec form, not the decomposition.
    /// Mul/add only, so it carries the crate's standing FMA-contraction carve-out and no
    /// divide.
    fn vec2_lerp(a: Self::Vec2f, b: Self::Vec2f, t: Self::Vec2f) -> Self::Vec2f;

    // `temp_vec4` (`float4 src = <rhs>;`) already exists on this axis (the Increment-5c
    // `m2_brick_cubic_hit` facet above) and is reused by the UI leaves rather than
    // re-declared; its Eval arm becomes the identity now that a leaf
    // (`ui_premultiplied_over_body`) legitimately runs over EvalCf.
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

    // The mutable `bool` local IS its value, held in a `Cell<bool>` for interior mutability (the
    // same shape `Var` uses for `float`). `Cell<bool>` is 1 byte and adds NO field to the ZST
    // backend marker, so `size_of::<EvalCf>() == 0` still holds.
    type BoolVar = core::cell::Cell<bool>;

    #[inline]
    fn decl_bool_var(_name: &'static str, init: bool) -> core::cell::Cell<bool> {
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
                // `Break(Return)` is a FUNCTION return. The RUNTIME-loop carrier is
                // [`runtime_for`](Cf::runtime_for) (Inc 4a), which FORWARDS it to the
                // function-scope IIFE's `?`; an UNROLLED loop's body never has a `ret`
                // (`dist_to_brick_exit` has no early return), so a `Return` reaching an
                // `unroll_for` is a body that wired a `ret` inside an `[unroll]` without an
                // enclosing IIFE: a bug, not a silent loop-break.
                core::ops::ControlFlow::Break(LoopOp::Return) => {
                    debug_assert!(
                        false,
                        "LoopOp::Return reached an unroll_for level: a `ret` in a runtime loop \
                         is carried by runtime_for (which forwards it to the function IIFE's \
                         `?`); an unrolled loop body has no early return"
                    );
                    break;
                }
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

    #[inline]
    fn brk() -> Flow {
        // The loop-break token. `runtime_for`/`unroll_for` CONSUME it (the `Break(Break)`
        // arms map it to a real `continue`/`break`); their consumer arms already exist+tested
        // (this is the matching PRODUCER, Inc 4b).
        core::ops::ControlFlow::Break(LoopOp::Break)
    }

    // ---- Increment 3 typed facets (native host: real casts, real ||, Cell out-param) ----
    type Uint = u32;
    type Uint3 = [u32; 3];
    type Vec3f = [f32; 3];
    type OutVec3 = core::cell::Cell<[f32; 3]>;
    type RetCell = core::cell::Cell<u32>;
    type Buf<'a> = &'a [u32];

    #[inline]
    fn vec3_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    #[inline]
    fn vec3_add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    #[inline]
    fn vec3_div_scalar(v: [f32; 3], s: f32) -> [f32; 3] {
        // The HLSL `float3 / float` broadcasts `s` to a `float3` then divides component-wise
        // (the spike disassembly's `OpCompositeConstruct %93 %93 %93` then `OpFDiv`).
        [v[0] / s, v[1] / s, v[2] / s]
    }
    #[inline]
    fn vec3_mul_scalar(v: [f32; 3], s: f32) -> [f32; 3] {
        [v[0] * s, v[1] * s, v[2] * s]
    }
    #[inline]
    fn vec3_from_uints(x: u32, y: u32, z: u32) -> [f32; 3] {
        // The implicit uint→float in the HLSL `float3(ix, iy, iz)` ctor (`OpConvertUToF`).
        [x as f32, y as f32, z as f32]
    }

    #[inline]
    fn vec3_x(v: [f32; 3]) -> f32 {
        v[0]
    }
    #[inline]
    fn vec3_y(v: [f32; 3]) -> f32 {
        v[1]
    }
    #[inline]
    fn vec3_z(v: [f32; 3]) -> f32 {
        v[2]
    }

    #[inline]
    fn uint3_x(d: [u32; 3]) -> u32 {
        d[0]
    }
    #[inline]
    fn uint3_y(d: [u32; 3]) -> u32 {
        d[1]
    }
    #[inline]
    fn uint3_z(d: [u32; 3]) -> u32 {
        d[2]
    }

    #[inline]
    fn float_to_uint(f: f32) -> u32 {
        // `f as u32` — the SATURATING cast Rust 2024 gives `as`. For the reachable inputs
        // (guard 1 has already rejected any negative `rel`, so `f >= 0` here), this matches
        // HLSL's `OpConvertFToU` truncation. (The negative case never reaches this on Eval —
        // the `?` in `if_ret` short-circuits the IIFE before the cast statements run.)
        f as u32
    }

    #[inline]
    fn uadd(a: u32, b: u32) -> u32 {
        // The host grid index is constructed to be in-bounds (the bounds guard ran), so the
        // arithmetic does not overflow on any reachable input; `wrapping_add` matches HLSL's
        // modular `OpIAdd` for the (unreachable) overflow case rather than panicking in debug.
        a.wrapping_add(b)
    }
    #[inline]
    fn umul(a: u32, b: u32) -> u32 {
        a.wrapping_mul(b)
    }

    #[inline]
    fn named_uint(_sym: &'static str, val: u32) -> u32 {
        // The symbol is an Emit-only printing concern; Eval uses the concrete value.
        val
    }

    #[inline]
    fn buffer_load(buf: &[u32], idx: u32) -> u32 {
        // `grid[idx]` — the real slice index. The index was constructed in-bounds (the
        // bounds guard `ix >= dims.x || ...` ran and returned BRICK_OUTSIDE_GRID otherwise,
        // and the grid length == dims.x*dims.y*dims.z), so this never panics on a reachable
        // input. The oracle sweep's out-of-grid inputs early-return before reaching here.
        buf[idx as usize]
    }

    #[inline]
    fn uge(a: u32, b: u32) -> bool {
        a >= b
    }

    #[inline]
    fn or(a: bool, b: bool) -> bool {
        // Eager: both masks are already-computed (side-effect-free `<`/`>=`), so `a || b` is
        // result-equivalent to the short-circuit. (The tail-skip is preserved by statement
        // order + the `ret`'s `?`, not by short-circuiting the comparands.)
        a || b
    }

    // ---- Increment 4f: the B1 sor-retreat condition leaves (native host) --------------

    #[inline]
    fn ugt(a: u32, b: u32) -> bool {
        a > b
    }

    #[inline]
    fn and2(a: bool, b: bool) -> bool {
        // Eager: both masks are already-computed (the `it > 0u` guard + the Lipschitz `<`, each
        // a side-effect-free read), so `a && b` is result-equivalent to the GPU short-circuit
        // (the SAME eager-mask equivalence `or` carries).
        a && b
    }

    #[inline]
    fn uint_lit(x: u32) -> u32 {
        // The literal IS its value; the `<x>u` suffix is an Emit-only printing concern.
        x
    }

    #[inline]
    fn temp_vec3(_name: &'static str, v: [f32; 3]) -> [f32; 3] {
        // IDENTITY on Eval — materialization is an emit-only printing concern.
        v
    }
    #[inline]
    fn temp_uint(_name: &'static str, u: u32) -> u32 {
        u
    }

    #[inline]
    fn out_vec3_assign(o: &core::cell::Cell<[f32; 3]>, v: [f32; 3]) {
        o.set(v);
    }

    #[inline]
    fn if_ret(cell: &core::cell::Cell<u32>, cond: bool, value: u32) -> Flow {
        if cond {
            cell.set(value);
            core::ops::ControlFlow::Break(LoopOp::Return)
        } else {
            core::ops::ControlFlow::Continue(())
        }
    }

    #[inline]
    fn ret(cell: &core::cell::Cell<u32>, value: u32) -> Flow {
        cell.set(value);
        core::ops::ControlFlow::Break(LoopOp::Return)
    }

    // ---- Increment 4a: the runtime `[loop]` + the FLOAT return facet (native host) ----
    type RetCellF = core::cell::Cell<f32>;
    type Vec4f = [f32; 4];

    #[inline]
    fn decl_param(_name: &'static str, init: f32) -> core::cell::Cell<f32> {
        // On Eval a suppressed-decl param is IDENTICAL to `decl_var` — the "no decl
        // recorded" distinction is an Emit-only printing concern (a `Cell` has no printed
        // identity). The carried-state model (a `Cell<f32>` the loop closure mutates and the
        // tail reads) is the SAME interior-mutability pattern the other vars use.
        core::cell::Cell::new(init)
    }

    #[inline]
    fn temp_float(_name: &'static str, x: f32) -> f32 {
        // IDENTITY on Eval — materialization is an emit-only printing concern.
        x
    }

    #[inline]
    fn select(cond: bool, t: f32, e: f32) -> f32 {
        // Eager `if cond { t } else { e }` — both arms are already computed (pure float
        // arithmetic). On the degenerate `denom ≈ 0` path the discarded then-arm produces an
        // `inf` (`f_lo * (hi - lo) / 0`), but it is NEVER read (the else bisection arm is
        // selected), so the Eval result matches the GPU (which also computes both ternary
        // arms and selects). This is the SAME value-select shape `FieldScalar::select` has.
        if cond { t } else { e }
    }

    #[inline]
    fn call2(_fn_sym: &'static str, _a: [f32; 4], _b: f32) -> f32 {
        // On Eval the frozen callee (`m2_cubic_eval`) is invoked through a closure the
        // generic body threads (the field-call seam, like `sdf_normal_body`'s `sdf`
        // closure) — NOT through this hook, which is the EMIT call-site recorder. The Eval
        // body never calls `Cf::call2` (it calls the closure directly), so this is
        // UNREACHED on Eval. A panic is the honest signal instead of a wrong value.
        unreachable!(
            "Cf::call2 is the EMIT call-site recorder; the Eval body invokes the frozen \
             callee through the threaded closure, not this hook"
        )
    }

    #[inline]
    fn call1(_fn_sym: &'static str, _a: [f32; 3]) -> f32 {
        // On Eval the frozen callee (`field_distance`) is invoked through the `field` closure
        // the generic body threads (the A1 field-call seam, like `m2_regula_falsi_body`'s
        // `m2_cubic_eval` closure) — NOT through this hook, which is the EMIT call-site
        // recorder. The Eval body never calls `Cf::call1` (it calls the closure directly), so
        // this is UNREACHED on Eval. A panic is the honest signal instead of a wrong value.
        unreachable!(
            "Cf::call1 is the EMIT call-site recorder; the Eval body invokes the frozen \
             callee through the threaded closure, not this hook"
        )
    }

    #[inline]
    fn runtime_for<F: FnMut(usize) -> Flow>(
        _attr: &'static str,
        _iv: &'static str,
        _bound_sym: &'static str,
        bound_val: usize,
        mut body: F,
    ) -> Flow {
        for i in 0..bound_val {
            match body(i) {
                // Fall through to the next iteration.
                core::ops::ControlFlow::Continue(()) => {}
                // The loop CONSUMES its own continue/break.
                core::ops::ControlFlow::Break(LoopOp::Continue) => continue,
                core::ops::ControlFlow::Break(LoopOp::Break) => break,
                // FORWARD the function return to the caller's `?` (the function-scope IIFE):
                // an in-loop `ret_f` short-circuits the tail `ret_f`, so the EARLY `mid`
                // (not the final-iteration `mid`) is the result.
                core::ops::ControlFlow::Break(LoopOp::Return) => {
                    return core::ops::ControlFlow::Break(LoopOp::Return);
                }
            }
        }
        // Natural completion (and the consumed break) fall through to the function tail.
        core::ops::ControlFlow::Continue(())
    }

    #[inline]
    fn if_else<T: FnOnce() -> Flow, E: FnOnce() -> Flow>(cond: bool, then: T, els: E) -> Flow {
        if cond { then() } else { els() }
    }

    #[inline]
    fn if_ret_f(cell: &core::cell::Cell<f32>, cond: bool, value: f32) -> Flow {
        if cond {
            cell.set(value);
            core::ops::ControlFlow::Break(LoopOp::Return)
        } else {
            core::ops::ControlFlow::Continue(())
        }
    }

    #[inline]
    fn ret_f(cell: &core::cell::Cell<f32>, value: f32) -> Flow {
        cell.set(value);
        core::ops::ControlFlow::Break(LoopOp::Return)
    }

    // ---- Increment 4b.2: the BOOL return + OUT-FLOAT facets (native host) -------------
    type RetCellB = core::cell::Cell<bool>;
    type OutFloat = core::cell::Cell<f32>;

    #[inline]
    fn ret_b(cell: &core::cell::Cell<bool>, value: bool) -> Flow {
        cell.set(value);
        core::ops::ControlFlow::Break(LoopOp::Return)
    }

    #[inline]
    fn out_float_assign(o: &core::cell::Cell<f32>, v: f32) {
        o.set(v);
    }

    #[inline]
    fn if_hit_ret_b(
        hit_out: &core::cell::Cell<f32>,
        ret_out: &core::cell::Cell<bool>,
        cond: bool,
        rt_val: f32,
    ) -> Flow {
        if cond {
            // Write `hit_t = rt;` BEFORE the `Break(Return)` short-circuits the IIFE, so the
            // oracle reads the FRESH `rt` (matching the committed `hit_t = rt; return true;`
            // statement order — the design's keystone ordering invariant).
            hit_out.set(rt_val);
            ret_out.set(true);
            core::ops::ControlFlow::Break(LoopOp::Return)
        } else {
            core::ops::ControlFlow::Continue(())
        }
    }

    // ---- Increment 5b: the COMPUTED-bool return facet (native host) -------------------

    #[inline]
    fn ret_b_expr(cell: &core::cell::Cell<bool>, value: bool) -> Flow {
        // The computed bool (`tmax > tmin`, already a host `bool`) is the function's value: deposit
        // it into the cell and return. Identical to `ret_b` on Eval except the value is COMPUTED (a
        // mask) rather than a literal.
        cell.set(value);
        core::ops::ControlFlow::Break(LoopOp::Return)
    }

    // ---- Increment 4e: the BOOL mutable-local facets (native host) --------------------

    #[inline]
    fn decl_bool_param(_name: &'static str, init: bool) -> core::cell::Cell<bool> {
        // On Eval a suppressed-decl bool param is IDENTICAL to `decl_bool_var` — the "no decl
        // recorded" distinction is an Emit-only printing concern (a `Cell` has no printed
        // identity). The carried-state model is the SAME `Cell<bool>` interior-mutability shape.
        core::cell::Cell::new(init)
    }

    #[inline]
    fn get_bool_var(v: &core::cell::Cell<bool>) -> bool {
        v.get()
    }

    #[inline]
    fn set_bool_var(v: &core::cell::Cell<bool>, val: bool) {
        v.set(val);
    }

    // ---- Increment 5a: the SIGNED-INT subsystem + M4Level access-text (native host) ----
    type Int = i32;
    type RetCellI = core::cell::Cell<i32>;

    #[inline]
    fn iv_uint(iv: usize) -> u32 {
        // The host `for` counter narrowed to a `uint` — `L < BRICK_LEVELS = 3` always fits a `u32`.
        iv as u32
    }

    #[inline]
    fn int_lit_signed(x: i32) -> i32 {
        // The literal IS its value; the bare (vs `<x>u`) spelling is an Emit-only printing concern.
        x
    }

    #[inline]
    fn int_from_uint(u: u32) -> i32 {
        // `u as i32` — the in-range value-preserving cast (`L < BRICK_LEVELS = 3` always fits an
        // `i32`), the byte-mirror of HLSL's `(int)L` (`OpBitcast`/`OpUConvert` for a small index).
        u as i32
    }

    #[inline]
    fn all3_ge(p: [f32; 3], o: [f32; 3]) -> bool {
        // `all(p >= o)` — the three lanes ANDed (the host reduction of the GPU `all` intrinsic over
        // a component-wise `>=`). Eager (both comparands are pure float reads).
        p[0] >= o[0] && p[1] >= o[1] && p[2] >= o[2]
    }

    #[inline]
    fn all3_lt(p: [f32; 3], hi: [f32; 3]) -> bool {
        // `all(p < hi)` — the upper-corner analogue (strict `<`: `p == hi` is EXCLUDED).
        p[0] < hi[0] && p[1] < hi[1] && p[2] < hi[2]
    }

    #[inline]
    fn pc_uint(_field: &'static str) -> u32 {
        // On Eval the push-constant `pc.brick_levels` is read through the THREADED CLOSURE the
        // generic body carries (the `call1` field-call seam discipline) — NOT through this hook,
        // which is the EMIT bare-text recorder. The Eval body never calls `Cf::pc_uint` (it calls
        // the closure directly), so this is UNREACHED on Eval. A panic is the honest signal.
        unreachable!(
            "Cf::pc_uint is the EMIT bare-text recorder; the Eval body reads pc.brick_levels \
             through the threaded fixture closure, not this hook"
        )
    }

    #[inline]
    fn level_field_vec3(_l: usize, _field: &'static str) -> [f32; 3] {
        // On Eval the `m2_levels[L].<field>` read is served by the THREADED CLOSURE indexing the
        // host fixture (the `call1` discipline) — NOT through this hook, the EMIT access-text
        // recorder. UNREACHED on Eval; a panic is the honest signal.
        unreachable!(
            "Cf::level_field_vec3 is the EMIT access-text recorder; the Eval body reads the level \
             fixture through the threaded closure, not this hook"
        )
    }

    #[inline]
    fn level_field_scalar(_l: usize, _field: &'static str) -> f32 {
        unreachable!(
            "Cf::level_field_scalar is the EMIT access-text recorder; the Eval body reads the level \
             fixture through the threaded closure, not this hook"
        )
    }

    #[inline]
    fn if_ret_i(cell: &core::cell::Cell<i32>, cond: bool, value: i32) -> Flow {
        if cond {
            cell.set(value);
            core::ops::ControlFlow::Break(LoopOp::Return)
        } else {
            core::ops::ControlFlow::Continue(())
        }
    }

    #[inline]
    fn ret_i(cell: &core::cell::Cell<i32>, value: i32) -> Flow {
        cell.set(value);
        core::ops::ControlFlow::Break(LoopOp::Return)
    }

    // ---- Increment 5c: the DDA marcher subsystem (EMIT-ONLY — every hook unreachable) ----
    //
    // `m2_brick_cubic_hit` calls `m2_corner` → `atlas.SampleLevel(...)` (a `Texture3D` the CPU
    // cannot run), so `m2_brick_cubic_hit_body::<EvalCf>` is NEVER instantiated and NONE of these
    // hooks is ever reached on Eval. A `unreachable!` is the honest signal (the `call1` discipline,
    // precedented by Inc 5a's level/pc Eval hooks) — a wrong value would silently fork the .spv.

    // The array / resource / float4 handles are unit ZSTs on Eval (never constructed — the body is
    // EMIT-ONLY — but the trait requires concrete associated types). `size_of::<EvalCf>() == 0`
    // still holds (these are body-LOCAL types, not fields on the marker).
    type IntArr = ();
    type FloatArr = ();
    type ResTok = ();

    #[inline]
    fn decl_array_int(_name: &'static str, _len: u32) {
        unreachable!("Cf::decl_array_int is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn decl_array_float(_name: &'static str, _len: u32) {
        unreachable!("Cf::decl_array_float is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn arr_int_get(_a: (), _idx: u32) -> i32 {
        unreachable!("Cf::arr_int_get is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn arr_float_get(_a: (), _idx: u32) -> f32 {
        unreachable!("Cf::arr_float_get is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn arr_int_set(_a: (), _idx: u32, _v: i32) {
        unreachable!("Cf::arr_int_set is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn arr_float_set(_a: (), _idx: u32, _v: f32) {
        unreachable!("Cf::arr_float_set is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn arr_int_add_assign(_a: (), _idx: u32, _v: i32) {
        unreachable!("Cf::arr_int_add_assign is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn arr_float_add_assign(_a: (), _idx: u32, _v: f32) {
        unreachable!("Cf::arr_float_add_assign is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }

    #[inline]
    fn call_corner(
        _fn_sym: &'static str,
        _atlas: (),
        _smp: (),
        _tile_org: [f32; 3],
        _cx: u32,
        _cy: u32,
        _cz: u32,
        _inv_atlas: f32,
        _band_half: f32,
    ) -> f32 {
        unreachable!("Cf::call_corner is EMIT-ONLY: atlas.SampleLevel cannot run on the CPU")
    }
    #[inline]
    fn call_coeffs(_fn_sym: &'static str, _s: (), _lo_g: [f32; 3], _rd_v: [f32; 3]) -> [f32; 4] {
        unreachable!("Cf::call_coeffs is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn call_marmitt(_fn_sym: &'static str, _coeffs: [f32; 4], _a: f32, _b: f32) -> f32 {
        unreachable!("Cf::call_marmitt is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn call_clamp_index_int(_fn_sym: &'static str, _g: f32) -> i32 {
        unreachable!("Cf::call_clamp_index_int is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }

    #[inline]
    fn smax(_a: i32, _b: i32) -> i32 {
        unreachable!("Cf::smax is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn uint_from_int(_a: i32) -> u32 {
        unreachable!("Cf::uint_from_int is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn slt(_a: i32, _b: i32) -> bool {
        unreachable!("Cf::slt is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn sadd(_a: i32, _b: i32) -> i32 {
        unreachable!("Cf::sadd is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn float_from_int(_a: i32) -> f32 {
        unreachable!("Cf::float_from_int is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn float_from_uint(a: u32) -> f32 {
        // `(float)<uint>` — the value-preserving `uint -> float` cast (Track B Increment G1's
        // `(float)lo` / `(float)hi`). REACHABLE on Eval since `pack_material_id_ba_body` runs over
        // `EvalCf` (the byte-split `lo`/`hi` are `< 256`, so `a as f32` is exact). `m2_brick_cubic_hit`
        // (the prior consumer) is EMIT-ONLY and never reaches this on Eval, so making it concrete is
        // harmless to that body.
        a as f32
    }
    #[inline]
    fn usub(_a: u32, _b: u32) -> u32 {
        unreachable!("Cf::usub is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn umin(_a: u32, _b: u32) -> u32 {
        unreachable!("Cf::umin is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn sint_eq(_a: i32, _b: i32) -> bool {
        unreachable!("Cf::sint_eq is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn temp_int(_name: &'static str, _x: i32) -> i32 {
        unreachable!("Cf::temp_int is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }

    #[inline]
    fn captured_uint(_name: &'static str) -> u32 {
        unreachable!("Cf::captured_uint is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn select_uint(_cond: bool, _t: u32, _e: u32) -> u32 {
        unreachable!("Cf::select_uint is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn vec3_dyn_index(_v: [f32; 3], _idx: u32) -> f32 {
        unreachable!("Cf::vec3_dyn_index is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn vec3_from_scalars(_x: f32, _y: f32, _z: f32) -> [f32; 3] {
        unreachable!("Cf::vec3_from_scalars is EMIT-ONLY: m2_brick_cubic_hit_body is never run over EvalCf")
    }
    #[inline]
    fn temp_vec4(_name: &'static str, v: [f32; 4]) -> [f32; 4] {
        // Identity on Eval — a temp is an Emit-only materialization concern. This arm was an
        // "EMIT-ONLY" honest panic while `m2_brick_cubic_hit_body` was its only caller (that
        // body is never run over EvalCf); UI-ADVANCED S1's `ui_premultiplied_over_body` IS
        // oracle-swept, so the identity is now the correct Eval semantics — the same shape
        // `temp_vec3`/`temp_float`/`temp_vec2` have always had.
        v
    }

    // ---- Track B Increment G1: the `float2` axis + bitwise `uint` `&`/`>>` (native host) ----
    // The mutable `float2` cell IS its value, held in a `Cell<[f32; 2]>` (the same interior-
    // mutability shape the other ret-cells use). `Cell<[f32; 2]>` is body-LOCAL (not a field on
    // the marker), so `size_of::<EvalCf>() == 0` still holds.
    type Vec2f = [f32; 2];
    type RetCellV2 = core::cell::Cell<[f32; 2]>;

    #[inline]
    fn and_u(a: u32, b: u32) -> u32 {
        // The bitwise AND (`id & 255u`) — the byte mask. The host `&` matches HLSL's `OpBitwiseAnd`.
        a & b
    }

    #[inline]
    fn shr_u(a: u32, b: u32) -> u32 {
        // The logical right shift (`id >> 8u`) — the high-byte select. The host `>>` over a `u32` is
        // logical (zero-fill), matching HLSL's `OpShiftRightLogical` for the `uint` operand.
        a >> b
    }

    #[inline]
    fn vec2_from_scalars(x: f32, y: f32) -> [f32; 2] {
        [x, y]
    }

    #[inline]
    fn ret_vec2(cell: &core::cell::Cell<[f32; 2]>, value: [f32; 2]) -> Flow {
        cell.set(value);
        core::ops::ControlFlow::Break(LoopOp::Return)
    }

    // ---- Track B Increment G2: the `oct_encode` octahedral encoder (native host) ----
    // The mutable `float3` param / `float2` local ARE their values, each held in a `Cell` for
    // interior mutability (the same shape `Var`/`decl_param` use). Both are body-LOCAL (not fields on
    // the marker), so `size_of::<EvalCf>() == 0` still holds.
    type Vec3Var = core::cell::Cell<[f32; 3]>;
    type Vec2Var = core::cell::Cell<[f32; 2]>;

    #[inline]
    fn decl_param_vec3(_name: &'static str, init: [f32; 3]) -> core::cell::Cell<[f32; 3]> {
        // On Eval a suppressed-decl `float3` param is IDENTICAL to a `decl_var` — the "no decl
        // recorded" distinction is an Emit-only printing concern (a `Cell` has no printed identity).
        core::cell::Cell::new(init)
    }

    #[inline]
    fn get_var_vec3(v: &core::cell::Cell<[f32; 3]>) -> [f32; 3] {
        v.get()
    }

    #[inline]
    fn set_var_vec3(v: &core::cell::Cell<[f32; 3]>, val: [f32; 3]) {
        v.set(val);
    }

    #[inline]
    fn decl_var_vec2(_name: &'static str, init: [f32; 2]) -> core::cell::Cell<[f32; 2]> {
        core::cell::Cell::new(init)
    }

    #[inline]
    fn get_var_vec2(v: &core::cell::Cell<[f32; 2]>) -> [f32; 2] {
        v.get()
    }

    #[inline]
    fn set_var_vec2(v: &core::cell::Cell<[f32; 2]>, val: [f32; 2]) {
        v.set(val);
    }

    #[inline]
    fn vec3_xy(v: [f32; 3]) -> [f32; 2] {
        // `n.xy` — drop the `.z` lane.
        [v[0], v[1]]
    }

    #[inline]
    fn vec2_yx(v: [f32; 2]) -> [f32; 2] {
        // `e.yx` — swap the two lanes.
        [v[1], v[0]]
    }

    #[inline]
    fn vec2_x(v: [f32; 2]) -> f32 {
        v[0]
    }

    #[inline]
    fn vec2_y(v: [f32; 2]) -> f32 {
        v[1]
    }

    #[inline]
    fn vec2_abs(v: [f32; 2]) -> [f32; 2] {
        // `abs(e.yx)` — component-wise. The host `f32::abs` matches HLSL's per-lane `abs`.
        [v[0].abs(), v[1].abs()]
    }

    #[inline]
    fn vec2_mul(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
        [a[0] * b[0], a[1] * b[1]]
    }

    #[inline]
    fn vec2_mul_scalar(v: [f32; 2], s: f32) -> [f32; 2] {
        // The HLSL `float2 * float` broadcasts `s` to a `float2` then multiplies component-wise.
        [v[0] * s, v[1] * s]
    }

    #[inline]
    fn vec2_add_scalar(v: [f32; 2], s: f32) -> [f32; 2] {
        // The HLSL `float2 + float` broadcasts `s` to a `float2` then adds component-wise.
        [v[0] + s, v[1] + s]
    }

    #[inline]
    fn vec2_rsub_scalar(s: f32, v: [f32; 2]) -> [f32; 2] {
        // `1.0 - abs(e.yx)` — the HLSL `float - float2` broadcasts the scalar LHS then subtracts.
        [s - v[0], s - v[1]]
    }

    #[inline]
    fn select_bare(cond: bool, t: f32, e: f32) -> f32 {
        // Eager `if cond { t } else { e }` — both arms are pure `±1.0` literals, so the eager form is
        // result-equivalent to the GPU ternary (which computes both arms and selects). The SAME shape
        // `select` / `FieldScalar::select` use; the bare vs wrapped spelling is an Emit-only concern.
        if cond { t } else { e }
    }

    // ---- Rung E: the particle-leaf prerequisite facets (native host) -------------------

    #[inline]
    fn ushl(a: u32, b: u32) -> u32 {
        // `wrapping_shl` MASKS the amount by 31, which is exactly the `OpBitwiseAnd %b
        // %uint_31` DXC emits ahead of `OpShiftLeftLogical` (measured). The plain `a << b`
        // would panic in debug for `b >= 32` where the GPU silently shifts by `b & 31`.
        a.wrapping_shl(b)
    }

    #[inline]
    fn uxor(a: u32, b: u32) -> u32 {
        a ^ b
    }

    #[inline]
    fn uor(a: u32, b: u32) -> u32 {
        a | b
    }

    #[inline]
    fn asuint(x: f32) -> u32 {
        // The BIT-REINTERPRET (`OpBitcast`), not the numeric cast: `to_bits` is total and
        // preserves every bit including a NaN's payload and the sign of a zero.
        x.to_bits()
    }

    #[inline]
    fn asfloat(u: u32) -> f32 {
        f32::from_bits(u)
    }

    #[inline]
    fn f16tof32(u: u32) -> f32 {
        crate::half::f16_bits_to_f32(u)
    }

    #[inline]
    fn f32tof16(x: f32) -> u32 {
        crate::half::f32_to_f16_bits(x)
    }

    #[inline]
    fn vec3_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        // The left-associated fold `(a.x*b.x + a.y*b.y) + a.z*b.z`. `OpDot` may contract into
        // an FMA chain on the GPU, so this mirror is close but NOT part of a bit-exact
        // contract (see the trait doc).
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[inline]
    fn sin(x: f32) -> f32 {
        // The same `nightly`/`std` shim `FieldScalar::sqrt` uses: stable `core` has no trig, so
        // a strictly-`no_std` build takes the intrinsic and the default build links `std`.
        #[cfg(feature = "nightly")]
        {
            core::intrinsics::sinf32(x)
        }
        #[cfg(not(feature = "nightly"))]
        {
            f32::sin(x)
        }
    }

    #[inline]
    fn cos(x: f32) -> f32 {
        #[cfg(feature = "nightly")]
        {
            core::intrinsics::cosf32(x)
        }
        #[cfg(not(feature = "nightly"))]
        {
            f32::cos(x)
        }
    }

    #[inline]
    fn rsqrt(x: f32) -> f32 {
        // `1.0 / sqrt(x)` through the SAME `nightly`/`std` sqrt shim `FieldScalar::sqrt` uses,
        // so this arm stays `no_std`-clean. It is a CLOSE mirror of the GPU's approximate
        // `InverseSqrt` (2 ULP allowed), not a bit-exact one — see the trait doc.
        f32::lit(1.0).div(FieldScalar::sqrt(x))
    }

    // ---- UI-ADVANCED S1: the `ui_rect` fragment-leaf facets ------------------------------

    // The `float4` return cell — the same interior-mutability shape `RetCellV2` uses.
    type RetCellV4 = core::cell::Cell<[f32; 4]>;

    #[inline]
    fn ret_vec4(cell: &core::cell::Cell<[f32; 4]>, value: [f32; 4]) -> Flow {
        cell.set(value);
        Flow::Break(LoopOp::Return)
    }

    #[inline]
    fn vec2_sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
        [a[0] - b[0], a[1] - b[1]]
    }

    #[inline]
    fn vec2_sub_scalar(v: [f32; 2], s: f32) -> [f32; 2] {
        [v[0] - s, v[1] - s]
    }

    #[inline]
    fn vec2_max_scalar(v: [f32; 2], s: f32) -> [f32; 2] {
        [v[0].max(s), v[1].max(s)]
    }

    #[inline]
    fn vec2_length(v: [f32; 2]) -> f32 {
        // The host norm through the SAME `nightly`/`std` sqrt shim the field leaves use, so
        // this arm stays `no_std`-clean. Sqrt-family precision — see the trait doc.
        FieldScalar::sqrt(v[0].mul(v[0]).add(v[1].mul(v[1])))
    }

    #[inline]
    fn vec2_dot(a: [f32; 2], b: [f32; 2]) -> f32 {
        // The left-associated fold. `OpDot` may contract into an FMA on the GPU, so this is a
        // CLOSE mirror, not a bit-exact one (the trait doc's standing carve-out).
        a[0].mul(b[0]).add(a[1].mul(b[1]))
    }

    #[inline]
    fn vec2_smoothstep(e0: [f32; 2], e1: [f32; 2], x: [f32; 2]) -> [f32; 2] {
        // The spec polynomial per lane: `t = clamp((x - e0) / (e1 - e0), 0, 1); t*t*(3 - 2t)`.
        // Contains a DIVIDE, so it is exact only at the saturated ends (the trait doc).
        #[inline]
        fn lane(e0: f32, e1: f32, x: f32) -> f32 {
            let t = x.sub(e0).div(e1.sub(e0)).clamp01();
            t.mul(t).mul(f32::lit(3.0).sub(f32::lit(2.0).mul(t)))
        }
        [lane(e0[0], e1[0], x[0]), lane(e0[1], e1[1], x[1])]
    }

    #[inline]
    fn vec2_fwidth(_v: [f32; 2]) -> [f32; 2] {
        // A device derivative has NO host semantics — the honest-panic discipline (`call1`'s
        // Eval arm): a loud panic, never a wrong value. The one leaf that spells `fwidth`
        // (`ui_screen_px_range_body`) is deliberately not oracle-swept.
        unreachable!("fwidth is a fragment-stage derivative; the Eval oracle never reaches it")
    }

    #[inline]
    fn vec2_rdiv_scalar(s: f32, v: [f32; 2]) -> [f32; 2] {
        [s.div(v[0]), s.div(v[1])]
    }

    #[inline]
    fn select_vec2(cond: bool, t: [f32; 2], e: [f32; 2]) -> [f32; 2] {
        // Eager: both arms are pure swizzles (no side effects), matching the GPU computing
        // both operands of an `OpSelect`-shaped ternary.
        if cond { t } else { e }
    }

    #[inline]
    fn vec4_xy(v: [f32; 4]) -> [f32; 2] {
        [v[0], v[1]]
    }

    #[inline]
    fn vec4_zw(v: [f32; 4]) -> [f32; 2] {
        [v[2], v[3]]
    }

    #[inline]
    fn vec4_yz(v: [f32; 4]) -> [f32; 2] {
        [v[1], v[2]]
    }

    #[inline]
    fn vec4_xw(v: [f32; 4]) -> [f32; 2] {
        [v[0], v[3]]
    }

    #[inline]
    fn vec4_alpha(v: [f32; 4]) -> f32 {
        v[3]
    }

    #[inline]
    fn vec4_from_scalars(x: f32, y: f32, z: f32, w: f32) -> [f32; 4] {
        [x, y, z, w]
    }

    #[inline]
    fn vec4_mul_scalar(v: [f32; 4], s: f32) -> [f32; 4] {
        [v[0].mul(s), v[1].mul(s), v[2].mul(s), v[3].mul(s)]
    }

    #[inline]
    fn vec4_add(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        [
            a[0].add(b[0]),
            a[1].add(b[1]),
            a[2].add(b[2]),
            a[3].add(b[3]),
        ]
    }

    #[inline]
    fn temp_vec2(_name: &'static str, v: [f32; 2]) -> [f32; 2] {
        // Identity on Eval — a temp is an Emit-only materialization concern.
        v
    }

    #[inline]
    fn named_uint_val(_sym: &'static str, val: u32) -> u32 {
        val
    }

    #[inline]
    fn vec2_frac(v: [f32; 2]) -> [f32; 2] {
        // HLSL's own definition of `frac` (and `GLSL.std.450 Fract`): `x - floor(x)`, so a
        // negative input wraps upward (`frac(-0.25) == 0.75`) rather than truncating toward
        // zero the way `%` would.
        [v[0] - v[0].floor(), v[1] - v[1].floor()]
    }

    #[inline]
    fn vec2_lerp(a: [f32; 2], b: [f32; 2], t: [f32; 2]) -> [f32; 2] {
        // `FMix`'s SPECIFIED form `x * (1 - t) + y * t`, not the `a + t * (b - a)`
        // decomposition — the two round differently and the committed shader spells the
        // intrinsic (see the trait doc).
        [
            a[0] * (1.0 - t[0]) + b[0] * t[0],
            a[1] * (1.0 - t[1]) + b[1] * t[1],
        ]
    }
}
