# Per-entity state machines and timers (`machine … on entity`)

The synthesized design (four biased drafts → adversarial verification → three-lens judging), with
every owner fork resolved. Rationale and rejected alternatives → [`DECISIONS.md`](DECISIONS.md)
§M1–M9. Reference scenarios: a 10k-enemy AI chart and a mana-costed ability — both appear below.

## What a machine is

One `#[repr(C)]` **table component** and one generated system. The component holds: the current
leaf byte, the previous leaf byte, a countdown clock `f32`, the inbox bitfield, event payload
slots, and the author's `state { … }` fields — **with field elision**: a field is emitted only if
the chart uses it (a pure event-switched machine compiles to a 1-byte `#[repr(transparent)]`
struct; a `const _: () = assert!(size_of::<M>() == N)` pins every layout). The system is a single
`for_each_chunk_entities` pass: unconditional `clock -= dt` (INFINITY = no timer — the identity
element makes the tick branchless), an inbox test predicted not-taken, then a `match` on the leaf
byte holding the flattened chart. A transition is three in-place stores in the same row visit — no
archetype change, no migration, no scheduler round-trip.

The Harel-lite semantics (nesting, LCA exit/enter chains, innermost-wins inheritance, root-level
handlers) are inherited from the `state_chart!` core (R2); only the two ends differ from the global
form: a component byte instead of `State<S>`, and a `match` instead of `run_if(in_state)`.

## Header groups

```
machine EnemyBrain on entity
with {
    joins    (tf: &Transform, with Enemy)     // extra columns/filters fused into the same pass
    state    { stun_secs: f32 }               // author fields, same struct
    inbox    (StunHit = max, Interrupt)       // event edges; per-event re-deposit policy:
                                              //   replace (default) | max | sum | ignore
    identity                                  // binds `me: Entity` (Entities param, slab lookup
                                              //   on FIRING rows only)
    uses     (focus: res<PlayerFocus>, mut dmg: emit<DamageDealt>)
    fail     CastFailed                       // failure channel for `activate`
    schedule fixed                            // DEFAULT is fixed (owner call, replay determinism);
                                              //   `schedule update` is the explicit exception
    order    (after lift_player_focus)
    history                                   // opt-in shallow history (+1 byte); `history deep`
                                              //   (+8 bytes) also restores the clock and lifts R-HIST
    parallel                                  // opt-in par driver (legal once R4 lands)
    publish  tracked                          // opt-in Mut-driven pass so Changed<M> works;
                                              //   DEFAULT OFF (D6). Bumps ONLY on leaf commit.
}
```

## Transition forms

| Form | Trigger |
|---|---|
| `on E => T` | event `E` deposited into this row |
| `after D => T` | the clock reached zero; `D` is a verbatim expression (`after 2.0`, `after self.stun_secs`) — armed by the transition entering the leaf |
| `when P => T` | polled predicate, true now |
| `while P for D => T` | predicate held for `D` seconds; a false sample **re-arms** the window (one construct because the reset IS the hysteresis semantics) |
| `on E activate => T` | event + the machine-level `can_activate { EXPR else Reason; }` / `commit { … }` gate (the GAS shape: cost, guard and failure reason declared once per machine; mints `enum <M>Fail` and a per-row reason byte) |

Modifiers: `(params)`, `if GUARD`, an action block or `;`, `=> history`, and the mandatory
`else { … } | else ignore;` on guarded per-entity transitions (R-ELSE). A transition declared
outside any `state` is a root-superstate handler, inherited by every leaf and overridden by a
leaf's own handler for the same event. Arbitration: **first declared wins** (M6; the global machine
is aligned to this by the R2 route merge).

## Event routing

`inbox (E)` does not put an `EventReader` into the pass. It generates one **router system per
event**: O(events), resolving the participant to a row via `get_component_mut` (generation check
inside; dead/foreign target = silent `None`) and depositing a bit + payload into the row before the
pass runs (an ordering edge is emitted automatically). The participant's declared context
(`victim: entity(EnemyBrain)`) is checked by a **debug_assert in the router** (F6) — zero release
cost, loud wrong-target sends in debug. Latency: one frame under `EveryFrame`; variable under
`WaitForFixed` on frames with no substep — which is why the machine's schedule domain vs the app's
`EventUpdatePolicy` mismatch is a **build-time diagnostic** (F4+F5 merged).

## Observation from outside

Accessors on the component: `.entered()` (`prev != leaf`, same frame, free), `.exited()`,
`.fail()` (the activation-failure reason byte, also mirrored to the `fail` event lane for off-row
consumers). A system whose body names them without `order (after <Machine>)` is a compile error
(R-ORD). `Changed<M>` is dead by type under the default driver — that is deliberate (M7);
`publish tracked` is the priced escape.

## Refusals

| | Closes |
|---|---|
| R-Q: `query<…>` in guard/enter/exit/tick/commit | the O(N)-per-row lookup design becomes unwritable; blessed forms: `res<>`, events, `near` |
| R-ELSE: guarded transition without `else` | 500 NPCs silently doing nothing |
| R-ORD: `.entered()`/`.fail()` without the ordering edge | ordering by convention |
| R-HIST: `=> history` into a clock-arming leaf (shallow) | a restored `Attack` re-arming 0.8s and dealing damage twice; `history deep` lifts it |
| R-CLOCK: > 8 clock slots from interference coloring | unbounded row growth; the error lists the conflicting leaves |
| R-PAR (until R4): `parallel` + event emission | `par_for_each_chunk` requires `Fn + Send + Sync`, `send` takes `&mut self` today |
| R-ARITY: merged pass params > 12 | a trait-error wall on generated tuples |
| R-DENSE: `on entity` + dense storage | the chunked driver const-rejects dense terms |

## Cost model (10 000 enemies)

One O(N) pass; zero added archetypes; zero migrations per transition; ~800 KB/frame of column
traffic (Transform read + machine RMW), memory-bound. The emitted diagnostic note prints the
per-row columns, byte widths, clock/payload slot counts, the schedule domain, and this **measured**
caveat (ruling D1): with leaf states shuffled across rows the 5-arm match costs an additional
3.4–4.0 ns/row — invariant from L2 to DRAM working sets — so state coherence, not arm count, sets
the branch price; a side row-order index is banned by Principle 0 and table sorting is rejected on
row_ptr-churn cost, so the note is informational, not a switch.

## Reference scenario 1 — enemy AI

```
resource PlayerFocus { who: option<entity>, pos: Vec3 } with { init }
event StunHit { victim: entity(EnemyBrain), seconds: f32 }

machine EnemyBrain on entity
with {
    joins (tf: &Transform, with Enemy)
    state { stun_secs: f32 }
    inbox (StunHit = max)
    identity
    uses  (focus: res<PlayerFocus>, mut dmg: emit<DamageDealt>)
    order (after lift_player_focus)
}
{
    initial Idle;
    on StunHit => Stunned;                                          // root handler: any -> Stunned

    state Idle   { after 2.0 => Patrol; }
    state Patrol { when focus.dist2_from(tf.translation) < 15.0*15.0 => Chase; }
    state Chase  {
        when  focus.dist2_from(tf.translation) < 2.0*2.0 => Attack;
        while focus.dist2_from(tf.translation) > 25.0*25.0 for 3.0 => Patrol;   // hysteresis
    }
    state Attack {
        enter { if let Some(v) = focus.who {
            dmg.send(DamageDealt::new(me, v, 12.0)).ok(); } }
        after 0.8 => Chase;
    }
    state Stunned {
        enter { self.stun_secs = arg; }                             // arg = event payload slot
        after self.stun_secs => Idle;                               // duration from the event
    }
}
```

The focus helper (`dist2_from` returning `+INFINITY` when no player) lives beside the block as
ordinary Rust — the identity element keeps every guard branchless and the chart ticking with no
player present. The broadcast is correct because the source count is 1 (or 4 in co-op); N-to-N
sensing is what [`SPATIAL.md`](SPATIAL.md) exists for.

## Reference scenario 2 — mana-costed ability

```
machine Firestrike on entity
with { joins (mut mana: &mut Mana)  inbox (CastRequest, Interrupt)  identity
       fail CastFailed  uses (mut hit: emit<AbilityHit>)  order (after regen_mana) }
{
    initial Ready;
    can_activate { mana.cur >= 30.0 else NotEnoughMana; }
    commit       { mana.cur -= 30.0; }

    state Ready { on CastRequest activate => Casting; }
    state Casting {
        initial Windup;
        on Interrupt => Recovery;                    // composite handler: both leaves, no refund
        state Windup { after 0.25 => Casting.Active; }
        state Active { tick { hit.send(AbilityHit::new(me, 40.0)).ok(); }
                       after 0.10 => Recovery; }
    }
    state Recovery { after 0.40 => Cooldown; }
    state Cooldown { after 6.0  => Ready;    }
}
```

Failure surfaces twice by design: the pull byte on the row (`b.fail()`, zero cost, R-ORD-gated) and
the push event on the `fail` lane (sound, telemetry, netcode — one frame later). One definition
runs on the player and on 500 NPCs because "who has the machine" is "who has the component".

## Known limits

Orthogonal (AND) regions are not expressible — two machines on one entity are two components and
two passes (the compile-time flattening of AND regions costs the product of region sizes, which is
the very explosion regions exist to avoid). A consumer in another schedule cannot use `entered()`
(ordering edges do not cross schedules) — use events or `publish tracked`. Two same-frame events of
different types on one leaf: first-declared wins, the loser's deposit stays for the next frame's
pass only if its policy keeps it.
