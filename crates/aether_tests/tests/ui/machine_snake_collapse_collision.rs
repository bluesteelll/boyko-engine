//! Generated names are the snake_case COLLAPSE of the flattened state path, and that collapse
//! is lossy: `AB` and `Ab` are two distinct enum variants that mint one `__state_chart_m__ab`.
//! rustc would report the duplicate on generated tokens; the chart reports it on the two states.
//!
//! The colliding PAIR moved at rung A7, when the collapse stopped spelling a run of capitals one
//! letter per word (`GOLD` → `g_o_l_d` → `gold`). `AB`/`A_b`, the pair this fixture shipped with,
//! no longer collides. A compile-fail fixture whose input stopped being a fault is a fixture that
//! passes for the wrong reason, so the input was re-aimed at a pair the CURRENT rule collapses
//! rather than the golden re-blessed.
//!
//! Two things moved at R2, when the flattening became `boyko_macros::state_chart!` and a leaf's
//! routes merged into ONE system. The minted name lost its event segment
//! (`__aether_m__ab__e` → `__state_chart_m__ab`), and the caret moved from the second `on` onto
//! the second STATE — which is the token that actually collides; the route was never the fault.
//!
//! The two routes are a CYCLE (`AB → Ab → AB`) rather than the pair of self-loops this fixture
//! shipped with, so the fixture pins exactly one fault. Self-loops leave `Ab` unreachable, and a
//! compile-fail fixture carrying a second latent fault passes even when the fault it names stops
//! firing — the collision check simply runs first.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial AB;
        state AB {
            on E => Ab;
        }
        state Ab {
            on E => AB;
        }
    }
}

fn main() {}
