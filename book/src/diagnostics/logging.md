# Logging & Error Codes

> `boyko_log` is the engine's own structured logger: five macros, per-target levels, a compile-time
> ceiling that deletes disabled call sites, and a registry of diagnostic codes such as
> `boyko-E3002`.

## What it is

[`boyko_log`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_log) replaces
`println!`, `log` and `tracing` for the engine and for games built on it. No workspace crate depends
on `log` or `tracing` directly; a test refuses the dependency.

A call site passes up to three gates:

| Gate | Decided | What it checks |
|------|---------|----------------|
| Target ceiling | compile time | The target's own `STATIC_CEILING`. |
| Global ceiling | compile time | The build profile's `GLOBAL_CEILING` (see [build profiles](#build-profiles)). |
| Runtime level | run time | One byte per target, read with one relaxed load. |

The gates are joined with `&&`, so the format arguments are evaluated only when every gate passes.
A site above a compile-time ceiling is deleted together with its arguments. A site under the
ceiling whose target is switched off costs one byte load and one well-predicted branch.

Records are not formatted on the calling thread. The caller encodes the arguments into its lane's
ring buffer; a drain formats and writes them later.

**A default run logs nothing.** Every target's runtime level starts at `Off`, and nothing in the
crate runs at process start. You turn logging on at boot; see [Turning logging on](#turning-logging-on).

## Emitting records

The macros take a target type, then (for `warn!` and `error!`) a diagnostic code, then a format
literal and its arguments:

```rust,ignore
use boyko_log::{App, Render};

fn report(frame: u64, draws: u32) {
    boyko_log::info!(Render, "frame {} recorded {} draws", frame, draws);
    boyko_log::debug!(App, "frame {} done", frame);
}
```

| Macro | Code required | Notes |
|-------|---------------|-------|
| `error!`, `warn!` | yes: an `ErrorCode` / `WarnCode` | A wrong class does not compile. Each code declares a rate policy. |
| `info!`, `debug!`, `trace!` | no | — |
| `error_kv!`, `warn_kv!`, `info_kv!`, `debug_kv!`, `trace_kv!` | as above | Named fields: `info_kv!(T, "msg", count = n)`. |
| `dyn_error!`, `dyn_warn!`, `dyn_info!`, `dyn_debug!`, `dyn_trace!` | as above | The target is a run-time `TargetId` instead of a type. |

The engine's targets are unit types in `boyko_log` named after their subsystem: `Ecs`, `Query`,
`Events`, `Assets`, `Physics`, `Render`, `RhiVulkan`, `Ui`, `App`, `Host`, `Profiling` and so on.
Their names in control strings are lower case (`ecs`, `render`, `rhivulkan`).

## Turning logging on

The windowed host (`EnginePlugins` in `boyko_app`) reads these environment variables at boot:

| Variable | Effect |
|----------|--------|
| `BOYKO_LOG` | One level for every engine target: `off`, `error`, `warn`, `info`, `debug` or `trace`. An unrecognised value enables `info`, and the first record says what was applied. Output goes to stderr. |
| `BOYKO_LOG_PRESET` | A whole sink configuration: `dev`, `editor`, `shipping`, `shipping-min` or `off`. `BOYKO_LOG` still sets the level on top of it, except under `off`. |
| `BOYKO_LOG_FILE`, `BOYKO_LOG_BLOG` | With a preset: the text-log and binary-log paths. |

```powershell
$env:BOYKO_LOG = "debug"
cargo run -p boyko-app --example room
```

The presets combine the console, a text file, a binary file, file rotation and either a resident
sink thread or an in-frame drain:

| Preset | Destinations | Drain |
|--------|--------------|-------|
| `dev` | console, text file, ECS ring | sink thread |
| `editor` | console, text file (rotating), ECS ring | sink thread |
| `shipping` | binary file (rotating) | sink thread |
| `shipping-min` | text file (rotating) | in-frame, no extra thread |
| `off` | none | — |

The binary format (`.blog`) turns back into text with the `logdec` tool:
`cargo run -p boyko-log --bin logdec -- <file.blog>`.

A host without `boyko_app` drives the same lifecycle itself: `boyko_log::lifecycle::boot`
(or `boot_preset`), then `set_target_level` for the targets it wants, then `enable`.

### Per-target control strings

`boyko_log::control::apply_control_spec` applies a control string to engine and dynamic targets:

```text
ecs=debug, render=warn, physics=trace/4!
```

Each clause is `name=level`, with an optional `/shift` (keep 1 record in 2<sup>shift</sup>) and an
optional `!` (deliver this target's records synchronously, bypassing the ring). Targets the string
does not name keep their state. An unknown name rejects the whole string, and nothing is applied.
`load_control` reads the same grammar from the `BOYKO_LOG` variable or from a file. The windowed host
does not call either; its `BOYKO_LOG` is the single level above.

## Diagnostic codes

Every `warn!` and `error!` carries a code from one registry,
[`codes.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_log/src/codes.rs).
A code prints as `boyko-` plus a class letter and four digits:

| Class | Type | Used by |
|-------|------|---------|
| `E` | `ErrorCode` | `error!` |
| `W` | `WarnCode` | `warn!` |
| `B` | `PanicCode` | panic messages |

Every live code has a page under
[`docs/diagnostics/`](https://github.com/bluesteelll/boyko-engine/tree/master/docs/diagnostics)
that says what happened, why and how to fix it. When a run prints `boyko-E3002`, read
`docs/diagnostics/E3002.md`. A registry test fails for a live code with no page or no emitter.

Each code also declares how often it is delivered: `RatePolicy::Every`, `Once`, `OnceCounted`,
`EveryN(n)` or `MinIntervalMs(ms)`. The rate check is the fourth gate of `warn!` and `error!`.

A handful of terminal errors, such as a failed host boot, also print to stderr when no sink is
running, so a process never exits silently.

## Logging from a game

A game declares its own targets in the downstream band (ids 96 to 223) and its own code table, which
names a prefix:

```rust,ignore
use boyko_log::target::LogTarget;
use boyko_log::{Level, set_target_level};

boyko_log::define_target!(pub Combat, name = "combat", id = 96, ceiling = Level::Trace);

mod codes {
    use boyko_log::RatePolicy;
    boyko_log::declare_codes! {
        prefix = "mygame",
        (1, W, MYGAME_W0001, RatePolicy::Once, "the spawn budget is nearly spent"),
    }
}

fn boot_logging() {
    // Claims id 96 for "combat"; `false` (and boyko-E0104) if another name holds it.
    let _ = Combat::register();
    set_target_level(Combat::ID, Level::Debug);
}

fn on_wave(wave: u32, alive: u32) {
    boyko_log::info!(Combat, "wave {} started, {} alive", wave, alive);
    if alive > 900 {
        boyko_log::warn!(Combat, codes::MYGAME_W0001, "{} enemies alive", alive);
    }
}
```

- An id outside 96..=223 does not compile. Two crates claiming one id are reported at
  `register()`, with both names.
- `BOYKO_LOG` sets only the engine's targets. Set your own targets' levels yourself, as above.
- The table stores its prefix (`codes::PREFIX`), but the printer does not use it. The emission
  macros stamp the engine prefix on every call site, so the `warn!` above prints `boyko-W0001`,
  not `mygame-W0001`. In the output, a downstream code cannot be told apart from an engine code
  with the same class and number.
- Targets named only at run time (a mod, a script namespace) come from
  `boyko_log::target::register_dynamic_target(name, control)`: 32 slots, idempotent by name.

## Build profiles

The compile-time ceilings come from the `BOYKO_PROFILE` environment variable, read once by the
build script of `boyko_diag` at compile time:

| `BOYKO_PROFILE` | Log ceiling |
|-----------------|-------------|
| `dev` (default) | `trace` |
| `editor` | `debug` |
| `shipping` | `info` |
| `shipping-min` | `warn` |
| `off` | `off`: no site survives |

`custom` unlocks per-knob overrides such as `BOYKO_LOG_MAX_LEVEL`. Setting a knob next to a named
profile is a compile error. The same variable sets the profiler's tier; see
[Profiling](profiling.md).

## In the ECS

`LogPlugin` (in `boyko_ecs`, added by `EnginePlugins`) registers `log_drain_system` and two
resources, `LogRing` and `LogStats`. Under the `shipping-min` preset that system is the drain.
`LogRing` holds formatted lines for an in-game reader when the sink configuration enables the ECS
ring (the `dev` and `editor` presets). The host's plain `BOYKO_LOG` configuration does not.

## See also

- [Profiling](profiling.md): the other half of the diagnostics stack.
- [App & Plugins](../app/plugins.md): `EnginePlugins` and what it installs.
- [Windowed Host](../app/windowed-host.md): the other `BOYKO_*` environment variables.
- Source: [`crates/boyko_log`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_log);
  code pages in [`docs/diagnostics/`](https://github.com/bluesteelll/boyko-engine/tree/master/docs/diagnostics).
