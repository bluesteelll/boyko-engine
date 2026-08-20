//! §3.6's unknown-key diagnostic: the exhaustive key list plus a did-you-mean, on the key's own
//! span. A key Aether silently ignored would ship a material whose roughness the author believed
//! they had set.
use aether::aether;

aether! {
    material gold { base: (1.0, 0.72, 0.30), roughnes: 0.14 }
}

fn main() {}
