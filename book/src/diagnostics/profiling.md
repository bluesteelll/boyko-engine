# Profiling

> The engine's built-in CPU profiler: zones compiled in by build profile, armed at run time, folded
> into an ECS-owned store once per frame, and read by systems, an overlay or a telemetry file.

## What it is

Profiling spans three places:

| Part | Crate | Role |
|------|-------|------|
| Substrate | [`boyko_diag`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_diag) | The clock, the per-lane sample rings, the zone macros and the build-profile table. It has no dependencies and prints nothing itself. |
| Store | `boyko_ecs`, module [`profiling`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ecs/src/ecs/core/profiling) | The `Profiler` resource: a reservation-backed store that the fold fills once per frame, plus `ProfilerPlugin` and the engine's own zones. |
| Host | `boyko_app` | `EnginePlugins` arms the profiler from the environment (`BOYKO_PROFILE_ON`). The module `profiling` holds the telemetry writer and the measurement artifacts. |

A zone site passes two gates. The **tier** gate is a compile-time constant from the build profile,
so a site above the profile's tier is deleted. The **scope** gate is one atomic load of the arm
mask, so an unarmed site costs a load and a branch. Samples go into per-lane rings in `.bss`; the
fold copies them into the store at the top of each frame, outside the schedules it measures.

Logging shares the substrate: one clock and one lane numbering, so a log line can be placed inside
the zone it was written in. See [Logging & Error Codes](logging.md).

## Turning it on

**At build time**, `BOYKO_PROFILE` picks the profiling tier (and the log ceiling):

| `BOYKO_PROFILE` | Zone tier kept |
|-----------------|----------------|
| `dev` (default) | `Deep`: everything |
| `editor` | `Dev` |
| `shipping`, `shipping-min`, `off` | `Always` only |

`ZoneTier::Always` sites survive every profile, so a shipped game can still measure its own frame.
Under `custom`, `BOYKO_PROFILING_TIER` and the other `BOYKO_PROFILING_*` knobs override single
values.

**At run time**, nothing is armed by default. In the windowed host, set `BOYKO_PROFILE_ON`:

```powershell
$env:BOYKO_PROFILE_ON = "1"
cargo run -p boyko-app --example room
```

`EnginePlugins` adds `ProfilerPlugin` unconditionally; its `build` neither reserves nor commits memory. The
variable makes the host call `Profiler::arm`, which commits the store, calibrates the clock and
publishes the lane buffers. A host of your own calls `world.resource_mut::<Profiler>().arm(cfg)`
itself.

`ProfilerPlugin` binds the process-global profiler to one world. In a second world it inserts no
`Profiler`, and that world runs unprofiled.

## What the engine measures

Without any code of yours, an armed profiler records:

| Zone | Tier | What it brackets |
|------|------|------------------|
| `__frame` | `Always` | One whole frame; the primary CPU number. |
| `__events` | `Dev` | The event-buffer update. |
| `__fixed_step` | `Dev` | One run of the fixed schedule (N per frame). |
| `__main_run` | `Dev` | The main schedule. |
| `__fold` | `Always` | The profiler's own fold, outside `__frame`. |
| `__round`, `__round_width` | `Deep` | Each scheduler round that dispatched work, and how many systems it ran. |
| one zone per system | `Deep` | Each system's execution span. `Schedule::system_zones()` maps system names to zone ids. |

A zone whose tier the build profile does not keep is compiled out, so per-system spans exist only
under the `dev` profile.

With `boyko_ecs`'s `profiling-analysis` feature (admitted only by the `dev` and `editor` profiles),
the store also keeps per-system intervals and an analysis report that reads them against the
scheduler's conflict graph.

## Profiling your own code

A game crate declares its partition once at the crate root, then declares zones and opens them:

```rust,ignore
// lib.rs or main.rs: this crate's zones belong to the user region.
boyko_diag::profiling_partition!(User);

boyko_diag::declare_zone!(
    AI_THINK,
    name = "game.ai.think",
    scope = boyko_diag::profiling_abi::USER_SCOPE_BASE,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);

fn think() {
    // Bind the guard: `let _ = zone!(..)` would close the zone at once.
    let _zone = boyko_diag::zone!(AI_THINK);
    // ... the work being measured ...
}
```

- `zone!` returns an `Option<ZoneGuard>`: `None` when the tier or the scope refuses. The guard
  records the span when it drops.
- The partition keeps a game's zone ids and ring region apart from the engine's. A non-engine crate
  that declares `profiling_partition!(Engine)` does not compile.
- Scopes `USER_SCOPE_BASE` and above are the game's; scopes below it belong to the engine.
- `zone_dyn!`, `counter_dyn!` and `gauge_dyn!` take a zone registered at run time instead of a
  static one.

### Switching scopes at run time

A zone records only while its scope bit is armed. The engine's own zones use a scope that
`Profiler::arm` arms. A game's scopes, `USER_SCOPE_BASE` up to 63, are entities: `ProfilingScope`
names the bit, and the enable tag `ProfilingScopeEnabled` switches it.

```rust,ignore
use boyko_diag::profiling_abi::USER_SCOPE_BASE;
use boyko_ecs::ecs::core::profiling::{ProfilingScope, ProfilingScopeEnabled};
use boyko_ecs::prelude::*;

fn setup_scopes(mut commands: Commands) {
    // The bit the `AI_THINK` zone above was declared with.
    let ai = commands.spawn(ProfilingScope { bit: USER_SCOPE_BASE as u8, name: "ai" }).id();
    commands.entity(ai).enable::<ProfilingScopeEnabled>();
}
```

Disable the tag to stop recording that scope. The fold projects the enable bits into the arm mask
once per frame, so a toggle takes effect on the next frame. `register_scope(name)` mints a free bit
in the game range at run time, for zones registered at run time. See
[Enable Tags](../concepts/enable-tags.md).

## Reading the results

- **From a system.** `Res<Profiler>` exposes the store: `cell(row, zone)` for one frame's
  count/total/min/max, `lifetime(zone)` for the totals since arming, `histogram(zone)` once you
  `subscribe_histogram(zone)`, and `observed_kind(zone)`, which says whether a total is ticks, a
  count or a gauge level. Resolve zone names to ids once at setup and keep them in a
  `ProfiledZone` component.
- **On screen.** `boyko_ui` ships `ProfilingOverlayPlugin`, a reference overlay that writes the
  statistics of each `ProfiledZone` row into UI text without allocating. Add it after
  `ProfilerPlugin`. The windowed host does not draw UI yet, so the overlay needs a host that
  composites UI; see [UI Overview](../ui/overview.md).
- **To a file.** `boyko_app::profiling::stream::TelemetryStream` writes windowed statistics in the
  `boyko_diag::telemetry` format. The windowed host does not open a stream by itself. Decode a file
  with:

  ```powershell
  cargo run -p prof_decode -- telemetry.bin --zones --windows
  ```

  `prof_decode` prints `-` for a value the stream does not carry, rather than a zero.

The profiler reports its own losses (a full ring, a refused registration) through the logger at the
next fold, as `boyko-W92xx` and `boyko-E92xx` codes.

## Performance characteristics

| State | Cost per zone site |
|-------|--------------------|
| Tier above the build profile | None: the site is compiled out. |
| Compiled in, scope not armed | One atomic load and a predicted branch. |
| Armed | Two clock reads, two relaxed atomic adds and one 24-byte ring write. |

The fold runs once per frame and is itself measured (`__fold`). Measured costs live in
[Benchmarks](../reference/benchmarks.md).

## See also

- [Logging & Error Codes](logging.md): the shared substrate and the `BOYKO_PROFILE` table.
- [Parallel Scheduler](../scheduler.md): the rounds and systems the engine zones measure.
- [Enable Tags](../concepts/enable-tags.md): how scopes switch.
- Source: [`crates/boyko_diag`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_diag),
  [`profiling/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_ecs/src/ecs/core/profiling)
  and [`tools/prof_decode`](https://github.com/bluesteelll/boyko-engine/tree/master/tools/prof_decode).
