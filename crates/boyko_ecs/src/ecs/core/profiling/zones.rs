//! The engine's own zone sites — the frame driver's four, and the instrument's one.
//!
//! # Why the frame is FOUR zones and not one
//!
//! An earlier definition said the primary CPU number was *"the `Schedule::run` span"*. The host's
//! frame is `Time → events → Fixed×N → Main` — **two schedules, and `Fixed` runs N times** — so
//! "the `Schedule::run` span" is not one interval, and "the fold is outside the primary number" was
//! undefined across N+1 runs.
//!
//! | Zone | Bracket | Cardinality |
//! |---|---|---|
//! | [`FRAME`] | `update_with_delta` entry (**after** the fold returns) → exit | 1 per frame — **this is the primary CPU number** |
//! | [`EVENTS`] | step ③ `update_events` | 0 or 1 |
//! | [`FIXED_STEP`] | one `fixed.run(world)` inside step ④ | **N** per frame |
//! | [`MAIN_RUN`] | step ⑤ `schedule.run(world)` | 1 |
//! | [`FOLD`] | the fold itself | 1 per frame, **outside [`FRAME`] by construction** |
//!
//! # Nothing here is copied into `FrameRecord`, deliberately
//!
//! The corpus's `FrameRecord` carries `run_gross`, `fixed_total`, `main_total`,
//! `instrument_measured` and `fixed_steps`. Every one of them is **already** in a cell: `run_gross`
//! is `FRAME`'s `total` for that frame row, `fixed_total` is `FIXED_STEP`'s, `instrument_measured`
//! is `FOLD`'s, and `fixed_steps` — the substep count N — is `FIXED_STEP`'s `count`, because a
//! zone that opens N times per frame counts N.
//!
//! Copying them into the frame record would be a second statement of five facts the store already
//! holds, in a struct that is written by a different code path — which is how two numbers for one
//! quantity come to disagree. So `FrameRecord` does not grow at this rung, and the reducer reads
//! the cells like it reads every other zone's.
//!
//! # Tiers
//!
//! [`FRAME`] and [`FOLD`] are `Always`: frame time ships, and the instrument's own cost must be
//! measurable in the configuration a title actually runs — an instrument whose cost is only visible
//! in the build where it does not matter is not disclosed at all. The three inner brackets are
//! `Dev`: they are subsystem spans, and a shipped title pays nothing for them because a `const
//! false` in the gate's `&&` chain deletes the arm and its operands.

use boyko_diag::declare_zone;
use boyko_diag::profiling_abi::ZoneTier;

use crate::ecs::core::profiling::store::ROOT_SCOPE;

declare_zone!(
    FRAME,
    name = "__frame",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);

declare_zone!(
    EVENTS,
    name = "__events",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Dev,
);

declare_zone!(
    FIXED_STEP,
    name = "__fixed_step",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Dev,
);

declare_zone!(
    MAIN_RUN,
    name = "__main_run",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Dev,
);

declare_zone!(
    FOLD,
    name = "__fold",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);
