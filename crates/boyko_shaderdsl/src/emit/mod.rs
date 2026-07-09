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

use crate::scalar::FieldScalar;

mod cf;
mod shaders;

pub use shaders::*;

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

