//! The 3.2 pre-check: the span lands ON the 17th field, friendlier than the derive's error.
use aether::aether;

aether! {
    bundle Fat {
    f0: u32,
    f1: u32,
    f2: u32,
    f3: u32,
    f4: u32,
    f5: u32,
    f6: u32,
    f7: u32,
    f8: u32,
    f9: u32,
    f10: u32,
    f11: u32,
    f12: u32,
    f13: u32,
    f14: u32,
    f15: u32,
    f16: u32,
    }
}

fn main() {}
