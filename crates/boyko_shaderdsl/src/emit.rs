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

#[inline]
fn push(node: Node) -> u32 {
    ARENA.with(|a| {
        let mut a = a.borrow_mut();
        let id = a.len() as u32;
        a.push(node);
        id
    })
}

impl Emit {
    /// Records a symbolic INPUT node named `name_id` (an index into the printer's
    /// name table). Used by [`emit_hlsl_field`] to seed the traced parameters.
    fn input(name_id: u32) -> Self {
        Emit(push(Node::Input(name_id)))
    }
}

impl FieldScalar for Emit {
    type Vec3 = [Emit; 3];
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
    fn eq_u(op: u32, want: u32) -> EmitMask {
        // The op-discriminant equality is a HOST comparison: the generic
        // `combine`'s op-dispatch is a host `if op == ...` (the frozen
        // `if (op == OP_*)`), NOT a traced mask. So `eq_u` for `Emit` is never
        // reached by `field::combine` (it dispatches on the host `u32` directly).
        // It is provided to satisfy the trait; it records a constant mask.
        let l = push(Node::Lit(op as f32));
        let r = push(Node::Lit(want as f32));
        EmitMask(push(Node::Gt(l, r))) // placeholder; unreached by the field body
    }
}

// ---- The HLSL printer ---------------------------------------------------------

/// The symbolic input names the field is traced over, in the order [`Emit::input`]
/// is called by [`emit_hlsl_field`]. The printer prints `Node::Input(i)` as
/// `INPUT_NAMES[i]`.
const INPUT_NAMES: &[&str] = &["a", "b", "k"];

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
    matches!(node, Node::Input(_) | Node::Lit(_) | Node::Gt(_, _))
}

/// The operand spelling for node `id` at a USE site: a leaf inlines (its input
/// name or formatted literal, or the `a > b` comparison nested in a ternary
/// condition); a non-leaf names its already-emitted `tN` temp. The temp must exist
/// because the SSA walk emits nodes in arena order, which is topological.
fn operand_str(arena: &[Node], temps: &[Option<String>], id: u32) -> String {
    let node = arena[id as usize];
    match node {
        Node::Input(n) => INPUT_NAMES[n as usize].to_string(),
        Node::Lit(x) => fmt_lit(x),
        Node::Gt(a, b) => format!(
            "{} > {}",
            operand_str(arena, temps, a),
            operand_str(arena, temps, b)
        ),
        // Any non-leaf has a temp name assigned during the SSA walk.
        _ => temps[id as usize]
            .clone()
            .expect("invariant: every non-leaf node has an emitted temp before any use"),
    }
}

/// The HLSL expression that DEFINES node `id` (its temp's right-hand side), built
/// from its operands' names (a temp name, an input name, or an inlined literal).
fn define_str(arena: &[Node], temps: &[Option<String>], id: u32) -> String {
    let node = arena[id as usize];
    let op = |child: u32| operand_str(arena, temps, child);
    match node {
        Node::Input(_) | Node::Lit(_) | Node::Gt(_, _) => {
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
    }
}

/// Walks the recorded SSA arena into a `{ float tN = ...; ... return tROOT; }`
/// HLSL body. Each NON-leaf node becomes one `float tN` temp (so shared subtrees —
/// e.g. `hh` in `smin` — are computed ONCE, matching the frozen `hh` variable); the
/// leaves (inputs/literals) inline. The walk is in arena order, which is already
/// topological (SSA: a node only references strictly-earlier indices), so every
/// operand's temp exists before its use.
fn emit_body(arena: &[Node], root: u32) -> String {
    let mut temps: Vec<Option<String>> = vec![None; arena.len()];
    let mut out = String::new();
    let mut next = 0u32;
    for (i, &node) in arena.iter().enumerate() {
        if is_inline_leaf(node) {
            continue;
        }
        let name = format!("t{}", next);
        next += 1;
        let rhs = define_str(arena, &temps, i as u32);
        out.push_str(&format!("    float {} = {};\n", name, rhs));
        temps[i] = Some(name);
    }
    let ret = operand_str(arena, &temps, root);
    out.push_str(&format!("    return {};\n", ret));
    out
}

/// Runs `body` over a fresh symbolic-input arena and returns the HLSL `{ ... }`
/// statement body for the result node. `inputs` is the count of [`Emit::input`]
/// handles to seed (named by [`INPUT_NAMES`] in order); `body` receives them and
/// returns the result handle.
fn trace<F: FnOnce(&[Emit]) -> Emit>(inputs: usize, body: F) -> String {
    ARENA.with(|a| a.borrow_mut().clear());
    let mut ins = Vec::with_capacity(inputs);
    for i in 0..inputs {
        ins.push(Emit::input(i as u32));
    }
    let result = body(&ins);
    ARENA.with(|a| {
        let a = a.borrow();
        emit_body(&a, result.0)
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
