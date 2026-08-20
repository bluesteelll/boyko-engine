//! A planned construct names its rung — a misspelling and a not-yet-shipped construct are
//! different failures and deserve different messages.
//!
//! The subject was `material` until rung A5 shipped it; `scene` (rung A6) is now the only
//! remaining planned construct, and the message must name the rungs this build actually carries.
use aether::aether;

aether! {
    scene lab { }
}

fn main() {}
