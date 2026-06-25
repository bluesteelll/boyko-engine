//! The `Emit` backend: an SSA recorder + an HLSL printer (`feature = "emit"`).
//!
//! `impl FieldScalar for Emit` records each field op as one node in a build-time
//! SSA arena; the printer ([`emit_hlsl_field`]) walks the arena into HLSL textually
//! equivalent to the frozen `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli` field
//! bodies (`smin`/`smax`/`combine`/`sdf`).
//!
//! # Pass 1: NO `precise`
//!
//! The printer emits PLAIN `float` temps (no `precise` qualifier) to match the
//! frozen — non-`precise` — `sdf_field.hlsli`. Pass 2 adds `precise` + re-DXC + the
//! RTX gate; this pass is the CPU-verifiable refactor proof only. The bin
//! ([`crate::bin`]) PRINTS the generated bodies; it does NOT splice them into the
//! header or recompile any `.spv`.
//!
//! # `std`-side, codegen tooling — NOT a hot path
//!
//! This module is build-time tooling (gated OFF by default). The arena is a
//! thread-local `Vec<Node>` so the by-value `FieldScalar` ops can append without
//! threading a `&mut arena` through the generic field signature. This is NOT the
//! physics leaf and NEVER linked by a physics build — the lock-free / no-alloc
//! hot-path rules do not apply here.

use core::cell::RefCell;

use crate::cf::{Cf, Flow, LoopOp};
use crate::scalar::FieldScalar;

thread_local! {
    /// The build-time SSA arena. Recorded into by [`Emit`]'s [`FieldScalar`] ops,
    /// drained by the printer. Thread-local so the by-value ops can append without
    /// a `&mut arena` parameter (codegen tooling, single-threaded use).
    static ARENA: RefCell<Vec<Node>> = const { RefCell::new(Vec::new()) };
}

/// The HLSL scalar TYPE a materialized temp is declared with (O2).
///
/// The field/normal leaves are all `float`; the integer/bit leaves (`decode_snorm8`
/// and the A3/A4 brick-index family) introduce `uint` temps (a packed byte source, a
/// bit-AND / shift, a `(float)` numeric cast). The printer ([`emit_body`]) reads a
/// node's [`EmitTy`] to emit `float tN = …;` vs `uint tN = …;`.
///
/// MINIMAL by design (the reviewer's O2): exactly the two states the current leaves
/// need. A signed `Int` state is deferred until a leaf genuinely produces a negative
/// integer temp — `decode_snorm8`'s only integer op is an UNSIGNED byte extract, and
/// the snorm sign is carried by the `float` after the cast. Extend the enum (and the
/// one match in [`type_of`]) when A3/A4's index math needs `int`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmitTy {
    /// An HLSL `float` temp (the default — every field/normal node).
    Float,
    /// An HLSL `uint` temp (a packed byte source / bit op, before the numeric cast).
    Uint,
    /// An HLSL `float3` temp/value (the brick-cell `rel` / `cell_min` rhs — Increment 3).
    Float3,
    /// An HLSL `uint3` value (a `dims`-style parameter — Increment 3). No leaf currently
    /// materializes a `uint3` temp (`dims` is an inlined parameter), but the type is
    /// carried so [`assert_operand_ty`] can validate a `uint3` swizzle's source.
    Uint3,
    /// An HLSL `float4` value (the cubic-coefficient `c` parameter — Increment 4a). No leaf
    /// materializes a `float4` temp (`c` is an inlined call-through parameter); the type is
    /// carried so [`assert_operand_ty`] can validate the [`Node::Call2`] `float4` argument.
    Float4,
    /// An HLSL `bool` local (the B1 marcher's `hit` / `exhausted` flags — Increment 4d). The
    /// DECL-SITE type of a [`Cf::decl_bool_var`] local; spells the token `bool` (NOT `bool1`
    /// or a vector form). Carried only on a [`Stmt::DeclVar`] `ty` field; no node materializes
    /// a `bool` temp (the `false`/`true` init rhs is a [`Node::BoolLit`] inline leaf).
    Bool,
}

/// One SSA node — a recorded field op. Operands reference earlier nodes by their
/// arena index ([`Emit`] handle), so the arena is a topologically-ordered DAG
/// (a node only references strictly-smaller indices: SSA, no cycles).
#[derive(Clone, Copy, Debug)]
enum Node {
    /// A symbolic INPUT (a function parameter the field is traced over), printed
    /// verbatim by name (e.g. `p.x`, `acc`, `k`). Index into [`Emit::input`]'s name
    /// table is carried out-of-band; we store the name id.
    Input(u32),
    /// A float literal (`0.5`, `1.0`, `0.0`, ...). Printed as the formatted f32.
    Lit(f32),
    /// `a + b`.
    Add(u32, u32),
    /// `a - b`.
    Sub(u32, u32),
    /// `a * b`.
    Mul(u32, u32),
    /// `a / b`.
    Div(u32, u32),
    /// `-a`.
    Neg(u32),
    /// `min(a, b)`.
    Min(u32, u32),
    /// `max(a, b)`.
    Max(u32, u32),
    /// `clamp(a, 0.0, 1.0)`.
    Clamp01(u32),
    /// `lerp(self, a, h)` — the HLSL `lerp` intrinsic (the two-rounding op the
    /// frozen `smin` writes as `lerp(b, a, hh)`). Stored as `(self, a, h)`.
    Lerp(u32, u32, u32),
    /// `abs(a)`.
    Abs(u32),
    /// `sqrt(a)`.
    Sqrt(u32),
    /// `cond ? t : e` — the HLSL ternary (the frozen `(k > 0.0) ? _ : _`). The arms are
    /// spelled UN-wrapped (the brick-exit progress clamp). Recorded by
    /// [`FieldScalar::select`].
    Select(u32, u32, u32),
    /// `cond ? (t) : (e)` — the HLSL ternary with BOTH arms wrapped UNCONDITIONALLY (the
    /// committed `m2_regula_falsi` ternary form). A DISTINCT node from [`Node::Select`] so
    /// the brick-exit's un-wrapped `Select` printer is unperturbed; recorded ONLY by
    /// [`EmitCf::select`] (`Cf::select`), never [`FieldScalar::select`] (Increment 4a).
    SelectParen(u32, u32, u32),
    /// `a > b` — a Mask node (printed inline inside a ternary condition).
    Gt(u32, u32),
    /// `a < b` — a Mask node (`OpFOrdLessThan`), printed inline like [`Node::Gt`]. The
    /// brick-marcher's progress clamp `exit < BRICK_EXIT_EPS`.
    Lt(u32, u32),
    /// `a <= b` — a Mask node (`OpFOrdLessThanEqual`, a DISTINCT opcode from a swapped
    /// `>`), printed inline like [`Node::Gt`]. The brick-marcher's per-axis skip guard
    /// `abs(dir) <= BRICK_EXIT_EPS`.
    Le(u32, u32),
    /// `a >= b` — a Mask node (`OpFOrdGreaterThanEqual`, a DISTINCT opcode from a swapped
    /// `<=`), printed inline like [`Node::Gt`]. The B1 exhaustion re-march's mesh guard
    /// `t >= t_mesh`.
    Ge(u32, u32),
    /// `vec[iv]` — a dynamic index of a `float3` PARAMETER (`rd[a]`, `p[a]`,
    /// `cell_min[a]`) by the unroll induction variable. `(vec_id, iv_id)`: `vec_id`
    /// indexes [`Names::vec_in`] (the vector-parameter name table), `iv_id` references
    /// the induction-variable node ([`Node::Input`] carrying the iv's name). Printed
    /// INLINE (`is_inline_leaf` = true), so `p[a]` spells inline at every use — matching
    /// the committed body, which does NOT temp `p[a]`.
    VecIndex(u32, u32),
    /// A NAMED float literal — prints the SYMBOL (`BRICK_EXIT_EPS`) for Emit. `sym_id`
    /// indexes [`Names::named_lit`]; printed inline. The committed body spells
    /// `BRICK_EXIT_EPS` (not `1.0e-4`) in both the skip guard and the final clamp, so the
    /// symbol — not the value — reaches the HLSL.
    ///
    /// `val` carries the concrete numeric value (`1e-4`). On the `Emit` backend the
    /// printer spells the SYMBOL, so `val` is not read here (the `f32` Eval backend uses
    /// it directly, NOT through a `Node`); it is recorded for IR completeness and the
    /// Increment-2 const-fold path (which compares a node's `val` against a guard). Dead
    /// on the emit print path by design.
    NamedLit {
        sym_id: u32,
        #[allow(dead_code)]
        val: f32,
    },
    /// A call to the hand-written field function `sdf(<arg>)`. The operand is a
    /// VECTOR-expression handle into the separate [`VNode`] arena (the `float3`
    /// argument), so the printer emits `sdf(<vexpr>)`. The field is NOT inlined: the
    /// frozen HLSL keeps `sdf` a hand-written function owning the edit-list `[loop]`.
    FieldCall(u32),

    // ---- Integer / bit nodes (the A2 brick leaves; [`EmitTy::Uint`]) -------------
    /// A symbolic `uint` INPUT (a `uint`-typed function parameter), printed verbatim
    /// by name. Distinct from [`Node::Input`] only in its [`EmitTy`] so the printer
    /// declares any derived temp `uint` (the parameter itself inlines, so the tag is
    /// carried for its consumers via [`type_of`]).
    UintInput(u32),
    /// A `uint` literal (a bit mask / shift amount), printed as `Nu` (HLSL unsigned).
    UintLit(u32),
    /// `a == b` over two integer handles — a Mask node (printed inline inside a
    /// ternary condition, like [`Node::Gt`]). The snorm `q == i8::MIN` sentinel test.
    IntEq(u32, u32),
    // A2 FOUNDATION (not yet constructed): the packed-byte bit ops. `decode_snorm8`
    // reads an UNPACKED `i8` code (no AND/shift needed), but the reviewer's O2 calls
    // for the bit ops + the printer support as the brick-index (A3/A4) prerequisite.
    // The printer (`define_str`) already spells them; A3 wires the `FieldScalar` trait
    // methods + the host index math that constructs them. `#[allow(dead_code)]` keeps
    // the foundation in tree without a false `-D warnings` failure until then.
    /// `a & b` — a bitwise AND over two `uint` handles (the packed-byte extract).
    #[allow(dead_code)]
    And(u32, u32),
    /// `a >> b` — a logical right shift over `uint` handles (the byte select within
    /// a packed word).
    #[allow(dead_code)]
    Shr(u32, u32),
    /// `(float)a` — the HLSL NUMERIC `uint -> float` cast (value-preserving), NOT
    /// `asfloat` (a bit-reinterpret). Materialized as a `float` temp.
    UintToFloat(u32),

    // ---- Control-flow leaves (Increment 1; the brick-exit marcher) ---------------
    /// A `float3` VECTOR-PARAMETER marker (`p` / `rd` / `cell_min`). The `u32` is the
    /// [`Names::vec_in`] id. It is NEVER printed directly: [`EmitCf::index`] reads the
    /// vec id off it and emits a [`Node::VecIndex`]. Seeded as the three identical
    /// elements of the body's `[Emit; 3]` parameter (so `index(p, a)` recovers p's id).
    VecParamRef(u32),
    /// A named mutable LOCAL reference (`exit`) — prints the variable's name (NOT a `tN`
    /// temp). The `u32` is the [`Block`]/printer var id. Recorded by [`EmitCf::get_var`]
    /// so a `min(exit, t6)` rhs reads the running `exit`.
    VarRef(u32),
    /// A reference to a MATERIALIZED temp (`t0`..`t6`) — prints `t{seq}`, where `seq` is
    /// the temp's program-order sequence number (assigned by [`EmitCf::temp`] when it
    /// records the matching `Stmt::DeclTemp`). The committed-shape materialization is
    /// EXPLICIT (the body wraps a subexpression in [`Cf::temp`]); this node is the handle
    /// the wrapped value flows through, so later uses spell `t{seq}`.
    TempRef(u32),

    // ---- Increment 3: the brick-cell value model (uint / float3 / uint3 / buffer) ----
    //
    // The second CF leaf (`brick_cell_class`) introduces first-class `float3` and `uint`
    // values + a `StructuredBuffer<uint>` load. The `float3`/`uint3` VALUE nodes record
    // into THIS scalar arena (operands = arena ids); a node's result type is tagged by
    // [`type_of`] (Float3/Uint3/Uint), so the printer types a temp without recursion.

    /// A `float3` PARAMETER reference (`p` / `origin`) — prints the parameter NAME (an
    /// inline leaf). `u32` indexes [`Names::vec_in`]. Distinct from [`Node::VecParamRef`]
    /// (which is consumed by [`EmitCf::index`] into a dynamic `vec[iv]` and never printed):
    /// a `Vec3Param` IS spelled directly (`p`, `origin`) as a whole-`float3` operand.
    Vec3Param(u32),
    /// A `float3` swizzle to a `float` scalar (`rel.x`) — `(vec_id, axis)` where `axis`
    /// 0/1/2 = x/y/z. Printed inline (`<op(vec_id)>.x`). Result type [`EmitTy::Float`].
    Vec3Swizzle(u32, u8),
    /// `float3(x, y, z)` from THREE `uint` operands (`float3(ix, iy, iz)`) — the implicit
    /// uint→float ctor. Asserts all-`uint` operands (the single cross-type construct).
    Vec3FromUints(u32, u32, u32),
    /// `a + b` over two `float3` handles (`origin + offset`).
    Vec3Add(u32, u32),
    /// `a - b` over two `float3` handles (`p - origin`).
    Vec3Sub(u32, u32),
    /// `v * s` — a `float3` times a `float` scalar (`float3(...) * bw`). `(vec, scalar)`.
    Vec3MulScalar(u32, u32),
    /// `v / s` — a `float3` divided by a `float` scalar (`(p - origin) / bw`).
    Vec3DivScalar(u32, u32),

    /// A `uint3` PARAMETER reference (`dims`) — prints the parameter NAME (an inline leaf).
    /// `u32` indexes [`Names::uint3_in`]. Result type [`EmitTy::Uint3`].
    Uint3Param(u32),
    /// A `uint3` swizzle to a `uint` (`dims.x`) — `(uint3_id, axis)`. Printed inline
    /// (`<op>.x`). Result type [`EmitTy::Uint`].
    Uint3Swizzle(u32, u8),

    /// `(uint)f` — the HLSL NUMERIC `float -> uint` truncating cast (`(uint)rel.x`), NOT
    /// `asuint`. Materialized as a `uint` temp. Result type [`EmitTy::Uint`].
    FloatToUint(u32),
    /// `a + b` over two `uint` handles (the index accumulation). Result [`EmitTy::Uint`].
    UAdd(u32, u32),
    /// `a * b` over two `uint` handles (a row/slice stride). Result [`EmitTy::Uint`].
    UMul(u32, u32),
    /// `grid[idx]` — a `StructuredBuffer<uint>` load. `(buf_id, idx)`: `buf_id` indexes
    /// [`Names::buf_in`], `idx` is the index node. Printed INLINE (`grid[idx]`). Result
    /// [`EmitTy::Uint`].
    BufferLoad(u32, u32),

    /// `a >= b` over two `uint` handles — a Mask node (printed inline inside a condition,
    /// like [`Node::Gt`]). The bounds guard `ix >= dims.x` (`OpUGreaterThanEqual`).
    UGe(u32, u32),
    /// `a > b` over two `uint` handles — a Mask node (printed inline inside a condition,
    /// like [`Node::UGe`]). The B1 sor-retreat's `it > 0u` iteration guard
    /// (`OpUGreaterThan`, a DISTINCT opcode from a swapped `<`). The `uint` strict-`>`
    /// analogue of [`Node::Gt`] (Increment 4f).
    UGt(u32, u32),
    /// `a || b` over two MASK handles — a Mask node, the short-circuit OR (`rel.x < 0.0
    /// || rel.y < 0.0`). Printed inline (`<a> || <b>`); DXC lowers `a||b||c` to the
    /// short-circuit `OpBranchConditional` chain (spike E2a: zero `OpLogicalOr`).
    Or(u32, u32),
    /// `a && b` over two MASK handles — a Mask node, the LOGICAL AND (`it > 0u && sor_prev
    /// + d < ...`). Printed inline (`<a> && <b>`); DXC lowers `&&` to an
    /// `OpBranchConditional` short-circuit chain (like `||`). SEPARATE from the bitwise
    /// `uint` [`Node::And`] (overloading would mistype the result as `Uint` and print `&`).
    /// Both operands are pure side-effect-free reads, so the eager-mask form matches the
    /// GPU short-circuit (the SAME equivalence [`Node::Or`] carries — Increment 4f).
    And2(u32, u32),

    // ---- Increment 4a: the runtime `[loop]` call site + `float4` param --------------
    /// A `float4` PARAMETER reference (`c`, the cubic coefficients) — prints the parameter
    /// NAME (an inline leaf). `u32` indexes [`Names::vec4_in`]. Consumed ONLY by
    /// [`Node::Call2`] (`m2_cubic_eval(c, mid)`); never swizzled in `m2_regula_falsi`.
    /// Result type [`EmitTy::Float4`].
    Vec4Param(u32),
    /// A CALL to a frozen hand-written shader function of two args — `m2_cubic_eval(c,
    /// mid)`. `sym_id` indexes [`Names::call_in`]; `a`/`b` are the two argument node ids
    /// (heterogeneous types live in the node graph — `a` may be a `Vec4Param`, `b` a
    /// `float`). Materialized as a `float` temp (`m2_regula_falsi`'s `float f_mid =
    /// m2_cubic_eval(c, mid);`). Result type [`EmitTy::Float`].
    Call2 { sym_id: u32, a: u32, b: u32 },
    /// A CALL to a frozen hand-written shader function of ONE `float3` arg returning a
    /// `float` — `field_distance(p + L * t)` (Inc 4b). `sym_id` indexes [`Names::call_in`]
    /// (the SAME table [`Node::Call2`] uses — `intern_call`); `a` the single `float3`
    /// argument node id. The float3→float analogue of [`Node::Call2`]; distinct from
    /// [`Node::FieldCall`] which hardcodes the callee `sdf`. Result type [`EmitTy::Float`]
    /// (via [`type_of`]'s default arm, as for `Call2`).
    Call1 { sym_id: u32, a: u32 },

    // ---- Increment 4b.2: the BOOL return literal (`m2_surface_hit`) -----------------
    /// A `bool` LITERAL — prints `true` / `false` (lowering to `OpConstantTrue` /
    /// `OpConstantFalse` on the function's `OpTypeBool` return, NOT a `uint` 0/1). The
    /// committed `m2_surface_hit` returns `bool`; the spike read the `OpConstantTrue`/`False`
    /// off the binary, so the bool return is modeled as a real bool literal, not uint. An
    /// inline leaf, consumed ONLY by [`Stmt::Return`] (never `chk`-typed — `type_of` falls to
    /// the `Float` default, harmless since no arithmetic consumer reads it; the SAME no-`chk`
    /// discipline [`EmitCf::named_uint`] uses).
    BoolLit(bool),
}

/// One VECTOR (`float3`/`float2`) SSA node — the normal leaf is a vector expression
/// (`p ± e.xyy`, `normalize(n)`), so it records into this separate arena. Operands
/// reference earlier `VNode`s (the float3 dataflow) or scalar [`Node`]s (the float3
/// constructor's components), each by arena index.
#[derive(Clone, Copy, Debug)]
enum VNode {
    /// The symbolic `float3` input `p` (the normal's only vector parameter).
    InputP,
    /// The `float2 e = float2(GRAD_H, 0.0)` constant the GRAD_H swizzles read.
    EpsE,
    /// A swizzle off the (single) [`VNode::EpsE`] — `e.xyy` / `e.yxy` / `e.yyx`. The
    /// `u8` is the axis (0=`xyy`, 1=`yxy`, 2=`yyx`). Printed TEXTUALLY (not decomposed
    /// into a `float3(...)` constructor): the frozen `sdf_normal` uses the `.xyy` form
    /// and DXC's SPIR-V is sensitive to it. The source vector is implicitly `e` (the
    /// only `float2` in the body), so no operand handle is carried.
    Swizzle(u8),
    /// `a + b` over two `float3` handles.
    VAdd(u32, u32),
    /// `a - b` over two `float3` handles.
    VSub(u32, u32),
    /// `float3(x, y, z)` — the three components are SCALAR [`Node`] handles (the
    /// per-axis central differences `sdf(p+o) - sdf(p-o)`). Materialized as the named
    /// `float3 n` temp, so the final `normalize(n)` references it (RAW HLSL `normalize`:
    /// the value-level zero-guard is an Eval-only concern, invisible at the op level).
    Construct(u32, u32, u32),
}

/// A handle into the SSA arena — `#[repr(transparent)]` over the node index, so it
/// is a zero-cost newtype the field ops thread through as the backend scalar.
///
/// Implements [`FieldScalar`]: each op pushes one [`Node`] and returns its handle.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Emit(u32);

/// The `Mask` associated type for [`Emit`] — also a node handle (a `Gt` node),
/// distinguished from [`Emit`] at the type level so a mask can only flow into
/// [`FieldScalar::select`]'s condition.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct EmitMask(u32);

thread_local! {
    /// The build-time VECTOR SSA arena (the normal leaf's `float3`/`float2` dataflow),
    /// separate from the scalar [`ARENA`]. Drained by [`emit_normal_body`].
    static VARENA: RefCell<Vec<VNode>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// OUT-OF-BAND materialized-temp result types — `t{seq}`'s [`EmitTy`], keyed by the
    /// temp's program-order `seq` (the index IS `seq`, pushed in order by the `temp_*`
    /// combinators). [`type_of`]`(`[`Node::TempRef`]`(seq))` reads it. Kept off the frozen
    /// [`Node::TempRef`]`(u32)` node (widening it would risk the Inc-1/2 sync gate, which
    /// freezes `dist_to_brick_exit`'s `exit` path). The straight-line + Inc-1 leaves push
    /// only `Float` here (every `Cf::temp` is a `float`), so they are byte-unchanged.
    static TEMP_TYPES: RefCell<Vec<EmitTy>> = const { RefCell::new(Vec::new()) };

    /// OUT-OF-BAND materialized-temp NAMES — `Some("rel")` for a NAMED temp (the brick-cell
    /// `rel`/`ix`/`idx`), `None` for an ANONYMOUS temp (the brick-exit `t{seq}`), keyed by
    /// `seq` (the index IS `seq`). [`operand_str`]`(`[`Node::TempRef`]`(seq))` reads it to
    /// spell the name (named) or `t{seq}` (anonymous). Same out-of-band rationale as
    /// [`TEMP_TYPES`] — the frozen `TempRef` node stays byte-unchanged. The straight-line +
    /// Inc-1 leaves push only `None` (every `Cf::temp` is anonymous), so they are unchanged.
    static TEMP_NAMES: RefCell<Vec<Option<&'static str>>> = const { RefCell::new(Vec::new()) };
}

/// The result [`EmitTy`] of materialized temp `seq` (out-of-band, see [`TEMP_TYPES`]).
/// Defaults to [`EmitTy::Float`] when `seq` is past the recorded set (the straight-line
/// emit paths do not populate `TEMP_TYPES`, so every temp there is a `float`).
fn temp_type(seq: u32) -> EmitTy {
    TEMP_TYPES.with(|t| {
        t.borrow()
            .get(seq as usize)
            .copied()
            .unwrap_or(EmitTy::Float)
    })
}

/// The NAME of materialized temp `seq` (out-of-band, see [`TEMP_NAMES`]) — `Some("rel")`
/// for a named brick-cell temp, `None` for an anonymous `t{seq}`. Defaults to `None` when
/// `seq` is past the recorded set (the straight-line emit paths do not populate
/// `TEMP_NAMES`, so every temp there is anonymous).
fn temp_name(seq: u32) -> Option<&'static str> {
    TEMP_NAMES.with(|t| t.borrow().get(seq as usize).copied().flatten())
}

/// The HLSL [`EmitTy`] a node materializes as (O2). Every legacy field/normal node
/// is [`EmitTy::Float`]; only the integer/bit nodes are [`EmitTy::Uint`]. The
/// `UintToFloat` cast is the boundary — its RESULT is `float` (so consumers see a
/// `float`), its operand is `uint`. This is a per-NODE tag (the node's own result
/// type), not an operand walk: each int node declares its own result type, so the
/// printer needs no recursion to type a temp.
fn type_of(node: Node) -> EmitTy {
    match node {
        Node::UintInput(_) | Node::UintLit(_) | Node::And(_, _) | Node::Shr(_, _) => EmitTy::Uint,
        // Increment 3 `uint`-result nodes: the cell index math, the buffer load, the
        // `(uint)` cast, the `uint3` swizzle, and a named `uint` constant.
        Node::Uint3Swizzle(_, _)
        | Node::FloatToUint(_)
        | Node::UAdd(_, _)
        | Node::UMul(_, _)
        | Node::BufferLoad(_, _) => EmitTy::Uint,
        // `float3`-result nodes: the parameter ref, the constructors, the vector arithmetic.
        Node::Vec3Param(_)
        | Node::Vec3FromUints(_, _, _)
        | Node::Vec3Add(_, _)
        | Node::Vec3Sub(_, _)
        | Node::Vec3MulScalar(_, _)
        | Node::Vec3DivScalar(_, _) => EmitTy::Float3,
        // The `uint3` parameter ref is a whole-`uint3` value.
        Node::Uint3Param(_) => EmitTy::Uint3,
        // The `float4` parameter ref (`c`) is a whole-`float4` value (Increment 4a). The
        // `Call2` result (`m2_cubic_eval(c, mid)`) is a `float`.
        Node::Vec4Param(_) => EmitTy::Float4,
        // A materialized temp carries its OWN result type out-of-band (keyed by `seq`), set
        // when [`EmitCf::temp`] / [`EmitCf::temp_uint`] / [`EmitCf::temp_vec3`] recorded the
        // `Stmt::DeclTemp`. The straight-line leaves record only `float` temps (TEMP_TYPES is
        // empty there → the default `Float`), so they are byte-unchanged.
        Node::TempRef(seq) => temp_type(seq),
        // A `bool` literal IS a `bool` (W1 hardening, Increment 4e). NEVER reached today — a
        // `BoolLit` is always an inline leaf consumed by a `Stmt::Return`/`Stmt::DeclVar`, never
        // `type_of`'d for an arithmetic check — so this changes NO `.spv` (byte-neutral, proven
        // by re-running the cmp-`.spv` gate); it closes the future hole where a `bool`-typed
        // consumer would otherwise mis-read `Float`.
        Node::BoolLit(_) => EmitTy::Bool,
        // `UintToFloat`/`Vec3Swizzle` PRODUCE a float; every other node is float.
        _ => EmitTy::Float,
    }
}

/// The HLSL type keyword for an [`EmitTy`] (`float` / `uint` / `float3` / `uint3` / `float4` /
/// `bool`). The DECL-SITE type token a [`Stmt::DeclVar`] / [`Stmt::DeclTemp`] prints.
fn ty_keyword(ty: EmitTy) -> &'static str {
    match ty {
        EmitTy::Float => "float",
        EmitTy::Uint => "uint",
        EmitTy::Float3 => "float3",
        EmitTy::Uint3 => "uint3",
        EmitTy::Float4 => "float4",
        // The scalar `bool` decl token — `bool name = false;` (NOT `bool1` or a vector form).
        EmitTy::Bool => "bool",
    }
}

#[inline]
fn push(node: Node) -> u32 {
    ARENA.with(|a| {
        let mut a = a.borrow_mut();
        let id = a.len() as u32;
        a.push(node);
        id
    })
}

#[inline]
fn vpush(node: VNode) -> u32 {
    VARENA.with(|a| {
        let mut a = a.borrow_mut();
        let id = a.len() as u32;
        a.push(node);
        id
    })
}

impl Emit {
    /// Records a symbolic `float` INPUT node named `name_id` (an index into the
    /// printer's float-name table). Used by [`trace`]/[`trace_named`] to seed the
    /// traced parameters.
    fn input(name_id: u32) -> Self {
        Emit(push(Node::Input(name_id)))
    }

    /// Records a symbolic `uint` INPUT node named `name_id` (an index into the
    /// printer's uint-name table) — the packed-byte source an integer leaf decodes.
    fn uint_input(name_id: u32) -> Self {
        Emit(push(Node::UintInput(name_id)))
    }
}

impl FieldScalar for Emit {
    type Vec3 = [Emit; 3];
    type Mask = EmitMask;
    // The Emit integer is itself an `Emit` node handle: the `uint` type is carried
    // per-node via [`type_of`] (a handle pointing at a `UintLit`/`UintInput`/`And`
    // node IS a uint), so no separate handle newtype is needed.
    type Int = Emit;

    #[inline]
    fn lit(x: f32) -> Self {
        Emit(push(Node::Lit(x)))
    }

    #[inline]
    fn int_lit(x: i32) -> Emit {
        // The snorm leaf's only `int_lit` is the `i8::MIN` (-128) sentinel; lifted as a
        // `uint` two's-complement bit pattern (the HLSL `uint` literal the printer emits).
        Emit(push(Node::UintLit(x as u32)))
    }
    #[inline]
    fn int_eq(a: Emit, b: Emit) -> EmitMask {
        EmitMask(push(Node::IntEq(a.0, b.0)))
    }
    #[inline]
    fn int_to_float(a: Emit) -> Emit {
        Emit(push(Node::UintToFloat(a.0)))
    }

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Emit(push(Node::Add(self.0, rhs.0)))
    }
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Emit(push(Node::Sub(self.0, rhs.0)))
    }
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Emit(push(Node::Mul(self.0, rhs.0)))
    }
    #[inline]
    fn div(self, rhs: Self) -> Self {
        Emit(push(Node::Div(self.0, rhs.0)))
    }
    #[inline]
    fn neg(self) -> Self {
        Emit(push(Node::Neg(self.0)))
    }

    #[inline]
    fn min(self, rhs: Self) -> Self {
        Emit(push(Node::Min(self.0, rhs.0)))
    }
    #[inline]
    fn max(self, rhs: Self) -> Self {
        Emit(push(Node::Max(self.0, rhs.0)))
    }

    #[inline]
    fn clamp01(self) -> Self {
        Emit(push(Node::Clamp01(self.0)))
    }

    #[inline]
    fn lerp(self, a: Self, h: Self) -> Self {
        // Records the HLSL `lerp` intrinsic `lerp(self, a, h)` — the two-rounding
        // op the frozen `smin` writes as `lerp(b, a, hh)` (sdf_field.hlsli:112).
        Emit(push(Node::Lerp(self.0, a.0, h.0)))
    }

    #[inline]
    fn abs(self) -> Self {
        Emit(push(Node::Abs(self.0)))
    }
    #[inline]
    fn sqrt(self) -> Self {
        Emit(push(Node::Sqrt(self.0)))
    }

    #[inline]
    fn select(cond: EmitMask, t: Self, e: Self) -> Self {
        Emit(push(Node::Select(cond.0, t.0, e.0)))
    }

    #[inline]
    fn gt(self, rhs: Self) -> EmitMask {
        EmitMask(push(Node::Gt(self.0, rhs.0)))
    }

    #[inline]
    fn lt(self, rhs: Self) -> EmitMask {
        EmitMask(push(Node::Lt(self.0, rhs.0)))
    }

    #[inline]
    fn le(self, rhs: Self) -> EmitMask {
        EmitMask(push(Node::Le(self.0, rhs.0)))
    }

    #[inline]
    fn ge(self, rhs: Self) -> EmitMask {
        EmitMask(push(Node::Ge(self.0, rhs.0)))
    }

    fn eq_u(_op: u32, _want: u32) -> EmitMask {
        // W3: the op-discriminant equality is a HOST comparison — `field::combine`'s
        // op-dispatch is a host `if op == ...` (the frozen `if (op == OP_*)`), so it
        // dispatches on the `u32` directly and NEVER calls `eq_u` on the `Emit`
        // backend. Recording a node here (the old `push(Gt(Lit, Lit))`) would inject a
        // bogus mask into the live arena if the contract ever changed; a panic is the
        // honest signal that no `Emit` path may reach it. (`#[inline]` dropped: a cold
        // `unreachable!` must not bloat any caller's I-cache.)
        unreachable!(
            "FieldScalar::eq_u is never called on the Emit backend: combine's op-dispatch \
             is a host branch, not a traced mask"
        )
    }

    fn grad_offsets() -> [[Emit; 3]; 3] {
        // W3: the normal leaf is a VECTOR expression whose `float3`/`float2` dataflow
        // (`e.xyy`, `p ± offset`, `normalize(n)`) is recorded into [`VARENA`] by
        // [`emit_normal_body`] — NOT through this component-wise (`[Emit; 3]`) scalar
        // hook (the scalar granularity cannot carry the textual swizzle the SPIR-V gate
        // requires). `sdf_normal_body::<Emit>` is therefore never instantiated, so this
        // is UNREACHED. The old impl recorded literal axis vectors into the arena; a
        // panic is the honest signal instead of a silent wrong-node.
        unreachable!(
            "FieldScalar::grad_offsets is never called on the Emit backend: the normal is \
             recorded at vector granularity by emit_normal_body, not via sdf_normal_body::<Emit>"
        )
    }

    fn v_normalize(_a: [Emit; 3]) -> [Emit; 3] {
        // W3/W1: like `grad_offsets`, UNREACHED — the normal's final `normalize(n)` is
        // emitted as a LITERAL `return normalize(n);` string in [`emit_normal_body`]
        // (there is no `VNode::Normalize` node; the old comment misnamed it). Provided
        // only to satisfy the trait; a panic replaces the old identity-return stub.
        unreachable!(
            "FieldScalar::v_normalize is never called on the Emit backend: emit_normal_body \
             prints `normalize(n)` textually, it does not record this hook"
        )
    }
}

// ---- The HLSL printer ---------------------------------------------------------

/// The float-input names for the FIELD leaves (`smin`/`smax`/`combine`), in
/// [`Emit::input`] call order. The printer prints a [`Node::Input(i)`] as
/// `float_names[i]`. The integer/bit leaves pass their own table via [`Names`] (e.g.
/// `decode_snorm8`'s `["n", "band_half"]`), so the per-trace names are threaded, not
/// global.
const FIELD_INPUT_NAMES: &[&str] = &["a", "b", "k"];

/// The default `uint`-input table (empty — the float field/normal leaves take no
/// `uint` parameter). The integer leaves override it via [`Names`].
const NO_UINT_INPUTS: &[&str] = &[];

/// The default VECTOR-parameter table (empty — only the brick-marcher CF leaf indexes
/// a `float3` parameter by the unroll iv).
const NO_VEC_INPUTS: &[&str] = &[];

/// The default `uint3`-parameter table (empty — only the brick-cell CF leaf takes a
/// `uint3 dims` parameter).
const NO_UINT3_INPUTS: &[&str] = &[];

/// The default `StructuredBuffer<uint>`-parameter table (empty — only the brick-cell CF
/// leaf takes a `StructuredBuffer<uint> grid` parameter).
const NO_BUF_INPUTS: &[&str] = &[];

/// The default `out`-parameter table (empty — only the brick-cell CF leaf has an `out
/// float3 cell_min` parameter).
const NO_OUT_INPUTS: &[&str] = &[];

/// The default named-literal table (empty — only the brick-marcher CF leaf spells a
/// symbolic constant).
const NO_NAMED_LITS: &[&str] = &[];

/// The default mutable-local table (empty — only the brick-marcher CF leaf declares a
/// mutable local).
const NO_VARS: &[&str] = &[];

/// The default `float4`-parameter table (empty — only the regula-falsi CF leaf takes a
/// `float4 c` call-through parameter — Increment 4a).
const NO_VEC4_INPUTS: &[&str] = &[];

/// The default callee table (empty — only the regula-falsi CF leaf calls a frozen function
/// `m2_cubic_eval` — Increment 4a).
const NO_CALL_INPUTS: &[&str] = &[];

/// The per-trace symbolic-input name tables threaded through the printer: `float`
/// inputs ([`Node::Input`]) and `uint` inputs ([`Node::UintInput`]) are named
/// separately because a leaf's parameter list mixes the two (e.g. `decode_snorm8`'s
/// `float n` + `float band_half`, or A3's `uint code` byte source). Carried by-ref so
/// the recursive `operand_str`/`define_str` stay a single threaded slice each.
#[derive(Clone, Copy)]
struct Names<'a> {
    /// `float`-input names (indexed by [`Node::Input`]'s id).
    float_in: &'a [&'a str],
    /// `uint`-input names (indexed by [`Node::UintInput`]'s id).
    uint_in: &'a [&'a str],
    /// `float3`-VECTOR-parameter names (indexed by [`Node::VecIndex`]'s `vec_id` AND by
    /// [`Node::Vec3Param`]'s id) — the `rd` / `p` / `cell_min` the brick-marcher indexes by
    /// the unroll iv, plus the brick-cell's `p` / `origin` whole-`float3` params. Empty for
    /// the straight-line field/normal/decode leaves (they take no vector parameter).
    vec_in: &'a [&'a str],
    /// `uint3`-parameter names (indexed by [`Node::Uint3Param`]'s id) — the brick-cell's
    /// `dims`. Empty for every other leaf (Increment 3).
    uint3_in: &'a [&'a str],
    /// `StructuredBuffer<uint>`-parameter names (indexed by [`Node::BufferLoad`]'s
    /// `buf_id`) — the brick-cell's `grid`. Empty for every other leaf (Increment 3).
    buf_in: &'a [&'a str],
    /// `out`-PARAMETER names (indexed by [`Stmt::OutAssign`]'s `name_id`) — the brick-cell's
    /// `cell_min`. Its assignments print BARE (`cell_min = ...;`), not a local decl. Empty
    /// for every other leaf (Increment 3).
    out_in: &'a [&'a str],
    /// NAMED-literal symbols (indexed by [`Node::NamedLit`]'s `sym_id`) — `BRICK_EXIT_EPS`.
    /// Empty for leaves that use no named constant.
    named_lit: &'a [&'a str],
    /// MUTABLE-LOCAL names (indexed by [`Node::VarRef`]'s id) — `exit`. Empty for the
    /// straight-line leaves (they declare no mutable local).
    vars: &'a [&'a str],
    /// `float4`-PARAMETER names (indexed by [`Node::Vec4Param`]'s id) — `m2_regula_falsi`'s
    /// `c`. Empty for every other leaf (Increment 4a).
    vec4_in: &'a [&'a str],
    /// CALLEE names (indexed by [`Node::Call2`]'s `sym_id`) — `m2_cubic_eval`. Empty for
    /// every other leaf (Increment 4a).
    call_in: &'a [&'a str],
}

/// Formats one f32 literal the way the frozen HLSL writes it (`0.5`, `1.0`, `0.0`).
/// A short, deterministic rendering — enough for the smin/smax field constants.
fn fmt_lit(x: f32) -> String {
    if x == x.trunc() && x.abs() < 1.0e7 {
        // Integer-valued (small): render as `N.0` (matches `0.0` / `1.0` / `127.0` in the
        // frozen src). The `< 1.0e7` bound keeps this from spelling huge magnitudes as a
        // 30-digit decimal.
        format!("{:.1}", x)
    } else if x != 0.0 && (x.abs() >= 1.0e7 || x.abs() < 1.0e-4) {
        // Very large / very small magnitudes render in SCIENTIFIC form to match the frozen
        // src's `1.0e30` (a 30-digit decimal would parse to the same bits but not match the
        // committed text). Rust's `{:e}` gives `1e30`; normalize the mantissa to carry a
        // `.0` (`1e30` -> `1.0e30`) so it reads as a float literal. The only such literal in
        // any traced body is the brick-exit `1.0e30` init.
        let e = format!("{:e}", x); // e.g. "1e30" / "1.5e-5"
        if let Some((mantissa, exp)) = e.split_once('e') {
            if mantissa.contains('.') {
                format!("{mantissa}e{exp}")
            } else {
                format!("{mantissa}.0e{exp}")
            }
        } else {
            e
        }
    } else {
        // Non-integer (normal magnitude): a compact shortest round-trip (e.g. `0.5`).
        let s = format!("{}", x);
        if s.contains('.') || s.contains('e') {
            s
        } else {
            format!("{}.0", s)
        }
    }
}

/// True for nodes printed INLINE at every use site (no temp): the leaf operands
/// (inputs + literals) and the comparison (which only appears inside a ternary
/// condition). Every arithmetic node gets a `float tN = ...;` temp instead, so a
/// shared subtree (e.g. `hh`) is computed ONCE — matching the frozen `hh` variable.
fn is_inline_leaf(node: Node) -> bool {
    matches!(
        node,
        Node::Input(_)
            | Node::Lit(_)
            | Node::Gt(_, _)
            | Node::Lt(_, _)
            | Node::Le(_, _)
            | Node::Ge(_, _)
            | Node::IntEq(_, _)
            | Node::UintInput(_)
            | Node::UintLit(_)
            | Node::VecIndex(_, _)
            | Node::NamedLit { .. }
            | Node::VecParamRef(_)
            | Node::VarRef(_)
            | Node::TempRef(_)
            // Increment 3 inline leaves: a parameter ref / swizzle spells its name at every
            // use (`origin`, `rel.x`, `dims.x`); the buffer load spells `grid[idx]`; the
            // `>=`/`||` masks appear only inside a guard condition (never as a `tN` temp).
            | Node::Vec3Param(_)
            | Node::Vec3Swizzle(_, _)
            | Node::Uint3Param(_)
            | Node::Uint3Swizzle(_, _)
            | Node::BufferLoad(_, _)
            | Node::UGe(_, _)
            // Increment 4f: the `uint` `>` guard (`it > 0u`) and the logical `&&` appear
            // only inside a condition (inlined like `UGe`/`Or`), never as a `tN` temp.
            | Node::UGt(_, _)
            | Node::And2(_, _)
            | Node::Or(_, _)
            // Increment 4a: the `float4` parameter `c` spells its name (`c`) at every use;
            // it is consumed only by `Call2`. (The `Call2` itself materializes as a `f_mid`
            // temp, so it is NOT an inline leaf.)
            | Node::Vec4Param(_)
            // Increment 4b.2: a `bool` literal spells `true`/`false` inline (it is the
            // operand of a `Stmt::Return`, never a `tN` temp).
            | Node::BoolLit(_)
    )
}

/// An operand's CONTEXT in its parent infix expression — the parent's precedence class
/// together with this operand's side. Threaded into [`needs_paren_as_operand`] so the
/// brick `idx` line's inner `UAdd` (the LEFT operand of an additive parent) stays flat
/// (`a + b + c`, not `(a + b) + c`) while a `Sub` LEFT operand of a `Mul` still wraps
/// (`(t1 - p[a]) * t3`). The precedence is decided by the PARENT, the associativity by
/// the SIDE.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OperandPos {
    /// No enclosing infix op (a function-call argument, a comparison comparand, the root
    /// return) — the inline spelling needs no extra wrap regardless of its shape.
    Root,
    /// The LEFT operand of an additive (`+`/`-`) parent. A left-associative `+`/`-` LEFT
    /// operand of the SAME class needs NO wrap (`a + b + c` already groups left) — the
    /// brick `idx` line's load-bearing flat case.
    AddLeft,
    /// The RIGHT operand of an additive (`+`/`-`) parent. A same-class subexpression DOES
    /// wrap here (`a - (b - c)`).
    AddRight,
    /// An operand (either side) of a MULTIPLICATIVE (`*`/`/`) parent — an additive child
    /// always wraps (`(t1 - p[a]) * t3`), since `*`/`/` bind tighter than `+`/`-`.
    MulSide,
}

/// The operand spelling for node `id` at a USE site: a leaf inlines (its input
/// name or formatted literal, or the `a > b` comparison nested in a ternary
/// condition); a non-leaf names its already-emitted `tN` temp. The temp must exist
/// because the SSA walk emits nodes in arena order, which is topological. `pos` is the
/// operand's position in its parent (drives the position-aware paren rule); callers
/// where position is irrelevant (a leaf use, a function-call arg, the root return) pass
/// [`OperandPos::Left`] (the never-wrap position).
fn operand_str(
    arena: &[Node],
    names: Names,
    temps: &[Option<String>],
    id: u32,
    pos: OperandPos,
) -> String {
    let node = arena[id as usize];
    // A nested comparand / index / swizzle source is position-irrelevant — pass `Root`.
    let opl = |child: u32| operand_str(arena, names, temps, child, OperandPos::Root);
    match node {
        Node::Input(n) => names.float_in[n as usize].to_string(),
        Node::Lit(x) => fmt_lit(x),
        Node::Gt(a, b) => format!("{} > {}", opl(a), opl(b)),
        Node::Lt(a, b) => format!("{} < {}", opl(a), opl(b)),
        Node::Le(a, b) => format!("{} <= {}", opl(a), opl(b)),
        Node::Ge(a, b) => format!("{} >= {}", opl(a), opl(b)),
        // `vec[iv]` — the vector parameter's name (`rd`/`p`/`cell_min`) indexed by the
        // iv node's own spelling (`a`). Both inline, so `p[a]` spells inline at each use.
        Node::VecIndex(vec_id, iv_id) => format!("{}[{}]", names.vec_in[vec_id as usize], opl(iv_id)),
        // The named literal prints the SYMBOL (`BRICK_EXIT_EPS`), not its `val`.
        Node::NamedLit { sym_id, .. } => names.named_lit[sym_id as usize].to_string(),
        // A mutable-local read prints the variable NAME (`exit`), not a `tN` temp.
        Node::VarRef(v) => names.vars[v as usize].to_string(),
        // A materialized-temp reference prints its NAME (`rel`/`ix`, a named brick-cell
        // temp) or `t{seq}` (an anonymous brick-exit temp).
        Node::TempRef(seq) => match temp_name(seq) {
            Some(n) => n.to_string(),
            None => format!("t{seq}"),
        },
        // A vector-parameter marker is consumed by `EmitCf::index` (→ `VecIndex`); it is
        // never spelled directly.
        Node::VecParamRef(_) => {
            unreachable!("VecParamRef is consumed by index() into VecIndex, never printed")
        }
        Node::IntEq(a, b) => format!("{} == {}", opl(a), opl(b)),
        Node::UintInput(n) => names.uint_in[n as usize].to_string(),
        // A `uint` literal renders with the HLSL unsigned suffix (`0xFFu`, `8u`).
        Node::UintLit(u) => format!("{}u", u),
        // ---- Increment 3 inline leaves -------------------------------------------------
        // A `float3`/`uint3` parameter spells its NAME (`origin`, `dims`).
        Node::Vec3Param(n) => names.vec_in[n as usize].to_string(),
        Node::Uint3Param(n) => names.uint3_in[n as usize].to_string(),
        // A swizzle spells `<source>.x` (the source is itself an inline leaf — a param ref
        // or a `rel` temp ref — so it never needs a wrap; pass `Left`).
        Node::Vec3Swizzle(v, axis) => format!("{}.{}", opl(v), AXIS[axis as usize]),
        Node::Uint3Swizzle(v, axis) => format!("{}.{}", opl(v), AXIS[axis as usize]),
        // `grid[idx]` — the buffer name + the index (an inline leaf, the `idx` temp ref).
        Node::BufferLoad(buf, idx) => format!("{}[{}]", names.buf_in[buf as usize], opl(idx)),
        // The `>=` / `||` masks appear only inside a guard condition (inlined like `Gt`).
        Node::UGe(a, b) => format!("{} >= {}", opl(a), opl(b)),
        Node::Or(a, b) => format!("{} || {}", opl(a), opl(b)),
        // The `uint` `>` guard (`it > 0u`) and the logical `&&` (Increment 4f) — inlined inside
        // a condition. Both operands spell at Root (`opl` is the Root shorthand here): a
        // comparison comparand needs no wrap (falls to `needs_paren_as_operand`'s `_ => false`),
        // and an `&&` operand at Root excludes every wrap class, so `it > 0u && sor_prev + d <
        // FIELD_LIPSCHITZ_L * sor_step_prev` prints FLAT (no parens), byte-identical to committed.
        Node::UGt(a, b) => format!("{} > {}", opl(a), opl(b)),
        Node::And2(a, b) => format!("{} && {}", opl(a), opl(b)),
        // A `float4` parameter (`c`) spells its NAME — a whole-`float4` call-through operand
        // (Increment 4a). Consumed only by `Call2`; never swizzled.
        Node::Vec4Param(n) => names.vec4_in[n as usize].to_string(),
        // A `bool` literal spells `true`/`false` (Increment 4b.2) — the `m2_surface_hit`
        // return value (`OpConstantTrue`/`OpConstantFalse`, NOT a `uint`).
        Node::BoolLit(b) => if b { "true".to_string() } else { "false".to_string() },
        // A non-leaf operand: if it was MATERIALIZED as a `tN` temp (the straight-line
        // field/normal/decode/cubic emit materializes every non-leaf via the SSA walk,
        // so `temps[id]` is always `Some` there), use the temp name. Otherwise — the CF
        // body's UN-`temp`'d nodes (`abs(t0)` in the skip guard) — INLINE it via
        // `define_str` (recursing through the same rule). This keeps the existing leaves
        // byte-unchanged (their temps are always present) while letting a CF condition
        // spell `abs(t0)` inline, matching the committed body.
        _ => match temps.get(id as usize).and_then(|t| t.as_ref()) {
            Some(name) => name.clone(),
            None => {
                // An UN-`temp`'d non-leaf used as an operand (the CF body's inline
                // subexpressions). Inline it via `define_str`, PARENTHESIZING per the
                // POSITION-AWARE rule so it composes correctly inside a higher-precedence
                // parent — e.g. `(t1 - p[a]) * t3`, NOT `t1 - p[a] * t3` — yet a LEFT
                // same-precedence operand stays flat (`ix + iy*dims.x + iz*...`, NOT
                // `(ix + iy*dims.x) + iz*...`). A function-call form / a comparison need no
                // wrap.
                let inner = define_str(arena, names, temps, id);
                if needs_paren_as_operand(node, pos) {
                    format!("({inner})")
                } else {
                    inner
                }
            }
        },
    }
}

/// The HLSL swizzle component letters, indexed by axis (0=x, 1=y, 2=z).
const AXIS: [&str; 3] = ["x", "y", "z"];

/// True for a node whose inline spelling must be PARENTHESIZED in the parent `pos`.
///
/// PRECEDENCE-AND-ASSOCIATIVITY-AWARE (Increment 3): the precedence comes from the
/// PARENT's class, the wrap-or-not from the SIDE.
/// - A `Neg`/`Select` always groups (a unary minus / a ternary, side-independent).
/// - An ADDITIVE node (`Add`/`Sub`/`UAdd`):
///   - under a MULTIPLICATIVE parent ([`OperandPos::MulSide`]) → ALWAYS wraps (`(t1 -
///     p[a]) * t3` — `*`/`/` bind tighter), preserving the existing leaves byte-for-byte.
///   - as the RIGHT operand of an additive parent ([`OperandPos::AddRight`]) → wraps
///     (`a - (b - c)`).
///   - as the LEFT operand of an additive parent ([`OperandPos::AddLeft`]) → NO wrap
///     (left-associativity: `a + b + c` is already `(a + b) + c`) — the brick `idx`
///     line's load-bearing flat case.
///   - at the root ([`OperandPos::Root`]) → no wrap.
///
/// `Mul`/`Div`/`UMul` bind tighter than `+`/`-`, so they never need a wrap as an additive
/// or multiplicative operand (the brick `idx`'s products are bare `iy*dims.x`); they are
/// NOT in the set.
fn needs_paren_as_operand(node: Node, pos: OperandPos) -> bool {
    match node {
        // A unary minus / a ternary always groups (side-independent).
        Node::Neg(..) | Node::Select(..) | Node::SelectParen(..) => true,
        // Additive infix nodes (scalar `+`/`-`/`uint +`, AND the `float3` `+`/`-`): wrap
        // under a multiplicative parent (precedence: `(p - origin) / bw`, `(t1 - p[a]) *
        // t3`) or in the additive-RIGHT position (associativity); the additive-LEFT + root
        // positions stay flat (the brick `idx` line's `ix + iy*dims.x + iz*...`).
        Node::Add(..)
        | Node::Sub(..)
        | Node::UAdd(..)
        | Node::Vec3Add(..)
        | Node::Vec3Sub(..) => matches!(pos, OperandPos::MulSide | OperandPos::AddRight),
        _ => false,
    }
}

/// M1 (review carry-in): asserts an operand node's HLSL [`EmitTy`] is `want`. A
/// FLOAT arithmetic node (`Add`/`Mul`/...) must not take a `uint` operand and a bit
/// op (`And`/`Shr`/`UintToFloat`) must take a `uint` operand — the int/float boundary
/// introduced by the brick leaves is now load-bearing, so a mistyped operand (a
/// future int-leaf transcription slip) would emit a textually-valid but
/// SEMANTICALLY-WRONG body that silently forks the `.spv`. A `debug_assert!`
/// (compiled out in release) catches it at generation time. The current cubic leaf
/// is all-float, so this mainly guards the A4 / int-leaf family; wired now because
/// the boundary exists.
fn assert_operand_ty(arena: &[Node], id: u32, want: EmitTy) {
    debug_assert_eq!(
        type_of(arena[id as usize]),
        want,
        "operand node {id} has type {:?} but the consuming op expects {want:?} — a \
         mistyped int/float operand in a generated leaf",
        type_of(arena[id as usize])
    );
}

/// The HLSL expression that DEFINES node `id` (its temp's right-hand side), built
/// from its operands' names (a temp name, an input name, or an inlined literal).
fn define_str(arena: &[Node], names: Names, temps: &[Option<String>], id: u32) -> String {
    let node = arena[id as usize];
    // Position-aware operand spellings: `op` = position-irrelevant (Root — function-call
    // args, comparison comparands, the `min`/`max`/`abs` intrinsics); `opl`/`opr` = the
    // LEFT/RIGHT operand of an ADDITIVE parent; `opm` = an operand of a MULTIPLICATIVE
    // parent (an additive child always wraps). The existing float leaves use `op`/`opm`
    // (their products always wrapped an additive child), so they stay byte-identical.
    let op = |child: u32| operand_str(arena, names, temps, child, OperandPos::Root);
    let opl = |child: u32| operand_str(arena, names, temps, child, OperandPos::AddLeft);
    let opr = |child: u32| operand_str(arena, names, temps, child, OperandPos::AddRight);
    let opm = |child: u32| operand_str(arena, names, temps, child, OperandPos::MulSide);
    // M1: the FLOAT arithmetic nodes take FLOAT operands; the bit/cast nodes take
    // UINT operands. Check before spelling (the `Mask` operand of `Select` and the
    // `Gt`/`IntEq` comparands are excluded — a comparison is an inlined leaf typed
    // `Float` by `type_of`, but its operands' types are the comparison's own concern).
    let chk = |child: u32, want: EmitTy| assert_operand_ty(arena, child, want);
    match node {
        Node::Input(_)
        | Node::Lit(_)
        | Node::Gt(_, _)
        | Node::Lt(_, _)
        | Node::Le(_, _)
        | Node::Ge(_, _)
        | Node::IntEq(_, _)
        | Node::UintInput(_)
        | Node::UintLit(_)
        | Node::VecIndex(_, _)
        | Node::NamedLit { .. }
        | Node::VecParamRef(_)
        | Node::VarRef(_)
        | Node::TempRef(_)
        // A `bool` literal (`true`/`false`) is inlined as the operand of a `Stmt::Return`
        // (Increment 4b.2), never materialized as a temp.
        | Node::BoolLit(_) => {
            // Leaves are inlined, never defined as a temp.
            unreachable!("leaf nodes are inlined, not defined")
        }
        Node::Add(a, b) => {
            chk(a, EmitTy::Float);
            chk(b, EmitTy::Float);
            format!("{} + {}", opl(a), opr(b))
        }
        Node::Sub(a, b) => {
            chk(a, EmitTy::Float);
            chk(b, EmitTy::Float);
            format!("{} - {}", opl(a), opr(b))
        }
        Node::Mul(a, b) => {
            chk(a, EmitTy::Float);
            chk(b, EmitTy::Float);
            format!("{} * {}", opm(a), opm(b))
        }
        Node::Div(a, b) => {
            chk(a, EmitTy::Float);
            chk(b, EmitTy::Float);
            format!("{} / {}", opm(a), opm(b))
        }
        Node::Neg(a) => {
            chk(a, EmitTy::Float);
            format!("-{}", op(a))
        }
        Node::Min(a, b) => {
            chk(a, EmitTy::Float);
            chk(b, EmitTy::Float);
            format!("min({}, {})", op(a), op(b))
        }
        Node::Max(a, b) => {
            chk(a, EmitTy::Float);
            chk(b, EmitTy::Float);
            format!("max({}, {})", op(a), op(b))
        }
        Node::Clamp01(a) => {
            chk(a, EmitTy::Float);
            format!("clamp({}, 0.0, 1.0)", op(a))
        }
        Node::Lerp(s, a, h) => {
            chk(s, EmitTy::Float);
            chk(a, EmitTy::Float);
            chk(h, EmitTy::Float);
            format!("lerp({}, {}, {})", op(s), op(a), op(h))
        }
        Node::Abs(a) => {
            chk(a, EmitTy::Float);
            format!("abs({})", op(a))
        }
        Node::Sqrt(a) => {
            chk(a, EmitTy::Float);
            format!("sqrt({})", op(a))
        }
        Node::Select(_c, t, e) => {
            // The `_c` condition is a Mask leaf (a `Gt`/`IntEq`/`Le` node), not a float
            // value — only the two value arms are typed here. Arms spelled at `Root` (the
            // committed brick-exit progress-clamp `(exit < EPS) ? BRICK_EXIT_EPS : exit`,
            // arms UN-wrapped — a leaf is position-irrelevant). UNCHANGED by Inc 4a.
            chk(t, EmitTy::Float);
            chk(e, EmitTy::Float);
            format!("({}) ? {} : {}", op(_c), op(t), op(e))
        }
        Node::SelectParen(_c, t, e) => {
            // Increment 4a: wrap BOTH arms UNCONDITIONALLY — `(cond) ? (then) : (else)` —
            // the committed `m2_regula_falsi` ternary form (`? (lo - ...) : (0.5 * ...)`).
            // `needs_paren_as_operand` would wrap the then arm (a `Sub`) but NOT the else arm
            // (a `Mul`, which it explicitly excludes), so a precedence-aware position cannot
            // reproduce the committed parens; the unconditional wrap does. A DISTINCT node
            // from `Select` (recorded only by `Cf::select`, never `FieldScalar::select`) so
            // the brick-exit's `Select` printer stays UN-wrapped and byte-identical. The
            // spike (STEP 1) proved the no-`-O` `.spv` is byte-identical WITH the parens.
            chk(t, EmitTy::Float);
            chk(e, EmitTy::Float);
            format!("({}) ? ({}) : ({})", op(_c), op(t), op(e))
        }
        Node::Call2 { sym_id, a, b } => {
            // `m2_cubic_eval(c, mid)` — a frozen-function call site (Increment 4a). The two
            // args spell at `Root` (function-call args, position-irrelevant): `a` a `float4`
            // (`c`), `b` a `float` (`mid`). The `a` operand is checked `Float4`; `b` is the
            // mid var ref (a `Float`), checked by the printer's leaf typing — left unchecked
            // here since `b` may be a `VarRef`/`TempRef` whose `type_of` is `Float`.
            chk(a, EmitTy::Float4);
            format!("{}({}, {})", names.call_in[sym_id as usize], op(a), op(b))
        }
        Node::Call1 { sym_id, a } => {
            // `field_distance(p + L * t)` — a frozen single-`float3`-arg call site (Inc 4b).
            // The arg spells at `Root` (a function-call arg, position-irrelevant) and is a
            // `float3` (the `p + L * t` probe point), checked `Float3`.
            chk(a, EmitTy::Float3);
            format!("{}({})", names.call_in[sym_id as usize], op(a))
        }
        Node::FieldCall(_) => {
            // `FieldCall` belongs to the NORMAL leaf (a vector expression printed by
            // `sexpr_str`/`vexpr_str`), never to the scalar field body's printer.
            unreachable!("FieldCall is a normal-leaf node, not a scalar field node")
        }
        Node::And(a, b) => {
            chk(a, EmitTy::Uint);
            chk(b, EmitTy::Uint);
            format!("{} & {}", op(a), op(b))
        }
        Node::Shr(a, b) => {
            chk(a, EmitTy::Uint);
            chk(b, EmitTy::Uint);
            format!("{} >> {}", op(a), op(b))
        }
        // The HLSL numeric cast (value-preserving), NOT `asfloat` (a bit-reinterpret).
        Node::UintToFloat(a) => {
            chk(a, EmitTy::Uint);
            format!("(float){}", op(a))
        }

        // ---- Increment 3 materialized (non-leaf) nodes ---------------------------------
        // `float3(ix, iy, iz)` — the implicit uint→float ctor (asserts all-`uint`).
        Node::Vec3FromUints(x, y, z) => {
            chk(x, EmitTy::Uint);
            chk(y, EmitTy::Uint);
            chk(z, EmitTy::Uint);
            format!("float3({}, {}, {})", op(x), op(y), op(z))
        }
        Node::Vec3Add(a, b) => {
            chk(a, EmitTy::Float3);
            chk(b, EmitTy::Float3);
            format!("{} + {}", opl(a), opr(b))
        }
        Node::Vec3Sub(a, b) => {
            chk(a, EmitTy::Float3);
            chk(b, EmitTy::Float3);
            format!("{} - {}", opl(a), opr(b))
        }
        // `float3 * float` / `float3 / float`: the vector is the multiplicative side, the
        // scalar a `float`. (`float3(...) * bw` / `(p - origin) / bw`.)
        Node::Vec3MulScalar(v, s) => {
            chk(v, EmitTy::Float3);
            chk(s, EmitTy::Float);
            format!("{} * {}", opm(v), opm(s))
        }
        Node::Vec3DivScalar(v, s) => {
            chk(v, EmitTy::Float3);
            chk(s, EmitTy::Float);
            format!("{} / {}", opm(v), opm(s))
        }
        // `(uint)f` — the HLSL float→uint truncating cast.
        Node::FloatToUint(a) => {
            chk(a, EmitTy::Float);
            format!("(uint){}", op(a))
        }
        // `a + b` / `a * b` over `uint`s (the cell index math).
        Node::UAdd(a, b) => {
            chk(a, EmitTy::Uint);
            chk(b, EmitTy::Uint);
            format!("{} + {}", opl(a), opr(b))
        }
        Node::UMul(a, b) => {
            chk(a, EmitTy::Uint);
            chk(b, EmitTy::Uint);
            format!("{} * {}", opm(a), opm(b))
        }

        // The Increment-3 inline leaves (`Vec3Param`/`Vec3Swizzle`/`Uint3Param`/
        // `Uint3Swizzle`/`BufferLoad`/`UGe`/`Or`) — plus the Increment-4f `UGt`/`And2` masks —
        // are spelled by `operand_str`, never materialized as a temp, so they must not reach
        // `define_str`.
        Node::Vec3Param(_)
        | Node::Vec3Swizzle(_, _)
        | Node::Uint3Param(_)
        | Node::Uint3Swizzle(_, _)
        | Node::BufferLoad(_, _)
        | Node::UGe(_, _)
        | Node::UGt(_, _)
        | Node::And2(_, _)
        | Node::Or(_, _)
        // The Increment-4a `float4` parameter `c` is an inline leaf (spelled by operand_str).
        | Node::Vec4Param(_) => {
            unreachable!("inline leaves are spelled by operand_str, not defined")
        }
    }
}

/// Emits the `float tN = ...;` (or `uint tN`) temp declarations for every NON-leaf
/// node in arena order (which is topological — SSA: a node only references
/// strictly-earlier indices), so shared subtrees are computed ONCE. Returns the
/// `temps` table (each non-leaf node's emitted temp name) for the caller to spell
/// the `return`. Factored out of [`emit_body`] so the scalar return ([`emit_body`])
/// and the `float4` construct return ([`emit_body_vec4`]) share the identical
/// temp-emission walk.
fn emit_temps(arena: &[Node], names: Names) -> (String, Vec<Option<String>>) {
    let mut temps: Vec<Option<String>> = vec![None; arena.len()];
    let mut out = String::new();
    let mut next = 0u32;
    for (i, &node) in arena.iter().enumerate() {
        if is_inline_leaf(node) {
            continue;
        }
        let name = format!("t{}", next);
        next += 1;
        let rhs = define_str(arena, names, &temps, i as u32);
        // O2: declare the temp with its node's HLSL type (`float` for every field/
        // normal node; `uint` for an integer/bit node). The default `Float` keeps
        // the existing smin/smax/sdf_normal emit byte-identical.
        let ty = ty_keyword(type_of(node));
        out.push_str(&format!("    {} {} = {};\n", ty, name, rhs));
        temps[i] = Some(name);
    }
    (out, temps)
}

/// Walks the recorded SSA arena into a `{ float tN = ...; ... return tROOT; }`
/// HLSL body. Each NON-leaf node becomes one `float tN` temp (so shared subtrees —
/// e.g. `hh` in `smin` — are computed ONCE, matching the frozen `hh` variable); the
/// leaves (inputs/literals) inline.
fn emit_body(arena: &[Node], names: Names, root: u32) -> String {
    let (mut out, temps) = emit_temps(arena, names);
    let ret = operand_str(arena, names, &temps, root, OperandPos::Root);
    out.push_str(&format!("    return {};\n", ret));
    out
}

/// Like [`emit_body`], but the FOUR `roots` are spelled as a `return float4(r0, r1,
/// r2, r3);` construct — the `jcgt_cubic_coeffs` `[c0, c1, c2, c3]` array return,
/// mirroring the GPU `m2_jcgt_cubic_coeffs`'s `return float4(c0, c1, c2, c3)`. The
/// shared temp-emission walk ([`emit_temps`]) computes the k-basis / cubic subtrees
/// ONCE; each root operand inlines its (already-emitted) `tN` temp into the
/// constructor, the SAME shape the frozen HLSL has (`float c0 = ...; return
/// float4(c0, ...)`). The textual temp NAMES differ from the frozen source
/// (`tN` vs `c0`), which is invisible to the `.spv` (DXC strips local debug names).
fn emit_body_vec4(arena: &[Node], names: Names, roots: [u32; 4]) -> String {
    let (mut out, temps) = emit_temps(arena, names);
    let r = |id: u32| operand_str(arena, names, &temps, id, OperandPos::Root);
    out.push_str(&format!(
        "    return float4({}, {}, {}, {});\n",
        r(roots[0]),
        r(roots[1]),
        r(roots[2]),
        r(roots[3])
    ));
    out
}

/// Runs `body` over a fresh symbolic-input arena and returns the HLSL `{ ... }`
/// statement body for the result node. `inputs` is the count of [`Emit::input`]
/// (`float`) handles to seed, named by [`FIELD_INPUT_NAMES`] in order; `body` receives
/// them and returns the result handle. The field/normal leaves take no `uint`
/// parameter ([`NO_UINT_INPUTS`]); an integer leaf uses [`trace_named`].
fn trace<F: FnOnce(&[Emit]) -> Emit>(inputs: usize, body: F) -> String {
    let names = Names {
        float_in: FIELD_INPUT_NAMES,
        uint_in: NO_UINT_INPUTS,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
    };
    ARENA.with(|a| a.borrow_mut().clear());
    let mut ins = Vec::with_capacity(inputs);
    for i in 0..inputs {
        ins.push(Emit::input(i as u32));
    }
    let result = body(&ins);
    ARENA.with(|a| {
        let a = a.borrow();
        emit_body(&a, names, result.0)
    })
}

/// Like [`trace`], but for a leaf with a CUSTOM parameter list: `float_names` /
/// `uint_names` are the per-leaf input tables (e.g. `decode_snorm8`'s `["n",
/// "band_half"]` floats + no uints). `body` receives the seeded `float` handles
/// (named by `float_names`) and the seeded `uint` handles (named by `uint_names`),
/// in that order, and returns the result node. The two seed groups are pushed
/// float-first so a leaf's `Node::Input`/`Node::UintInput` ids index their own table.
fn trace_named<F: FnOnce(&[Emit], &[Emit]) -> Emit>(
    float_names: &[&str],
    uint_names: &[&str],
    body: F,
) -> String {
    let names = Names {
        float_in: float_names,
        uint_in: uint_names,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
    };
    ARENA.with(|a| a.borrow_mut().clear());
    let mut floats = Vec::with_capacity(float_names.len());
    for i in 0..float_names.len() {
        floats.push(Emit::input(i as u32));
    }
    let mut uints = Vec::with_capacity(uint_names.len());
    for i in 0..uint_names.len() {
        uints.push(Emit::uint_input(i as u32));
    }
    let result = body(&floats, &uints);
    ARENA.with(|a| {
        let a = a.borrow();
        emit_body(&a, names, result.0)
    })
}

/// Like [`trace_named`], but `body` returns FOUR result handles (the `[c0, c1, c2,
/// c3]` cubic-coefficient array), emitted as a `return float4(...)` construct
/// ([`emit_body_vec4`]). `jcgt_cubic_coeffs` is the only leaf with a `float4` return;
/// the four roots share the recorded k-basis / cubic temps in the one arena, so the
/// constructor inlines the four already-emitted temps.
fn trace_named_vec4<F: FnOnce(&[Emit], &[Emit]) -> [Emit; 4]>(
    float_names: &[&str],
    uint_names: &[&str],
    body: F,
) -> String {
    let names = Names {
        float_in: float_names,
        uint_in: uint_names,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
    };
    ARENA.with(|a| a.borrow_mut().clear());
    let mut floats = Vec::with_capacity(float_names.len());
    for i in 0..float_names.len() {
        floats.push(Emit::input(i as u32));
    }
    let mut uints = Vec::with_capacity(uint_names.len());
    for i in 0..uint_names.len() {
        uints.push(Emit::uint_input(i as u32));
    }
    let result = body(&floats, &uints);
    let roots = [result[0].0, result[1].0, result[2].0, result[3].0];
    ARENA.with(|a| {
        let a = a.borrow();
        emit_body_vec4(&a, names, roots)
    })
}

/// Generates the HLSL bodies of the frozen field functions (`smin`/`smax`/`combine`)
/// by TRACING the generic [`crate::field`] bodies over the [`Emit`] backend, and
/// returns them as a single HLSL source string.
///
/// For Pass 1 this PRINTS (the bin writes it to stdout/a file). It does NOT splice
/// into `sdf_field.hlsli` and does NOT recompile any `.spv` — that is Pass 2 (which
/// also adds `precise`). The output is the textual-equivalence artifact the owner
/// eyeballs against the frozen `smin`/`smax`/`combine`.
pub fn emit_hlsl_field() -> String {
    use crate::field;

    // smin(a, b, k): traced over the three scalar inputs a, b, k.
    let smin_body = trace(3, |i| field::smin::<Emit>(i[0], i[1], i[2]));
    // smax(a, b, k) = -smin(-a, -b, k).
    let smax_body = trace(3, |i| field::smax::<Emit>(i[0], i[1], i[2]));
    // combine for each op (the host op-dispatch picks the formula). The smooth
    // (k > 0) arm shows the traced smin/smax; the hard arm is the ternary's
    // `min`/`max` else. `a` = acc, `b` = d here (INPUT_NAMES reused).
    let combine_union = trace(3, |i| {
        field::combine::<Emit>(i[0], i[1], field::op::UNION, i[2])
    });
    let combine_subtract = trace(3, |i| {
        field::combine::<Emit>(i[0], i[1], field::op::SUBTRACT, i[2])
    });
    let combine_intersect = trace(3, |i| {
        field::combine::<Emit>(i[0], i[1], field::op::INTERSECT, i[2])
    });

    format!(
        "// === GENERATED by boyko_shaderdsl::emit (Pass 1, NO precise) ===\n\
         // Traced from the generic field body in `boyko_shaderdsl::field` over the\n\
         // `Emit` backend (SSA temps; shared subtrees emitted once). Eyeball this\n\
         // against `sdf_field.hlsli:110-129`.\n\
         // (`a`=acc, `b`=d, `k`=smoothness in `combine`; `a`,`b`,`k` in smin/smax.)\n\
         \n\
         float smin(float a, float b, float k) {{\n{smin_body}}}\n\
         \n\
         float smax(float a, float b, float k) {{\n{smax_body}}}\n\
         \n\
         // combine(acc=a, d=b, op, k) — UNION branch:\n\
         float combine_union(float a, float b, float k) {{\n{combine_union}}}\n\
         \n\
         // combine(acc=a, d=b, op, k) — SUBTRACT branch:\n\
         float combine_subtract(float a, float b, float k) {{\n{combine_subtract}}}\n\
         \n\
         // combine(acc=a, d=b, op, k) — INTERSECT branch:\n\
         float combine_intersect(float a, float b, float k) {{\n{combine_intersect}}}\n",
    )
}

// ---- The normal leaf (a VECTOR expression) ------------------------------------

/// The three GRAD_H swizzle spellings, indexed by axis (`Swizzle`'s `u8`). The
/// frozen `sdf_normal` reads `e.xyy` (x-axis), `e.yxy` (y-axis), `e.yyx` (z-axis)
/// off `float2 e = float2(GRAD_H, 0.0)` — these textual swizzles are load-bearing for
/// the SPIR-V gate (decomposing them into `float3(GRAD_H,0,0)` would fork the
/// baseline disassembly).
const SWIZZLES: [&str; 3] = ["e.xyy", "e.yxy", "e.yyx"];

/// The inline HLSL spelling of a VECTOR node `id` — the normal's `float3`/`float2`
/// subexpressions are printed inline (no temps), matching the compact frozen
/// `sdf_normal`. `scalar` is the scalar arena (for the `float3(...)` constructor's
/// component handles); `varena` is the vector arena.
fn vexpr_str(varena: &[VNode], scalar: &[Node], id: u32) -> String {
    match varena[id as usize] {
        VNode::InputP => "p".to_string(),
        VNode::EpsE => "e".to_string(),
        VNode::Swizzle(axis) => SWIZZLES[axis as usize].to_string(),
        VNode::VAdd(a, b) => format!(
            "{} + {}",
            vexpr_str(varena, scalar, a),
            vexpr_str(varena, scalar, b)
        ),
        VNode::VSub(a, b) => format!(
            "{} - {}",
            vexpr_str(varena, scalar, a),
            vexpr_str(varena, scalar, b)
        ),
        VNode::Construct(x, y, z) => format!(
            "float3(\n        {},\n        {},\n        {})",
            sexpr_str(varena, scalar, x),
            sexpr_str(varena, scalar, y),
            sexpr_str(varena, scalar, z)
        ),
    }
}

/// The inline HLSL spelling of a SCALAR node `id` for the normal body — only the
/// node kinds the central differences use (`FieldCall` and `Sub`); the `FieldCall`'s
/// argument is a vector handle, so the scalar and vector printers recurse into each
/// other.
fn sexpr_str(varena: &[VNode], scalar: &[Node], id: u32) -> String {
    match scalar[id as usize] {
        Node::FieldCall(v) => format!("sdf({})", vexpr_str(varena, scalar, v)),
        Node::Sub(a, b) => format!(
            "{} - {}",
            sexpr_str(varena, scalar, a),
            sexpr_str(varena, scalar, b)
        ),
        // The normal body only ever forms `FieldCall - FieldCall`; no other scalar
        // node reaches this printer.
        other => unreachable!("normal scalar printer reached an unexpected node: {other:?}"),
    }
}

/// Builds the `sdf_normal` body by recording the SAME dataflow [`crate::normal::sdf_normal_body`]
/// expresses (the GRAD_H swizzle offsets, the six `sdf` probe calls, the three
/// central differences, and the final `normalize`), into the VECTOR arena, then
/// prints it in the frozen textual shape (`float2 e` / `float3 n = float3(...)` /
/// `return normalize(n)`).
///
/// It records the dataflow directly (rather than monomorphizing `sdf_normal_body`
/// over `Emit`) because the normal is a VECTOR expression and `Emit::Vec3` is
/// `[Emit; 3]` (scalar granularity) — too coarse to carry the textual swizzle the
/// SPIR-V gate needs. The structure here is operand-for-operand the same as the
/// generic body, so the single-source contract holds: the generic body is the CPU
/// source (proven byte-identical to the host normal by `eval_normal_byte_identity`),
/// and this is its HLSL twin (pinned to the committed header by `sdf_field_edsl_sync`).
fn emit_normal_body() -> String {
    ARENA.with(|a| a.borrow_mut().clear());
    VARENA.with(|a| a.borrow_mut().clear());

    let p = vpush(VNode::InputP);
    // `float2 e = float2(GRAD_H, 0.0)` — declared in the printed body; the swizzles
    // read it implicitly (it is the body's only `float2`).
    let _e = vpush(VNode::EpsE);
    // The GRAD_H swizzle offsets `e.xyy` / `e.yxy` / `e.yyx` (one per axis).
    let offsets = [
        vpush(VNode::Swizzle(0)),
        vpush(VNode::Swizzle(1)),
        vpush(VNode::Swizzle(2)),
    ];
    // Per axis: sdf(p + offset) - sdf(p - offset) — the central difference. The
    // `field` callback records a `FieldCall` node (NOT inlined: `sdf` stays the
    // hand-written `[loop]` function).
    let mut comps = [0u32; 3];
    for (axis, &o) in offsets.iter().enumerate() {
        let plus = vpush(VNode::VAdd(p, o));
        let minus = vpush(VNode::VSub(p, o));
        let f_plus = push(Node::FieldCall(plus));
        let f_minus = push(Node::FieldCall(minus));
        comps[axis] = push(Node::Sub(f_plus, f_minus));
    }
    let n = vpush(VNode::Construct(comps[0], comps[1], comps[2]));

    // The `float3 n = float3(...)` constructor is materialized as the NAMED temp `n`
    // (matching the frozen `float3 n`), so `return normalize(n)` references the name
    // — the gradient is NOT recomputed in the return. The body shape is therefore
    // byte-for-byte the frozen `sdf_normal` (which the SPIR-V gate freezes).
    //
    // W2 (deferred): the `float2 e = float2(GRAD_H, 0.0)` head and the `normalize(n)`
    // tail are HARDCODED format-string literals here, NOT arena-materialized vector
    // nodes (there is no `VNode::Eps`-construct or `VNode::Normalize`). This is fine
    // while the normal is the ONLY vector leaf, but the vector printer should be
    // generalized — `e` as a recorded `float2` constructor node and `normalize` as a
    // recorded unary vector node — WHEN a second vector-returning leaf lands (A2's
    // leaves are scalar-returning, so it is not needed yet).
    VARENA.with(|va| {
        ARENA.with(|sa| {
            let va = va.borrow();
            let sa = sa.borrow();
            let n_expr = vexpr_str(&va, &sa, n);
            format!(
                "    float2 e = float2(GRAD_H, 0.0);\n    \
                 float3 n = {n_expr};\n    \
                 return normalize(n);\n",
            )
        })
    })
}

/// Generates the HLSL `sdf_normal` body by recording the generic normal leaf's
/// dataflow over the vector arena (see [`emit_normal_body`]) and returns the full
/// `float3 sdf_normal(float3 p) { ... }` function as a string.
///
/// The generated body is spliced between the `// === GENERATED NORMAL BEGIN/END ===`
/// sentinels in `crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli`. The `sdf_field_edsl_sync`
/// test pins the committed header to this output; the `field_probe_gate` SPIR-V
/// tripwire pins it to the frozen baseline disassembly.
pub fn emit_hlsl_normal() -> String {
    let body = emit_normal_body();
    format!("float3 sdf_normal(float3 p) {{\n{body}}}\n")
}

// ---- The brick snorm decode leaf (A2) -----------------------------------------

/// Generates the HLSL `m2_decode` body — the GPU-spliceable WORLD-SCALE step of the
/// snorm decode (`n * band_half`) — by tracing the generic [`crate::brick::snorm_scale`]
/// over the `Emit` backend, and returns the full `float m2_decode(float n, float
/// band_half) { ... }` function.
///
/// Only the SCALE is shader code: the byte → normalized-float step ([`crate::brick::
/// snorm_normalize`]) is done by the fixed-function `R8_SNORM` sampler in HARDWARE, so
/// it is never spliced (see the [`crate::brick`] module doc). The `f32` Eval
/// instantiation of the WHOLE [`crate::brick::decode_snorm8`] (byte → normalize →
/// scale) is the CPU oracle, locked byte-identical to the host by the
/// `eval_byte_identity` to-bits sweep.
///
/// The generated body is spliced between the `// === GENERATED decode_snorm8
/// BEGIN/END ===` sentinels in `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`.
/// The `sdf_field_edsl_sync` test pins the committed shader to this output.
pub fn emit_hlsl_decode_snorm8() -> String {
    use crate::brick;

    // m2_decode(n, band_half) = n * band_half — traced over the two float inputs.
    let body = trace_named(&["n", "band_half"], NO_UINT_INPUTS, |f, _u| {
        brick::snorm_scale::<Emit>(f[0], f[1])
    });
    format!("float m2_decode(float n, float band_half) {{\n{body}}}\n")
}

// ---- The M2 cubic-surface leaves (A3) -----------------------------------------

/// Generates the HLSL `m2_cubic_eval` body — the Horner evaluation of the JCGT cubic
/// `c3·t³ + c2·t² + c1·t + c0` at `t` — by tracing the generic
/// [`crate::brick::cubic_eval`] over the `Emit` backend, and returns the full
/// `float m2_cubic_eval(float4 c, float t) { ... }` function.
///
/// The coefficient float4 `c` is read through the SAME scalar accessors the frozen
/// GPU uses (`c.x` = c0 ... `c.w` = c3, NOT `c[0]`), spelled as the traced input
/// names so the emitted body is byte-identical to the committed `m2_cubic_eval`. The
/// `f32` Eval instantiation is the host `boyko_sdf_math::brick::cubic_eval` (the CPU
/// oracle the `eval_byte_identity` to-bits sweep locks).
///
/// The generated body is spliced between the `// === GENERATED m2_cubic_eval
/// BEGIN/END ===` sentinels in
/// `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`. The
/// `sdf_field_edsl_sync` test pins the committed shader to this output; the cmp-`.spv`
/// gate pins it to the frozen baseline.
pub fn emit_hlsl_cubic_eval() -> String {
    use crate::brick;

    // The coefficient array is read as `c.x..c.w` (the frozen `float4 c` swizzle
    // accessors), `t` as the second parameter. `cubic_eval` indexes `c[0]..c[3]`,
    // so the names map `c[0]=c.x, c[1]=c.y, c[2]=c.z, c[3]=c.w` by input order.
    let body = trace_named(&["c.x", "c.y", "c.z", "c.w", "t"], NO_UINT_INPUTS, |f, _u| {
        let c = [f[0], f[1], f[2], f[3]];
        brick::cubic_eval::<Emit>(&c, f[4])
    });
    format!("float m2_cubic_eval(float4 c, float t) {{\n{body}}}\n")
}

/// Generates the HLSL `m2_jcgt_cubic_coeffs` body — the 8-corner → k-basis → cubic
/// coefficient fold returning `float4(c0, c1, c2, c3)` — by tracing the generic
/// [`crate::brick::jcgt_cubic_coeffs`] over the `Emit` backend, and returns the full
/// `float4 m2_jcgt_cubic_coeffs(float s[8], float3 a, float3 b) { ... }` function.
///
/// The 8 corners are read through the frozen array accessors `s[0]..s[7]` (NOT a
/// swizzle), `a`/`b` through `a.x/a.y/a.z` / `b.x/b.y/b.z` — spelled as the traced
/// input names so the body matches the committed `m2_jcgt_cubic_coeffs`. The four
/// coefficients are returned as the GPU's `float4(c0, c1, c2, c3)` construct (the
/// `[S; 4]` array return printed by [`emit_body_vec4`]). The `f32` Eval instantiation
/// is the host `boyko_sdf_math::brick::jcgt_cubic_coeffs`.
///
/// The generated body is spliced between the `// === GENERATED m2_jcgt_cubic_coeffs
/// BEGIN/END ===` sentinels in
/// `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`. The
/// `sdf_field_edsl_sync` test pins the committed shader to this output; the cmp-`.spv`
/// gate pins it to the frozen baseline.
pub fn emit_hlsl_jcgt_cubic_coeffs() -> String {
    use crate::brick;

    // s[0..7], a.x/a.y/a.z, b.x/b.y/b.z — the EXACT frozen accessor spelling (array
    // index for the corners, swizzle for the two float3s).
    let float_names = &[
        "s[0]", "s[1]", "s[2]", "s[3]", "s[4]", "s[5]", "s[6]", "s[7]", "a.x", "a.y", "a.z", "b.x",
        "b.y", "b.z",
    ];
    let body = trace_named_vec4(float_names, NO_UINT_INPUTS, |f, _u| {
        let s = [f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7]];
        let a = [f[8], f[9], f[10]];
        let b = [f[11], f[12], f[13]];
        brick::jcgt_cubic_coeffs::<Emit>(&s, a, b)
    });
    format!("float4 m2_jcgt_cubic_coeffs(float s[8], float3 a, float3 b) {{\n{body}}}\n")
}

// ===============================================================================
// The CONTROL-FLOW emit surface (Increment 1) — the STMT IR + `EmitCf` recorder +
// the block-scoped temp printer + the brick-exit generator.
//
// FIREWALL (option B): this whole surface lives in the `emit` module, which is
// `#[cfg(feature = "emit")]`-gated as a unit — a non-emit (physics) build cannot
// even NAME `Stmt` / `Block` / `EmitCf`, so a physics call is a hard compile error.
// ===============================================================================

/// One control-flow STATEMENT — the IR the [`Cf`] combinators record on the [`Emit`]
/// backend, walked by the printer into HLSL. Distinct from [`Node`] (the SSA
/// EXPRESSION arena): a `Stmt` SEQUENCES expressions and carries the loop/branch
/// structure. The expression operands reference [`Node`] arena ids.
enum Stmt {
    /// `<ty> <var> = <rhs>;` — a mutable-local declaration (`float exit = 1.0e30;`).
    /// `rhs` is the init expression's [`Node`] id.
    DeclVar { var: Var, ty: EmitTy, rhs: u32 },
    /// `<ty> t<seq> = <rhs>;` (anonymous) or `<ty> <name> = <rhs>;` (named) — a
    /// MATERIALIZED temp. `dist_to_brick_exit` uses ANONYMOUS temps (`float t0 = rd[a];`,
    /// `name = None`); `brick_cell_class` uses NAMED temps (`float3 rel = ...;`, `uint ix =
    /// ...;`, `name = Some("rel")`) to match the committed body's named locals. `seq` is
    /// the temp's program-order sequence number (the `TempRef` handle's id, used for the
    /// out-of-band type lookup AND the anonymous `t{seq}` spelling); `rhs` the materialized
    /// expression's [`Node`] id. Recorded by [`EmitCf::temp`] / [`EmitCf::temp_vec3`] /
    /// [`EmitCf::temp_uint`] (the EXPLICIT materialization the body requests).
    DeclTemp {
        seq: u32,
        name: Option<&'static str>,
        ty: EmitTy,
        rhs: u32,
    },
    /// `<var> = <rhs>;` — a mutable-local assignment (`exit = min(exit, t6);`).
    Assign { var: Var, rhs: u32 },
    /// `<name> = <rhs>;` — an OUT-PARAMETER assignment (`cell_min = origin;`). Distinct
    /// from [`Stmt::DeclVar`] (which prints a `<ty> <name> = <rhs>;` LOCAL declaration):
    /// `cell_min` is an HLSL `out` PARAMETER, so its writes are BARE assignments (no
    /// `float3` keyword). `name_id` indexes the out-param name table (carried by the
    /// printer); `rhs` the value expression's [`Node`] id. Recorded by
    /// [`EmitCf::out_vec3_assign`]; printed as `cell_min = <rhs>;`.
    OutAssign { name_id: u32, rhs: u32 },
    /// `<attr> for (uint <iv> = 0u; <iv> < <n>u; ++<iv>) { <body> }` — an UNROLLED loop.
    /// `iv` is the induction-variable name; `n` the trip count; `body` the loop block.
    UnrollFor {
        attr: &'static str,
        iv: &'static str,
        n: usize,
        body: Block,
    },
    /// `if (<cond>) { <then> }` — `cond` is the condition expression's [`Node`] id (a
    /// comparison mask), `then` the taken block (here only a [`Stmt::Continue`]).
    If { cond: u32, then: Block },
    /// `if (<cond>) { <then> } else { <els>}` — the TWO-arm branch (Increment 4a). Distinct
    /// from [`Stmt::If`] (single-arm). `cond` is the condition mask's [`Node`] id; `then` /
    /// `els` the two blocks (here the `hi = mid; f_hi = f_mid;` / `lo = mid; f_lo = f_mid;`
    /// assigns). Recorded by [`EmitCf::if_else`].
    IfElse {
        cond: u32,
        then: Block,
        els: Block,
    },
    /// `<attr> for (uint <iv> = 0u; <iv> < <bound_sym>; ++<iv>) { <body> }` — a RUNTIME loop
    /// (Increment 4a). Distinct from [`Stmt::UnrollFor`] in spelling the BOUND SYMBOL
    /// (`M2_MARMITT_ITERS`) in the header, NOT a `<n>u` literal — the key difference that
    /// (with `attr = "[loop]"`) makes DXC emit a genuine `OpLoop` (verified by the GO/NO-GO
    /// spike: `OpLoopMerge` present). `iv` is the induction-variable name (`"i"`);
    /// `bound_sym` the symbol the header spells; `body` the loop block. Recorded by
    /// [`EmitCf::runtime_for`].
    Loop {
        attr: &'static str,
        iv: &'static str,
        bound_sym: &'static str,
        body: Block,
    },
    /// `continue;` — the loop-continue (skip the rest of the iteration).
    Continue,
    /// `break;` — the loop-break (exit the loop). Recorded by [`EmitCf::brk`] (Inc 4b);
    /// mirrors [`Stmt::Continue`]. `sdf_soft_shadow`'s `if (t > T_MAX) { break; }`.
    Break,
    /// `return <expr>;` — a function return. LIVE in Increment 3: recorded by
    /// [`EmitCf::ret`] / [`EmitCf::if_ret`] (the SOLE return mechanism for the early-return
    /// marcher `brick_cell_class`). The brick-exit (Increment 1) spells its single final
    /// return directly in the printer (not via this), so its body records no `Return`.
    Return(u32),
}

/// A sequence of [`Stmt`]s — a control-flow BLOCK (a `{ ... }`).
struct Block {
    stmts: Vec<Stmt>,
}

/// A mutable-local handle — indexes the per-emit [`VARS`] name table. The variable
/// carries its OWN name (`exit`), not a `tN` temp name.
///
/// `pub` because it is the [`EmitCf`]'s [`Cf::Var`] associated type (a public trait's
/// associated type leaks the concrete type into the public API surface), but its single
/// field is private, so it is an opaque handle.
#[derive(Clone, Copy)]
pub struct Var(u32);

/// An `out`-PARAMETER name handle (the brick-cell's `cell_min`) — indexes the printer's
/// out-param name table. The [`EmitCf`]'s [`Cf::OutVec3`] associated type. Its assignments
/// print BARE (`cell_min = ...;`), not a local declaration.
#[derive(Clone, Copy)]
pub struct OutParam(u32);

/// An `out float`-PARAMETER name handle (`m2_surface_hit`'s `hit_t`) — indexes the SAME
/// printer out-param name table ([`Names::out_in`]) [`OutParam`] uses. The [`EmitCf`]'s
/// [`Cf::OutFloat`] associated type (Increment 4b.2). Its assignments print BARE
/// (`hit_t = <rhs>;`), not a `float hit_t = ...;` local declaration. Distinct type from
/// [`OutParam`] only so the `float` / `float3` out-param facets stay separate at the call site;
/// both record a [`Stmt::OutAssign`] indexing `out_in`.
#[derive(Clone, Copy)]
pub struct OutFloatParam(u32);

/// A `StructuredBuffer<uint>`-PARAMETER name handle (the brick-cell's `grid`) — indexes
/// the printer's [`Names::buf_in`] table. The [`EmitCf`]'s [`Cf::Buf`] associated type.
#[derive(Clone, Copy)]
pub struct BufParam(u32);

/// The RETURN-VALUE cell handle on the Emit backend — a ZST (the return value travels in
/// the recorded [`Stmt::Return`], not in a cell). The [`EmitCf`]'s [`Cf::RetCell`]
/// associated type; a unit so `&cell` is a zero-cost ignored argument.
#[derive(Clone, Copy)]
pub struct RetCell;

/// The FLOAT RETURN-VALUE cell handle on the Emit backend — a ZST (the return value travels
/// in the recorded [`Stmt::Return`], not in a cell). The [`EmitCf`]'s [`Cf::RetCellF`]
/// associated type (Increment 4a). Distinct type from [`RetCell`] only so the float / uint
/// return facets stay separate at the call site; the recorded `Stmt::Return` is identical.
#[derive(Clone, Copy)]
pub struct RetCellF;

/// The BOOL RETURN-VALUE cell handle on the Emit backend — a ZST (the `true`/`false` travels
/// in the recorded [`Stmt::Return`] as a [`Node::BoolLit`], not in a cell). The [`EmitCf`]'s
/// [`Cf::RetCellB`] associated type (Increment 4b.2). Distinct type from [`RetCellF`] /
/// [`RetCell`] only so the bool / float / uint return facets stay separate at the call site.
#[derive(Clone, Copy)]
pub struct RetCellB;

thread_local! {
    /// The STMT block stack: the recorder pushes a [`Block`] on combinator entry
    /// (`unroll_for` / `if_`) and pops it into its parent on exit, so the top is always
    /// the block currently being recorded. The bottom (index 0) is the function body.
    static STMTS: RefCell<Vec<Block>> = const { RefCell::new(Vec::new()) };

    /// The per-emit mutable-local names (`exit`), indexed by [`Var`] / [`Node::VarRef`].
    static VARS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// The per-emit named-literal symbols (`BRICK_EXIT_EPS`), indexed by
    /// [`Node::NamedLit`]'s `sym_id`. Deduped so repeated `named_lit("BRICK_EXIT_EPS", _)`
    /// calls share one id.
    static NAMED_LITS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// The per-emit CALLEE names (`m2_cubic_eval`), indexed by [`Node::Call2`]'s `sym_id`
    /// (Increment 4a). Deduped so repeated `call2("m2_cubic_eval", ...)` calls share one id.
    static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// The monotone program-order temp counter (`t0`, `t1`, ...). Bumped by
    /// [`EmitCf::temp`] each time a temp is MATERIALIZED, so the `t{seq}` numbering follows
    /// recording (= print) order across the whole body, nested blocks included.
    static TEMP_SEQ: RefCell<u32> = const { RefCell::new(0) };
}

/// Pushes `stmt` into the block currently being recorded (the top of [`STMTS`]).
fn record_stmt(stmt: Stmt) {
    STMTS.with(|s| {
        s.borrow_mut()
            .last_mut()
            .expect("invariant: a block is on the STMTS stack while recording")
            .stmts
            .push(stmt);
    });
}

/// Records a mutable-local DECLARATION of an arbitrary [`EmitTy`]: seeds the `name` in the
/// [`VARS`] table (so [`Node::VarRef`] later spells `name`) and records the `Stmt::DeclVar`
/// with the given `ty` + `rhs`. Shared by [`EmitCf::decl_var`] (`EmitTy::Float`) and
/// [`EmitCf::decl_bool_var`] (`EmitTy::Bool`); a future `decl_uint_var`/`decl_int_var` is a
/// trivial mirror (pass its own `ty`). Threading the `ty` (vs the old hardcoded
/// `EmitTy::Float`) is the Increment-4d generalization — the `float` path is byte-unchanged
/// (it still passes `EmitTy::Float`).
fn record_decl_var(name: &'static str, ty: EmitTy, rhs: u32) -> Var {
    let var = VARS.with(|v| {
        let mut v = v.borrow_mut();
        let id = v.len() as u32;
        // The HLSL local's name (threaded from the body — `exit` in Increment 1; Inc 3+4d
        // declare more than one, each with its own name).
        v.push(name);
        Var(id)
    });
    record_stmt(Stmt::DeclVar { var, ty, rhs });
    var
}

/// Records a materialized temp: assigns the next program-order `seq`, registers the
/// temp's result [`EmitTy`] out-of-band (so [`type_of`]`(`[`Node::TempRef`]`(seq))` reads
/// it), records the `Stmt::DeclTemp` (named or anonymous), and returns a [`Node::TempRef`]
/// handle so later uses spell the temp's name/`t{seq}`. Shared by [`Cf::temp`] (anonymous
/// `float`) and the Increment-3 named-temp combinators ([`Cf::temp_vec3`] /
/// [`Cf::temp_uint`]).
fn record_temp(name: Option<&'static str>, ty: EmitTy, x: Emit) -> Emit {
    let rhs = x.0;
    let seq = TEMP_SEQ.with(|c| {
        let mut c = c.borrow_mut();
        let s = *c;
        *c += 1;
        s
    });
    // Register the temp's result type + name at index == seq (pushed in program order, so
    // the index IS the seq). `type_of`/`operand_str` of a `TempRef(seq)` read them.
    TEMP_TYPES.with(|t| {
        let mut t = t.borrow_mut();
        debug_assert_eq!(t.len() as u32, seq, "TEMP_TYPES must stay seq-indexed");
        t.push(ty);
    });
    TEMP_NAMES.with(|t| {
        let mut t = t.borrow_mut();
        debug_assert_eq!(t.len() as u32, seq, "TEMP_NAMES must stay seq-indexed");
        t.push(name);
    });
    record_stmt(Stmt::DeclTemp { seq, name, ty, rhs });
    Emit(push(Node::TempRef(seq)))
}

/// Registers a named-literal symbol, returning its (deduped) `sym_id`.
fn intern_named_lit(sym: &'static str) -> u32 {
    NAMED_LITS.with(|n| {
        let mut n = n.borrow_mut();
        if let Some(i) = n.iter().position(|&s| s == sym) {
            i as u32
        } else {
            let id = n.len() as u32;
            n.push(sym);
            id
        }
    })
}

/// Registers a callee name, returning its (deduped) `sym_id` (Increment 4a).
fn intern_call(sym: &'static str) -> u32 {
    CALLS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(i) = c.iter().position(|&s| s == sym) {
            i as u32
        } else {
            let id = c.len() as u32;
            c.push(sym);
            id
        }
    })
}

/// The control-flow EMIT backend — a unit ZST that RECORDS each combinator into the
/// STMT IR ([`STMTS`]) + the SSA arena ([`ARENA`]). The `Emit` value axis ([`Emit`] as
/// [`FieldScalar`]) supplies the arithmetic nodes; this supplies the control flow.
#[derive(Clone, Copy)]
pub struct EmitCf;

impl Cf for EmitCf {
    type Scalar = Emit;
    // On Emit the mutable local is a NAMED handle (a `u32` indexing the `VARS` name table).
    type Var = Var;
    // The induction variable is the iv SSA node handle (a `UintInput` printing `a`).
    type Iv = Emit;

    fn decl_var(name: &'static str, init: Emit) -> Var {
        // `init` is an `Emit` handle: read its arena id DIRECTLY (no transmute — `Scalar`
        // is `Emit` here, so `init.0` is a plain field access). A `float` decl — the `ty` is
        // threaded into the shared `record_decl_var` (the SAME `EmitTy::Float` it hardcoded
        // before — byte-unchanged).
        record_decl_var(name, EmitTy::Float, init.0)
    }

    // On Emit a `bool` local is the SAME named-handle shape `Var` uses (a `u32` indexing the
    // `VARS` name table); only the decl-site `ty` differs.
    type BoolVar = Var;

    fn decl_bool_var(name: &'static str, init: bool) -> Var {
        // The `false`/`true` init rhs is a `Node::BoolLit` (printed `false`/`true`, the SAME node
        // the proven bool-RETURN path uses). The `ty` is `EmitTy::Bool` → the printer spells
        // `bool <name> = <init>;` (the `bool` token via `ty_keyword`).
        record_decl_var(name, EmitTy::Bool, push(Node::BoolLit(init)))
    }

    fn get_var(v: &Var) -> Emit {
        // Read the running value: a `VarRef` node printing the variable's name (`exit`).
        Emit(push(Node::VarRef(v.0)))
    }

    fn set_var(v: &Var, val: Emit) {
        record_stmt(Stmt::Assign {
            var: *v,
            rhs: val.0,
        });
    }

    fn index(vec: [Emit; 3], iv: Emit) -> Emit {
        // The seeded `[Emit; 3]` carries the vec id in each element's `VecParamRef`.
        let vec_id = ARENA.with(|a| match a.borrow()[vec[0].0 as usize] {
            Node::VecParamRef(id) => id,
            other => unreachable!("index() expected a VecParamRef parameter, got {other:?}"),
        });
        Emit(push(Node::VecIndex(vec_id, iv.0)))
    }

    fn named_lit(sym: &'static str, val: f32) -> Emit {
        let sym_id = intern_named_lit(sym);
        Emit(push(Node::NamedLit { sym_id, val }))
    }

    fn temp(x: Emit) -> Emit {
        // An ANONYMOUS `float` temp (`float t{seq} = ...;`, the brick-exit materialization).
        record_temp(None, EmitTy::Float, x)
    }

    fn unroll_for<F: FnMut(Emit) -> Flow>(attr: &'static str, n: usize, mut body: F) {
        // The iv is a `uint` loop variable named `a` (the committed body's induction var).
        // Seeded as a `UintInput` so `VecIndex`'s operand prints `a`.
        let iv = Emit(push(Node::UintInput(0)));
        // Push the loop body block, record the body ONCE (the unroll is structural — DXC
        // unrolls it), then pop and wrap into a `Stmt::UnrollFor` in the parent.
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        // The body's `?` cannot early-return on Emit (every `if_` returns Fallthrough), so
        // the whole loop body is recorded; the `Flow` result is discarded.
        let _ = body(iv);
        let body_block = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the loop body block was pushed above")
        });
        record_stmt(Stmt::UnrollFor {
            attr,
            iv: "a",
            n,
            body: body_block,
        });
    }

    fn if_<F: FnOnce() -> Flow>(cond: EmitMask, body: F) -> Flow {
        // Record the THEN block (here a single `Continue`), wrap into `Stmt::If`, and
        // FALL THROUGH (return `Continue`) so the recorder keeps recording the live tail —
        // the `continue` is captured structurally inside the `Stmt::If`, not by control
        // flow. (Eval is the path where `?` actually skips the tail.)
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        let _ = body(); // records `Stmt::Continue` into the then-block (via `EmitCf::cont`)
        let then = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the then block was pushed above")
        });
        record_stmt(Stmt::If { cond: cond.0, then });
        Flow::Continue(())
    }

    fn cont() -> Flow {
        // SIDE EFFECT: record a `continue` into the current (then) block. The returned
        // `Break(Continue)` is the loop-continue token (consumed by Eval; ignored by
        // `if_`'s emit).
        record_stmt(Stmt::Continue);
        Flow::Break(LoopOp::Continue)
    }

    fn brk() -> Flow {
        // SIDE EFFECT: record a `break` into the current (then) block — mirrors `cont`. The
        // returned `Break(LoopOp::Break)` is ignored by `if_`'s emit (the break is captured
        // structurally inside the `Stmt::If`; the recorder keeps recording the live tail);
        // on Eval it is the real loop-break token `runtime_for` consumes.
        record_stmt(Stmt::Break);
        Flow::Break(LoopOp::Break)
    }

    // ---- Increment 3 typed facets (the brick-cell value model recorder) -------------
    // On Emit every value is an `Emit` SSA-node handle; the out-param / buffer / ret-cell
    // are NAME handles (the value travels in the recorded statement, not in a cell).
    type Uint = Emit;
    type Uint3 = Emit;
    type Vec3f = Emit;
    type OutVec3 = OutParam;
    type RetCell = RetCell;
    type Buf<'a> = BufParam;

    fn vec3_sub(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Vec3Sub(a.0, b.0)))
    }
    fn vec3_add(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Vec3Add(a.0, b.0)))
    }
    fn vec3_div_scalar(v: Emit, s: Emit) -> Emit {
        Emit(push(Node::Vec3DivScalar(v.0, s.0)))
    }
    fn vec3_mul_scalar(v: Emit, s: Emit) -> Emit {
        Emit(push(Node::Vec3MulScalar(v.0, s.0)))
    }
    fn vec3_from_uints(x: Emit, y: Emit, z: Emit) -> Emit {
        Emit(push(Node::Vec3FromUints(x.0, y.0, z.0)))
    }

    fn vec3_x(v: Emit) -> Emit {
        Emit(push(Node::Vec3Swizzle(v.0, 0)))
    }
    fn vec3_y(v: Emit) -> Emit {
        Emit(push(Node::Vec3Swizzle(v.0, 1)))
    }
    fn vec3_z(v: Emit) -> Emit {
        Emit(push(Node::Vec3Swizzle(v.0, 2)))
    }

    fn uint3_x(d: Emit) -> Emit {
        Emit(push(Node::Uint3Swizzle(d.0, 0)))
    }
    fn uint3_y(d: Emit) -> Emit {
        Emit(push(Node::Uint3Swizzle(d.0, 1)))
    }
    fn uint3_z(d: Emit) -> Emit {
        Emit(push(Node::Uint3Swizzle(d.0, 2)))
    }

    fn float_to_uint(f: Emit) -> Emit {
        Emit(push(Node::FloatToUint(f.0)))
    }

    fn uadd(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::UAdd(a.0, b.0)))
    }
    fn umul(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::UMul(a.0, b.0)))
    }

    fn named_uint(sym: &'static str, _val: u32) -> Emit {
        // A `uint` named constant interns into the SAME named-literal table the `float`
        // `named_lit` uses (the printer spells the SYMBOL); the `val` is Emit-irrelevant.
        // The node is `NamedLit`, but its CONSUMERS (the `ret` of `BRICK_OUTSIDE_GRID`)
        // never read `type_of` for an arithmetic check — a bare `return SYM;` types via the
        // return printer with no `chk`. To keep `type_of(NamedLit) == Float` from mis-typing
        // a `uint` consumer, this leaf's only use is a direct `ret` (no `chk`).
        let sym_id = intern_named_lit(sym);
        Emit(push(Node::NamedLit {
            sym_id,
            val: f32::NAN,
        }))
    }

    fn buffer_load(buf: BufParam, idx: Emit) -> Emit {
        Emit(push(Node::BufferLoad(buf.0, idx.0)))
    }

    fn uge(a: Emit, b: Emit) -> EmitMask {
        EmitMask(push(Node::UGe(a.0, b.0)))
    }

    fn or(a: EmitMask, b: EmitMask) -> EmitMask {
        EmitMask(push(Node::Or(a.0, b.0)))
    }

    // ---- Increment 4f: the B1 sor-retreat condition leaves (recorder) -----------------

    fn ugt(a: Emit, b: Emit) -> EmitMask {
        // A `uint` `>` mask (`it > 0u`) — the `uint` strict-`>` analogue of `uge`'s `UGe` node.
        EmitMask(push(Node::UGt(a.0, b.0)))
    }

    fn and2(a: EmitMask, b: EmitMask) -> EmitMask {
        // The logical `&&` mask — a `And2` node (textual `&&`), DISTINCT from the bitwise `uint`
        // `And` node. Mirrors `or`'s `Or` node (textual `||`); DXC lowers both to a short-circuit
        // `OpBranchConditional` chain.
        EmitMask(push(Node::And2(a.0, b.0)))
    }

    fn uint_lit(x: u32) -> Emit {
        // A bare `uint` literal (`0u`) — the `UintLit` node (printed `<x>u`, an inline leaf typed
        // `Uint`). DISTINCT from `named_uint` (which spells a SYMBOL via the named-lit table).
        Emit(push(Node::UintLit(x)))
    }

    fn temp_vec3(name: &'static str, v: Emit) -> Emit {
        // A NAMED `float3` temp (`float3 rel = ...;`).
        record_temp(Some(name), EmitTy::Float3, v)
    }
    fn temp_uint(name: &'static str, u: Emit) -> Emit {
        // A NAMED `uint` temp (`uint ix = ...;`).
        record_temp(Some(name), EmitTy::Uint, u)
    }

    fn out_vec3_assign(o: &OutParam, v: Emit) {
        // A bare `cell_min = <rhs>;` (NO decl — `cell_min` is an `out` parameter).
        record_stmt(Stmt::OutAssign {
            name_id: o.0,
            rhs: v.0,
        });
    }

    fn if_ret(_cell: &RetCell, cond: EmitMask, value: Emit) -> Flow {
        // Record `if (<cond>) { return <value>; }` — the then-block is EXACTLY ONE
        // `Stmt::Return` (no spurious assign; the deleted dual set_var+ret mechanism), then
        // FALL THROUGH (the recorder keeps recording the tail structurally). The `_cell` is
        // a ZST on Emit (the value travels in the `Stmt::Return`, not a cell).
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        record_stmt(Stmt::Return(value.0));
        let then = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the if_ret then block was pushed above")
        });
        record_stmt(Stmt::If {
            cond: cond.0,
            then,
        });
        Flow::Continue(())
    }

    fn ret(_cell: &RetCell, value: Emit) -> Flow {
        // The SOLE return mechanism — record a single `Stmt::Return(value)` into the current
        // block (the function body's tail `return grid[idx];`). Fall through on Emit.
        record_stmt(Stmt::Return(value.0));
        Flow::Continue(())
    }

    // ---- Increment 4a: the runtime `[loop]` + the FLOAT return facet (recorder) ------
    // On Emit every value is an `Emit` SSA-node handle; `c` is a `Vec4Param` node, the
    // ret-cell a ZST (the value travels in the recorded `Stmt::Return`).
    type RetCellF = RetCellF;
    type Vec4f = Emit;

    fn decl_param(name: &'static str, _init: Emit) -> Var {
        // SUPPRESSED-DECL: seed a `VARS` name entry (so get_var/set_var spell `hi`, `hi =
        // ...;`) but record NO `Stmt::DeclVar` — `lo`/`hi`/`f_lo`/`f_hi` are HLSL SIGNATURE
        // parameters, so a `float hi = ...;` redecl would diverge the committed text. `_init`
        // (the param's symbolic seed) is unused: a parameter is already bound by name.
        VARS.with(|v| {
            let mut v = v.borrow_mut();
            let id = v.len() as u32;
            v.push(name);
            Var(id)
        })
    }

    fn temp_float(name: &'static str, x: Emit) -> Emit {
        // A NAMED `float` temp (`float denom = ...;` / `float f_mid = ...;`).
        record_temp(Some(name), EmitTy::Float, x)
    }

    fn select(cond: EmitMask, t: Emit, e: Emit) -> Emit {
        // A `SelectParen` node — the printer wraps BOTH arms (the committed regula-falsi
        // ternary). DISTINCT from `FieldScalar::select`'s `Select` (the brick-exit's
        // un-wrapped clamp), so the brick-exit `.spv` is unperturbed.
        Emit(push(Node::SelectParen(cond.0, t.0, e.0)))
    }

    fn call2(fn_sym: &'static str, a: Emit, b: Emit) -> Emit {
        // `m2_cubic_eval(c, mid)` — a frozen-function call site. The callee name interns into
        // the per-emit `CALLS` table; `a`/`b` are the two argument node ids.
        let sym_id = intern_call(fn_sym);
        Emit(push(Node::Call2 {
            sym_id,
            a: a.0,
            b: b.0,
        }))
    }

    fn call1(fn_sym: &'static str, a: Emit) -> Emit {
        // `field_distance(p + L * t)` — a frozen single-`float3`-arg call site (Inc 4b). The
        // callee name interns into the SAME per-emit `CALLS` table `call2` uses; `a` is the
        // single `float3` argument node id.
        let sym_id = intern_call(fn_sym);
        Emit(push(Node::Call1 { sym_id, a: a.0 }))
    }

    fn runtime_for<F: FnMut(Emit) -> Flow>(
        attr: &'static str,
        iv: &'static str,
        bound_sym: &'static str,
        _bound_val: usize,
        mut body: F,
    ) -> Flow {
        // The iv is a `uint` loop variable named `iv` (threaded single-source — no hardcoded
        // "a"). Seeded as a `UintInput` carrying the iv name so any body `vec[iv]` would
        // print `i`; `m2_regula_falsi` does not index by `i`, but the single-source discipline
        // is pinned for Inc 4c (pick_material_id references `i`).
        let iv_node = Emit(push(Node::UintInput(0)));
        // Push the loop body block, record the body ONCE (the `?` never early-returns on Emit
        // — every guard records structurally), then pop and wrap into a `Stmt::Loop`.
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        let _ = body(iv_node);
        let body_block = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the loop body block was pushed above")
        });
        record_stmt(Stmt::Loop {
            attr,
            iv,
            bound_sym,
            body: body_block,
        });
        // ALWAYS fall through on Emit (the body was recorded once; the function tail's
        // `ret_f` is recorded after this returns).
        Flow::Continue(())
    }

    fn if_else<T: FnOnce() -> Flow, E: FnOnce() -> Flow>(cond: EmitMask, then: T, els: E) -> Flow {
        // Record the THEN block, then the ELSE block (each a push/record/pop), wrap into a
        // `Stmt::IfElse`, and FALL THROUGH so the recorder keeps recording the live tail.
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        let _ = then();
        let then_block = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the if_else then block was pushed above")
        });
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        let _ = els();
        let els_block = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the if_else else block was pushed above")
        });
        record_stmt(Stmt::IfElse {
            cond: cond.0,
            then: then_block,
            els: els_block,
        });
        Flow::Continue(())
    }

    fn if_ret_f(_cell: &RetCellF, cond: EmitMask, value: Emit) -> Flow {
        // `if (<cond>) { return <value>; }` — the then-block is EXACTLY ONE `Stmt::Return`
        // (the float early-return guard; identical recorded shape to `if_ret`'s uint guard).
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        record_stmt(Stmt::Return(value.0));
        let then = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the if_ret_f then block was pushed above")
        });
        record_stmt(Stmt::If {
            cond: cond.0,
            then,
        });
        Flow::Continue(())
    }

    fn ret_f(_cell: &RetCellF, value: Emit) -> Flow {
        // The float return — a single `Stmt::Return(value)` (the tail `return mid;`).
        record_stmt(Stmt::Return(value.0));
        Flow::Continue(())
    }

    // ---- Increment 4b.2: the BOOL return + OUT-FLOAT facets (recorder) ---------------
    // On Emit the ret-cell is a ZST (the `true`/`false` travels in the `Stmt::Return` as a
    // `BoolLit`); the out-float is a NAME handle (the value travels in the `Stmt::OutAssign`).
    type RetCellB = RetCellB;
    type OutFloat = OutFloatParam;

    fn ret_b(_cell: &RetCellB, value: bool) -> Flow {
        // The bool return — a single `Stmt::Return` carrying a `BoolLit` (printed `true`/
        // `false`, NOT a `uint`). The function-tail `return false;`. Fall through on Emit.
        record_stmt(Stmt::Return(push(Node::BoolLit(value))));
        Flow::Continue(())
    }

    fn out_float_assign(o: &OutFloatParam, v: Emit) {
        // A bare `hit_t = <rhs>;` (NO decl — `hit_t` is an `out` parameter). Records into the
        // SAME `Stmt::OutAssign` (indexing `out_in`) the brick-cell's `cell_min` uses.
        record_stmt(Stmt::OutAssign {
            name_id: o.0,
            rhs: v.0,
        });
    }

    fn if_hit_ret_b(
        hit_out: &OutFloatParam,
        _ret_out: &RetCellB,
        cond: EmitMask,
        rt_val: Emit,
    ) -> Flow {
        // Record `if (<cond>) { hit_t = <rt>; return true; }` — the then-block carries BOTH
        // statements IN ORDER (the out-float assign THEN the bool `return true;`), NOT the
        // single-statement `if_ret_f`. Then FALL THROUGH (the recorder keeps recording the live
        // tail structurally — the `?` never early-returns on Emit). The two committed statements
        // print exactly as the committed `hit_t = rt; return true;`.
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        record_stmt(Stmt::OutAssign {
            name_id: hit_out.0,
            rhs: rt_val.0,
        });
        record_stmt(Stmt::Return(push(Node::BoolLit(true))));
        let then = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the if_hit_ret_b then block was pushed above")
        });
        record_stmt(Stmt::If {
            cond: cond.0,
            then,
        });
        Flow::Continue(())
    }

    // ---- Increment 4e: the BOOL mutable-local facets (recorder) -----------------------

    fn decl_bool_param(name: &'static str, _init: bool) -> Var {
        // SUPPRESSED-DECL (bool): seed a `VARS` name entry (so set_bool_var/get_bool_var spell
        // `hit`, `hit = ...;`) but record NO `Stmt::DeclVar` — `hit` is declared by the
        // hand-written re-march preamble (`hit = false;`), so a `bool hit = false;` redecl would
        // diverge the committed text. The bool mirror of `decl_param` (the `float` suppressed
        // decl); `_init` is unused (a suppressed local is already bound by name).
        VARS.with(|v| {
            let mut v = v.borrow_mut();
            let id = v.len() as u32;
            v.push(name);
            Var(id)
        })
    }

    fn get_bool_var(_v: &Var) -> bool {
        // The generated span never EMITS a read of `hit` (no `Stmt`/`Node` references the flag's
        // VALUE — the span mutates `hit` by NAME via `set_bool_var`); the body's tail constructs a
        // `(hit, t)` tuple ONLY for the Eval oracle's result, and the Emit PRODUCER discards that
        // tuple. So this records NO statement and pushes NO node — it returns a placeholder `false`
        // that is byte-neutral (the SSA arena / STMT IR are untouched). The value is irrelevant
        // (discarded by the producer); a panic here would be wrong because the tuple IS constructed
        // on both backends (unlike `call1`, which the producer routes around with a closure).
        false
    }

    fn set_bool_var(v: &Var, val: bool) {
        // `hit = <val>;` — a `Stmt::Assign` whose rhs is a `Node::BoolLit` (printed `true`/
        // `false`, the SAME node the proven bool-return path uses). Reuses the shipped
        // `Stmt::Assign` printer (the `float` `set_var` path); the only delta is the bool-literal
        // rhs.
        record_stmt(Stmt::Assign {
            var: *v,
            rhs: push(Node::BoolLit(val)),
        });
    }
}

// ---- The STMT printer + the brick-exit generator -------------------------------

/// The INLINE HLSL spelling of expression `expr` for a STATEMENT rhs / condition. A leaf
/// (incl. a `TempRef`/`VarRef`/`VecIndex`/`NamedLit`) spells via [`operand_str`]; a
/// non-leaf (an arithmetic / comparison / select node) spells via [`define_str`] (its
/// operands recurse through the same inline rule). Because EVERY materialized
/// subexpression was wrapped in [`Cf::temp`] (→ a `TempRef` leaf), an `expr` here bottoms
/// out at temps/inputs/literals — no recursive temp emission is needed (temps are
/// EXPLICIT [`Stmt::DeclTemp`]s recorded in program order, not auto-hoisted).
fn inline_expr(arena: &[Node], names: Names, temps: &[Option<String>], expr: u32) -> String {
    let node = arena[expr as usize];
    if is_inline_leaf(node) {
        operand_str(arena, names, temps, expr, OperandPos::Root)
    } else {
        // The statement rhs / condition is a ROOT-position expression (no enclosing infix
        // op), so an additive top-level node needs no wrap.
        define_str(arena, names, temps, expr)
    }
}

/// Prints a [`Block`]'s statements at indent `depth`. Temps are EXPLICIT
/// (`Stmt::DeclTemp`, recorded in program order by [`Cf::temp`]), so this is a flat
/// in-order walk — no dominance analysis. `temps` maps a temp's [`Node::TempRef`] seq to
/// its `t{seq}` name (filled as `DeclTemp`s are printed).
fn print_block(block: &Block, arena: &[Node], names: Names, depth: usize, out: &mut String) {
    let pad = "    ".repeat(depth);
    for stmt in &block.stmts {
        match stmt {
            Stmt::DeclVar { var, ty, rhs } => {
                let rhs_s = inline_expr(arena, names, &[], *rhs);
                out.push_str(&format!(
                    "{pad}{} {} = {};\n",
                    ty_keyword(*ty),
                    names.vars[var.0 as usize],
                    rhs_s
                ));
            }
            Stmt::DeclTemp {
                seq,
                name,
                ty,
                rhs,
            } => {
                let rhs_s = inline_expr(arena, names, &[], *rhs);
                // A NAMED temp (`float3 rel = ...;`, the brick-cell locals) prints its name;
                // an ANONYMOUS temp (`float t0 = ...;`, the brick-exit) prints `t{seq}`.
                match name {
                    Some(n) => out.push_str(&format!("{pad}{} {n} = {};\n", ty_keyword(*ty), rhs_s)),
                    None => {
                        out.push_str(&format!("{pad}{} t{seq} = {};\n", ty_keyword(*ty), rhs_s))
                    }
                }
            }
            Stmt::Assign { var, rhs } => {
                let rhs_s = inline_expr(arena, names, &[], *rhs);
                out.push_str(&format!("{pad}{} = {};\n", names.vars[var.0 as usize], rhs_s));
            }
            Stmt::OutAssign { name_id, rhs } => {
                // A BARE out-parameter assignment (`cell_min = <rhs>;`) — NO `float3` decl.
                let rhs_s = inline_expr(arena, names, &[], *rhs);
                out.push_str(&format!(
                    "{pad}{} = {};\n",
                    names.out_in[*name_id as usize],
                    rhs_s
                ));
            }
            Stmt::UnrollFor { attr, iv, n, body } => {
                out.push_str(&format!("{pad}{attr}\n"));
                out.push_str(&format!("{pad}for (uint {iv} = 0u; {iv} < {n}u; ++{iv}) {{\n"));
                print_block(body, arena, names, depth + 1, out);
                out.push_str(&format!("{pad}}}\n"));
            }
            Stmt::If { cond, then } => {
                let cond_s = inline_expr(arena, names, &[], *cond);
                out.push_str(&format!("{pad}if ({cond_s}) {{\n"));
                print_block(then, arena, names, depth + 1, out);
                out.push_str(&format!("{pad}}}\n"));
            }
            Stmt::IfElse { cond, then, els } => {
                let cond_s = inline_expr(arena, names, &[], *cond);
                out.push_str(&format!("{pad}if ({cond_s}) {{\n"));
                print_block(then, arena, names, depth + 1, out);
                out.push_str(&format!("{pad}}} else {{\n"));
                print_block(els, arena, names, depth + 1, out);
                out.push_str(&format!("{pad}}}\n"));
            }
            Stmt::Loop {
                attr,
                iv,
                bound_sym,
                body,
            } => {
                // The header spells the BOUND SYMBOL (`M2_MARMITT_ITERS`), NOT a `<n>u`
                // literal — the difference (with `attr = "[loop]"`) that makes DXC emit a
                // genuine OpLoop.
                out.push_str(&format!("{pad}{attr}\n"));
                out.push_str(&format!(
                    "{pad}for (uint {iv} = 0u; {iv} < {bound_sym}; ++{iv}) {{\n"
                ));
                print_block(body, arena, names, depth + 1, out);
                out.push_str(&format!("{pad}}}\n"));
            }
            Stmt::Continue => out.push_str(&format!("{pad}continue;\n")),
            Stmt::Break => out.push_str(&format!("{pad}break;\n")),
            Stmt::Return(expr) => {
                let expr_s = inline_expr(arena, names, &[], *expr);
                out.push_str(&format!("{pad}return {expr_s};\n"));
            }
        }
    }
}

/// Generates the HLSL `dist_to_brick_exit` body — the empty-skip slab marcher with the
/// `[unroll]` loop + the data-dependent `continue` — by tracing the generic
/// [`crate::brick::dist_to_brick_exit_body`] over the `EmitCf` backend (whose
/// `Cf::Scalar = Emit` supplies the SSA-node arithmetic), and returns the full
/// `float dist_to_brick_exit(float3 p, float3 rd, float3 cell_min, float bw) { ... }`.
///
/// The body is spliced between the `// === GENERATED dist_to_brick_exit BEGIN/END ===`
/// sentinels in `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`. The
/// `sdf_field_edsl_sync` test pins the committed shader to this output; the cmp-`.spv`
/// gate proves it re-DXCs byte-identical to the committed `.comp.spv`.
pub fn emit_hlsl_dist_to_brick_exit() -> String {
    use crate::brick;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // The three `float3` parameters, each seeded as a `VecParamRef` so `index(p, a)`
    // recovers the parameter id and prints `p[a]` / `rd[a]` / `cell_min[a]`. `bw` is a
    // scalar `float` input (named `bw`).
    let p_id = push(Node::VecParamRef(0)); // vec_in[0] = "p"
    let rd_id = push(Node::VecParamRef(1)); // vec_in[1] = "rd"
    let cm_id = push(Node::VecParamRef(2)); // vec_in[2] = "cell_min"
    let p = [Emit(p_id), Emit(p_id), Emit(p_id)];
    let rd = [Emit(rd_id), Emit(rd_id), Emit(rd_id)];
    let cell_min = [Emit(cm_id), Emit(cm_id), Emit(cm_id)];
    let bw = Emit::input(0); // float_in[0] = "bw"

    let result = brick::dist_to_brick_exit_body::<EmitCf>(p, rd, cell_min, bw);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["bw"];
    let vec_in = ["p", "rd", "cell_min"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: &["a"],
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut body = String::new();
        // The statements (DeclVar exit + the UnrollFor with its explicit DeclTemps).
        print_block(&body_block, &arena, names, 1, &mut body);
        // The final return — `(exit < BRICK_EXIT_EPS) ? BRICK_EXIT_EPS : exit;`. The root
        // SELECT is spelled INLINE (matching the committed ternary); its cond/then/else
        // are leaves (the `Lt` comparison, the `BRICK_EXIT_EPS` named lit, the `exit` var).
        let ret = inline_expr(&arena, names, &[], result.0);
        body.push_str(&format!("    return {ret};\n"));

        format!(
            "float dist_to_brick_exit(float3 p, float3 rd, float3 cell_min, float bw) {{\n{body}}}\n"
        )
    })
}

/// Generates the HLSL `brick_cell_class` body — the pointer-grid cell lookup with the two
/// EARLY-RETURN guards (negative-rel + bounds), the `uint` index math, the
/// `StructuredBuffer<uint>` load, and the two `out float3 cell_min` writes — by tracing
/// the generic [`crate::brick::brick_cell_class_body`] over the `EmitCf` backend, and
/// returns the full `uint brick_cell_class(StructuredBuffer<uint> grid, float3 origin,
/// float bw, uint3 dims, float3 p, out float3 cell_min) { ... }`.
///
/// The body is spliced between the `// === GENERATED brick_cell_class BEGIN/END ===`
/// sentinels in `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl` (the function
/// DEFINITION only — the three call sites are UNTOUCHED). The `sdf_field_edsl_sync` test
/// pins the committed shader to this output; the one-shot cmp-`.spv` proves it re-DXCs
/// byte-identical to the committed `.comp.spv`. The host `host_brick_cell` (in
/// `boyko_rhi_vulkan::compute`) stays hand-written (firewall option B).
pub fn emit_hlsl_brick_cell_class() -> String {
    use crate::brick;

    // Fresh recorder state (incl. the Increment-3 TEMP_TYPES out-of-band table).
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the parameters as their typed marker nodes / name handles:
    //   grid     → BufParam(0)   (buf_in[0]   = "grid")
    //   origin   → Vec3Param(0)  (vec_in[0]   = "origin")
    //   bw       → Input(0)      (float_in[0] = "bw")
    //   dims     → Uint3Param(0) (uint3_in[0] = "dims")
    //   p        → Vec3Param(1)  (vec_in[1]   = "p")
    //   cell_min → OutParam(0)   (out_in[0]   = "cell_min")
    let grid = BufParam(0);
    let origin = Emit(push(Node::Vec3Param(0)));
    let bw = Emit::input(0);
    let dims = Emit(push(Node::Uint3Param(0)));
    let p = Emit(push(Node::Vec3Param(1)));
    let cell_min = OutParam(0);
    let cls = RetCell;

    brick::brick_cell_class_body::<EmitCf>(grid, origin, bw, dims, p, &cell_min, &cls);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["bw"];
    let vec_in = ["origin", "p"];
    let uint3_in = ["dims"];
    let buf_in = ["grid"];
    let out_in = ["cell_min"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &vec_in,
        uint3_in: &uint3_in,
        buf_in: &buf_in,
        out_in: &out_in,
        named_lit: &named_lit,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut body = String::new();
        // The whole body is recorded statements (DeclTemp rel, OutAssign cell_min, the two
        // guard `If { Return }`s, the `uint` temps, the conditional OutAssign, the buffer
        // load Return) — a flat in-order walk; the early returns are structural `Stmt::If`s.
        print_block(&body_block, &arena, names, 1, &mut body);

        // The signature is two lines: the continuation is indented by 22 spaces (aligned
        // under the first parameter, matching the committed `sdf_gbuffer_composite.hlsl`).
        const SIG_INDENT: &str = "                      "; // 22 spaces
        format!(
            "uint brick_cell_class(StructuredBuffer<uint> grid, float3 origin, float bw, uint3 dims,\n\
             {SIG_INDENT}float3 p, out float3 cell_min) {{\n{body}}}\n"
        )
    })
}

/// Generates the HLSL `m2_regula_falsi` body — the regula-falsi root refinement with a
/// RUNTIME `[loop]` (the smallest genuine `OpLoop`), five loop-carried Phi vars, an in-loop
/// early return, and a `m2_cubic_eval(c, mid)` call site — by tracing the generic
/// [`crate::brick::m2_regula_falsi_body`] over the `EmitCf` backend (whose `Cf::Scalar =
/// Emit` supplies the SSA-node arithmetic), and returns the full `float m2_regula_falsi(
/// float4 c, float lo, float hi, float f_lo, float f_hi) { ... }`.
///
/// The four carried params {lo, hi, f_lo, f_hi} are SUPPRESSED-DECL signature parameters
/// (seeded via [`Cf::decl_param`], no `float lo = ...;` redecl); `mid` is a TRUE local
/// (`float mid = lo;`). The `[loop]` header spells the BOUND SYMBOL `M2_MARMITT_ITERS`, NOT
/// `8u` (the difference that makes DXC emit an `OpLoop`). The cubic call is recorded as a
/// `m2_cubic_eval(c, mid)` call site — the leaf body is generated separately
/// ([`emit_hlsl_cubic_eval`]); this spells only the call.
///
/// The body is spliced between the `// === GENERATED m2_regula_falsi BEGIN/END ===`
/// sentinels in `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl` (the function
/// DEFINITION only — the two call sites in `m2_marmitt_root` are UNTOUCHED). The
/// `m2_regula_falsi_matches_edsl_emit` test pins the committed shader to this output; the
/// cmp-`.spv` proves it re-DXCs byte-identical to the committed `.comp.spv`.
pub fn emit_hlsl_m2_regula_falsi() -> String {
    use crate::brick;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the parameters as their typed marker nodes / name handles:
    //   c    → Vec4Param(0) (vec4_in[0]  = "c")    — the cubic coefficients (call-through)
    //   lo   → Input(0)     (float_in[0] = "lo")
    //   hi   → Input(1)     (float_in[1] = "hi")
    //   f_lo → Input(2)     (float_in[2] = "f_lo")
    //   f_hi → Input(3)     (float_in[3] = "f_hi")
    // The four scalar params are seeded as `Input` nodes ONLY for the `decl_param`'s `init`
    // (which is unused on Emit — a suppressed-decl param is bound by name, not by an init
    // expression); the VARS entry the body's get/set resolves is created by `decl_param`.
    let c = Emit(push(Node::Vec4Param(0)));
    let lo = Emit::input(0);
    let hi = Emit::input(1);
    let f_lo = Emit::input(2);
    let f_hi = Emit::input(3);
    let out = RetCellF;

    // The cubic-eval seam: on Emit it records a `m2_cubic_eval(c, mid)` call node.
    brick::m2_regula_falsi_body::<EmitCf, _>(
        c,
        lo,
        hi,
        f_lo,
        f_hi,
        |c, mid| EmitCf::call2("m2_cubic_eval", c, mid),
        &out,
    );

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["lo", "hi", "f_lo", "f_hi"];
    let vec4_in = ["c"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let call_in = CALLS.with(|c| c.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: &vec4_in,
        call_in: &call_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut body = String::new();
        // The whole body is recorded statements (DeclVar mid, the runtime `Stmt::Loop` with
        // its DeclTemp denom/f_mid, the SelectParen assign, the early `If { Return }`, the
        // `Stmt::IfElse` bracket update) + the tail `Stmt::Return(mid)` — a flat in-order
        // walk; the early return is a structural `Stmt::If`.
        print_block(&body_block, &arena, names, 1, &mut body);
        format!(
            "float m2_regula_falsi(float4 c, float lo, float hi, float f_lo, float f_hi) {{\n{body}}}\n"
        )
    })
}

/// Generates the HLSL `sdf_soft_shadow` LOOP+TAIL SPAN — the first marcher `[loop]` (the
/// fixed-budget penumbra-min cone-trace), with the `field_distance(p + L * t)` call site, the
/// in-loop occluder-hit early return, and the `t > T_MAX` BREAK — by tracing the generic
/// [`crate::shadow::sdf_soft_shadow_body`] over the `EmitCf` backend (whose `Cf::Scalar =
/// Emit` supplies the SSA-node arithmetic), and returns ONLY the span
/// (`    float res = 1.0;\n    float t = SHADOW_MINT;\n    [loop]\n    for ... { ... }\n
/// return clamp(res, 0.0, 1.0);\n`) — NOT a wrapped function.
///
/// Distinct from [`emit_hlsl_m2_regula_falsi`] (a WHOLE function) in returning a SPAN only:
/// the `dot(n, L)` early-return PREAMBLE stays HAND-WRITTEN inline above the
/// `// === GENERATED sdf_soft_shadow BEGIN ===` sentinel (framing (b)), so the generator emits
/// only the statements spliced BETWEEN the sentinels inside `sdf_soft_shadow`. The span is
/// printed at depth 1 (4-space indent), matching the committed L454-468.
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the
/// `sdf_soft_shadow` text-sync test pins the committed shader span to this output.
pub fn emit_hlsl_sdf_soft_shadow() -> String {
    use crate::shadow;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the three `float3` parameters as `Vec3Param` markers (the whole-`float3` value
    // model `vec3_add`/`vec3_mul_scalar` operate on):
    //   p → Vec3Param(0) (vec_in[0] = "p")
    //   n → Vec3Param(1) (vec_in[1] = "n")  — consumed only by the hand-written preamble
    //                                          (UNUSED in the generated span)
    //   L → Vec3Param(2) (vec_in[2] = "L")
    let p = Emit(push(Node::Vec3Param(0)));
    let n = Emit(push(Node::Vec3Param(1)));
    let l = Emit(push(Node::Vec3Param(2)));
    let out = RetCellF;

    // The field-distance seam: on Emit it records a `field_distance(p + L * t)` call node.
    shadow::sdf_soft_shadow_body::<EmitCf, _>(p, n, l, |q| EmitCf::call1("field_distance", q), &out);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    // No scalar `float` parameters — `p`/`n`/`L` are all `float3` (vec_in).
    let float_in: [&str; 0] = [];
    let vec_in = ["p", "n", "L"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let call_in = CALLS.with(|c| c.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: &call_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is recorded statements (DeclVar res/t, the runtime `Stmt::Loop` with
        // its DeclTemp d, the min `Stmt::Assign res`, the in-loop occluder-hit `If { Return }`,
        // the step `Stmt::Assign t`, the `t > T_MAX` `If { Break }`) + the tail
        // `Stmt::Return(clamp01)` — a flat in-order walk at depth 1 (4-space indent), matching
        // the committed L454-468. NO function-signature wrap (the span is spliced inside the
        // hand-written `sdf_soft_shadow`).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the HLSL `m2_surface_hit` REFINE LOOP+TAIL SPAN — the production brick-marcher's
/// analytic-residual signed refine (the fixed-budget `[loop]` sphere-trace from the cubic
/// candidate onto the EXACT field), with the `field_distance(ro + rd * rt)` call site, the
/// in-loop converged-hit (`hit_t = rt; return true;`), the `rt < 0 || rt > T_MAX` BREAK, and
/// the function-tail `return false;` — by tracing the generic
/// [`crate::surface::m2_surface_hit_refine_body`] over the `EmitCf` backend (whose `Cf::Scalar =
/// Emit` supplies the SSA-node arithmetic), and returns ONLY the span (NOT a wrapped function).
///
/// Distinct from [`emit_hlsl_m2_regula_falsi`] (a WHOLE function) in returning a SPAN only: the
/// integer cell-addressing PREAMBLE (the 7-param header, the entry `hit_t = t_world;` default,
/// the field-unpacks, the rel/tile float-guard early returns, the M5 toroidal-slot integer math,
/// and the `m2_brick_span` / `m2_brick_cubic_hit` / `select_level` call sites) stays HAND-WRITTEN
/// inline above/around the `// === GENERATED m2_surface_hit_refine BEGIN/END ===` sentinels
/// (framing (b)), so the generator emits only the statements spliced BETWEEN the sentinels inside
/// `m2_surface_hit`. The span is printed at depth 1 (4-space indent), matching the committed
/// L1184-1205.
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the
/// `m2_surface_hit_refine` text-sync test pins the committed shader span to this output.
pub fn emit_hlsl_m2_surface_hit_refine() -> String {
    use crate::surface;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's inputs:
    //   ro     → Vec3Param(0) (vec_in[0]   = "ro")    — the world ray origin
    //   rd     → Vec3Param(1) (vec_in[1]   = "rd")    — the world ray direction
    //   cand_t → Input(0)     (float_in[0] = "cand_t")— the cubic candidate world t
    //   hit_t  → OutFloatParam(0) (out_in[0] = "hit_t") — the `out float` written by the in-loop hit
    // `ro`/`rd`/`cand_t` are the ONLY values the generated span sees (the integer cell-addressing
    // preamble — `lvl`/`atlas`/`atlas_smp`/`t_world` + the slot math — stays hand-written above).
    let ro = Emit(push(Node::Vec3Param(0)));
    let rd = Emit(push(Node::Vec3Param(1)));
    let cand_t = Emit::input(0);
    let hit_out = OutFloatParam(0);
    let ret_out = RetCellB;

    // The field-distance seam: on Emit it records a `field_distance(ro + rd * rt)` call node.
    surface::m2_surface_hit_refine_body::<EmitCf, _>(
        ro,
        rd,
        cand_t,
        |q| EmitCf::call1("field_distance", q),
        &hit_out,
        &ret_out,
    );

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["cand_t"];
    let vec_in = ["ro", "rd"];
    let out_in = ["hit_t"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let call_in = CALLS.with(|c| c.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: &out_in,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: &call_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is recorded statements (DeclVar rt, the runtime `Stmt::Loop` with its
        // DeclTemp d, the composite converged-hit `If { OutAssign hit_t; Return true }`, the
        // DeclTemp step, the `Stmt::Assign rt`, the `rt < 0 || rt > T_MAX` `If { Break }`) + the
        // tail `Stmt::Return(false)` — a flat in-order walk at depth 1 (4-space indent), matching
        // the committed L1184-1205. NO function-signature wrap (the span is spliced inside the
        // hand-written `m2_surface_hit`).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the HLSL B1 over-relaxation ACCEPT-REFINE LOOP SPAN — the production main-marcher's
/// settle-onto-surface refine (the fixed-budget `[loop]` SIGNED sphere-trace that corrects an
/// over-relaxed accept off the surface), with the `sdf(ro + rd * t)` call site, the
/// `abs(rd_) < EPS` BREAK, and the `t = t + step` accumulation — by tracing the generic
/// [`crate::refine::b1_accept_refine_body`] over the `EmitCf` backend (whose `Cf::Scalar = Emit`
/// supplies the SSA-node arithmetic), and returns ONLY the span (NOT a wrapped function).
///
/// A near-CLONE of [`emit_hlsl_m2_surface_hit_refine`], STRICTLY SIMPLER: there is NO return
/// facet (no `out float hit_t`, no bool return, no composite converged-hit `if_hit_ret_b`), so
/// the only seeded inputs are `ro`/`rd` (`Vec3Param`) and `t_seed` (a scalar `float` input). The
/// four producer deltas from the `m2_surface_hit_refine` template: the field seam interns `"sdf"`
/// (this site folds the ANALYTIC field via the hand-written `sdf`, NOT `field_distance`); the
/// float input is named `"t_seed"`; there are no out/ret cells; and the span prints at DEPTH 3
/// (the committed site nests main→`for (it)`→`if (d < EPS)`→this refine loop), matching the
/// committed L1442-1452 indentation (12-space `[loop]`).
///
/// The integer/over-relaxation marcher PREAMBLE (the `hit = true; exhausted = false;` accept
/// block, the rationale comment, and the outer `break;` that exits the MARCHER loop) stays
/// HAND-WRITTEN inline above/around the `// === GENERATED b1_accept_refine BEGIN/END ===`
/// sentinels (framing (b)), so the generator emits only the statements spliced BETWEEN the
/// sentinels.
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the `b1_accept_refine`
/// text-sync test pins the committed shader span to this output.
pub fn emit_hlsl_b1_accept_refine() -> String {
    use crate::refine;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's inputs:
    //   ro     → Vec3Param(0) (vec_in[0]   = "ro")     — the world ray origin
    //   rd     → Vec3Param(1) (vec_in[1]   = "rd")     — the world ray direction
    //   t_seed → Input(0)     (float_in[0] = "t_seed") — the carried candidate world t
    // `ro`/`rd`/`t_seed` are the ONLY values the generated span sees (the over-relaxation marcher
    // preamble stays hand-written above; `t` is the enclosing carried var, a suppressed-decl).
    let ro = Emit(push(Node::Vec3Param(0)));
    let rd = Emit(push(Node::Vec3Param(1)));
    let t_seed = Emit::input(0);

    // The field-distance seam: on Emit it records a `sdf(ro + rd * t)` call node (the ANALYTIC
    // field via the hand-written `sdf`, interned `"sdf"` — NOT `field_distance`).
    let _ = refine::b1_accept_refine_body::<EmitCf, _>(ro, rd, t_seed, |q| EmitCf::call1("sdf", q));

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["t_seed"];
    let vec_in = ["ro", "rd"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let call_in = CALLS.with(|c| c.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: &call_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is ONE recorded statement (the runtime `Stmt::Loop` with its DeclTemp
        // rd_, the `abs(rd_) < EPS` `If { Break }`, the DeclTemp step, and the `Stmt::Assign t`) —
        // a single in-order walk at DEPTH 3 (12-space indent), matching the committed L1442-1452.
        // The site nests main→`for (it)`→`if (d < EPS)`→this loop, hence depth 3 (vs the depth-1
        // span of `m2_surface_hit_refine`). NO function-signature wrap (the span is spliced inside
        // the hand-written B1 marcher).
        print_block(&body_block, &arena, names, 3, &mut span);
        span
    })
}

/// Generates the HLSL B1 EXHAUSTION RE-MARCH inner-loop SPAN — the production main-marcher's
/// BUG-B1-HOLE-3 budget-exhaustion recovery (the plain `omega == 1.0` sphere-trace from the
/// ORIGINAL seed, run when the over-relaxed fast pass exhausted `MAX_IT` mid-field), with the
/// `sdf(p)` call site, the `t >= t_mesh` mesh-guard BREAK, the `d < EPS` accept (`hit = true;
/// break;`), the `t > T_MAX` miss BREAK, and the `t = t + d` plain step — by tracing the generic
/// [`crate::remarch::b1_exhaustion_remarch_body`] over the `EmitCf` backend (whose `Cf::Scalar =
/// Emit` supplies the SSA-node arithmetic), and returns ONLY the span (NOT a wrapped function).
///
/// A near-CLONE of [`emit_hlsl_b1_accept_refine`] (Inc 4c) with the 4 Increment-4e facets: the
/// FLOAT mesh guard `t >= t_mesh` (`FieldScalar::ge`); a NAMED `float3 p` temp (`Cf::temp_vec3`,
/// vs the inline `ro + rd * t` of `b1_accept_refine`); and the in-loop `hit = true;`
/// (`Cf::set_bool_var`) carried by a SUPPRESSED-DECL bool (`Cf::decl_bool_param`/`Cf::get_bool_var`).
/// The seeded inputs are `ro`/`rd` (`Vec3Param`), `t_seed`/`t_mesh` (scalar `float` inputs); the
/// field seam interns `"sdf"` (the ANALYTIC field, NOT `field_distance`); there are no out/ret cells.
/// The span prints at DEPTH 2 (the committed site nests main→`if (exhausted)`→this re-march loop),
/// matching the committed L1520-1535 indentation (8-space `[loop]`).
///
/// The `if (exhausted) { ... }` WRAPPER (the `t = t_seed;` re-seed, the `hit = false;` reset, the
/// BUG-B1-HOLE-3 rationale comment, and the closing brace) stays HAND-WRITTEN inline above/around
/// the `// === GENERATED b1_exhaustion_remarch BEGIN/END ===` sentinels (framing (b)), so the
/// generator emits only the statements spliced BETWEEN the sentinels.
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the `b1_exhaustion_remarch`
/// text-sync test pins the committed shader span to this output.
pub fn emit_hlsl_b1_exhaustion_remarch() -> String {
    use crate::remarch;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's inputs:
    //   ro     → Vec3Param(0) (vec_in[0]   = "ro")     — the world ray origin
    //   rd     → Vec3Param(1) (vec_in[1]   = "rd")     — the world ray direction
    //   t_seed → Input(0)     (float_in[0] = "t_seed") — the original seed the fast pass used
    //   t_mesh → Input(1)     (float_in[1] = "t_mesh") — the mesh-occlusion depth bound
    // `ro`/`rd`/`t_seed`/`t_mesh` are the ONLY values the generated span sees (the `if (exhausted)`
    // re-seed wrapper stays hand-written above; `t`/`hit` are the enclosing carried vars, both
    // suppressed-decl).
    let ro = Emit(push(Node::Vec3Param(0)));
    let rd = Emit(push(Node::Vec3Param(1)));
    let t_seed = Emit::input(0);
    let t_mesh = Emit::input(1);

    // The field-distance seam: on Emit it records a `sdf(p)` call node (the ANALYTIC field via the
    // hand-written `sdf`, interned `"sdf"` — NOT `field_distance`).
    let _ = remarch::b1_exhaustion_remarch_body::<EmitCf, _>(ro, rd, t_seed, t_mesh, |q| {
        EmitCf::call1("sdf", q)
    });

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["t_seed", "t_mesh"];
    let vec_in = ["ro", "rd"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let call_in = CALLS.with(|c| c.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: &call_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is ONE recorded statement (the runtime `Stmt::Loop` with its `t >= t_mesh`
        // `If { Break }`, the DeclTemp p, the DeclTemp d, the composite accept `If { Assign hit;
        // Break }`, the `Stmt::Assign t`, and the `t > T_MAX` `If { Break }`) — a single in-order
        // walk at DEPTH 2 (8-space indent), matching the committed L1520-1535. The site nests
        // main→`if (exhausted)`→this loop, hence depth 2 (vs the depth-3 span of `b1_accept_refine`
        // and the depth-1 span of `m2_surface_hit_refine`). NO function-signature wrap (the span is
        // spliced inside the hand-written `if (exhausted)` wrapper).
        print_block(&body_block, &arena, names, 2, &mut span);
        span
    })
}

/// Generates the HLSL B1 over-relaxation SOR-FAIL-RETREAT STEP SPAN — the production main-marcher's
/// per-iteration Keinert over-relaxed step (`t += d * omega`) with the Lipschitz-aware retreat-to-
/// plain (`if (it > 0u && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev) { t = safe_t + sor_prev;
/// omega = 1.0; continue; }`), the 0%-gated `else { t += d; }` plain arm, and the `t > T_MAX` miss
/// break — by tracing the generic [`crate::sor::b1_sor_retreat_body`] over the `EmitCf` backend, and
/// returns ONLY the span (NOT a wrapped function).
///
/// The Increment-4f facets vs the prior B1 rungs: the `if (omega > 1.0) { ... } else { ... }`
/// TWO-arm branch ([`Cf::if_else`]); the CAPTURED `uint` `it` (seeded `Emit::uint_input(0)`, named
/// `"it"` — the `uint` analogue of Inc4e's captured `float t_mesh`); the `uint` `>` guard
/// ([`Cf::ugt`]) joined by a logical `&&` ([`Cf::and2`]) to the Lipschitz `<`, with `0u` a bare
/// [`Cf::uint_lit`]; the mid-body `continue` ([`Cf::cont`]). `d` is the already-sampled field value
/// (a `float` INPUT — there is NO field call in this span); the carried marcher state
/// (`t`/`omega`/`safe_t`/`sor_prev`/`sor_step_prev` floats, `exhausted` bool) are SUPPRESSED-DECL
/// vars (declared by the hand-written B1 preamble). The span prints at DEPTH 2 (the committed site
/// nests main→`for (uint it)`→this step), matching the committed L1459-1498 indentation (8-space
/// `if (omega > 1.0)`).
///
/// The enclosing marcher (the `for (uint it...)` header, the mesh-guard, the M1/M2 brick islands,
/// `float d = sdf(p);`, the `d < EPS` accept block, and ALL BUG-B1-HOLE rationale comments) stays
/// HAND-WRITTEN inline around the `// === GENERATED b1_sor_retreat BEGIN/END ===` sentinels
/// (framing (b)); the rationale travels in [`crate::sor`]'s module doc. This is the SECOND generated
/// sentinel inside the one hand-written `for (uint it)` loop (Inc4c's accept-refine at depth 3 + this
/// at depth 2).
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the `b1_sor_retreat`
/// text-sync test pins the committed shader span to this output.
pub fn emit_hlsl_b1_sor_retreat() -> String {
    use crate::sor;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's inputs:
    //   d  → Input(0)     (float_in[0] = "d")  — THIS iteration's already-sampled field value (a
    //                                            hand-written `float d = sdf(p);` above the span).
    //   it → UintInput(0) (uint_in[0]  = "it") — the hand-written `for (uint it)` loop's CAPTURED
    //                                            induction var (the `uint` analogue of `t_mesh`).
    // `Node::UintInput(0)` already prints `uint_in[0]` = `"it"` (ZERO printer change for the read).
    let d = Emit::input(0);
    let it = Emit::uint_input(0);

    // The carried marcher state — SUPPRESSED-DECL vars declared by the hand-written B1 preamble
    // (so get/set spell `t`/`t = ...;` but NO `float t = ...;` redecl is recorded inside the span).
    // The `_init` seeds are unused on Emit (a suppressed local is bound by name); pass the `d` input
    // handle as a byte-neutral placeholder (decl_param/decl_bool_param record NO statement + push NO
    // node on Emit, so the placeholder is never referenced).
    let t = EmitCf::decl_param("t", d);
    let omega = EmitCf::decl_param("omega", d);
    let safe_t = EmitCf::decl_param("safe_t", d);
    let sor_prev = EmitCf::decl_param("sor_prev", d);
    let sor_step_prev = EmitCf::decl_param("sor_step_prev", d);
    let exhausted = EmitCf::decl_bool_param("exhausted", false);

    let _ = sor::b1_sor_retreat_body::<EmitCf>(
        d,
        it,
        &t,
        &omega,
        &safe_t,
        &sor_prev,
        &sor_step_prev,
        &exhausted,
    );

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["d"];
    let uint_in = ["it"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: &uint_in,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is TWO recorded statements (the `Stmt::IfElse` over `omega > 1.0` — its
        // then-block carrying the DeclTemp step_len, the composite retreat `If { Assign t; Assign
        // omega; Continue }`, the four trailing Assigns; its else-block the plain `Assign t` — and
        // the `t > T_MAX` composite miss `If { Assign exhausted; Break }`) — a single in-order walk
        // at DEPTH 2 (8-space `if (omega > 1.0)`), matching the committed L1459-1498. The site nests
        // main→`for (uint it)`→this step. NO function-signature wrap (the span is spliced inside the
        // hand-written B1 marcher loop).
        print_block(&body_block, &arena, names, 2, &mut span);
        span
    })
}

/// Traces a SINGLE-STATEMENT bool-decl producer body over `EmitCf` and returns ONLY that one
/// `bool <name> = <init>;` line (at depth 1 — 4-space indent, matching the committed B1 preamble
/// decls at L1316/L1327 inside `main`). Shared by [`emit_hlsl_b1_decl_hit`] /
/// [`emit_hlsl_b1_decl_exhausted`]: the body records exactly one [`Stmt::DeclVar`] whose `rhs` is
/// a [`Node::BoolLit`] (a pure literal — NO inputs/temps/named-lits/calls), so every name table
/// is empty except `vars` (the single declared name). The `body` closure records the decl into the
/// freshly-seeded function block; this helper does the reset/seed/pop/print harness around it.
fn emit_hlsl_b1_decl<F: FnOnce()>(body: F) -> String {
    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Record the single `Stmt::DeclVar` (the producer calls `EmitCf::decl_bool_var`).
    body();

    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    // Only `vars` is populated (the single declared name); the bool literal rhs touches no
    // other name table.
    let vars = VARS.with(|v| v.borrow().clone());
    let float_in: [&str; 0] = [];
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The single `bool <name> = <init>;` decl at depth 1 (4-space indent, matching the
        // committed L1316/L1327 inside `main`).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the B1 marcher's `bool hit = false;` preamble decl (the committed
/// `sdf_gbuffer_composite.hlsl:1316`) by tracing [`crate::decl::b1_decl_hit_body`] over `EmitCf`,
/// and returns ONLY that one line. The FIRST rung of the B1-marcher single-source ladder
/// (Increment 4d): a TYPED `bool` decl facet ([`Cf::decl_bool_var`]). The two B1 bool preamble
/// decls (`hit` here, `exhausted` in [`emit_hlsl_b1_decl_exhausted`]) are NON-CONTIGUOUS in the
/// committed shader (separated by 4 `float` decls + the BUG-B1-HOLE-3 comment), so each is its own
/// generated sentinel pair; the float decls + comments between them stay hand-written.
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the `b1_decl_hit`
/// text-sync test pins the committed shader line to this output.
pub fn emit_hlsl_b1_decl_hit() -> String {
    use crate::decl;
    emit_hlsl_b1_decl(|| {
        let _ = decl::b1_decl_hit_body::<EmitCf>();
    })
}

/// Generates the B1 marcher's `bool exhausted = true;` preamble decl (the committed
/// `sdf_gbuffer_composite.hlsl:1327`) by tracing [`crate::decl::b1_decl_exhausted_body`] over
/// `EmitCf`, and returns ONLY that one line. The BUG-B1-HOLE-3 budget-exhaustion flag (init
/// `true`, cleared by every in-loop `break`); see [`emit_hlsl_b1_decl_hit`] for the rung framing.
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the `b1_decl_exhausted`
/// text-sync test pins the committed shader line to this output.
pub fn emit_hlsl_b1_decl_exhausted() -> String {
    use crate::decl;
    emit_hlsl_b1_decl(|| {
        let _ = decl::b1_decl_exhausted_body::<EmitCf>();
    })
}
