//! C1's case: a **half-typed `plugin`** — the ordinary mid-edit state of every block that has one
//! — must not erase the block.
//!
//! `plugin Arena` is missing its `;`, so it fails to parse. Under the shape this rung first
//! shipped, that single fault made §4's "clauses need a plugin" rule fire against every sibling
//! system, the whole-block rule failed, and the expansion was DROPPED: one typo erased `Health`,
//! `boot` and `tick`, and every downstream use of them became its own unresolved-name error — the
//! exact sea §7.3 exists to prevent, re-created by the mechanism built to prevent it.
//!
//! §4's rules now run over `constructs ∪ broken`: the broken `plugin` still OCCUPIES the plugin
//! slot, so the clause rule does not fire and every sibling expands. The contract this file pins
//! is therefore the SIZE of the `.stderr` — one error, for the one fault — against a `main` that
//! uses all four names.
//!
//! `add_plugin(Arena)` is the second half of the measurement, and the reason the `plugin` stub is
//! not just `pub struct Arena;`. A plugin is only ever referenced through that call, which needs
//! the TRAIT: a bare unit struct would trade "cannot find value `Arena`" for "the trait bound
//! `Arena: Plugin` is not satisfied" at the same line — a different error, not one fewer. With the
//! `impl Plugin` in the stub, the call compiles and the file is left with exactly the author's own
//! typo to fix.
use aether::aether;
use boyko_ecs::App;

aether! {
    component Health { hp: f32 }

    plugin Arena

    system boot(mut cmds: commands) on startup {
        cmds.spawn(Health { hp: 1.0 });
    }

    system tick(q: query<&Health>) on update { let _ = &q; }
}

fn main() {
    let mut app = App::new();
    app.add_plugin(Arena);
    let _ = Health { hp: 2.0 };
    let _ = boot;
    let _ = tick;
}
