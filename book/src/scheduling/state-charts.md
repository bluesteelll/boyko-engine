# State Charts

> `state_chart!` turns a nested state machine into a flat `States` enum, one system per leaf state,
> and two functions that register them.

## What it is

[States](states.md) give you one enum value per state and run conditions such as
`in_state(..)`. A **state chart** adds structure on top: nested states, `enter` / `exit` actions,
and event-driven transitions that a parent state can declare once for all its children.

The `state_chart!` macro in `boyko_macros` does the flattening at compile time:

- each **leaf** state becomes one variant of a plain `States` enum;
- each **composite** state becomes a `const fn in_<name>(self) -> bool` predicate on that enum;
- each leaf that has transitions gets **one system**, registered with `run_if(in_state(leaf))`;
- every exit/enter chain is resolved in the macro, so the runtime walks no tree and keeps no extra
  data.

The macro is the engine's only state-machine code generator. The Aether language's `machine`
construct lowers onto it; see [State Machines](../aether/state-machines.md).

## Defining a chart

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_macros::{Component, Resource, event};

#[event] struct AssetsReady   { #[parameter] count: u32 }
#[event] struct PausePressed  { #[parameter] frame: u32 }
#[event] struct PlayerDied    { #[parameter] frame: u32 }
#[event] struct RestartPressed { #[parameter] frame: u32 }

#[derive(Resource)]
struct Score { lives: u32 }

#[derive(Component)]
struct HudRoot;

boyko_macros::state_chart! {
    chart GameFlow;
    initial Boot;

    state Boot { on AssetsReady => Playing; }
    state Playing {
        initial Running;
        enter(mut cmds: Commands) { cmds.spawn(HudRoot); }
        state Running { on PausePressed => Playing.Paused; }
        state Paused  { on PausePressed => Playing.Running; }
        // Declared on the composite, inherited by Running and Paused.
        on PlayerDied(score: Res<Score>) if score.lives == 0 => GameOver;
    }
    state GameOver { on RestartPressed => Boot; }
}
```

The grammar:

| Item | Form |
|------|------|
| Head | `chart Name;` then `initial State;` |
| State | `state Name { ... }`. A state with nested states is a composite; give it `initial Child;` so a transition into it knows which leaf to enter. |
| Entry / exit action | `enter(params) { body }`, `exit(params) { body }` |
| Transition | `on Event(params) if guard => Target { action }`, or end it with `;` instead of an action block. `(params)` and `if guard` are optional. |
| Target | A path from the chart root: `GameOver`, `Playing.Paused`. A composite target enters its `initial` leaf. |

Parameters are ordinary system parameters written as `name: Type` (`Res<T>`, `ResMut<T>`,
`Commands`, `Query<..>`, ...). Each event type must be an [`#[event]`](../concepts/events.md) and must
be registered before the schedule runs. The guard sees the parameters, not the event's fields.

The expansion refers to `::boyko_ecs::…` paths, so the crate that invokes the macro must depend on
`boyko-ecs` under its own name.

## What it generates

For the chart above:

```rust,ignore
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameFlow { Boot, PlayingRunning, PlayingPaused, GameOver }

impl States for GameFlow {}

impl GameFlow {
    pub const fn in_playing(self) -> bool { /* PlayingRunning | PlayingPaused */ }
}

// One system per leaf with transitions:
//   __state_chart_game_flow__boot, __state_chart_game_flow__playing_running, ...
// Plus the two registration functions:
pub fn __state_chart_install_game_flow(app: &mut App) { /* insert_state + entry chain */ }
pub fn __state_chart_systems_game_flow(b: &mut ScheduleBuilder) { /* add_system(..).run_if(in_state(..)) */ }
```

Leaf variants concatenate the path (`Playing.Running` becomes `PlayingRunning`). Generated function
names use the snake-case form of the chart and leaf names.

## Registering a chart

Call the two generated functions from a plugin:

```rust,ignore
use boyko_ecs::prelude::*;

struct GameFlowPlugin;

impl Plugin for GameFlowPlugin {
    fn build(&self, app: &mut App) {
        // insert_state(GameFlow::Boot), plus a startup system running the
        // initial leaf's `enter` chain, outermost first.
        __state_chart_install_game_flow(app);
        // One `run_if(in_state(leaf))` system per leaf that has transitions.
        app.add_systems_cfg(|b| __state_chart_systems_game_flow(b));
    }
}
```

`insert_state` alone sets a value but runs no `enter` action. The install function adds the
startup system so the initial state's entry actions run once.

## Semantics

- **Innermost wins.** A transition declared on a composite applies to every descendant leaf that
  does not declare its own transition for the same event.
- **Exit and enter follow the least common ancestor.** A transition exits the source-side states
  below the common ancestor, innermost first, then runs the transition's action, then enters the
  target-side states, outermost first. `Running → Paused` therefore neither exits nor re-enters
  `Playing`.
- **One transition per frame, first declared wins.** A leaf's routes are merged into one system.
  It drains every event it listens to, picks the first route in declaration order that accepted an
  event, and runs only that route's exit → action → enter chain. It then writes
  `NextState::Pending(target)`; the state changes at the next state-transition point, like any
  `NextState` write.
- **A failed guard skips the event.** It does not consume the frame; another route can still win.

## Charts the macro refuses

These are compile errors, reported at the offending tokens:

- a target, or the chart's `initial`, that is a composite with no `initial` of its own;
- an `initial` that names no child, or an `initial` inside a leaf;
- two transitions on one state for the same event;
- two states that flatten to the same name (`A.BC` and `AB.C` both spell `ABC`), or whose generated
  names collide after snake-casing (`AB` and `Ab` both become `ab`);
- one parameter name used with two different types in the routes and actions a leaf's system
  merges;
- **an unreachable state**: one the machine can never enter by following transitions from the
  chart's initial state. The check follows transitions transitively, so a state targeted only
  from other unreachable states is refused too. There is one error per dead branch: it names the
  branch's outermost state (which covers everything nested in it) and the chart.

The reachability check has one known gap. `NextState<C>` is a public resource, so ordinary code can
move a chart into a state that no transition targets. Such a chart is refused even though the
program would work.

## Performance characteristics

| Aspect | Cost |
|--------|------|
| Systems | One per leaf with transitions, gated by `in_state`. Leaves not current skip their system. |
| Hierarchy at run time | None. Exit/enter chains and inheritance are resolved at compile time. |
| Superstate test | `in_<composite>(self)` is a `const fn` `matches!` over the leaf variants. |
| Extra data | None beyond the `State<C>` / `NextState<C>` resources every `States` type has. |

## See also

- [States](states.md): `States`, `NextState`, `in_state`, `on_enter`, `on_exit`.
- [Events](../concepts/events.md): declaring and registering the events a chart reads.
- [Run Conditions](run-conditions.md): how `run_if` gates a system.
- [State Machines](../aether/state-machines.md): the Aether front-end for the same macro.
- Source: [`state_chart/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_macros/src/state_chart)
  in `boyko_macros`.
