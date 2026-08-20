//! §3.3's diagnostic: `query(...)` with parens is refused ON the paren, with the fix spelled.
use aether::aether;

aether! {
    system s(q: query(&mut Transform)) {}
}

fn main() {}
