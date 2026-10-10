//! §3.5: a transition targeting a composite with no `initial` names the fix and a leaf path.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial Boot;
        state Boot {
            on Go => Playing;
        }
        state Playing {
            state Running {}
        }
    }
}

fn main() {}
