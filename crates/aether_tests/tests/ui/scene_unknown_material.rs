//! §3.7's own diagnostic: a `material:` prop naming no sibling material. The error must land on
//! the REFERENCE, list the materials that do exist, and suggest the near one.
use aether::aether;

aether! {
    material gold { base: (1.0, 0.72, 0.30) }
    material lamp { base: (0.02, 0.02, 0.02) }

    scene lab {
        entity { material: gol }
    }
}

fn main() {}
