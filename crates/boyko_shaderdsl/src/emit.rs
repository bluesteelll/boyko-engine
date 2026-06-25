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
    /// `cond ? t : e` — the HLSL ternary (the frozen `(k > 0.0) ? _ : _`).
    Select(u32, u32, u32),
    /// `a > b` — a Mask node (printed inline inside a ternary condition).
    Gt(u32, u32),
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

/// The HLSL [`EmitTy`] a node materializes as (O2). Every legacy field/normal node
/// is [`EmitTy::Float`]; only the integer/bit nodes are [`EmitTy::Uint`]. The
/// `UintToFloat` cast is the boundary — its RESULT is `float` (so consumers see a
/// `float`), its operand is `uint`. This is a per-NODE tag (the node's own result
/// type), not an operand walk: each int node declares its own result type, so the
/// printer needs no recursion to type a temp.
fn type_of(node: Node) -> EmitTy {
    match node {
        Node::UintInput(_) | Node::UintLit(_) | Node::And(_, _) | Node::Shr(_, _) => EmitTy::Uint,
        // `UintToFloat` PRODUCES a float (the cast result); every other node is float.
        _ => EmitTy::Float,
    }
}

/// The HLSL type keyword for an [`EmitTy`] (`float` / `uint`).
fn ty_keyword(ty: EmitTy) -> &'static str {
    match ty {
        EmitTy::Float => "float",
        EmitTy::Uint => "uint",
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
}

/// Formats one f32 literal the way the frozen HLSL writes it (`0.5`, `1.0`, `0.0`).
/// A short, deterministic rendering — enough for the smin/smax field constants.
fn fmt_lit(x: f32) -> String {
    if x == x.trunc() && x.abs() < 1.0e7 {
        // Integer-valued: render as `N.0` (matches `0.0` / `1.0` in the frozen src).
        format!("{:.1}", x)
    } else {
        // Non-integer: a compact shortest round-trip (e.g. `0.5`).
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
            | Node::IntEq(_, _)
            | Node::UintInput(_)
            | Node::UintLit(_)
    )
}

/// The operand spelling for node `id` at a USE site: a leaf inlines (its input
/// name or formatted literal, or the `a > b` comparison nested in a ternary
/// condition); a non-leaf names its already-emitted `tN` temp. The temp must exist
/// because the SSA walk emits nodes in arena order, which is topological.
fn operand_str(arena: &[Node], names: Names, temps: &[Option<String>], id: u32) -> String {
    let node = arena[id as usize];
    match node {
        Node::Input(n) => names.float_in[n as usize].to_string(),
        Node::Lit(x) => fmt_lit(x),
        Node::Gt(a, b) => format!(
            "{} > {}",
            operand_str(arena, names, temps, a),
            operand_str(arena, names, temps, b)
        ),
        Node::IntEq(a, b) => format!(
            "{} == {}",
            operand_str(arena, names, temps, a),
            operand_str(arena, names, temps, b)
        ),
        Node::UintInput(n) => names.uint_in[n as usize].to_string(),
        // A `uint` literal renders with the HLSL unsigned suffix (`0xFFu`, `8u`).
        Node::UintLit(u) => format!("{}u", u),
        // Any non-leaf has a temp name assigned during the SSA walk.
        _ => temps[id as usize]
            .clone()
            .expect("invariant: every non-leaf node has an emitted temp before any use"),
    }
}

/// The HLSL expression that DEFINES node `id` (its temp's right-hand side), built
/// from its operands' names (a temp name, an input name, or an inlined literal).
fn define_str(arena: &[Node], names: Names, temps: &[Option<String>], id: u32) -> String {
    let node = arena[id as usize];
    let op = |child: u32| operand_str(arena, names, temps, child);
    match node {
        Node::Input(_)
        | Node::Lit(_)
        | Node::Gt(_, _)
        | Node::IntEq(_, _)
        | Node::UintInput(_)
        | Node::UintLit(_) => {
            // Leaves are inlined, never defined as a temp.
            unreachable!("leaf nodes are inlined, not defined")
        }
        Node::Add(a, b) => format!("{} + {}", op(a), op(b)),
        Node::Sub(a, b) => format!("{} - {}", op(a), op(b)),
        Node::Mul(a, b) => format!("{} * {}", op(a), op(b)),
        Node::Div(a, b) => format!("{} / {}", op(a), op(b)),
        Node::Neg(a) => format!("-{}", op(a)),
        Node::Min(a, b) => format!("min({}, {})", op(a), op(b)),
        Node::Max(a, b) => format!("max({}, {})", op(a), op(b)),
        Node::Clamp01(a) => format!("clamp({}, 0.0, 1.0)", op(a)),
        Node::Lerp(s, a, h) => format!("lerp({}, {}, {})", op(s), op(a), op(h)),
        Node::Abs(a) => format!("abs({})", op(a)),
        Node::Sqrt(a) => format!("sqrt({})", op(a)),
        Node::Select(c, t, e) => format!("({}) ? {} : {}", op(c), op(t), op(e)),
        Node::FieldCall(_) => {
            // `FieldCall` belongs to the NORMAL leaf (a vector expression printed by
            // `sexpr_str`/`vexpr_str`), never to the scalar field body's printer.
            unreachable!("FieldCall is a normal-leaf node, not a scalar field node")
        }
        Node::And(a, b) => format!("{} & {}", op(a), op(b)),
        Node::Shr(a, b) => format!("{} >> {}", op(a), op(b)),
        // The HLSL numeric cast (value-preserving), NOT `asfloat` (a bit-reinterpret).
        Node::UintToFloat(a) => format!("(float){}", op(a)),
    }
}

/// Walks the recorded SSA arena into a `{ float tN = ...; ... return tROOT; }`
/// HLSL body. Each NON-leaf node becomes one `float tN` temp (so shared subtrees —
/// e.g. `hh` in `smin` — are computed ONCE, matching the frozen `hh` variable); the
/// leaves (inputs/literals) inline. The walk is in arena order, which is already
/// topological (SSA: a node only references strictly-earlier indices), so every
/// operand's temp exists before its use.
fn emit_body(arena: &[Node], names: Names, root: u32) -> String {
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
    let ret = operand_str(arena, names, &temps, root);
    out.push_str(&format!("    return {};\n", ret));
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
