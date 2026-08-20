//! Reachability must not decide whether a name is checked: `Lonely` is never a transition
//! target, so the lazy retargeting path would never have looked at its `initial` typo.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial Idle;
        state Idle {
            on Go => Idle;
        }
        state Lonely {
            initial Runing;
            state Running {}
        }
    }
}

fn main() {}
