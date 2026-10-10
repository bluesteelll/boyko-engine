//! Rung A7's DX surface, in a target the LINTER compiles.
//!
//! # The lint half (the recorded A7 candidate, measured before it was fixed)
//!
//! An eight-param `system` produced, under `cargo clippy --all-targets -- -D warnings`:
//!
//! ```text
//! warning: this function has too many arguments (8/7)
//!   --> crates\aether_tests\tests\a7_probe.rs:21:1
//!    |
//! 21 | / aether! {
//!    | |_^
//!    = note: this warning originates in the macro `aether`
//! ```
//!
//! — a lint about a signature the author did not write, spanned on the macro token, where no
//! `#[allow]` of theirs can reach it and "take fewer arguments" is not advice about a system's
//! data dependencies. `expand.rs::arity_allow` now rides on every generated fn whose arity the
//! user controls, and THIS FILE is the gate: the `wide` system below has eight params, so the
//! existing `-D warnings` clippy run fails the day the suppression is dropped. Nothing here
//! asserts it in Rust — the assertion IS the target compiling clean under the linter.
//!
//! # The header half
//!
//! §6.3's `aether v1;` is exercised where it has to work: a real block, on a real `App`, whose
//! systems must still register and run with the header present. A header that parsed but changed
//! the expansion would pass every parse test and break every user.

use aether::aether;
use boyko_ecs::App;

aether! {
    aether v1;

    component Marker {
        n: u32,
    }

    plugin Wide;

    system boot(mut cmds: commands) on startup {
        cmds.spawn(Marker { n: 7 });
    }

    // EIGHT params — one past clippy's default `too-many-arguments-threshold`. The count is the
    // point of the fixture; the body only has to use each binding so nothing is optimized into
    // an unused-variable warning.
    system wide(
        q: query<&Marker>,
        mut cmds: commands,
        probe: mut res<Probe>,
        a: local<u32>,
        b: local<u8>,
        c: local<u16>,
        d: local<i8>,
        e: local<i16>
    ) on update {
        for m in &q {
            probe.seen += m.n;
        }
        probe.frames += 1;
        let _ = (&mut cmds, *a, *b, *c, *d, *e);
    }
}

/// The observation channel — aether systems are plain fns, so a resource is the only honest way
/// to see one run.
#[derive(boyko_macros::Resource)]
struct Probe {
    seen: u32,
    frames: u32,
}

#[test]
fn a_version_headed_block_registers_and_runs_its_systems() {
    let mut app = App::new();
    app.insert_resource(Probe { seen: 0, frames: 0 });
    app.add_plugin(Wide);

    app.update();
    app.update();

    let probe = app.world_mut().resource::<Probe>();
    assert_eq!(probe.frames, 2, "the eight-param system ran once per frame");
    assert_eq!(probe.seen, 14, "it saw the startup spawn through its query, both frames");
}
