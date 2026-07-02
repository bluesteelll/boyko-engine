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
    /// An HLSL SIGNED `int` value (`select_level`'s `-1` / `(int)L` — Increment 5a). The result
    /// type of [`Node::IntLit`] / [`Node::IntFromUint`] (both inline leaves, consumed only by a
    /// `Stmt::Return`). No node materializes an `int` temp (the `int` return type comes from the
    /// hand-written `int select_level` SIGNATURE, not a decl). Spells the token `int`.
    Int,
    /// An HLSL `float2` value (`pack_material_id_ba`'s `float2((float)lo / 255.0, (float)hi / 255.0)`
    /// return — Track B Increment G1). The result type of [`Node::Vec2FromScalars`] (the `float2(x,
    /// y)` ctor of two `float` scalars), consumed only by a `Stmt::Return`. No node materializes a
    /// `float2` temp (the `float2` return type comes from the hand-written `float2 pack_material_id_ba`
    /// SIGNATURE, not a decl). Spells the token `float2`.
    Float2,
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
    /// `sin(a)` — the HLSL `sin` intrinsic (the B2 interp slerp weights). Recorded ONLY
    /// by [`InterpBackend`](crate::interp::InterpBackend) (`interp_trs`); the SDF field
    /// op-set is transcendental-free, so no field/marcher body records it.
    Sin(u32),
    /// `cos(a)` — the HLSL `cos` intrinsic (carried for the B2 interp facet; the slerp
    /// body itself uses `sin`/`acos`, but the backend op-set exposes `cos` for
    /// completeness of the transcendental facet).
    Cos(u32),
    /// `acos(a)` — the HLSL `acos` intrinsic (the B2 interp slerp angle `theta =
    /// acos(dot)`). Recorded ONLY by [`InterpBackend`](crate::interp::InterpBackend).
    Acos(u32),
    /// `cond ? t : e` — the HLSL ternary (the frozen `(k > 0.0) ? _ : _`). The arms are
    /// spelled UN-wrapped (the brick-exit progress clamp). Recorded by
    /// [`FieldScalar::select`].
    Select(u32, u32, u32),
    /// `cond ? (t) : (e)` — the HLSL ternary with BOTH arms wrapped UNCONDITIONALLY (the
    /// committed `m2_regula_falsi` ternary form). A DISTINCT node from [`Node::Select`] so
    /// the brick-exit's un-wrapped `Select` printer is unperturbed; recorded ONLY by
    /// [`EmitCf::select`] (`Cf::select`), never [`FieldScalar::select`] (Increment 4a).
    SelectParen(u32, u32, u32),
    /// `cond ? t : e` — the HLSL ternary with NO parentheses on ANY of the three parts (the committed
    /// `oct_encode` sign-ternary `e.x >= 0.0 ? 1.0 : -1.0`, Track B Increment G2). DISTINCT from
    /// [`Node::Select`] (which wraps the CONDITION — `(cond) ? t : e`) and [`Node::SelectParen`] (which
    /// wraps the condition AND both arms): the committed `oct_encode` spells the bare form, so a
    /// distinct node prints all three parts un-wrapped. Recorded ONLY by [`EmitCf::select_bare`]; the
    /// arms (`1.0`/`-1.0`) + condition (`e.x >= 0.0`) are all inline leaves, so the bare spelling is the
    /// exact committed text. Groups like the other ternaries in [`needs_paren_as_operand`] (it is the
    /// `float2(<a>, <b>)` ctor arg, so the grouping is never exercised).
    SelectBare(u32, u32, u32),
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
    /// `a == b` — a FLOAT equality Mask node (`OpFOrdEqual`), printed inline like
    /// [`Node::Gt`]. The B2 interp exact-at-`prev==curr` keystone's per-component test
    /// (folded by `&&` and consumed by `select`). Recorded ONLY by
    /// [`InterpBackend`](crate::interp::InterpBackend); the SDF field op-set has no float
    /// `==` (its only equality is the integer [`Node::IntEq`]).
    FEq(u32, u32),
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
    // The packed-byte bit ops. `decode_snorm8` reads an UNPACKED `i8` code (no AND/shift
    // needed), so these were dead foundation until Track B Increment G1 wired the
    // `Cf::and_u` / `Cf::shr_u` methods + the `pack_material_id_ba` host index math that
    // CONSTRUCTS them (`id & 255u`, `id >> 8u`). The printer (`define_str`) already spelled
    // them (`{} & {}` / `{} >> {}`, unparenthesized).
    /// `a & b` — a bitwise AND over two `uint` handles (the packed-byte extract, `id & 255u`).
    And(u32, u32),
    /// `a >> b` — a logical right shift over `uint` handles (the byte select within a packed
    /// word, `id >> 8u`).
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

    // ---- Increment 5a: the SIGNED-INT subsystem + the M4Level access-text (`select_level`) ----
    /// A SIGNED `int` LITERAL — prints the bare signed value (`-1`), DISTINCT from
    /// [`Node::UintLit`]'s `<x>u` unsigned suffix. `select_level`'s tail `return -1;`. An inline
    /// leaf, result type [`EmitTy::Int`], consumed ONLY by a [`Stmt::Return`].
    IntLit(i32),
    /// `(int)<operand>` — the HLSL value-preserving `uint -> int` cast (`select_level`'s `(int)L`).
    /// The operand is the loop iv node (a `UintInput` printing `L`). An inline leaf (spells `(int)L`
    /// at its single use), result type [`EmitTy::Int`].
    IntFromUint(u32),
    /// A PUSH-CONSTANT `uint` FIELD read by BARE TEXT — prints the literal `field` string
    /// (`pc.brick_levels`). `sym_id` indexes [`Names::pc_in`]. `select_level`'s `L >= pc.brick_levels`
    /// guard comparand. An inline leaf, result type [`EmitTy::Uint`] (a `uint` push-constant field).
    PcUint(u32),
    /// An `M4Level` array-element field read by ACCESS TEXT — prints `m2_levels[<L>].<field>`.
    /// `(iv_id, field_id, is_vec3)`: `iv_id` references the loop iv node (a `UintInput` printing
    /// `L`), `field_id` indexes [`Names::level_field`] (the member+swizzle text, e.g.
    /// `origin_brick_world.xyz`). The `M4Level` STRUCT LAYOUT is NOT modeled — only the access text.
    /// An inline leaf; its result type is [`EmitTy::Float3`] (a `.xyz` swizzle) or [`EmitTy::Float`]
    /// (a `.w` swizzle), carried by the `is_vec3` flag.
    LevelField {
        iv_id: u32,
        field_id: u32,
        is_vec3: bool,
    },
    /// `p >= o` — a component-wise `float3` `>=` producing a bool3 (`OpFOrdGreaterThanEqual` over
    /// vectors). NEVER printed alone: it is the operand of an [`Node::All3`] (`all(p >= o)`). A Mask
    /// node (typed `Float` by `type_of`, like the other comparisons — never `chk`-typed).
    Bool3Ge(u32, u32),
    /// `p < hi` — the component-wise `float3` `<` analogue of [`Node::Bool3Ge`] (`OpFOrdLessThan`
    /// over vectors). The operand of an [`Node::All3`] (`all(p < hi)`).
    Bool3Lt(u32, u32),
    /// `all(<bool3>)` — the HLSL `all` intrinsic reducing a component-wise vector compare
    /// ([`Node::Bool3Ge`] / [`Node::Bool3Lt`]) to a single bool. `select_level`'s `all(p >= o)` /
    /// `all(p < hi)`. An inline leaf (a mask, appears only inside the `&&` condition); printed
    /// `all(<operand>)`.
    All3(u32),

    // ---- Increment 5c: the named-local-array subsystem (`m2_brick_cubic_hit`) ----
    //
    // The DDA marcher carries four named LOCAL ARRAYS (`int cell[3]`, `int step[3]`, `float
    // t_next[3]`, `float t_delta[3]`) + a per-cell `float s[8]` corner buffer. Each array is an
    // UNINITIALIZED named decl ([`Stmt::DeclArray`]); a per-element access ([`Node::ArrayElem`])
    // is an INLINE leaf (`cell[axis]` / `s[k]`); a per-element store / compound-add are
    // statements ([`Stmt::ArrayStore`] / [`Stmt::ArrayAddAssign`]). The array NAME is carried
    // out-of-band (the printer's ARRAY_NAMES table), the element TYPE out-of-band (ARRAY_ELEM_TYS).

    /// A named-local-array ELEMENT read — `cell[axis]` / `s[0]` / `t_next[axis]`. `(arr, idx)`:
    /// `arr` indexes the printer's array-name table ([`Names::array`]); `idx` is the index node
    /// (a [`Node::UintLit`] `0`/`1`/`2` or the iv `axis`/`cx`). Printed INLINE (`is_inline_leaf` =
    /// true), so each use spells `cell[axis]` at the use site (matching the committed body). The
    /// result type is read OUT-OF-BAND ([`array_elem_ty`] keyed by `arr`) — `int` for `cell`/`step`,
    /// `float` for `t_next`/`t_delta`/`s` (the access-chain carries no type itself).
    ArrayElem { arr: u32, idx: u32 },

    /// `(int)<uint>` — the HLSL value-preserving `uint -> int` cast of a `uint` arithmetic value
    /// (`(uint)max(cell[0], 0)`). DISTINCT from [`Node::IntFromUint`] only in being recorded by
    /// the Increment-5c [`Cf::int_from_uint`] for an arbitrary `uint` node (Increment 5a's seeded
    /// the loop iv `L`); both print `(int)<operand>` and are inline leaves typed [`EmitTy::Int`].
    /// NOT added as a new variant — see the reuse note in [`Cf::int_from_uint`].

    /// `max(a, b)` over two SIGNED `int` handles (`max(cell[0], 0)`). The signed-int analogue of
    /// [`Node::Max`] (which is FLOAT). Materialized as an `int` temp (none in this body — it is
    /// always the operand of a `(uint)` cast, so it is a non-leaf consumed by [`Node::UintFromInt`]).
    /// Result type [`EmitTy::Int`].
    SMax(u32, u32),

    /// `(uint)<int>` — the HLSL value-preserving `int -> uint` cast (`(uint)max(cell[0], 0)`). The
    /// `int -> uint` analogue of [`Node::IntFromUint`]'s `uint -> int`. Materialized as a `uint`
    /// temp (none here — it is the inner of a `min(..., W - 2u)`, a non-leaf operand). Result type
    /// [`EmitTy::Uint`].
    UintFromInt(u32),

    /// `a < b` over two SIGNED `int` handles — a Mask node (`OpSLessThan`, the SIGNED `<`, DISTINCT
    /// from the FLOAT [`Node::Lt`] and the UNSIGNED comparisons). The DDA exit guard's `cell[axis] <
    /// 0`. Printed inline like [`Node::Lt`].
    SLt(u32, u32),

    /// `a + b` over two SIGNED `int` handles (`c0 + 1`). The signed-int analogue of [`Node::Add`]
    /// (FLOAT) / [`Node::UAdd`] (`uint`). Additive (wraps under a multiplicative/cast parent — see
    /// [`needs_paren_as_operand`], the `(float)(c0 + 1)` cast-operand wrap). Result type
    /// [`EmitTy::Int`].
    SAdd(u32, u32),

    /// `(float)<int>` — the HLSL value-preserving `int -> float` cast (`(float)c0` / `(float)(c0 +
    /// 1)`). The `int -> float` analogue of [`Node::UintToFloat`]. An inline leaf typed
    /// [`EmitTy::Float`]; its operand spells at a CAST position (an additive operand wraps —
    /// `(float)(c0 + 1)`, NOT `(float)c0 + 1`).
    FloatFromInt(u32),

    /// `(float)<uint>` — the HLSL value-preserving `uint -> float` cast (`(float)cx`). The `uint ->
    /// float` analogue of [`Node::FloatFromInt`] / [`Node::UintToFloat`] (the LATTER materializes a
    /// temp; this is an INLINE leaf for the `lo_g` `float3` constructor's per-component `- (float)cx`).
    /// Routed separately so the `lo_g` ctor spells `(float)cx` inline at the use site. An inline leaf
    /// typed [`EmitTy::Float`].
    FloatFromUint(u32),

    /// `a - b` over two `uint` handles (`W - 2u` / `W - 1u`). The `uint` SUBTRACT — the `uint`
    /// analogue of [`Node::Sub`] (FLOAT). Additive (wraps under a multiplicative parent like
    /// [`Node::UAdd`]). Result type [`EmitTy::Uint`].
    USub(u32, u32),

    /// `min(a, b)` over two `uint` handles (`min((uint)max(cell[0], 0), W - 2u)`). The `uint` MIN —
    /// the `uint` analogue of [`Node::Min`] (FLOAT). Materialized as a `uint` temp (`uint cx = ...;`).
    /// DISTINCT from [`Node::Min`] only so its operands are checked `Uint` (not `Float`). Result type
    /// [`EmitTy::Uint`].
    UMin(u32, u32),

    /// `a == b` over two SIGNED `int` handles — a Mask node (`OpIEqual`, the integer `==`). The DDA
    /// exit guard's `step[axis] == 0`. Printed inline like [`Node::IntEq`] (the SAME `==` spelling;
    /// DISTINCT only at the type level — its operands are `int` array elements, not `uint`).
    SIntEq(u32, u32),

    /// A `uint` NESTED axis-select — `(<c0>) ? 0u : ((<c1>) ? 1u : 2u)`. The `uint`-typed analogue of
    /// [`Node::SelectParen`] (FLOAT arms). `(c, t, e)`: both arms are `uint` (`0u` / a nested select).
    /// The DDA's `axis = (t_next[0] <= t_next[1] && t_next[0] <= t_next[2]) ? 0u : (...)`. Result type
    /// [`EmitTy::Uint`]. Printed `(<c>) ? <t> : <e>` (the arms WRAPPED only when themselves a select
    /// — the committed text wraps the nested select but NOT the `0u`/`1u`/`2u` leaves).
    SelectParenU(u32, u32, u32),

    /// A DYNAMIC float3-PARAMETER index — `rd_v[axis]` / `ro_v[0]`. `(vec_id, idx)`: `vec_id` indexes
    /// [`Names::vec_in`] (the whole-`float3` PARAMETER name, e.g. `rd_v`); `idx` is the index node (a
    /// [`Node::UintLit`] literal `0`/`1`/`2` or the iv `axis`). DISTINCT from [`Node::VecIndex`]
    /// (which reads a SEEDED `[Emit; 3]` `VecParamRef`): a `Vec3DynIndex` indexes a WHOLE `Vec3Param`
    /// by an arbitrary index node, so a single seeded `Vec3Param` is BOTH passed whole (the
    /// `call_coeffs` `rd_v` arg) AND indexed `rd_v[axis]`. An inline leaf typed [`EmitTy::Float`].
    Vec3DynIndex { vec_id: u32, idx: u32 },

    /// `float3(<x>, <y>, <z>)` from THREE `float` SCALAR expressions (the `lo_g` ctor). DISTINCT from
    /// [`Node::Vec3FromUints`] (three `uint` operands, implicit `uint->float`): here all three are
    /// already-`float` arithmetic expressions. Materialized as a `float3` temp. Result type
    /// [`EmitTy::Float3`].
    Vec3FromScalars(u32, u32, u32),

    /// `float2(<x>, <y>)` from TWO `float` SCALAR expressions — `pack_material_id_ba`'s
    /// `float2((float)lo / 255.0, (float)hi / 255.0)` return (Track B Increment G1). The `float2`
    /// analogue of [`Node::Vec3FromScalars`] (both components already-`float` arithmetic). Asserts
    /// both operands `Float`; result type [`EmitTy::Float2`]. NEVER materialized as a temp (the
    /// committed body returns the ctor directly), so it is composed inline by the `Stmt::Return`
    /// printer.
    Vec2FromScalars(u32, u32),

    // ---- Track B Increment G2: the `oct_encode` `float2` component ops (`EmitTy::Float2`/`Float`) ----
    /// A `float2`/`float3` → `float2` MULTI-LANE swizzle — `(src, mask)` where `mask` indexes
    /// [`VEC2_SWIZZLES`] (`"xy"` for `n.xy`, `"yx"` for `e.yx`). The source may be a `Vec3f` (`n.xy`)
    /// or a `Vec2f` (`e.yx`) — the swizzle text alone determines the result lanes. Printed INLINE
    /// (`<src>.xy` / `<src>.yx`). Result type [`EmitTy::Float2`].
    Vec2Swizzle(u32, u8),
    /// A `float2` → `float` SINGLE-COMPONENT swizzle (`e.x` / `e.y`) — `(src, axis)` where `axis` 0/1 =
    /// x/y. Printed INLINE (`<src>.x` / `<src>.y`). DISTINCT from [`Node::Vec3Swizzle`] only in being
    /// recorded for a `float2` source (the spelling is the SAME `.x`/`.y`). Result type
    /// [`EmitTy::Float`].
    Vec2Comp(u32, u8),
    /// `abs(v)` over a `float2` (`abs(e.yx)`) — component-wise. Result type [`EmitTy::Float2`]. The
    /// `float2` analogue of the scalar [`Node::Abs`]; printed `abs(<v>)`.
    Vec2Abs(u32),
    /// `a * b` over two `float2` handles (`(1.0 - abs(e.yx)) * float2(...)`) — component-wise. Result
    /// type [`EmitTy::Float2`]. The `float2` analogue of [`Node::Mul`]; both operands are the
    /// MULTIPLICATIVE side (`*`/`/` bind tighter than `+`/`-`).
    Vec2Mul(u32, u32),
    /// `v * s` — a `float2` times a `float` scalar (`e * 0.5`). `(vec, scalar)`. Result type
    /// [`EmitTy::Float2`]. The `float2` analogue of [`Node::Vec3MulScalar`].
    Vec2MulScalar(u32, u32),
    /// `v + s` — a `float2` plus a `float` scalar broadcast (`... + 0.5`). `(vec, scalar)`. Result type
    /// [`EmitTy::Float2`]. ADDITIVE (wraps under a multiplicative parent) — the `float2` analogue of
    /// [`Node::Add`].
    Vec2AddScalar(u32, u32),
    /// `s - v` — a `float` scalar (broadcast) MINUS a `float2`, scalar on the LEFT (`1.0 - abs(e.yx)`).
    /// `(scalar, vec)`. Result type [`EmitTy::Float2`]. ADDITIVE (wraps under a multiplicative parent —
    /// `(1.0 - abs(e.yx)) * float2(...)`). DISTINCT from a `float2 - float`: the operand ORDER is
    /// scalar-then-vector, so the printer spells `<s> - <v>`.
    Vec2RSubScalar(u32, u32),

    /// A CALL to a frozen hand-written shader function with N HETEROGENEOUS args — `m2_corner(atlas,
    /// atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half)` / `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)`
    /// / `m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo)`. `sym_id` indexes [`Names::call_in`] (the
    /// SAME table [`Node::Call1`]/[`Node::Call2`] use); `arg_lo`/`arg_count` slice the flat
    /// [`CALL_ARGS`] side-table (the variadic arg node ids — the arena [`Node`] is `Copy`, so the
    /// arg list lives out-of-band). `ret` carries the result [`EmitTy`] (`Float` for `m2_corner`/
    /// `m2_marmitt_root`, `Float4` for `m2_jcgt_cubic_coeffs`). Printed `sym(<op(a0)>, <op(a1)>, ...)`.
    CallN {
        sym_id: u32,
        arg_lo: u32,
        arg_count: u32,
        ret: EmitTy,
    },

    /// A RESOURCE-PARAMETER reference — `atlas` (a `Texture3D<float>`) / `atlas_smp` (a
    /// `SamplerState`). `u32` indexes [`Names::res_in`] (the resource-name table). It is a
    /// CALL-THROUGH-ONLY operand (consumed by a [`Node::CallN`] `m2_corner(atlas, atlas_smp, ...)`);
    /// the CPU cannot run `atlas.SampleLevel`, so this is an EMIT-ONLY node (the body is never
    /// instantiated over `EvalCf`). An inline leaf; spells its NAME. Carries no scalar/vector type
    /// ([`type_of`] falls to the default `Float`, harmless — it is never an arithmetic operand).
    ResRef(u32),

    /// A by-NAME ARRAY argument — `s` (the whole `float s[8]` passed to `m2_jcgt_cubic_coeffs(s,
    /// ...)`). `u32` indexes [`Names::array`] (the SAME array-name table [`Node::ArrayElem`] uses).
    /// DISTINCT from [`Node::ArrayElem`] (a per-ELEMENT `s[k]` read): this passes the WHOLE array by
    /// name. An inline leaf; spells the array NAME (`s`). Carries no element type ([`type_of`]
    /// default `Float`, harmless — it is only ever a `CallN` arg, never an arithmetic operand).
    ArrName(u32),
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

    /// The FLAT variadic-call argument pool (Increment 5c) — a [`Node::CallN`] slices it by
    /// `(arg_lo, arg_count)`. The arena [`Node`] is `Copy` (fixed size), so a heterogeneous
    /// argument list (`m2_corner`'s 8 args, `m2_jcgt_cubic_coeffs`'s 3) lives in THIS side table,
    /// not in the node. Each entry is an arg's [`Node`] arena id; the printer reads `CALL_ARGS[arg_lo
    /// .. arg_lo + arg_count]` and spells `sym(op(a0), op(a1), ...)`. Cleared per-emit.
    static CALL_ARGS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };

    /// OUT-OF-BAND named-local-array ELEMENT types (Increment 5c) — `cell`/`step` are `int` arrays,
    /// `t_next`/`t_delta`/`s` are `float` arrays, keyed by the array's name-table index (the index IS
    /// the [`Names::array`] id). [`array_elem_ty`]`(`[`Node::ArrayElem`]`{arr,..})` reads it so the
    /// printer types a `cell[axis]` access `int` (vs a `t_next[axis]` `float`) without a type on the
    /// access node. Pushed in [`Stmt::DeclArray`] order (the index IS the array id). Cleared per-emit.
    static ARRAY_ELEM_TYS: RefCell<Vec<EmitTy>> = const { RefCell::new(Vec::new()) };
}

/// The element [`EmitTy`] of named-local array `arr` (out-of-band, see [`ARRAY_ELEM_TYS`]).
/// Defaults to [`EmitTy::Float`] when `arr` is past the recorded set (no array decls outside
/// `m2_brick_cubic_hit`, so the default is never read on other leaves).
fn array_elem_ty(arr: u32) -> EmitTy {
    ARRAY_ELEM_TYS.with(|t| t.borrow().get(arr as usize).copied().unwrap_or(EmitTy::Float))
}

/// Pushes `arg` ids into [`CALL_ARGS`] and returns the `(arg_lo, arg_count)` slice (Increment 5c).
fn record_call_args(args: &[u32]) -> (u32, u32) {
    CALL_ARGS.with(|c| {
        let mut c = c.borrow_mut();
        let lo = c.len() as u32;
        c.extend_from_slice(args);
        (lo, args.len() as u32)
    })
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

/// The declared [`EmitTy`] of mutable local `id` (out-of-band, see [`VAR_TYPES`]). Defaults to
/// [`EmitTy::Float`] when `id` is past the recorded set (the pre-G2 leaves declare only `float`/`bool`
/// scalar vars and do not necessarily seed [`VAR_TYPES`], so the default preserves their behavior).
fn var_type(id: u32) -> EmitTy {
    VAR_TYPES.with(|t| t.borrow().get(id as usize).copied().unwrap_or(EmitTy::Float))
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
        // Increment 5a: a push-constant `uint` field read (`pc.brick_levels`) is a `uint`.
        Node::PcUint(_) => EmitTy::Uint,
        // Increment 5a: the SIGNED-`int` value surfaces (`-1`, `(int)L`) are `int`.
        Node::IntLit(_) | Node::IntFromUint(_) => EmitTy::Int,
        // Increment 5c: the signed-int value surfaces (`max(cell[0],0)`, `c0 + 1`) are `int`.
        Node::SMax(_, _) | Node::SAdd(_, _) => EmitTy::Int,
        // Increment 5c: the `uint`-result nodes — the `int -> uint` cast, the `uint` subtract,
        // the `uint` min, and the nested `uint` axis-select.
        Node::UintFromInt(_) | Node::USub(_, _) | Node::UMin(_, _) | Node::SelectParenU(_, _, _) => {
            EmitTy::Uint
        }
        // Increment 5c: the `int->float`/`uint->float` casts + the dynamic float3 index are `float`;
        // the `float3` ctor of 3 scalars is a `float3`; a `CallN` carries its result type out-of-band.
        Node::FloatFromInt(_) | Node::FloatFromUint(_) | Node::Vec3DynIndex { .. } => EmitTy::Float,
        Node::Vec3FromScalars(_, _, _) => EmitTy::Float3,
        // Track B Increment G1: the `float2(x, y)` ctor of two `float` scalars is a `float2`.
        Node::Vec2FromScalars(_, _) => EmitTy::Float2,
        // Track B Increment G2: the `float2` component ops are `float2` (the multi-lane swizzle, abs,
        // mul, mul-scalar, add-scalar, scalar-minus-vec); the SINGLE-component `e.x`/`e.y` swizzle is a
        // `float`. ALL above the `_ => Float` catch-all (O1 — each typed node has an explicit arm).
        Node::Vec2Swizzle(_, _)
        | Node::Vec2Abs(_)
        | Node::Vec2Mul(_, _)
        | Node::Vec2MulScalar(_, _)
        | Node::Vec2AddScalar(_, _)
        | Node::Vec2RSubScalar(_, _) => EmitTy::Float2,
        Node::Vec2Comp(_, _) => EmitTy::Float,
        // A mutable-local READ carries the var's DECLARED type out-of-band ([`var_type`], keyed by the
        // `VARS` id) — `float3` for `oct_encode`'s `n`, `float2` for `e`, `float`/`bool` for every
        // pre-G2 scalar var (the default). Above the `_ => Float` catch-all so the typed-var reads
        // (`n.x` consumers' `Float3` `chk`) pass; pre-G2 scalar vars resolve `Float` either way.
        Node::VarRef(id) => var_type(id),
        Node::CallN { ret, .. } => ret,
        // Increment 5c: a named-local-array ELEMENT carries the array's element type out-of-band
        // (`int` for `cell`/`step`, `float` for `t_next`/`t_delta`/`s`), keyed by the array id.
        Node::ArrayElem { arr, .. } => array_elem_ty(arr),
        // Increment 5c: a resource ref / a by-name array arg / a signed-int `==` mask / a signed-int
        // `<` mask are NEVER arithmetic operands (`ResRef`/`ArrName` are call-through-only;
        // `SIntEq`/`SLt` are masks inside the DDA-exit `||` condition), so they fall to the `Float`
        // default like the other masks — never `chk`-typed.
        // Increment 5a: an `M4Level` field read is a `float3` (`.xyz`) or `float` (`.w`), carried by
        // the `is_vec3` flag (the `M4Level` layout is NOT modeled — only the access text + its type).
        Node::LevelField { is_vec3, .. } => {
            if is_vec3 {
                EmitTy::Float3
            } else {
                EmitTy::Float
            }
        }
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
        // The SIGNED `int` token (Increment 5a). NEVER reached today as a DECL-SITE token: no
        // node materializes an `int` temp/local (the `int` return type comes from the hand-written
        // `int select_level` SIGNATURE, and `IntLit`/`IntFromUint` are inline-leaf `Stmt::Return`
        // operands, never `Stmt::DeclTemp`/`Stmt::DeclVar` `ty` fields). Spelled for exhaustiveness
        // + any future `int` decl (a 2-line mirror).
        EmitTy::Int => "int",
        // The `float2` token (Track B Increment G1). NEVER reached today as a DECL-SITE token: no
        // node materializes a `float2` temp (the `float2` return type comes from the hand-written
        // `float2 pack_material_id_ba` SIGNATURE, and the `Vec2FromScalars` ctor is an inline-leaf-
        // composed `Stmt::Return` operand, never a `Stmt::DeclTemp`/`Stmt::DeclVar` `ty` field).
        // Spelled for exhaustiveness + any future `float2` decl (a 2-line mirror).
        EmitTy::Float2 => "float2",
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

// ---- The B2 interp backend: `impl InterpBackend for Emit` ---------------------
//
// The TRS-interpolation body ([`crate::interp::transform_pair_interp_body`]) records
// into the SAME arena / `Node` IR the field bodies use — the recorder is NOT forked.
// Only the three transcendental nodes (`Sin`/`Cos`/`Acos`) and the float `==`
// (`FEq`) are new; everything else reuses the existing arithmetic/select/comparison
// nodes.

impl crate::interp::InterpBackend for Emit {
    type Mask = EmitMask;

    #[inline]
    fn lit(x: f32) -> Self {
        Emit(push(Node::Lit(x)))
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
    fn abs(self) -> Self {
        Emit(push(Node::Abs(self.0)))
    }
    #[inline]
    fn sqrt(self) -> Self {
        Emit(push(Node::Sqrt(self.0)))
    }
    #[inline]
    fn sin(self) -> Self {
        Emit(push(Node::Sin(self.0)))
    }
    #[inline]
    fn cos(self) -> Self {
        Emit(push(Node::Cos(self.0)))
    }
    #[inline]
    fn acos(self) -> Self {
        Emit(push(Node::Acos(self.0)))
    }
    #[inline]
    fn select(cond: EmitMask, t: Self, e: Self) -> Self {
        Emit(push(Node::Select(cond.0, t.0, e.0)))
    }
    #[inline]
    fn lt(self, rhs: Self) -> EmitMask {
        EmitMask(push(Node::Lt(self.0, rhs.0)))
    }
    #[inline]
    fn gt(self, rhs: Self) -> EmitMask {
        EmitMask(push(Node::Gt(self.0, rhs.0)))
    }
    #[inline]
    fn and(a: EmitMask, b: EmitMask) -> EmitMask {
        EmitMask(push(Node::And2(a.0, b.0)))
    }
    #[inline]
    fn eq(self, rhs: Self) -> EmitMask {
        EmitMask(push(Node::FEq(self.0, rhs.0)))
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

/// The default push-constant field-text table (empty — only `select_level` reads a push-constant
/// field `pc.brick_levels` — Increment 5a).
const NO_PC_INPUTS: &[&str] = &[];

/// The default `M4Level` access-text table (empty — only `select_level` reads `m2_levels[L].…`
/// fields — Increment 5a).
const NO_LEVEL_FIELDS: &[&str] = &[];

/// The default named-local-array name table (empty — only `m2_brick_cubic_hit` declares local
/// arrays `cell`/`step`/`t_next`/`t_delta`/`s` — Increment 5c).
const NO_ARRAY: &[&str] = &[];

/// The default resource-parameter name table (empty — only `m2_brick_cubic_hit` takes the
/// `atlas`/`atlas_smp` resource params — Increment 5c).
const NO_RES_INPUTS: &[&str] = &[];

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
    /// PUSH-CONSTANT field-text (indexed by [`Node::PcUint`]'s `sym_id`) — `pc.brick_levels`.
    /// Empty for every leaf but `select_level` (Increment 5a).
    pc_in: &'a [&'a str],
    /// `M4Level` ACCESS-TEXT (indexed by [`Node::LevelField`]'s `field_id`) — the member+swizzle
    /// (`origin_brick_world.xyz` / `origin_brick_world.w` / `dims_atlas_dim.xyz`). The printer
    /// spells `m2_levels[<L>].<level_field[id]>`. Empty for every leaf but `select_level`
    /// (Increment 5a).
    level_field: &'a [&'a str],
    /// NAMED-LOCAL-ARRAY names (indexed by [`Node::ArrayElem`]'s `arr`, [`Node::ArrName`]'s id, AND
    /// [`Stmt::DeclArray`]'s `arr`) — `cell` / `step` / `t_next` / `t_delta` / `s`. The printer
    /// spells `cell[<idx>]` / `s` (by-name). Empty for every leaf but `m2_brick_cubic_hit`
    /// (Increment 5c).
    array: &'a [&'a str],
    /// RESOURCE-PARAMETER names (indexed by [`Node::ResRef`]'s id) — `atlas` / `atlas_smp`. The
    /// printer spells the resource name as a [`Node::CallN`] arg. Empty for every leaf but
    /// `m2_brick_cubic_hit` (Increment 5c).
    res_in: &'a [&'a str],
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
            | Node::FEq(_, _)
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
            // Increment 5a: the signed-int return values (`-1`, `(int)L`) spell inline (operands of
            // a `Stmt::Return`); the push-constant field read (`pc.brick_levels`) spells inline; the
            // `all(...)` reduction + its component-wise `>=`/`<` operands appear only inside the `&&`
            // condition (inlined like the other masks); the level-field read spells `m2_levels[L].…`
            // inline at its single `temp_vec3`/`temp_float` rhs.
            | Node::IntLit(_)
            | Node::IntFromUint(_)
            | Node::PcUint(_)
            | Node::LevelField { .. }
            | Node::Bool3Ge(_, _)
            | Node::Bool3Lt(_, _)
            | Node::All3(_)
            // Increment 5c inline leaves: a named-array element (`cell[axis]`/`s[0]`) spells inline at
            // each use; the signed `<`/`==` masks appear only inside the DDA-exit `||` condition; the
            // `(float)`-cast leaves (`(float)c0`/`(float)cx`) spell inline in the `lo_g` ctor; the
            // dynamic float3 index (`rd_v[axis]`) spells inline; the resource refs (`atlas`/
            // `atlas_smp`) + the by-name array arg (`s`) spell their name as a `CallN` arg.
            | Node::ArrayElem { .. }
            | Node::SLt(_, _)
            | Node::SIntEq(_, _)
            | Node::FloatFromInt(_)
            | Node::FloatFromUint(_)
            | Node::Vec3DynIndex { .. }
            | Node::ResRef(_)
            | Node::ArrName(_)
            // Track B Increment G2: the `float2` swizzles (`n.xy`, `e.yx`, `e.x`, `e.y`) spell
            // `<src>.<mask>` inline at every use (like `Vec3Swizzle`), never a `tN` temp. The `float2`
            // ARITHMETIC nodes (abs/mul/mul-scalar/add-scalar/scalar-sub) are NOT leaves — they are
            // composed inline by `define_str` (the committed `oct_encode` temps NOTHING; every value is
            // inline within the `n = ...;` / `e = ...;` / `return ...;` statements).
            | Node::Vec2Swizzle(_, _)
            | Node::Vec2Comp(_, _)
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

/// The spelling of an ARRAY-SUBSCRIPT / DYNAMIC-INDEX index node (Increment 5c). The committed
/// body spells a LITERAL index BARE (`cell[0]`, `rd_v[0]`, `s[1]`) — NOT `cell[0u]` — so a
/// [`Node::UintLit`] in an index POSITION drops its `u` suffix (the subscript is `int`-typed by
/// HLSL, the bare literal compiles to the same `OpConstant`). A NON-literal index (the iv `axis` /
/// the `cx`/`cy`/`cz` temp) spells normally via [`operand_str`].
fn array_index_str(arena: &[Node], names: Names, temps: &[Option<String>], idx: u32) -> String {
    match arena[idx as usize] {
        // A literal subscript spells BARE (`0`, not `0u`).
        Node::UintLit(u) => format!("{u}"),
        // The iv (`axis`) / a `uint` temp (`cx`) spells its name.
        _ => operand_str(arena, names, temps, idx, OperandPos::Root),
    }
}

/// The spelling of a `(float)`-cast OPERAND (Increment 5c). A cast binds TIGHTER than `+`/`-`, so an
/// ADDITIVE operand must WRAP — `(float)(c0 + 1)`, NOT `(float)c0 + 1` (the O1 review carry-in,
/// confirmed against the committed L1036). A non-additive operand (a leaf `cx`/`c0`) spells bare.
/// Implemented by spelling at [`OperandPos::MulSide`] (which wraps an additive child like a cast does).
fn cast_operand_str(arena: &[Node], names: Names, temps: &[Option<String>], a: u32) -> String {
    operand_str(arena, names, temps, a, OperandPos::MulSide)
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
        // The B2 interp exact-at-equal keystone's float `==` (`OpFOrdEqual`) — a Mask
        // spelled inline inside the `&&` fold, like the other comparison masks.
        Node::FEq(a, b) => format!("{} == {}", opl(a), opl(b)),
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
        // Track B Increment G2: the `float2` swizzles — a multi-lane `<src>.xy`/`<src>.yx` (the source
        // is an inline leaf — a `VarRef` `n`/`e` — so it needs no wrap) and a single-component
        // `<src>.x`/`<src>.y`. Both spell inline at each use, matching the committed `oct_encode`.
        Node::Vec2Swizzle(v, mask) => format!("{}.{}", opl(v), VEC2_SWIZZLES[mask as usize]),
        Node::Vec2Comp(v, axis) => format!("{}.{}", opl(v), AXIS[axis as usize]),
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
        // ---- Increment 5a inline leaves -----------------------------------------------
        // A SIGNED `int` literal spells the bare signed value (`-1`), NOT the `<x>u` of UintLit.
        Node::IntLit(i) => format!("{i}"),
        // The `(int)L` cast — the operand is the iv node (`L`), an inline leaf (no wrap).
        Node::IntFromUint(u) => format!("(int){}", opl(u)),
        // A push-constant `uint` field spells its bare access text (`pc.brick_levels`).
        Node::PcUint(sym_id) => names.pc_in[sym_id as usize].to_string(),
        // An `M4Level` field read spells `m2_levels[<L>].<field>` (the iv inline + the access text).
        Node::LevelField { iv_id, field_id, .. } => {
            format!("m2_levels[{}].{}", opl(iv_id), names.level_field[field_id as usize])
        }
        // The component-wise vector compares spell `p >= o` / `p < hi` (inside `all(...)`); both
        // operands are inline leaves (whole-`float3` temp refs), so no wrap.
        Node::Bool3Ge(a, b) => format!("{} >= {}", opl(a), opl(b)),
        Node::Bool3Lt(a, b) => format!("{} < {}", opl(a), opl(b)),
        // `all(<bool3>)` — the reduction; its operand is the `Bool3Ge`/`Bool3Lt` inline leaf.
        Node::All3(a) => format!("all({})", opl(a)),
        // ---- Increment 5c inline leaves -----------------------------------------------
        // A named-local-array element spells `<name>[<idx>]` (`cell[axis]`, `s[0]`). The idx is an
        // inline leaf (a `UintLit` `0u`/.. — printed `0`/.. WITHOUT the `u` suffix for the array
        // subscript — or the iv `axis`/`cx`), spelled at Root.
        Node::ArrayElem { arr, idx } => format!("{}[{}]", names.array[arr as usize], array_index_str(arena, names, temps, idx)),
        // The signed-int `<` / `==` masks spell `cell[axis] < 0` / `step[axis] == 0` (inside the
        // DDA-exit `||` condition); both operands are inline leaves (an array element / a literal).
        Node::SLt(a, b) => format!("{} < {}", opl(a), opl(b)),
        Node::SIntEq(a, b) => format!("{} == {}", opl(a), opl(b)),
        // The `(float)`-cast leaves spell `(float)<operand>`. The operand is at a CAST position: an
        // additive operand WRAPS (`(float)(c0 + 1)`, NOT `(float)c0 + 1`), a leaf does not (`(float)cx`).
        Node::FloatFromInt(a) => format!("(float){}", cast_operand_str(arena, names, temps, a)),
        Node::FloatFromUint(a) => format!("(float){}", cast_operand_str(arena, names, temps, a)),
        // The dynamic float3-parameter index spells `<name>[<idx>]` (`rd_v[axis]`, `ro_v[0]`); the idx
        // is an inline leaf (a `UintLit` `0`/.. or the iv `axis`), spelled at Root WITHOUT the `u`.
        Node::Vec3DynIndex { vec_id, idx } => format!("{}[{}]", names.vec_in[vec_id as usize], array_index_str(arena, names, temps, idx)),
        // A resource ref / a by-name array arg spell their NAME (a call-through-only operand).
        Node::ResRef(n) => names.res_in[n as usize].to_string(),
        Node::ArrName(n) => names.array[n as usize].to_string(),
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

/// The `float2`-producing MULTI-LANE swizzle masks, indexed by a [`Node::Vec2Swizzle`] mask id
/// (Track B Increment G2). `0 = "xy"` (the `n.xy` lane-keep), `1 = "yx"` (the `e.yx` lane-swap).
const VEC2_SWIZZLES: [&str; 2] = ["xy", "yx"];

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
        // A unary minus / a ternary always groups (side-independent). The Increment-5c `uint`
        // nested axis-select ([`Node::SelectParenU`]) groups like the other ternaries.
        Node::Neg(..) | Node::Select(..) | Node::SelectParen(..) | Node::SelectParenU(..) => true,
        // Track B Increment G2: the BARE ternary groups ONLY when nested in an infix parent (an
        // additive/multiplicative operand); at Root (the `float2(<a>, <b>)` ctor arg — the ONLY
        // position `oct_encode` uses it) it stays UN-wrapped, matching the committed bare ctor args.
        Node::SelectBare(..) => !matches!(pos, OperandPos::Root),
        // Additive infix nodes (scalar `+`/`-`/`uint +`, the `float3` `+`/`-`, AND the Increment-5c
        // signed-`int` `+` / `uint` `-`): wrap under a multiplicative OR CAST parent (precedence:
        // `(p - origin) / bw`, `(t1 - p[a]) * t3`, `(float)(c0 + 1)`, `W - 2u`'s consumers) or in
        // the additive-RIGHT position (associativity); the additive-LEFT + root positions stay flat
        // (the brick `idx` line's `ix + iy*dims.x + iz*...`). A `cast` operand is spelled at
        // [`OperandPos::MulSide`] ([`cast_operand_str`]), so the `SAdd` of `(float)(c0 + 1)` wraps here.
        Node::Add(..)
        | Node::Sub(..)
        | Node::UAdd(..)
        | Node::Vec3Add(..)
        | Node::Vec3Sub(..)
        | Node::SAdd(..)
        | Node::USub(..)
        // Track B Increment G2: the additive `float2` nodes — the `+ s` broadcast (`... + 0.5`) and
        // the scalar-minus-vec (`1.0 - abs(e.yx)`) — wrap under a multiplicative parent (the
        // `(1.0 - abs(e.yx)) * float2(...)` keystone) or in the additive-RIGHT position, like the
        // scalar/`float3` additive nodes.
        | Node::Vec2AddScalar(..)
        | Node::Vec2RSubScalar(..) => matches!(pos, OperandPos::MulSide | OperandPos::AddRight),
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
        | Node::FEq(_, _)
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
        Node::Sin(a) => {
            chk(a, EmitTy::Float);
            format!("sin({})", op(a))
        }
        Node::Cos(a) => {
            chk(a, EmitTy::Float);
            format!("cos({})", op(a))
        }
        Node::Acos(a) => {
            chk(a, EmitTy::Float);
            format!("acos({})", op(a))
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
        Node::SelectBare(_c, t, e) => {
            // Track B Increment G2: the BARE ternary — NO parens on the condition OR the arms (the
            // committed `oct_encode` sign-ternary `e.x >= 0.0 ? 1.0 : -1.0`). The condition is a Mask
            // leaf (a `Ge` node, spelled `e.x >= 0.0` at Root), the arms are `Lit` leaves (`1.0`/`-1.0`,
            // also Root). DISTINCT from `Select`'s `(cond) ? t : e` (which wraps the condition).
            chk(t, EmitTy::Float);
            chk(e, EmitTy::Float);
            format!("{} ? {} : {}", op(_c), op(t), op(e))
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

        // ---- Increment 5c non-leaf (materialized / composed) nodes ----------------------
        // `max(cell[0], 0)` — the SIGNED-int max (both `int`). The committed body NEVER temps it (it
        // is the inner of `(uint)max(...)`), so it reaches here only via the `(uint)` cast's `op`.
        Node::SMax(a, b) => {
            chk(a, EmitTy::Int);
            chk(b, EmitTy::Int);
            format!("max({}, {})", op(a), op(b))
        }
        // `(uint)max(cell[0], 0)` — the `int -> uint` cast. The operand is the `SMax` (or a `cell[0]`
        // element), checked `Int`.
        Node::UintFromInt(a) => {
            chk(a, EmitTy::Int);
            format!("(uint){}", op(a))
        }
        // `c0 + 1` — the SIGNED-int add (both `int`). The committed `boundary = (float)(c0 + 1);`
        // temps the cast, NOT the add, so the add reaches here as the cast's operand (wrapped by
        // `cast_operand_str`'s MulSide).
        Node::SAdd(a, b) => {
            chk(a, EmitTy::Int);
            chk(b, EmitTy::Int);
            format!("{} + {}", opl(a), opr(b))
        }
        // `W - 2u` / `W - 1u` — the `uint` subtract (both `uint`). The `min(..., W - 2u)` consumer
        // spells it at `MulSide`-or-Root; as a `min` arg it spells at `op` (Root), bare.
        Node::USub(a, b) => {
            chk(a, EmitTy::Uint);
            chk(b, EmitTy::Uint);
            format!("{} - {}", opl(a), opr(b))
        }
        // `min(a, b)` over two `uint`s (`min((uint)max(cell[0], 0), W - 2u)`). The args spell at Root
        // (a `min` intrinsic arg, position-irrelevant); both checked `Uint`.
        Node::UMin(a, b) => {
            chk(a, EmitTy::Uint);
            chk(b, EmitTy::Uint);
            format!("min({}, {})", op(a), op(b))
        }
        // The nested `uint` axis-select `(<c0>) ? 0u : ((<c1>) ? 1u : 2u)`. The arms are `uint`; the
        // condition is a Mask (the `And2` of two `Le`s / a nested `SelectParenU`). The arms spell at
        // Root — the `0u`/`1u`/`2u` leaves need no wrap, and the nested `SelectParenU` self-wraps via
        // `needs_paren_as_operand` (its `Select` family arm), producing `... ? 0u : (... ? 1u : 2u)`.
        Node::SelectParenU(c, t, e) => {
            // `op(e)` spells the else arm at Root: a `0u`/`1u`/`2u` `UintLit` leaf needs no wrap,
            // while a NESTED `SelectParenU` self-wraps (its `Select`-family arm in
            // `needs_paren_as_operand` returns `true` regardless of position) — producing
            // `(c0) ? 0u : ((c1) ? 1u : 2u)`, byte-identical to the committed nested ternary.
            format!("({}) ? {} : {}", op(c), op(t), op(e))
        }
        // `float3(<x>, <y>, <z>)` from three already-`float` scalar expressions (the `lo_g` ctor).
        // Each component is checked `Float`; spelled at Root (a ctor arg, position-irrelevant).
        Node::Vec3FromScalars(x, y, z) => {
            chk(x, EmitTy::Float);
            chk(y, EmitTy::Float);
            chk(z, EmitTy::Float);
            format!("float3({}, {}, {})", op(x), op(y), op(z))
        }
        // `float2(<x>, <y>)` from two already-`float` scalar expressions (the `pack_material_id_ba`
        // return). Each component is checked `Float`; spelled at Root (a ctor arg, position-
        // irrelevant), so `(float)lo / 255.0` spells flat — `(float)lo / 255.0`, not wrapped.
        Node::Vec2FromScalars(x, y) => {
            chk(x, EmitTy::Float);
            chk(y, EmitTy::Float);
            format!("float2({}, {})", op(x), op(y))
        }
        // ---- Track B Increment G2: the `oct_encode` `float2` arithmetic (non-leaf, composed) ----
        // `abs(e.yx)` — component-wise `float2` abs. The operand is the `Vec2Swizzle` `e.yx` (a leaf),
        // spelled at Root (an `abs(...)` intrinsic arg, position-irrelevant).
        Node::Vec2Abs(a) => {
            chk(a, EmitTy::Float2);
            format!("abs({})", op(a))
        }
        // `a * b` over two `float2`s (`(1.0 - abs(e.yx)) * float2(...)`). Both operands are the
        // multiplicative side: the LEFT (`Vec2RSubScalar`, an additive node) WRAPS via
        // `needs_paren_as_operand`'s MulSide arm → `(1.0 - abs(e.yx))`; the RIGHT (`Vec2FromScalars`,
        // a ctor) needs no wrap.
        Node::Vec2Mul(a, b) => {
            chk(a, EmitTy::Float2);
            chk(b, EmitTy::Float2);
            format!("{} * {}", opm(a), opm(b))
        }
        // `v * s` — `float2 * float` (`e * 0.5`). The vector is the multiplicative side, the scalar a
        // `float`. (`e` is a `VarRef` leaf, `0.5` a `Lit` leaf — neither wraps.)
        Node::Vec2MulScalar(v, s) => {
            chk(v, EmitTy::Float2);
            chk(s, EmitTy::Float);
            format!("{} * {}", opm(v), opm(s))
        }
        // `v + s` — `float2 + float` broadcast (`(e * 0.5) + 0.5`). The vector is the additive-LEFT
        // operand (`e * 0.5`, a `Vec2MulScalar` — multiplicative, no wrap), the scalar the additive
        // RIGHT (`0.5`, a `Lit` leaf). Spells flat `e * 0.5 + 0.5`, byte-identical to the committed.
        Node::Vec2AddScalar(v, s) => {
            chk(v, EmitTy::Float2);
            chk(s, EmitTy::Float);
            format!("{} + {}", opl(v), opr(s))
        }
        // `s - v` — `float - float2`, scalar on the LEFT (`1.0 - abs(e.yx)`). The scalar is the
        // additive-LEFT (`1.0`, a `Lit` leaf), the `float2` the additive-RIGHT (`abs(e.yx)`, a
        // `Vec2Abs` — a function-call form, no wrap). Spells `1.0 - abs(e.yx)`.
        Node::Vec2RSubScalar(s, v) => {
            chk(s, EmitTy::Float);
            chk(v, EmitTy::Float2);
            format!("{} - {}", opl(s), opr(v))
        }
        // A variadic heterogeneous call `sym(op(a0), op(a1), ...)` — `m2_corner(atlas, atlas_smp,
        // tile_org, cx, cy, cz, inv_atlas, band_half)` / `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)` /
        // `m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo)`. The args slice the `CALL_ARGS` side-table;
        // each spells at Root (a call arg, position-irrelevant). The args' types are heterogeneous
        // (resource / array / float3 / float), so they are NOT `chk`'d here (a `ResRef`/`ArrName`
        // types `Float` by default — never an arithmetic operand).
        Node::CallN { sym_id, arg_lo, arg_count, .. } => {
            let args = CALL_ARGS.with(|c| {
                let c = c.borrow();
                c[arg_lo as usize..(arg_lo + arg_count) as usize].to_vec()
            });
            let spelled: Vec<String> = args.iter().map(|&a| op(a)).collect();
            format!("{}({})", names.call_in[sym_id as usize], spelled.join(", "))
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
        | Node::Vec4Param(_)
        // The Increment-5a inline leaves (the signed-int return values, the push-constant field,
        // the `all(...)` reduction + its component-wise operands, the level-field access text) are
        // ALL spelled by `operand_str`, never materialized as a temp.
        | Node::IntLit(_)
        | Node::IntFromUint(_)
        | Node::PcUint(_)
        | Node::LevelField { .. }
        | Node::Bool3Ge(_, _)
        | Node::Bool3Lt(_, _)
        | Node::All3(_)
        // The Increment-5c inline leaves (a named-array element, the signed `<`/`==` masks, the
        // `(float)`-cast leaves, the dynamic float3 index, the resource refs, the by-name array arg)
        // are ALL spelled by `operand_str`, never materialized as a temp.
        | Node::ArrayElem { .. }
        | Node::SLt(_, _)
        | Node::SIntEq(_, _)
        | Node::FloatFromInt(_)
        | Node::FloatFromUint(_)
        | Node::Vec3DynIndex { .. }
        | Node::ResRef(_)
        | Node::ArrName(_)
        // The Track B Increment G2 `float2` swizzles (`n.xy`/`e.yx`/`e.x`/`e.y`) are inline leaves
        // (spelled by `operand_str`), never materialized as a temp.
        | Node::Vec2Swizzle(_, _)
        | Node::Vec2Comp(_, _) => {
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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

/// Like [`emit_body`], but the TWELVE `roots` (the 3×4 ROW-MAJOR model affine) are
/// spelled as three `InterpModel` struct-field assignments returning the struct — the
/// B2 `interp_trs` output. The shared temp-emission walk ([`emit_temps`]) computes the
/// slerp / compose subtrees ONCE; each root operand inlines its already-emitted `tN`
/// temp into the `float4(...)` row constructor.
fn emit_body_rows12(arena: &[Node], names: Names, roots: [u32; 12]) -> String {
    let (mut out, temps) = emit_temps(arena, names);
    let r = |id: u32| operand_str(arena, names, &temps, id, OperandPos::Root);
    out.push_str("    InterpModel m;\n");
    out.push_str(&format!(
        "    m.row0 = float4({}, {}, {}, {});\n",
        r(roots[0]),
        r(roots[1]),
        r(roots[2]),
        r(roots[3])
    ));
    out.push_str(&format!(
        "    m.row1 = float4({}, {}, {}, {});\n",
        r(roots[4]),
        r(roots[5]),
        r(roots[6]),
        r(roots[7])
    ));
    out.push_str(&format!(
        "    m.row2 = float4({}, {}, {}, {});\n",
        r(roots[8]),
        r(roots[9]),
        r(roots[10]),
        r(roots[11])
    ));
    out.push_str("    return m;\n");
    out
}

/// The 20 `float`-input names for `interp_trs`, in the seed order the interp body reads
/// them: `prev` TRS (pos.xyz, rot.xyzw, scale.xyz) then `curr` TRS, spelled as the HLSL
/// struct-member accessors of the `TransformPair` parameter so the generated body
/// references `pair.prev.pos.x` etc. directly. `alpha` is seeded last (index 20).
const INTERP_INPUT_NAMES: &[&str] = &[
    "pair.prev.pos.x",
    "pair.prev.pos.y",
    "pair.prev.pos.z",
    "pair.prev.rot.x",
    "pair.prev.rot.y",
    "pair.prev.rot.z",
    "pair.prev.rot.w",
    "pair.prev.scale.x",
    "pair.prev.scale.y",
    "pair.prev.scale.z",
    "pair.curr.pos.x",
    "pair.curr.pos.y",
    "pair.curr.pos.z",
    "pair.curr.rot.x",
    "pair.curr.rot.y",
    "pair.curr.rot.z",
    "pair.curr.rot.w",
    "pair.curr.scale.x",
    "pair.curr.scale.y",
    "pair.curr.scale.z",
    "alpha",
];

/// Generates the HLSL `interp_trs` body — the per-instance TRS interpolation + 3×4
/// model-affine compose (Pillar B increment B2) — by tracing the generic
/// [`crate::interp::transform_pair_interp_body`] over the [`Emit`] backend, and returns
/// the FULL `InterpModel interp_trs(TransformPair pair, float alpha) { ... }` function.
///
/// A struct-returning function (not `out` params) because a NEW shader can define the
/// cleanest shape: the caller writes the returned `InterpModel` (three `float4` rows)
/// straight into the `RWStructuredBuffer<InstanceModelCol>` record. The generated body
/// is spliced between the `// === GENERATED interp_trs BEGIN/END ===` sentinels in
/// `crates/boyko_rhi_vulkan/shaders/interp_instances.comp.hlsl`; the
/// `interp_edsl_sync` test pins the committed shader to this output AND re-DXCs the
/// whole file to the committed `.spv`.
///
/// The `f32` Eval instantiation of the SAME body is the CPU oracle whose composed rows
/// byte-match `boyko_render::InstanceModelCol::from_global` for the interpolated TRS
/// (proven by the `boyko_shaderdsl` eval-mirror tests).
pub fn emit_hlsl_transform_interp() -> String {
    use crate::interp;

    let names = Names {
        float_in: INTERP_INPUT_NAMES,
        uint_in: NO_UINT_INPUTS,
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };
    ARENA.with(|a| a.borrow_mut().clear());
    let ins: Vec<Emit> = (0..INTERP_INPUT_NAMES.len())
        .map(|i| Emit::input(i as u32))
        .collect();
    let rows = interp::transform_pair_interp_body::<Emit>(
        [ins[0], ins[1], ins[2]],
        [ins[3], ins[4], ins[5], ins[6]],
        [ins[7], ins[8], ins[9]],
        [ins[10], ins[11], ins[12]],
        [ins[13], ins[14], ins[15], ins[16]],
        [ins[17], ins[18], ins[19]],
        ins[20],
    );
    let roots: [u32; 12] = core::array::from_fn(|i| rows[i].0);
    let body = ARENA.with(|a| {
        let a = a.borrow();
        emit_body_rows12(&a, names, roots)
    });
    format!("InterpModel interp_trs(TransformPair pair, float alpha) {{\n{body}}}\n")
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

    // ---- Increment 5c: the named-local-array statements (`m2_brick_cubic_hit`) ----
    /// `<elem_ty> <name>[<len>];` — an UNINITIALIZED named-local-array declaration (`int cell[3];`,
    /// `float s[8];`). `arr` indexes the [`Names::array`] name table; `elem_ty` the element type;
    /// `len` the array length. NO initializer (the committed body declares the arrays empty and
    /// fills them in the `[unroll]` setup loop). Recorded by [`EmitCf::decl_array_int`] /
    /// [`EmitCf::decl_array_float`].
    DeclArray { arr: u32, elem_ty: EmitTy, len: u32 },
    /// `<name>[<idx>] = <rhs>;` — a named-local-array element STORE (`cell[axis] = c0;`, `s[0] =
    /// <call>;`). `arr` indexes [`Names::array`]; `idx` the index node id; `rhs` the value node id.
    /// Recorded by [`EmitCf::arr_int_set`] / [`EmitCf::arr_float_set`].
    ArrayStore { arr: u32, idx: u32, rhs: u32 },
    /// `<name>[<idx>] += <rhs>;` — a named-local-array element COMPOUND-ADD (`cell[axis] +=
    /// step[axis];`, `t_next[axis] += t_delta[axis];`). The `+=` TOKEN is recorded as a DISTINCT
    /// statement (NOT desugared to `cell[axis] = cell[axis] + step[axis]`): the spike (R1) proved
    /// the `= +` form computes the access-chain TWICE at `-O0`, so it is NOT byte-identical. Recorded
    /// by [`EmitCf::arr_int_add_assign`] / [`EmitCf::arr_float_add_assign`].
    ArrayAddAssign { arr: u32, idx: u32, rhs: u32 },
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

/// The SIGNED-`int` RETURN-VALUE cell handle on the Emit backend — a ZST (the signed value travels
/// in the recorded [`Stmt::Return`] as a [`Node::IntLit`] / [`Node::IntFromUint`], not in a cell).
/// The [`EmitCf`]'s [`Cf::RetCellI`] associated type (Increment 5a). Distinct type from
/// [`RetCellB`] / [`RetCellF`] / [`RetCell`] only so the int / bool / float / uint return facets
/// stay separate at the call site.
#[derive(Clone, Copy)]
pub struct RetCellI;

/// The `float2` RETURN-VALUE cell handle on the Emit backend — a ZST (the `float2(...)` ctor travels
/// in the recorded [`Stmt::Return`] as a [`Node::Vec2FromScalars`], not in a cell). The [`EmitCf`]'s
/// [`Cf::RetCellV2`] associated type (Track B Increment G1). Distinct type from [`RetCellI`] /
/// [`RetCellB`] / [`RetCellF`] / [`RetCell`] only so the `float2` / int / bool / float / uint return
/// facets stay separate at the call site; the recorded `Stmt::Return` is identical.
#[derive(Clone, Copy)]
pub struct RetCellV2;

/// A NAMED-LOCAL-ARRAY name handle (Increment 5c) — indexes the printer's [`Names::array`] table.
/// The [`EmitCf`]'s [`Cf::IntArr`] AND [`Cf::FloatArr`] associated types (both are a `u32` array-id;
/// the element type is carried out-of-band in [`ARRAY_ELEM_TYS`], so one handle type serves both —
/// the `int`/`float` distinction lives in [`Stmt::DeclArray`]'s `elem_ty` + the [`ARRAY_ELEM_TYS`]
/// entry, NOT in the handle).
#[derive(Clone, Copy)]
pub struct ArrName(u32);

/// A RESOURCE-PARAMETER name handle (Increment 5c) — indexes the printer's [`Names::res_in`] table.
/// The [`EmitCf`]'s [`Cf::ResTok`] associated type (`atlas` / `atlas_smp`). A call-through-only
/// operand (consumed by a [`Node::CallN`] `m2_corner` arg).
#[derive(Clone, Copy)]
pub struct ResTok(u32);

thread_local! {
    /// The STMT block stack: the recorder pushes a [`Block`] on combinator entry
    /// (`unroll_for` / `if_`) and pops it into its parent on exit, so the top is always
    /// the block currently being recorded. The bottom (index 0) is the function body.
    static STMTS: RefCell<Vec<Block>> = const { RefCell::new(Vec::new()) };

    /// The per-emit mutable-local names (`exit`), indexed by [`Var`] / [`Node::VarRef`].
    static VARS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// OUT-OF-BAND mutable-local result TYPES (Track B Increment G2) — a [`Var`]'s declared
    /// [`EmitTy`], keyed by the var's [`VARS`] index (the index IS the [`Var`] id, pushed in
    /// declaration order). [`type_of`]`(`[`Node::VarRef`]`(id))` reads it so a `float3`/`float2`
    /// mutable local (`oct_encode`'s `n`/`e`) types its READS correctly (the consuming `vec3_div_scalar`
    /// / `vec2_*` ops `chk` a `Float3`/`Float2` operand). Kept off the frozen [`Node::VarRef`]`(u32)`
    /// node (the same out-of-band rationale [`TEMP_TYPES`] uses). Every PRE-G2 var is a `float`/`bool`
    /// scalar, so this table is byte-neutral for them — a `VarRef` past the recorded set (or a leaf that
    /// never seeded it) defaults to [`EmitTy::Float`], the prior behavior.
    static VAR_TYPES: RefCell<Vec<EmitTy>> = const { RefCell::new(Vec::new()) };

    /// The per-emit named-literal symbols (`BRICK_EXIT_EPS`), indexed by
    /// [`Node::NamedLit`]'s `sym_id`. Deduped so repeated `named_lit("BRICK_EXIT_EPS", _)`
    /// calls share one id.
    static NAMED_LITS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// The per-emit CALLEE names (`m2_cubic_eval`), indexed by [`Node::Call2`]'s `sym_id`
    /// (Increment 4a). Deduped so repeated `call2("m2_cubic_eval", ...)` calls share one id.
    static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// The per-emit PUSH-CONSTANT field-text (`pc.brick_levels`), indexed by [`Node::PcUint`]'s
    /// `sym_id` (Increment 5a). Deduped so repeated reads share one id.
    static PC_FIELDS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// The per-emit `M4Level` ACCESS-TEXT (`origin_brick_world.xyz`, ...), indexed by
    /// [`Node::LevelField`]'s `field_id` (Increment 5a). Deduped so repeated reads of the same
    /// member+swizzle share one id.
    static LEVEL_FIELDS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

    /// The per-emit NAMED-LOCAL-ARRAY names (`cell`/`step`/`t_next`/`t_delta`/`s`), indexed by
    /// [`Node::ArrayElem`]'s `arr` / [`Node::ArrName`]'s id / [`Stmt::DeclArray`]'s `arr` (Increment
    /// 5c). Pushed in `decl_array_*` order (the index IS the array id), so the index aligns with the
    /// matching [`ARRAY_ELEM_TYS`] entry.
    static ARRAY_NAMES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

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

/// Seeds a mutable local: pushes `name` into [`VARS`] and `ty` into [`VAR_TYPES`] IN LOCKSTEP (the
/// shared index is the [`Var`] id), returning the handle. The single chokepoint that keeps the two
/// tables aligned, so [`var_type`]`(`[`Node::VarRef`]`(id))` reads the var's declared type. Used by
/// both the DECLARED-local path ([`record_decl_var`]) and the SUPPRESSED-DECL param paths
/// ([`EmitCf::decl_param`] / [`EmitCf::decl_param_vec3`]) — the latter records NO `Stmt::DeclVar` but
/// must still register the var's name+type.
fn push_var(name: &'static str, ty: EmitTy) -> Var {
    VARS.with(|v| {
        let mut v = v.borrow_mut();
        let id = v.len() as u32;
        v.push(name);
        VAR_TYPES.with(|t| {
            let mut t = t.borrow_mut();
            debug_assert_eq!(t.len() as u32, id, "VAR_TYPES must stay var-id-indexed (== VARS len)");
            t.push(ty);
        });
        Var(id)
    })
}

/// Records a mutable-local DECLARATION of an arbitrary [`EmitTy`]: seeds the `name`+`ty` in the
/// [`VARS`]/[`VAR_TYPES`] tables (so [`Node::VarRef`] later spells `name` and types as `ty`) and
/// records the `Stmt::DeclVar` with the given `ty` + `rhs`. Shared by [`EmitCf::decl_var`]
/// (`EmitTy::Float`), [`EmitCf::decl_bool_var`] (`EmitTy::Bool`), and [`EmitCf::decl_var_vec2`]
/// (`EmitTy::Float2`); a future `decl_uint_var`/`decl_int_var` is a trivial mirror (pass its own
/// `ty`). Threading the `ty` (vs the old hardcoded `EmitTy::Float`) is the Increment-4d generalization
/// — the `float` path is byte-unchanged (it still passes `EmitTy::Float`).
fn record_decl_var(name: &'static str, ty: EmitTy, rhs: u32) -> Var {
    let var = push_var(name, ty);
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

/// Records a named-local-array DECLARATION (Increment 5c): seeds the `name` in the [`ARRAY_NAMES`]
/// table (so [`Node::ArrayElem`]/[`Node::ArrName`] later spell `name`) + the element type in
/// [`ARRAY_ELEM_TYS`] (keyed by the array id, so [`array_elem_ty`] reads it), and records the
/// `Stmt::DeclArray`. Returns the [`ArrName`] handle. The index pushed into both tables is the
/// SAME (they grow in lockstep), so `array_elem_ty(arr)` aligns with `ARRAY_NAMES[arr]`.
fn record_decl_array(name: &'static str, elem_ty: EmitTy, len: u32) -> ArrName {
    let arr = ARRAY_NAMES.with(|a| {
        let mut a = a.borrow_mut();
        let id = a.len() as u32;
        a.push(name);
        id
    });
    ARRAY_ELEM_TYS.with(|t| {
        let mut t = t.borrow_mut();
        debug_assert_eq!(t.len() as u32, arr, "ARRAY_ELEM_TYS must stay array-id-indexed");
        t.push(elem_ty);
    });
    record_stmt(Stmt::DeclArray { arr, elem_ty, len });
    ArrName(arr)
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

/// Registers a push-constant field-text, returning its (deduped) `sym_id` (Increment 5a).
fn intern_pc_field(field: &'static str) -> u32 {
    PC_FIELDS.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(i) = p.iter().position(|&s| s == field) {
            i as u32
        } else {
            let id = p.len() as u32;
            p.push(field);
            id
        }
    })
}

/// Registers an `M4Level` access-text (the member+swizzle), returning its (deduped) `field_id`
/// (Increment 5a).
fn intern_level_field(field: &'static str) -> u32 {
    LEVEL_FIELDS.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(i) = l.iter().position(|&s| s == field) {
            i as u32
        } else {
            let id = l.len() as u32;
            l.push(field);
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
        // SUPPRESSED-DECL: seed a `VARS`/`VAR_TYPES` name+type entry (so get_var/set_var spell `hi`, `hi
        // = ...;` and type the read `float`) but record NO `Stmt::DeclVar` — `lo`/`hi`/`f_lo`/`f_hi` are
        // HLSL SIGNATURE parameters, so a `float hi = ...;` redecl would diverge the committed text.
        // `_init` (the param's symbolic seed) is unused: a parameter is already bound by name.
        push_var(name, EmitTy::Float)
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

    // ---- Increment 5b: the COMPUTED-bool return facet (recorder) ----------------------

    fn ret_b_expr(_cell: &RetCellB, value: EmitMask) -> Flow {
        // The computed-bool return — a single `Stmt::Return` carrying the MASK node (`tmax > tmin`,
        // a `Gt` node printed inline by the `Stmt::Return` printer's `inline_expr`), NOT a `BoolLit`.
        // The function-tail `return tmax > tmin;`. Fall through on Emit.
        record_stmt(Stmt::Return(value.0));
        Flow::Continue(())
    }

    // ---- Increment 4e: the BOOL mutable-local facets (recorder) -----------------------

    fn decl_bool_param(name: &'static str, _init: bool) -> Var {
        // SUPPRESSED-DECL (bool): seed a `VARS`/`VAR_TYPES` name+type entry (so set_bool_var/
        // get_bool_var spell `hit`, `hit = ...;`) but record NO `Stmt::DeclVar` — `hit` is declared
        // by the hand-written re-march preamble (`hit = false;`), so a `bool hit = false;` redecl
        // would diverge the committed text. The bool mirror of `decl_param` (the `float` suppressed
        // decl); `_init` is unused (a suppressed local is already bound by name).
        //
        // Routed through `push_var` with `EmitTy::Bool` (vs the old direct `VARS.push`, which left
        // VAR_TYPES short by one entry, relying on the unstated "the bool is always the LAST decl"
        // invariant). BYTE-NEUTRAL: a bool var's `VarRef` is never `type_of`'d (`get_bool_var`
        // records no `VarRef`), so the `EmitTy::Bool` entry is never read — it only keeps the
        // VARS/VAR_TYPES tables aligned UNCONDITIONALLY, so `push_var`'s `debug_assert(t.len() ==
        // id)` stays satisfiable if a FUTURE producer routes a `push_var`-backed var (a `decl_param`/
        // `decl_var`, NOT a `temp_*` which uses the separate TEMP_TYPES table) through `push_var`
        // AFTER a bool decl.
        push_var(name, EmitTy::Bool)
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

    // ---- Increment 5a: the SIGNED-INT subsystem + M4Level access-text (recorder) ------
    // On Emit the signed-int value is an `Emit` SSA-node handle (typed `int` per-node via
    // `type_of`); the ret-cell is a ZST (the value travels in the recorded `Stmt::Return`).
    type Int = Emit;
    type RetCellI = RetCellI;

    fn iv_uint(iv: Emit) -> Emit {
        // The iv SSA node (a `UintInput` printing `L`) IS already typed `uint`, so the iv-as-value
        // read is identity — the same handle, spelling `L` at every use (`L >= pc.brick_levels`,
        // `(int)L`).
        iv
    }

    fn int_lit_signed(x: i32) -> Emit {
        // A SIGNED `int` literal (`-1`) — the `IntLit` node (printed bare `-1`, an inline leaf typed
        // `Int`). DISTINCT from `uint_lit`'s `UintLit` (printed `<x>u`).
        Emit(push(Node::IntLit(x)))
    }

    fn int_from_uint(u: Emit) -> Emit {
        // `(int)L` — the `IntFromUint` node (printed `(int)<operand>`, an inline leaf typed `Int`).
        // The operand is the loop iv node (`L`).
        Emit(push(Node::IntFromUint(u.0)))
    }

    fn all3_ge(p: Emit, o: Emit) -> EmitMask {
        // `all(p >= o)` — an `All3` over a `Bool3Ge` (a component-wise `float3` `>=`). The mask is
        // consumed only inside the `&&` condition (an inline leaf), never `chk`-typed.
        let cmp = push(Node::Bool3Ge(p.0, o.0));
        EmitMask(push(Node::All3(cmp)))
    }

    fn all3_lt(p: Emit, hi: Emit) -> EmitMask {
        // `all(p < hi)` — the upper-corner analogue (an `All3` over a `Bool3Lt`).
        let cmp = push(Node::Bool3Lt(p.0, hi.0));
        EmitMask(push(Node::All3(cmp)))
    }

    fn pc_uint(field: &'static str) -> Emit {
        // A push-constant `uint` field read by BARE TEXT (`pc.brick_levels`) — a `PcUint` node
        // (printed by `pc_in[sym_id]`, an inline leaf typed `Uint`). The field text interns into
        // the per-emit `PC_FIELDS` table.
        let sym_id = intern_pc_field(field);
        Emit(push(Node::PcUint(sym_id)))
    }

    fn level_field_vec3(l: Emit, field: &'static str) -> Emit {
        // `m2_levels[<L>].<field>` (`.xyz` swizzle) — a `LevelField` node typed `Float3`. The iv
        // handle's id carries `L`'s spelling; the access text interns into `LEVEL_FIELDS`.
        let field_id = intern_level_field(field);
        Emit(push(Node::LevelField {
            iv_id: l.0,
            field_id,
            is_vec3: true,
        }))
    }

    fn level_field_scalar(l: Emit, field: &'static str) -> Emit {
        // `m2_levels[<L>].<field>` (`.w` swizzle) — a `LevelField` node typed `Float`.
        let field_id = intern_level_field(field);
        Emit(push(Node::LevelField {
            iv_id: l.0,
            field_id,
            is_vec3: false,
        }))
    }

    fn if_ret_i(_cell: &RetCellI, cond: EmitMask, value: Emit) -> Flow {
        // `if (<cond>) { return <value>; }` — the then-block is EXACTLY ONE `Stmt::Return` (the
        // signed-int early-return guard; identical recorded shape to `if_ret_f`'s float guard).
        STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));
        record_stmt(Stmt::Return(value.0));
        let then = STMTS.with(|s| {
            s.borrow_mut()
                .pop()
                .expect("invariant: the if_ret_i then block was pushed above")
        });
        record_stmt(Stmt::If {
            cond: cond.0,
            then,
        });
        Flow::Continue(())
    }

    fn ret_i(_cell: &RetCellI, value: Emit) -> Flow {
        // The signed-int return — a single `Stmt::Return(value)` (the tail `return -1;`). Fall
        // through on Emit.
        record_stmt(Stmt::Return(value.0));
        Flow::Continue(())
    }

    // ---- Increment 5c: the DDA marcher subsystem (recorder) ---------------------------
    // On Emit the array / resource handles are NAME handles (`u32` indexing `ARRAY_NAMES` /
    // `Names::res_in`); the float4 value is an `Emit` SSA-node handle (the SAME `Vec4f = Emit` the
    // regula-falsi `c` uses). Every value is an `Emit`/`Var` node handle.
    type IntArr = ArrName;
    type FloatArr = ArrName;
    type ResTok = ResTok;

    fn decl_array_int(name: &'static str, len: u32) -> ArrName {
        // `int <name>[<len>];` — an UNINITIALIZED `int` array. Seeds the name + the `int` element
        // type (so `ArrayElem{arr}` types `int`), records the `Stmt::DeclArray`.
        record_decl_array(name, EmitTy::Int, len)
    }
    fn decl_array_float(name: &'static str, len: u32) -> ArrName {
        // `float <name>[<len>];` — an UNINITIALIZED `float` array.
        record_decl_array(name, EmitTy::Float, len)
    }

    fn arr_int_get(a: ArrName, idx: Emit) -> Emit {
        // `<name>[<idx>]` — an `int`-array element read (an inline `ArrayElem` leaf).
        Emit(push(Node::ArrayElem { arr: a.0, idx: idx.0 }))
    }
    fn arr_float_get(a: ArrName, idx: Emit) -> Emit {
        // `<name>[<idx>]` — a `float`-array element read.
        Emit(push(Node::ArrayElem { arr: a.0, idx: idx.0 }))
    }

    fn arr_int_set(a: ArrName, idx: Emit, v: Emit) {
        record_stmt(Stmt::ArrayStore {
            arr: a.0,
            idx: idx.0,
            rhs: v.0,
        });
    }
    fn arr_float_set(a: ArrName, idx: Emit, v: Emit) {
        record_stmt(Stmt::ArrayStore {
            arr: a.0,
            idx: idx.0,
            rhs: v.0,
        });
    }

    fn arr_int_add_assign(a: ArrName, idx: Emit, v: Emit) {
        // `<name>[<idx>] += <v>;` — the `+=` TOKEN (one access-chain — the R1 finding; NOT desugared).
        record_stmt(Stmt::ArrayAddAssign {
            arr: a.0,
            idx: idx.0,
            rhs: v.0,
        });
    }
    fn arr_float_add_assign(a: ArrName, idx: Emit, v: Emit) {
        record_stmt(Stmt::ArrayAddAssign {
            arr: a.0,
            idx: idx.0,
            rhs: v.0,
        });
    }

    fn call_corner(
        fn_sym: &'static str,
        atlas: ResTok,
        smp: ResTok,
        tile_org: Emit,
        cx: Emit,
        cy: Emit,
        cz: Emit,
        inv_atlas: Emit,
        band_half: Emit,
    ) -> Emit {
        // `m2_corner(atlas, atlas_smp, tile_org, cx, cy, cz, inv_atlas, band_half)` — the 8-arg
        // resource-bearing corner fetch. The resource refs are `ResRef` nodes; the rest are SSA
        // handles. The args go into the flat `CALL_ARGS` side-table; the `CallN` returns a `float`.
        let sym_id = intern_call(fn_sym);
        let atlas_n = push(Node::ResRef(atlas.0));
        let smp_n = push(Node::ResRef(smp.0));
        let (arg_lo, arg_count) = record_call_args(&[
            atlas_n, smp_n, tile_org.0, cx.0, cy.0, cz.0, inv_atlas.0, band_half.0,
        ]);
        Emit(push(Node::CallN {
            sym_id,
            arg_lo,
            arg_count,
            ret: EmitTy::Float,
        }))
    }

    fn call_coeffs(fn_sym: &'static str, s: ArrName, lo_g: Emit, rd_v: Emit) -> Emit {
        // `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)` — the by-name array arg `s` (an `ArrName` node),
        // `lo_g`/`rd_v` `float3` handles. Returns a `float4`.
        let sym_id = intern_call(fn_sym);
        let s_n = push(Node::ArrName(s.0));
        let (arg_lo, arg_count) = record_call_args(&[s_n, lo_g.0, rd_v.0]);
        Emit(push(Node::CallN {
            sym_id,
            arg_lo,
            arg_count,
            ret: EmitTy::Float4,
        }))
    }

    fn call_marmitt(fn_sym: &'static str, coeffs: Emit, a: Emit, b: Emit) -> Emit {
        // `m2_marmitt_root(coeffs, 0.0, seg_hi - seg_lo)` — a `float4` arg + two `float`s. Returns
        // a `float`.
        let sym_id = intern_call(fn_sym);
        let (arg_lo, arg_count) = record_call_args(&[coeffs.0, a.0, b.0]);
        Emit(push(Node::CallN {
            sym_id,
            arg_lo,
            arg_count,
            ret: EmitTy::Float,
        }))
    }

    fn call_clamp_index_int(fn_sym: &'static str, g: Emit) -> Emit {
        // `(int)m2_clamp_index(g_entry)` — a 1-arg `float -> uint` frozen call, immediately
        // `(int)`-cast. The inner call is a `CallN` (a `uint`-result call — NOT the `Float3`-arg
        // `Call1`, whose `chk` would reject the `float` `g_entry`), wrapped in an `IntFromUint` cast
        // node (printed `(int)m2_clamp_index(g_entry)`, the SAME `(int)` cast spelling Inc 5a's iv
        // cast uses). One `int` temp materializes at the body's `int c0 = ...;`.
        let sym_id = intern_call(fn_sym);
        let (arg_lo, arg_count) = record_call_args(&[g.0]);
        let call = push(Node::CallN {
            sym_id,
            arg_lo,
            arg_count,
            ret: EmitTy::Uint,
        });
        Emit(push(Node::IntFromUint(call)))
    }

    fn smax(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::SMax(a.0, b.0)))
    }
    fn uint_from_int(a: Emit) -> Emit {
        Emit(push(Node::UintFromInt(a.0)))
    }
    fn slt(a: Emit, b: Emit) -> EmitMask {
        EmitMask(push(Node::SLt(a.0, b.0)))
    }
    fn sadd(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::SAdd(a.0, b.0)))
    }
    fn float_from_int(a: Emit) -> Emit {
        Emit(push(Node::FloatFromInt(a.0)))
    }
    fn float_from_uint(a: Emit) -> Emit {
        Emit(push(Node::FloatFromUint(a.0)))
    }
    fn usub(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::USub(a.0, b.0)))
    }
    fn umin(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::UMin(a.0, b.0)))
    }
    fn sint_eq(a: Emit, b: Emit) -> EmitMask {
        EmitMask(push(Node::SIntEq(a.0, b.0)))
    }
    fn temp_int(name: &'static str, x: Emit) -> Emit {
        // A NAMED `int` temp (`int c0 = ...;`).
        record_temp(Some(name), EmitTy::Int, x)
    }

    fn captured_uint(name: &'static str) -> Emit {
        // A captured `uint` read by bare NAME (`W`) — interned into the per-emit `PC_FIELDS` table
        // (the SAME bare-text table `pc_uint` uses; the printer spells the bare `field` text). The
        // node is a `PcUint` (an inline leaf typed `Uint`), so `W - 2u` types correctly.
        let sym_id = intern_pc_field(name);
        Emit(push(Node::PcUint(sym_id)))
    }

    fn select_uint(cond: EmitMask, t: Emit, e: Emit) -> Emit {
        // The nested `uint` axis-select — a `SelectParenU` node (the condition `(...)`-wrapped, the
        // else arm self-wraps when itself a select). DISTINCT from `select`'s FLOAT `SelectParen`.
        Emit(push(Node::SelectParenU(cond.0, t.0, e.0)))
    }

    fn vec3_dyn_index(v: Emit, idx: Emit) -> Emit {
        // `<vec>[<idx>]` — a dynamic index of a WHOLE `Vec3Param`. The `v` handle is a `Vec3Param`
        // node; read its vec id off it (like `index()` reads a `VecParamRef`), so `rd_v[axis]` /
        // `ro_v[0]` print the parameter NAME + the index.
        let vec_id = ARENA.with(|a| match a.borrow()[v.0 as usize] {
            Node::Vec3Param(id) => id,
            other => unreachable!("vec3_dyn_index expected a Vec3Param parameter, got {other:?}"),
        });
        Emit(push(Node::Vec3DynIndex {
            vec_id,
            idx: idx.0,
        }))
    }

    fn vec3_from_scalars(x: Emit, y: Emit, z: Emit) -> Emit {
        Emit(push(Node::Vec3FromScalars(x.0, y.0, z.0)))
    }

    fn temp_vec4(name: &'static str, v: Emit) -> Emit {
        // A NAMED `float4` temp (`float4 coeffs = ...;`).
        record_temp(Some(name), EmitTy::Float4, v)
    }

    // ---- Track B Increment G1: the `float2` axis + bitwise `uint` `&`/`>>` (recorder) ----
    // On Emit the `float2` value is an `Emit` SSA-node handle (a `Vec2FromScalars` typed `Float2` via
    // `type_of`); the ret-cell is a ZST (the value travels in the recorded `Stmt::Return`).
    type Vec2f = Emit;
    type RetCellV2 = RetCellV2;

    fn and_u(a: Emit, b: Emit) -> Emit {
        // The bitwise AND (`id & 255u`) — the (previously dead) `And` node, printed UNPARENTHESIZED
        // (`{} & {}`). DISTINCT from `and2`'s `And2` (logical `&&`): this is `&` over two `uint`s,
        // result-typed `Uint`.
        Emit(push(Node::And(a.0, b.0)))
    }

    fn shr_u(a: Emit, b: Emit) -> Emit {
        // The logical right shift (`id >> 8u`) — the (previously dead) `Shr` node, printed
        // UNPARENTHESIZED (`{} >> {}`). The `id >> 8u & 255u` precedence is correct unparenthesized
        // (`>>` binds tighter than `&`).
        Emit(push(Node::Shr(a.0, b.0)))
    }

    fn vec2_from_scalars(x: Emit, y: Emit) -> Emit {
        // `float2(<x>, <y>)` — a `Vec2FromScalars` node typed `Float2`. NEVER materialized as a temp;
        // composed inline by the `Stmt::Return` printer (the committed body returns the ctor directly).
        Emit(push(Node::Vec2FromScalars(x.0, y.0)))
    }

    fn ret_vec2(_cell: &RetCellV2, value: Emit) -> Flow {
        // The `float2` return — a single `Stmt::Return(value)` carrying the `Vec2FromScalars` node
        // (the tail `return float2(...);`; the hand-written `float2 pack_material_id_ba` signature
        // supplies the return type). Fall through on Emit.
        record_stmt(Stmt::Return(value.0));
        Flow::Continue(())
    }

    // ---- Track B Increment G2: the `oct_encode` octahedral encoder (recorder) ----
    // On Emit a mutable `float3`/`float2` local is a NAMED `Var` handle (the SAME shape `Var` uses);
    // the `float2` value is an `Emit` SSA-node handle.
    type Vec3Var = Var;
    type Vec2Var = Var;

    fn decl_param_vec3(name: &'static str, _init: Emit) -> Var {
        // SUPPRESSED-DECL: seed a `VARS`/`VAR_TYPES` name+type entry (so get/set_var_vec3 spell `n`, `n
        // = ...;` and type the read `Float3` — the `n.x`/`n.xy` consumers `chk` a `Float3` operand) but
        // record NO `Stmt::DeclVar` — `n` is the HLSL signature parameter, so a `float3 n = ...;` redecl
        // would diverge the committed text. `_init` (the `Vec3Param` seed) is unused: a parameter is
        // already bound by name. Mirrors the scalar `decl_param`'s suppressed-decl path.
        push_var(name, EmitTy::Float3)
    }

    fn get_var_vec3(v: &Var) -> Emit {
        // Read the running `float3` value: a `VarRef` node printing the variable's name (`n`).
        Emit(push(Node::VarRef(v.0)))
    }

    fn set_var_vec3(v: &Var, val: Emit) {
        // A BARE `n = <rhs>;` (NO decl — `n` is the suppressed-decl param). The SAME `Stmt::Assign`
        // the scalar `set_var` records; the rhs is a `float3` expression node.
        record_stmt(Stmt::Assign {
            var: *v,
            rhs: val.0,
        });
    }

    fn decl_var_vec2(name: &'static str, init: Emit) -> Var {
        // A `float2 e = <init>;` decl — `record_decl_var` with `EmitTy::Float2` (so the printer spells
        // the `float2` token via `ty_keyword`). The `float2` analogue of `decl_var` (a `float` local).
        record_decl_var(name, EmitTy::Float2, init.0)
    }

    fn get_var_vec2(v: &Var) -> Emit {
        // Read the running `float2` value: a `VarRef` node printing the variable's name (`e`).
        Emit(push(Node::VarRef(v.0)))
    }

    fn set_var_vec2(v: &Var, val: Emit) {
        // `e = <rhs>;` — a `Stmt::Assign` whose rhs is a `float2` expression node.
        record_stmt(Stmt::Assign {
            var: *v,
            rhs: val.0,
        });
    }

    fn vec3_xy(v: Emit) -> Emit {
        // `n.xy` — a `Vec2Swizzle` with mask 0 (`"xy"`), typed `Float2`.
        Emit(push(Node::Vec2Swizzle(v.0, 0)))
    }

    fn vec2_yx(v: Emit) -> Emit {
        // `e.yx` — a `Vec2Swizzle` with mask 1 (`"yx"`), typed `Float2`.
        Emit(push(Node::Vec2Swizzle(v.0, 1)))
    }

    fn vec2_x(v: Emit) -> Emit {
        // `e.x` — a `Vec2Comp` with axis 0, typed `Float`.
        Emit(push(Node::Vec2Comp(v.0, 0)))
    }

    fn vec2_y(v: Emit) -> Emit {
        // `e.y` — a `Vec2Comp` with axis 1, typed `Float`.
        Emit(push(Node::Vec2Comp(v.0, 1)))
    }

    fn vec2_abs(v: Emit) -> Emit {
        Emit(push(Node::Vec2Abs(v.0)))
    }

    fn vec2_mul(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Vec2Mul(a.0, b.0)))
    }

    fn vec2_mul_scalar(v: Emit, s: Emit) -> Emit {
        Emit(push(Node::Vec2MulScalar(v.0, s.0)))
    }

    fn vec2_add_scalar(v: Emit, s: Emit) -> Emit {
        Emit(push(Node::Vec2AddScalar(v.0, s.0)))
    }

    fn vec2_rsub_scalar(s: Emit, v: Emit) -> Emit {
        // `1.0 - abs(e.yx)` — the scalar-LHS subtract (`(scalar, vec)` operand order).
        Emit(push(Node::Vec2RSubScalar(s.0, v.0)))
    }

    fn select_bare(cond: EmitMask, t: Emit, e: Emit) -> Emit {
        // A `SelectBare` node — the printer wraps NOTHING (the committed `oct_encode` sign-ternary
        // `e.x >= 0.0 ? 1.0 : -1.0`). DISTINCT from `select`'s `SelectParen` (both arms wrapped) and
        // `FieldScalar::select`'s `Select` (condition wrapped).
        Emit(push(Node::SelectBare(cond.0, t.0, e.0)))
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
            Stmt::DeclArray { arr, elem_ty, len } => {
                // `int cell[3];` / `float s[8];` — an UNINITIALIZED named-local array.
                out.push_str(&format!(
                    "{pad}{} {}[{len}];\n",
                    ty_keyword(*elem_ty),
                    names.array[*arr as usize]
                ));
            }
            Stmt::ArrayStore { arr, idx, rhs } => {
                // `cell[axis] = <rhs>;` — a per-element store (the idx spelled BARE for a literal).
                let idx_s = array_index_str(arena, names, &[], *idx);
                let rhs_s = inline_expr(arena, names, &[], *rhs);
                out.push_str(&format!(
                    "{pad}{}[{idx_s}] = {rhs_s};\n",
                    names.array[*arr as usize]
                ));
            }
            Stmt::ArrayAddAssign { arr, idx, rhs } => {
                // `cell[axis] += step[axis];` — the `+=` TOKEN (one access-chain — the R1 finding).
                let idx_s = array_index_str(arena, names, &[], *idx);
                let rhs_s = inline_expr(arena, names, &[], *rhs);
                out.push_str(&format!(
                    "{pad}{}[{idx_s}] += {rhs_s};\n",
                    names.array[*arr as usize]
                ));
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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

/// Generates the HLSL SSAO HORIZON-STEP span (Render P7 GROUP A) — ONE forward HBAO-lite
/// horizon tap's math (`delta = P' - P`, the squared-distance `falloff` range gate, the
/// `sampleCos = dot(delta,dir) / max(length(delta), SSAO_EPS)` horizon cosine, and the
/// `hc = max(hc, sampleCos * falloff)` running horizon max) — by tracing the generic
/// [`crate::ssao::ssao_horizon_step_body`] over the `EmitCf` backend, and returns ONLY the
/// span (NOT a wrapped function).
///
/// Framing (b): a SPAN, not a whole function. The forward neighbour reconstruct
/// (`generate_ray(px', py') * gViewT'`, the integer step-rounding + the bounds-clamp + the
/// `mask != 1 || view_t >= 1e30` skip), the `[unroll]` slice/step loops, the
/// rotation-table slice direction, and the final `occ → ao` fold stay HAND-WRITTEN inline
/// in `sdf_ssao.comp.hlsl` (around the `// === GENERATED ssao_horizon_step BEGIN/END ===`
/// sentinels). The hand-written loop body computes the per-tap neighbour into a `float3 Pp`
/// and the slice direction into a `float3 dir`, both of which this span reads by NAME; the
/// running horizon max is the suppressed-decl `float hc` the hand-written half-slice
/// preamble declared (`float hc_pos = 0.0;` / `float hc_neg = 0.0;` — the span writes
/// through the name `hc`). The full [`crate::ssao::ssao_estimate_body`] /
/// [`crate::ssao::ssao_slice_body`] are the EVAL ORACLE (the CPU mirror the host golden
/// gather calls); their loop STRUCTURE is mirrored by the hand-written HLSL loops, and the
/// re-DXC byte-identity gate (`ssao_edsl_sync`) proves the spliced text + glue compiles to
/// the committed `sdf_ssao.comp.spv` end-to-end.
///
/// The span prints at DEPTH 5 (20-space indent; the committed site nests
/// `main`→`if (center_lit)`→`for (slice)`→the half-slice brace→`for (step)`→this body).
/// The tuning consts spell SYMBOLICALLY
/// (`SSAO_RADIUS` / `SSAO_EPS`), so a value-spelled const cannot move the committed
/// `OpConstant` set.
pub fn emit_hlsl_ssao() -> String {
    use crate::ssao;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's inputs:
    //   P   → Vec3Param(0) (vec_in[0] = "P")  — the center world position
    //   Pp  → Vec3Param(1) (vec_in[1] = "Pp") — the forward-reconstructed neighbour `P'`
    //   n   → Vec3Param(2) (vec_in[2] = "n")  — the center surface normal `N` (the
    //                                            elevation reference; CONSTANT per pixel)
    // `hc` is the running horizon max the hand-written half-slice loop declared (`float
    // hc_pos = 0.0;` etc.); seeded as a SUPPRESSED-DECL `float` param (bound by NAME, no
    // recorded decl) so `set_var`/`get_var` spell `hc = ...;` with NO `float hc = ...;`
    // redecl. The `_init` seed is unused on Emit (a param is bound by name).
    let p = Emit(push(Node::Vec3Param(0)));
    let pp = Emit(push(Node::Vec3Param(1)));
    let n = Emit(push(Node::Vec3Param(2)));
    let hc = EmitCf::decl_param("hc", Emit::lit(0.0));

    // Record `hc = <horizon step>;` — the generated span is the `delta`/`d2`/`falloff`/
    // `elev` temps + this assign.
    let updated = ssao::ssao_horizon_step_body::<EmitCf>(p, pp, n, EmitCf::get_var(&hc));
    EmitCf::set_var(&hc, updated);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in: [&str; 0] = [];
    let vec_in = ["P", "Pp", "n"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
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
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // DEPTH 5 (20-space indent): the committed site nests `main`→`if (center_lit)`→
        // `for (sl)`→the half-slice brace→`for (sp/sn)`→this body.
        print_block(&body_block, &arena, names, 5, &mut span);
        span
    })
}

/// Generates the WHOLE HLSL `sdf_soft_shadow_ranged(float3 p, float3 n, float3 L, float
/// t_max)` function — the P6 R1 `t_max`-RANGED soft-shadow leaf consumed ONLY by the
/// deferred RESOLVE (`deferred_pbr.hlsl`). It traces [`crate::shadow::
/// sdf_soft_shadow_ranged_body`] over `EmitCf` (statement-for-statement identical to
/// [`emit_hlsl_sdf_soft_shadow`] except the escape break spells the PARAMETER `t_max`
/// instead of the frozen `T_MAX` symbol), and — UNLIKE `emit_hlsl_sdf_soft_shadow` which
/// returns a span spliced inside a hand-written marcher function — returns a COMPLETE
/// function (the resolve's per-light loop calls it directly; there is no hand-written
/// preamble to splice into).
///
/// B3 (option a): a SEPARATELY-NAMED entrypoint. The marcher's `sdf_soft_shadow` emit
/// (`emit_hlsl_sdf_soft_shadow`) is UNTOUCHED, so the marcher's frozen `.comp.spv` cannot
/// move. The `sdf_soft_shadow_ranged_matches_edsl_emit` sync pin (in `boyko_rhi_vulkan`)
/// pins the committed `deferred_pbr.hlsl` function to this output.
pub fn emit_hlsl_sdf_soft_shadow_ranged() -> String {
    use crate::shadow;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // p/n/L are the three `float3` params (Vec3Param markers); `t_max` is the new scalar
    // `float` param (float_in[0]), seeded as an `Input` node (the same suppressed-decl
    // mechanism `m2_regula_falsi`'s {lo,hi,..} use — bound by NAME, not by an init expr).
    let p = Emit(push(Node::Vec3Param(0)));
    let n = Emit(push(Node::Vec3Param(1)));
    let l = Emit(push(Node::Vec3Param(2)));
    let t_max = Emit::input(0);
    let out = RetCellF;

    // The field-distance seam: on Emit it records a `field_distance(p + L * t)` call node.
    shadow::sdf_soft_shadow_ranged_body::<EmitCf, _>(
        p,
        n,
        l,
        t_max,
        |q| EmitCf::call1("field_distance", q),
        &out,
    );

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["t_max"];
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut body = String::new();
        print_block(&body_block, &arena, names, 1, &mut body);
        format!("float sdf_soft_shadow_ranged(float3 p, float3 n, float3 L, float t_max) {{\n{body}}}\n")
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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

/// Generates the HLSL B1 main-marcher MESH-GUARD + PROBE-POINT span (SPAN A — the production
/// `for (uint it...)` loop's `if (t >= t_mesh) { exhausted = false; break; }` mesh-occlusion guard +
/// the `float3 p = ro + rd * t;` probe-point compute, contiguous right after the loop header `{` and
/// BEFORE the M1 brick island) by tracing the generic [`crate::marcher::b1_marcher_mesh_p_body`]
/// over the `EmitCf` backend, and returns ONLY the span (NOT a wrapped function).
///
/// Track-B-Increment-4g-g1 (literal completeness — owner-requested): pure REUSE of proven facets,
/// ZERO new machinery. The mesh guard reuses the FLOAT `t >= t_mesh` ([`FieldScalar::ge`]), the
/// in-guard `exhausted = false;` ([`EmitCf::set_bool_var`] over a SUPPRESSED-DECL bool —
/// [`EmitCf::decl_bool_param`]), and the `brk` ([`EmitCf::brk`]); the probe point reuses the NAMED
/// `float3 p` temp ([`EmitCf::temp_vec3`] over `ro + rd * t`, the SAME shape Inc-4e's re-march builds
/// `p` with). `p` is READ-ONLY in the loop, so it is a `temp_vec3` (a `float3 p = ...;` DeclTemp),
/// NOT a mutable `decl_var_vec3`; the hand-written M1/M2 islands + the fold span read `p` by NAME
/// (the established cross-splice name-sharing).
///
/// The seeded inputs are `ro`/`rd` (`Vec3Param`), `t_mesh` (a scalar `float` input); the carried
/// marcher state (`t` float, `exhausted` bool) are SUPPRESSED-DECL vars (declared by the hand-written
/// B1 preamble). The span prints at DEPTH 2 (8-space indent; the committed site nests
/// main→`for (uint it)`→these statements), matching the committed indentation.
///
/// The enclosing marcher (the `for (uint it...)` header + the M1/M2 brick islands + the accept
/// wrapper) stays HAND-WRITTEN inline around the `// === GENERATED b1_marcher_mesh_p BEGIN/END ===`
/// sentinels (framing (b)). The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the
/// `b1_marcher_mesh_p` text-sync test pins the committed span to this output.
pub fn emit_hlsl_b1_marcher_mesh_p() -> String {
    use crate::marcher;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
    //   t_mesh → Input(0)     (float_in[0] = "t_mesh") — the mesh-occlusion depth bound
    // `t` and `exhausted` are the enclosing carried vars (both suppressed-decl: `decl_param` for
    // `t`, `decl_bool_param` for `exhausted`), declared by the hand-written B1 preamble. The `_init`
    // seeds are unused on Emit (a suppressed local is bound by name); pass `t_mesh` as a byte-neutral
    // placeholder (decl_param/decl_bool_param record NO statement + push NO node, so it is never
    // referenced).
    let ro = Emit(push(Node::Vec3Param(0)));
    let rd = Emit(push(Node::Vec3Param(1)));
    let t_mesh = Emit::input(0);
    let t = EmitCf::decl_param("t", t_mesh);
    let exhausted = EmitCf::decl_bool_param("exhausted", false);

    let _ = marcher::b1_marcher_mesh_p_body::<EmitCf>(ro, rd, &t, t_mesh, &exhausted);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["t_mesh"];
    let vec_in = ["ro", "rd"];
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is TWO recorded statements — the mesh-guard `Stmt::If { cond: t >= t_mesh,
        // then: [Assign exhausted = false, Break] }` and the `Stmt::DeclTemp p = ro + rd * t` — a
        // single in-order walk at DEPTH 2 (8-space indent), matching the committed site (the M1
        // island follows hand-written). NO function-signature wrap (the span is spliced inside the
        // hand-written B1 marcher loop).
        print_block(&body_block, &arena, names, 2, &mut span);
        span
    })
}

/// Generates the HLSL B1 main-marcher ANALYTIC-FOLD DISTANCE span (SPAN B — the production
/// `for (uint it...)` loop's `float d = sdf(p);` analytic field sample, AFTER the M2 trilinear brick
/// island and BEFORE the `if (d < EPS)` accept wrapper) by tracing the generic
/// [`crate::marcher::b1_marcher_fold_d_body`] over the `EmitCf` backend, and returns ONLY the span
/// (NOT a wrapped function).
///
/// Track-B-Increment-4g-g1 (literal completeness): pure REUSE — the field-call seam
/// ([`EmitCf::call1`], interned `"sdf"` — the ANALYTIC field, NOT `field_distance`) into a NAMED
/// `float d` temp ([`EmitCf::temp_float`]). `p` is a CAPTURED `float3` input (SPAN A declared
/// `float3 p = ...;` above; each span is a separate emit with FRESH recorder state, so `p` is
/// re-seeded by NAME here as a `Vec3Param`). The hand-written `if (d < EPS)` accept wrapper reads `d`
/// by NAME (the cross-splice name-sharing).
///
/// The span prints at DEPTH 2 (8-space indent; the committed site nests main→`for (uint it)`→this
/// statement). The enclosing marcher (the loop header + the M1/M2 islands + the accept wrapper) stays
/// HAND-WRITTEN inline around the `// === GENERATED b1_marcher_fold_d BEGIN/END ===` sentinels
/// (framing (b)). The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the
/// `b1_marcher_fold_d` text-sync test pins the committed span to this output.
pub fn emit_hlsl_b1_marcher_fold_d() -> String {
    use crate::marcher;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's single input:
    //   p → Vec3Param(0) (vec_in[0] = "p") — the probe point (SPAN A's `float3 p`, re-seeded by name
    //                                        here as a captured `float3` — each span is a fresh emit).
    let p = Emit(push(Node::Vec3Param(0)));

    // The field-distance seam: on Emit it records a `sdf(p)` call node (the ANALYTIC field via the
    // hand-written `sdf`, interned `"sdf"` — NOT `field_distance`).
    let _ = marcher::b1_marcher_fold_d_body::<EmitCf, _>(p, |q| EmitCf::call1("sdf", q));

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in: [&str; 0] = [];
    let vec_in = ["p"];
    let call_in = CALLS.with(|c| c.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: &call_in,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is ONE recorded statement — the `Stmt::DeclTemp d = sdf(p)` — at DEPTH 2
        // (8-space indent), matching the committed site (the `if (d < EPS)` accept wrapper follows
        // hand-written). NO function-signature wrap (the span is spliced inside the hand-written B1
        // marcher loop).
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
    VAR_TYPES.with(|t| t.borrow_mut().clear());
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
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
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

/// Generates the HLSL `select_level` SCAN SPAN — the M4 clip-map LOD selector's `[unroll]`
/// finest-first containment scan (the per-level `[o, hi)` box test + the `L >= pc.brick_levels`
/// early-out + the `return (int)L;` hit + the tail `return -1;`) — by tracing the generic
/// [`crate::levels::select_level_body`] over the `EmitCf` backend (whose `Cf::Scalar = Emit`
/// supplies the SSA-node arithmetic), and returns ONLY the span (NOT a wrapped function).
///
/// Distinct from the WHOLE-function producers in returning a SPAN: the hand-written signature
/// `int select_level(float3 p) {` + the closing `}` stay un-generated (framing (b)), so the
/// generator emits only the statements spliced BETWEEN the `// === GENERATED select_level
/// BEGIN/END ===` sentinels. The span is printed at DEPTH 1 (4-space `[unroll]` indent), matching
/// the committed L1222-1234.
///
/// The `[unroll]` loop reuses [`Cf::runtime_for`] (attr `"[unroll]"`, the bound SYMBOL
/// `BRICK_LEVELS`); the `M4Level` reads (`m2_levels[L].…`) + the `pc.brick_levels` read are recorded
/// through the producer's `EmitCf::level_field_*` / `EmitCf::pc_uint` closures (on Emit these are the
/// access-text / bare-text recorders; on Eval they would be `unreachable!`, but the Eval body uses a
/// host-fixture closure instead).
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the `select_level` text-sync
/// test pins the committed shader span to this output.
pub fn emit_hlsl_select_level() -> String {
    use crate::levels;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    PC_FIELDS.with(|p| p.borrow_mut().clear());
    LEVEL_FIELDS.with(|l| l.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's single input:
    //   p → Vec3Param(0) (vec_in[0] = "p") — the world query point (the only generated-span input).
    // The loop iv `L` is seeded INTERNALLY by `runtime_for` (a `UintInput(0)` printing `uint_in[0]`
    // = `"L"`); the level-field / pc reads spell their bare access text (no name table seeding).
    let p = Emit(push(Node::Vec3Param(0)));
    let ret_out = RetCellI;

    // The fixture seams: on Emit they record the access-text / bare-text nodes.
    levels::select_level_body::<EmitCf, _, _, _>(
        p,
        EmitCf::level_field_vec3,
        EmitCf::level_field_scalar,
        || EmitCf::pc_uint("pc.brick_levels"),
        &ret_out,
    );

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    // `p` is the only `float3` parameter (vec_in); the loop iv `L` is the only `uint` name. The
    // level-field / pc reads carry their own text tables.
    let float_in: [&str; 0] = [];
    let vec_in = ["p"];
    let pc_in = PC_FIELDS.with(|p| p.borrow().clone());
    let level_field = LEVEL_FIELDS.with(|l| l.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: &["L"],
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: &pc_in,
        level_field: &level_field,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is the runtime `Stmt::Loop` (header `[unroll] for (uint L = 0u; L <
        // BRICK_LEVELS; ++L)`, body: the `L >= pc.brick_levels` `If { Break }`, the DeclTemp o/bw/hi,
        // the composite containment `If { Return (int)L }`) + the tail `Stmt::Return(-1)` — a flat
        // in-order walk at DEPTH 1 (4-space indent), matching the committed L1222-1234. NO
        // function-signature wrap (the span is spliced inside the hand-written `select_level`).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the HLSL `pack_material_id_ba` body — the G-buffer material-id packer (the 16-bit `uint
/// id` → low/high-byte split → `float2` UNORM pair) — by tracing the generic
/// [`crate::pack::pack_material_id_ba_body`] over the `EmitCf` backend (whose `Cf::Scalar = Emit`
/// supplies the SSA-node arithmetic), and returns ONLY the BODY span (between the hand-written
/// `float2 pack_material_id_ba(uint id) {` signature and the closing `}`).
///
/// Framing (b): the signature + closing brace stay hand-written; the body (L520-522) is spliced
/// between the `// === GENERATED pack_material_id_ba BEGIN/END ===` sentinels in
/// `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`. The Track-B-Increment-G1 facets: the
/// `float2` return ([`EmitCf::ret_vec2`] + [`EmitCf::vec2_from_scalars`] — the FIRST `float2`-
/// returning leaf, landing the minimal `float2` axis) and the bitwise `uint` `&`/`>>`
/// ([`EmitCf::and_u`] / [`EmitCf::shr_u`] — the two DEAD `Node::And`/`Node::Shr` nodes' methods,
/// whose printer arms already existed). The named `lo`/`hi` `uint` temps reuse [`EmitCf::temp_uint`];
/// `255u`/`8u` reuse [`EmitCf::uint_lit`]; `(float)lo` reuses [`EmitCf::float_from_uint`].
///
/// The span spells `uint lo = id & 255u;` / `uint hi = id >> 8u & 255u;` — `255u` (the committed
/// `0xFFu` re-spliced; the hex→decimal change is DXC-fold-neutral) and UNPARENTHESIZED (the
/// committed `(id >> 8) & 0xFFu`'s redundant parens removal is byte-identical, proven). The
/// `pack_material_id_ba` text-sync test pins the committed body to this output; the cmp-`.spv` (in
/// `boyko_rhi_vulkan`) is the byte-identity oracle. The span prints at DEPTH 1 (4-space indent).
pub fn emit_hlsl_pack_material_id_ba() -> String {
    use crate::pack;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's single input:
    //   id → UintInput(0) (uint_in[0] = "id") — the 16-bit material id (the only generated-span input).
    let id = Emit::uint_input(0);
    let ret_out = RetCellV2;

    let _ = pack::pack_material_id_ba_body::<EmitCf>(id, &ret_out);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    // `id` is the only `uint` input; there are no float/vec/level-field/pc names.
    let float_in: [&str; 0] = [];
    let names = Names {
        float_in: &float_in,
        uint_in: &["id"],
        vec_in: NO_VEC_INPUTS,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: NO_VARS,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is two `Stmt::DeclTemp` (`uint lo = id & 255u;`, `uint hi = id >> 8u &
        // 255u;`) + the tail `Stmt::Return(float2(...))` — a flat in-order walk at DEPTH 1 (4-space
        // indent), matching the committed L520-522. NO function-signature wrap (the span is spliced
        // inside the hand-written `pack_material_id_ba`).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the HLSL `oct_encode` BODY span — the octahedral-normal encoder's `n /= ...` normalize,
/// `float2 e = n.xy;`, the `if (n.z < 0.0)` lower-hemisphere fold, and the fused `return e * 0.5 +
/// 0.5;` — by tracing the generic [`crate::oct::oct_encode_body`] over the `EmitCf` backend (whose
/// `Cf::Scalar = Emit` supplies the SSA-node arithmetic), and returns ONLY the BODY span (between the
/// hand-written `float2 oct_encode(float3 n) {` signature and the closing `}`).
///
/// Framing (b): the signature + closing brace stay hand-written; the body (L508-513) is spliced
/// between the `// === GENERATED oct_encode BEGIN/END ===` sentinels INSIDE `oct_encode`. The new
/// facets: the mutable `float3` suppressed-decl param `n` ([`EmitCf::decl_param_vec3`] — `n = n /
/// ...;`, the R1 whole-variable form), the mutable `float2` local `e` ([`EmitCf::decl_var_vec2`] —
/// `float2 e = n.xy;`), the REAL fall-through `if_` (the `n.z < 0.0` branch), and the `float2`
/// component ops (the `.xy`/`.yx`/`.x`/`.y` swizzles, `abs`, `*`, `* s`, `+ s`, `s - v`). The span
/// prints at DEPTH 1 (4-space indent), matching the committed L508-513.
///
/// The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the `oct_encode` text-sync test
/// pins the committed shader span to this output.
pub fn emit_hlsl_oct_encode() -> String {
    use crate::oct;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's single input:
    //   n → Vec3Param(0) (vec_in[0] = "n") — the unit normal (the only generated-span input). It is the
    // SUPPRESSED-DECL mutable param: `decl_param_vec3("n", n)` seeds the `VARS` entry the body's
    // get/set_var_vec3 resolve (the `Vec3Param` seed itself is unused on Emit — a parameter is bound by
    // name). The `e` local is declared INSIDE the body (a second `VARS` entry).
    let n = Emit(push(Node::Vec3Param(0)));
    let ret_out = RetCellV2;

    let _ = oct::oct_encode_body::<EmitCf>(n, &ret_out);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    // `n` is the only `float3` input; there are no float/uint scalar inputs. The `vars` table holds
    // `n` (the suppressed-decl param) + `e` (the `float2` local), pushed in program order.
    let float_in: [&str; 0] = [];
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: NO_UINT_INPUTS,
        vec_in: &["n"],
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: NO_NAMED_LITS,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole span is a bare `Stmt::Assign` (`n = n / (...)`), a `Stmt::DeclVar` (`float2 e =
        // n.xy;`), a `Stmt::If` whose then-block is the single `e = ...;` fold assign, and the tail
        // `Stmt::Return(e * 0.5 + 0.5)` — a flat in-order walk at DEPTH 1 (4-space indent), matching the
        // committed L508-513. NO function-signature wrap (the span is spliced inside the hand-written
        // `oct_encode`).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the HLSL `m2_brick_span` body — the brick-AABB ray-span clip (the standard slab
/// method) with the `[unroll]` axis loop, the parallel-slab early `return false`, the near/far
/// swap, and the COMPUTED-bool tail `return tmax > tmin;` — by tracing the generic
/// [`crate::brick::m2_brick_span_body`] over the `EmitCf` backend (whose `Cf::Scalar = Emit`
/// supplies the SSA-node arithmetic), and returns ONLY the BODY span (between the hand-written
/// `bool m2_brick_span(...) {` signature and the closing `}`).
///
/// Framing (b): the signature + closing brace stay hand-written; the body (L970-993) is spliced
/// between the `// === GENERATED m2_brick_span BEGIN/END ===` sentinels in
/// `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`. The three new Increment-5b facets:
/// the COMPUTED-bool return ([`EmitCf::ret_b_expr`] — the `tmax > tmin` mask as the `Stmt::Return`
/// operand, vs the `bool`-literal [`EmitCf::ret_b`]); the TWO `out float` params (`t_enter`/
/// `t_exit`, two `out_in` entries); and the FALL-THROUGH swap `if_` (the `t1 > t2` swap whose body
/// returns `Flow::Continue(())`). The `[unroll]` axis loop reuses [`EmitCf::runtime_for`] with a
/// LITERAL bound (`"3u"`) so the in-loop early `return false` forwards through it (the
/// [`Cf::unroll_for`] form does NOT forward a `Return` — see the brick module doc).
///
/// The `p`/`rd`/`cell_min` `float3` params are seeded as `VecParamRef` (the `dist_to_brick_exit`
/// index discipline — they are indexed `p[a]`, never used whole), and `brick_world` is a scalar
/// `float` input. The cmp-`.spv` (in `boyko_rhi_vulkan`) is the byte-identity oracle; the
/// `m2_brick_span` text-sync test pins the committed body to this output.
pub fn emit_hlsl_m2_brick_span() -> String {
    use crate::brick;

    // Fresh recorder state.
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's inputs:
    //   p           → VecParamRef(0) (vec_in[0]   = "p")           — the world ray origin
    //   rd          → VecParamRef(1) (vec_in[1]   = "rd")          — the world ray direction
    //   cell_min    → VecParamRef(2) (vec_in[2]   = "cell_min")    — the brick lower world corner
    //   brick_world → Input(0)       (float_in[0] = "brick_world") — the brick edge length
    //   t_enter     → OutFloatParam(0) (out_in[0] = "t_enter")     — the span near `out float`
    //   t_exit      → OutFloatParam(1) (out_in[1] = "t_exit")      — the span far  `out float`
    // The three `float3` params are indexed `p[a]` (the `dist_to_brick_exit` `VecParamRef` discipline,
    // so each is seeded as three copies of one `VecParamRef` id).
    let p_id = push(Node::VecParamRef(0));
    let rd_id = push(Node::VecParamRef(1));
    let cm_id = push(Node::VecParamRef(2));
    let p = [Emit(p_id), Emit(p_id), Emit(p_id)];
    let rd = [Emit(rd_id), Emit(rd_id), Emit(rd_id)];
    let cell_min = [Emit(cm_id), Emit(cm_id), Emit(cm_id)];
    let brick_world = Emit::input(0);
    let t_enter = OutFloatParam(0);
    let t_exit = OutFloatParam(1);
    let ret_out = RetCellB;

    brick::m2_brick_span_body::<EmitCf>(p, rd, cell_min, brick_world, &t_enter, &t_exit, &ret_out);

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["brick_world"];
    let vec_in = ["p", "rd", "cell_min"];
    let out_in = ["t_enter", "t_exit"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        uint_in: &["a"],
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: &out_in,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: NO_CALL_INPUTS,
        pc_in: NO_PC_INPUTS,
        level_field: NO_LEVEL_FIELDS,
        array: NO_ARRAY,
        res_in: NO_RES_INPUTS,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole body is the two DeclVar (tmin/tmax) + the runtime `Stmt::Loop` (header `[unroll]
        // for (uint a = 0u; a < 3u; ++a)`, body: the DeclTemp lo/hi, the parallel-slab `If { If {
        // OutAssign t_enter; OutAssign t_exit; Return false } Continue }`, the DeclTemp inv, the
        // DeclVar t1/t2, the swap `If { DeclTemp tmp; Assign t1; Assign t2 }`, the `Assign tmin/tmax`)
        // + the tail two `OutAssign` + the `Stmt::Return(tmax > tmin)` — a flat in-order walk at DEPTH
        // 1 (4-space indent), matching the committed L970-993. NO function-signature wrap (the span is
        // spliced inside the hand-written `m2_brick_span`).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}

/// Generates the HLSL `m2_brick_cubic_hit` body — the 3D-DDA cubic-hit marcher (the LARGEST + FINAL
/// brick called body): the `[unroll]` per-axis DDA setup (with the 3-way `else { if }` direction
/// branch), the `[loop]` 3D-DDA march (the cell clamp, the 8 `m2_corner` fetches, the cubic
/// form+solve, the in-cell early `return seg_lo + local_t;`, the nearest-axis advance, the DDA-exit
/// guard), and the tail `return -1.0;` — by tracing the generic
/// [`crate::cubic_hit::m2_brick_cubic_hit_body`] over the `EmitCf` backend (whose `Cf::Scalar = Emit`
/// supplies the SSA-node arithmetic), and returns ONLY the BODY span (between the hand-written
/// signature + the `const uint W = M2_BRICK_ALLOC;` decl and the closing `}`).
///
/// Framing (b): the signature, the `if (t_exit <= t_enter) { return -1.0; }` early-out, the `const
/// uint W = M2_BRICK_ALLOC;` decl, and the closing brace stay HAND-WRITTEN; the body span (L1021-
/// 1102) is spliced between the `// === GENERATED m2_brick_cubic_hit BEGIN/END ===` sentinels in
/// `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`. EMIT-ONLY: the body is never
/// instantiated over `EvalCf` (`m2_corner` → `atlas.SampleLevel` cannot run on the CPU), so there is
/// NO eval sweep — the cmp-`.spv` is the SOLE byte-identity oracle. The `m2_brick_cubic_hit`
/// text-sync test pins the committed body to this output.
///
/// The `atlas`/`atlas_smp` are RESOURCE params (`ResTok`); `ro_v`/`rd_v`/`tile_org` are WHOLE
/// `float3` params (`Vec3Param` — indexed `rd_v[axis]` via `Vec3DynIndex` AND passed whole to
/// `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)`); `t_enter`/`t_exit`/`inv_atlas`/`band_half` are scalar
/// `float` inputs; `w` is the captured `uint W` (read via `captured_uint("W")` → the `PcUint`
/// bare-text path); `ret_out` is the `RetCellF`.
pub fn emit_hlsl_m2_brick_cubic_hit() -> String {
    use crate::cubic_hit;

    // Fresh recorder state (incl. the Increment-5c array tables + the CALL_ARGS side-table).
    ARENA.with(|a| a.borrow_mut().clear());
    STMTS.with(|s| s.borrow_mut().clear());
    VARS.with(|v| v.borrow_mut().clear());
    VAR_TYPES.with(|t| t.borrow_mut().clear());
    NAMED_LITS.with(|n| n.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
    PC_FIELDS.with(|p| p.borrow_mut().clear());
    CALL_ARGS.with(|c| c.borrow_mut().clear());
    ARRAY_NAMES.with(|a| a.borrow_mut().clear());
    ARRAY_ELEM_TYS.with(|t| t.borrow_mut().clear());
    TEMP_SEQ.with(|c| *c.borrow_mut() = 0);
    TEMP_TYPES.with(|t| t.borrow_mut().clear());
    TEMP_NAMES.with(|t| t.borrow_mut().clear());

    // Seed the function body block (the bottom of the STMTS stack).
    STMTS.with(|s| s.borrow_mut().push(Block { stmts: Vec::new() }));

    // Seed the span's inputs:
    //   atlas       → ResTok(0)    (res_in[0]   = "atlas")        — the brick atlas Texture3D
    //   atlas_smp   → ResTok(1)    (res_in[1]   = "atlas_smp")    — the atlas SamplerState
    //   ro_v        → Vec3Param(0) (vec_in[0]   = "ro_v")         — the ray origin (voxel units)
    //   rd_v        → Vec3Param(1) (vec_in[1]   = "rd_v")         — the ray direction (voxel units)
    //   tile_org    → Vec3Param(2) (vec_in[2]   = "tile_org")     — the atlas-voxel tile origin
    //   t_enter     → Input(0)     (float_in[0] = "t_enter")      — the brick-span near bound
    //   t_exit      → Input(1)     (float_in[1] = "t_exit")       — the brick-span far bound
    //   inv_atlas   → Input(2)     (float_in[2] = "inv_atlas")    — the atlas-voxel inverse dim
    //   band_half   → Input(3)     (float_in[3] = "band_half")    — the snorm decode half-band
    //   w           → captured_uint("W")                          — the `const uint W` above the span
    // `ro_v`/`rd_v`/`tile_org` are WHOLE `Vec3Param`s (NOT the `VecParamRef`-x3 of the index-only
    // params): they are indexed `rd_v[axis]` (Vec3DynIndex reads the vec id off the Vec3Param) AND
    // passed whole to `m2_jcgt_cubic_coeffs(s, lo_g, rd_v)`.
    let atlas = ResTok(0);
    let atlas_smp = ResTok(1);
    let ro_v = Emit(push(Node::Vec3Param(0)));
    let rd_v = Emit(push(Node::Vec3Param(1)));
    let tile_org = Emit(push(Node::Vec3Param(2)));
    let t_enter = Emit::input(0);
    let t_exit = Emit::input(1);
    let inv_atlas = Emit::input(2);
    let band_half = Emit::input(3);
    let w = EmitCf::captured_uint("W");
    let ret_out = RetCellF;

    cubic_hit::m2_brick_cubic_hit_body::<EmitCf>(
        atlas, atlas_smp, ro_v, rd_v, tile_org, t_enter, t_exit, inv_atlas, band_half, w, &ret_out,
    );

    // Pop the function body block and print it.
    let body_block = STMTS.with(|s| {
        s.borrow_mut()
            .pop()
            .expect("invariant: the function body block was pushed above")
    });

    let float_in = ["t_enter", "t_exit", "inv_atlas", "band_half"];
    let vec_in = ["ro_v", "rd_v", "tile_org"];
    let res_in = ["atlas", "atlas_smp"];
    let named_lit = NAMED_LITS.with(|n| n.borrow().clone());
    let call_in = CALLS.with(|c| c.borrow().clone());
    let pc_in = PC_FIELDS.with(|p| p.borrow().clone());
    let array = ARRAY_NAMES.with(|a| a.borrow().clone());
    let vars = VARS.with(|v| v.borrow().clone());
    let names = Names {
        float_in: &float_in,
        // `axis`/`iter` are the two loop iv names (seeded as `UintInput(0)` by `runtime_for`); the
        // captured `W` rides the `pc_in` bare-text table, not `uint_in`.
        uint_in: &["axis"],
        vec_in: &vec_in,
        uint3_in: NO_UINT3_INPUTS,
        buf_in: NO_BUF_INPUTS,
        out_in: NO_OUT_INPUTS,
        named_lit: &named_lit,
        vars: &vars,
        vec4_in: NO_VEC4_INPUTS,
        call_in: &call_in,
        pc_in: &pc_in,
        level_field: NO_LEVEL_FIELDS,
        array: &array,
        res_in: &res_in,
    };

    ARENA.with(|a| {
        let arena = a.borrow();
        let mut span = String::new();
        // The whole body is the DeclVar `t` + the four DeclArray + the `[unroll]` setup `Stmt::Loop`
        // + the `[loop]` DDA `Stmt::Loop` + the tail `Stmt::Return(-1.0)` — a flat in-order walk at
        // DEPTH 1 (4-space indent), matching the committed L1021-1102. NO function-signature wrap (the
        // span is spliced inside the hand-written `m2_brick_cubic_hit`, BELOW the `const uint W` decl).
        print_block(&body_block, &arena, names, 1, &mut span);
        span
    })
}
