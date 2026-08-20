//! §3.3: a system runs on exactly one schedule — the SECOND `on` carries the error.
use aether::aether;

aether! {
    plugin P;

    system tick() on update on fixed {}
}

fn main() {}
