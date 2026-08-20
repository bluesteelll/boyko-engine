//! §7.3 / §8 R3's dedicated case: **one broken construct, every sibling still resolvable.**
//!
//! This is the rung-A7 gate for the recovery contract, and it is a `compile_fail` case whose
//! whole point is what is NOT in the `.stderr`. A macro that aborts the block on the first parse
//! error erases every item it would have emitted, so a single typo becomes one honest error plus
//! an unresolved-name error for each of the block's other constructs — the concentrated failure
//! mode `view!`-style macros are reported for, and the one that makes an editor useless exactly
//! while the author is mid-edit.
//!
//! The block below has ONE fault (a field written without its `:`). Everything else in the file
//! references a name that only exists if recovery worked:
//!
//! * `Health` — a sibling declared BEFORE the fault (survives even in an abort-at-first-error
//!   parser, so it proves nothing alone);
//! * `Player` and `tick` — siblings declared AFTER it, which an aborting parser never reaches;
//! * `Broken` — the broken construct's OWN name, resolving through the §7.3 stub.
//!
//! So the pinned `.stderr` must hold exactly one error. Any regression in recovery shows up here
//! as extra E0422/E0425 entries, blessed only by someone reading what they are.
use aether::aether;

aether! {
    component Health { hp: f32 }

    component Broken { hp f32 }

    tag Player;

    system tick(q: query<&Health>) { let _ = &q; }
}

fn main() {
    let _ = Health { hp: 1.0 };
    let _ = Player;
    let _ = Broken;
    let _ = tick;
}
