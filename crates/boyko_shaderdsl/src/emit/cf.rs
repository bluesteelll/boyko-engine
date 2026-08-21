//! `impl Cf for EmitCf` — the control-flow EMIT backend's trait implementation.
//!
//! Split out of `emit.rs` (pure structural move). The `EmitCf` ZST itself and the
//! STMT IR it records into live in the parent module ([`super`]); this file holds
//! only the [`Cf`](crate::cf::Cf) trait impl. `use super::*` surfaces every private
//! engine item (`push`, `record_stmt`, `Node`, `Var`, the `intern_*` helpers, …)
//! the impl calls.

use super::*;

use crate::cf::{Cf, Flow, LoopOp};

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

    // ---- Rung E: the particle-leaf prerequisite facets (recorder) ---------------------
    // Every node below prints its operands at `OperandPos::BitSide` (the infix three) or at
    // Root (the four intrinsic calls + `dot`), so the recorder itself carries no spelling
    // decision — it only pushes the node.

    fn ushl(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Shl(a.0, b.0)))
    }

    fn uxor(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Xor(a.0, b.0)))
    }

    fn uor(a: Emit, b: Emit) -> Emit {
        // The BITWISE `|` over two `uint` values — DISTINCT from `or`'s `Or` node (the logical
        // `||` over two Masks), which is why it is a separate node and not an overload.
        Emit(push(Node::BitOr(a.0, b.0)))
    }

    fn asuint(x: Emit) -> Emit {
        Emit(push(Node::AsUint(x.0)))
    }

    fn asfloat(u: Emit) -> Emit {
        Emit(push(Node::AsFloat(u.0)))
    }

    fn f16tof32(u: Emit) -> Emit {
        Emit(push(Node::F16ToF32(u.0)))
    }

    fn f32tof16(x: Emit) -> Emit {
        Emit(push(Node::F32ToF16(x.0)))
    }

    fn vec3_dot(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Vec3Dot(a.0, b.0)))
    }

    fn sin(x: Emit) -> Emit {
        // The SAME `Node::Sin` the `InterpBackend` recorder pushes — one node, one printer arm,
        // so the trig spells identically whichever backend axis authored the leaf.
        Emit(push(Node::Sin(x.0)))
    }

    fn cos(x: Emit) -> Emit {
        Emit(push(Node::Cos(x.0)))
    }

    fn rsqrt(x: Emit) -> Emit {
        Emit(push(Node::Rsqrt(x.0)))
    }

    // ---- UI-ADVANCED S1: the `ui_rect` fragment-leaf facets ------------------------------

    // The `float4` return cell — a ZST; the expression travels in the recorded `Stmt::Return`.
    type RetCellV4 = RetCellV4;

    fn ret_vec4(_cell: &RetCellV4, value: Emit) -> Flow {
        // The `float4` return — a single `Stmt::Return(value)` (the hand-written
        // `float4 ui_unpack_rgba8` / `float4 ui_premultiplied_over` signatures supply the
        // return type). Fall through on Emit.
        record_stmt(Stmt::Return(value.0));
        Flow::Continue(())
    }

    fn vec2_sub(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Vec2Sub(a.0, b.0)))
    }

    fn vec2_sub_scalar(v: Emit, s: Emit) -> Emit {
        Emit(push(Node::Vec2SubScalar(v.0, s.0)))
    }

    fn vec2_max_scalar(v: Emit, s: Emit) -> Emit {
        Emit(push(Node::Vec2MaxScalar(v.0, s.0)))
    }

    fn vec2_length(v: Emit) -> Emit {
        Emit(push(Node::Vec2Length(v.0)))
    }

    fn vec2_dot(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Vec2Dot(a.0, b.0)))
    }

    fn vec2_smoothstep(e0: Emit, e1: Emit, x: Emit) -> Emit {
        Emit(push(Node::Vec2Smoothstep(e0.0, e1.0, x.0)))
    }

    fn vec2_fwidth(v: Emit) -> Emit {
        Emit(push(Node::Vec2Fwidth(v.0)))
    }

    fn vec2_rdiv_scalar(s: Emit, v: Emit) -> Emit {
        Emit(push(Node::Vec2RDivScalar(s.0, v.0)))
    }

    fn select_vec2(cond: EmitMask, t: Emit, e: Emit) -> Emit {
        // The cond-wrapped, arms-bare `float2` ternary (`(p.x > 0.0) ? r.yz : r.xw`).
        Emit(push(Node::SelectVec2(cond.0, t.0, e.0)))
    }

    fn vec4_xy(v: Emit) -> Emit {
        // `clip.xy` — a `Vec4SwizzleV2` with mask 0 (`"xy"`), typed `Float2`.
        Emit(push(Node::Vec4SwizzleV2(v.0, 0)))
    }

    fn vec4_zw(v: Emit) -> Emit {
        Emit(push(Node::Vec4SwizzleV2(v.0, 1)))
    }

    fn vec4_yz(v: Emit) -> Emit {
        Emit(push(Node::Vec4SwizzleV2(v.0, 2)))
    }

    fn vec4_xw(v: Emit) -> Emit {
        Emit(push(Node::Vec4SwizzleV2(v.0, 3)))
    }

    fn vec4_alpha(v: Emit) -> Emit {
        Emit(push(Node::Vec4Alpha(v.0)))
    }

    fn vec4_from_scalars(x: Emit, y: Emit, z: Emit, w: Emit) -> Emit {
        Emit(push(Node::Vec4FromScalars(x.0, y.0, z.0, w.0)))
    }

    fn vec4_mul_scalar(v: Emit, s: Emit) -> Emit {
        Emit(push(Node::Vec4MulScalar(v.0, s.0)))
    }

    fn vec4_add(a: Emit, b: Emit) -> Emit {
        Emit(push(Node::Vec4Add(a.0, b.0)))
    }

    fn temp_vec2(name: &'static str, v: Emit) -> Emit {
        // A NAMED `float2` temp (`float2 rx = ...;`). The `float4` analogue (`temp_vec4`)
        // already exists above (the Increment-5c facet) and is reused by the UI leaves.
        record_temp(Some(name), EmitTy::Float2, v)
    }
}
