//! The `at BARE_PATH { … }` trap, and the reason it earns a golden of its own: the error it
//! produces CONTRADICTS the source. `camera at MY_POSE { aspect: 1.5 }` parses `MY_POSE { aspect:
//! 1.5 }` as one struct-literal expression — the node body is gone — and the required-key rule
//! then tells an author who is looking straight at `aspect: 1.5` that the node needs an `aspect:`
//! key. The message must name what happened (a struct literal) and the fix (parenthesize), or the
//! author's only remaining move is to add a SECOND `aspect:` and watch that fail too.
//!
//! Pinned as a golden rather than only as a message unit-test because the SPAN is half the
//! contract: the refusal lands on the `camera` head, and the hint below it names the pose.
use aether::aether;

const MY_POSE: () = ();

aether! {
    scene lab {
        camera at MY_POSE { aspect: 1.5 }
    }
}

fn main() {}
