//! A duplicate material key, with BOTH spans — the one diagnostic class where a single span
//! answers a question the reader does not have. They are looking at the duplicate; what they need
//! is the earlier line they already said it on, which in a seven-key material can be pages away.
//!
//! Golden rather than a message-only unit test for the same reason `material_duplicate_name` is
//! one: only a real compilation can hold two spans in place, and the second one is exactly what a
//! future edit would drop silently.
use aether::aether;

aether! {
    material gold {
        base: (1.0, 0.72, 0.30),
        roughness: 0.14,
        metallic: 1.0,
        roughness: 0.20,
    }
}

fn main() {}
