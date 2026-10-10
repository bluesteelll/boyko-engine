//! §3.7's symmetric diagnostic: a `mesh` node naming no `let` binding of its own scene.
//!
//! TWO scenes, deliberately. A mesh binding table is SCENE-scoped, not block-scoped — unlike a
//! `material`, which §4 puts in the block's symbol table. `props` below names `floor`, which IS
//! declared in this `aether!` block, just not in `props`. A message phrased "in this aether block"
//! would therefore be a lie the reader can only disprove by hunting; the scope has to be named.
use aether::aether;

aether! {
    scene lab {
        let floor = plane(22.0);
        let block = cube(1.0);

        mesh floor;
        mesh block;
    }

    scene props {
        let crate_box = cube(1.0);

        mesh floor;
    }
}

fn main() {}
