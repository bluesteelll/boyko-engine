//! §3.7's published per-head refusal, verbatim: a hemisphere fill has no shadow-caster form. The
//! flag would otherwise expand to a `ShadowCaster` on an entity nothing can cast from.
use aether::aether;

aether! {
    scene lab {
        sky { sky: (0.28, 0.36, 0.50), ground: (0.15, 0.14, 0.13), casts_shadow }
    }
}

fn main() {}
