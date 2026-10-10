//! R2 reachability: a state no transition targets, and which is not the chart's `initial`, can
//! never be entered. The chart reads as if `Victory` were live; the machine can never get there.
//!
//! Severity is a hard ERROR rather than a warning, and the reason is mechanical rather than
//! stylistic: stable proc-macros have no diagnostic channel (`proc_macro::Diagnostic` is
//! nightly), so "warn" would mean emitting nothing at all — an analysis whose finding is invisible
//! is not a softer gate, it is no gate. Every sibling chart fault here is already an error.
//!
//! The dead COMPOSITE below is reported once, at its root, rather than once per leaf underneath
//! it: an unused eight-leaf branch should read as one finding.
use aether::aether;

aether! {
    plugin P;

    machine M {
        initial Playing;

        state Playing {
            on Lost => GameOver;
        }

        state GameOver {
            on Restart => Playing;
        }

        state Victory {
            on Restart => Playing;
        }

        state Credits {
            initial Roll;
            state Roll {
                on Skip => Credits.Done;
            }
            state Done {
                on Skip => Playing;
            }
        }
    }
}

fn main() {}
