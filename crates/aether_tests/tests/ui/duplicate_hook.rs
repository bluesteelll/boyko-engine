//! Duplicate hook KEYS are the parser's own early check (the derive would refuse too, but the
//! parser owns the better span — the §3.1 pre-check rule).
use aether::aether;

fn f() {}
fn g() {}

aether! {
    component A {
        on_add = f,
        on_add = g,
    }
}

fn main() {}
