//! W2's fn-half probe: a duplicate where ONE OF THE TWO did not parse.
//!
//! `material gold` is declared twice; the second one's `metallic` value is missing, so it fails to
//! parse. If §4's duplicate rule read the broken construct as absent, the collision would go
//! unreported HERE and surface downstream as rustc's E0428 over two macro-generated fns — which
//! the rung-A5 measurement showed puts both of its labels on the `aether!` token and names no user
//! token anywhere. That is the precise fault this rule was written to own.
//!
//! Running over `constructs ∪ broken`, Aether reports it itself, on the second declaration's own
//! ident, with a second span at the first. The pinned `.stderr` is the evidence for both halves:
//! the two-span Aether diagnostic is present, and E0428 is absent.
use aether::aether;

aether! {
    material gold { base: (1.0, 0.72, 0.30) }

    material gold { base: (0.1, 0.1, 0.1), metallic: }
}

fn main() {}
