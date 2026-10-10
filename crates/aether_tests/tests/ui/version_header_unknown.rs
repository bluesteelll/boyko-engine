//! §6.3's version gate, at the boundary it exists to hold: a block that declares a syntax this
//! aether does not speak is refused on the VERSION's own token, with the supported list — the
//! §6.1 canonical shape applied to the header. The alternative is what makes the gate worth
//! having: without it, v2 source parses against the v1 table and every construct reports its own
//! unrelated fault.
use aether::aether;

aether! {
    aether v2;

    component Health { hp: f32 }
}

fn main() {}
