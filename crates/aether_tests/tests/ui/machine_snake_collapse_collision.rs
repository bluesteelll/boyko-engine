//! Generated names are the snake_case COLLAPSE of the flattened state path, and that collapse
//! is lossy: `AB` and `Ab` are two distinct enum variants that mint one `__aether_m__ab__e`.
//! rustc would report the duplicate on generated tokens; Aether reports it on the two states.
//!
//! The colliding PAIR moved at rung A7, when the collapse stopped spelling a run of capitals one
//! letter per word (`GOLD` → `g_o_l_d` → `gold`). `AB`/`A_b`, the pair this fixture shipped with,
//! no longer collides — it now mints `__aether_m__ab__e` and `__aether_m__a_b__e`. A compile-fail
//! fixture whose input stopped being a fault is a fixture that passes for the wrong reason, so the
//! input was re-aimed at a pair the CURRENT rule collapses rather than the golden re-blessed.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial AB;
        state AB {
            on E => AB;
        }
        state Ab {
            on E => Ab;
        }
    }
}

fn main() {}
