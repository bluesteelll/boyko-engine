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

    // ---- Increment 3 typed facets (the `brick_cell_class` value model) -----------
    //
    // A `uint` class + a `float3` cell_min out-param + a `StructuredBuffer<uint>` load
    // — the second CF leaf's value types. Each is an associated TYPE instantiated as a
    // body-local / parameter (NOT stored on the ZST backend), so the
    // `size_of::<EvalCf>() == 0` guarantee still holds. Eval is native Rust (real
    // casts, real `||`, a `Cell` out-param); Emit records SSA nodes / statements.

    /// The `uint` scalar — `u32` on Eval, the SSA-node handle [`Scalar`](Self::Scalar)
    /// on Emit (a handle carrying [`crate::emit`]'s `uint` [`EmitTy`] per-node). The
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
    /// decl). Owned (no lifetime), passed by `&` to [`out_vec3_assign`](Self::
    /// out_vec3_assign) — the SAME `&Self::Var` idiom [`get_var`](Self::get_var) uses.
    type OutVec3;

    /// The RETURN-VALUE cell (`Cell<u32>` on Eval — the body-local cell the IIFE reads
    /// after an early return; a ZST on Emit, the value travels in the recorded
    /// `Stmt::Return`). Owned, passed by `&` to [`ret`](Self::ret) / [`if_ret`](Self::
    /// if_ret).
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
    /// travels in the recorded `Stmt::Return`). The float analogue of [`RetCell`](Self::
    /// RetCell). Owned, passed by `&` to [`ret_f`](Self::ret_f) / [`if_ret_f`](Self::
    /// if_ret_f).
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

    /// `if (cond) { then } else { els }` — the TWO-arm branch (the existing [`if_`](Self::
    /// if_) is single-arm). `m2_regula_falsi`'s `if (f_lo * f_mid <= 0.0) { hi = mid; f_hi =
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
}
