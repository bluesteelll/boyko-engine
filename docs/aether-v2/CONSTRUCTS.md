# Aether v2 — construct surface (delta over v1)

The v1 baseline is catalogued exhaustively in
[`../AETHER-V1-SURFACE-REVIEW.md`](../AETHER-V1-SURFACE-REVIEW.md); this file specifies only what
**changes**. Rationale per decision → [`DECISIONS.md`](DECISIONS.md). Per-entity `machine` has its
own file → [`MACHINES.md`](MACHINES.md).

Shared rules across all v2 constructs: every group line is `keyword payload`; every list is
parenthesised with free line-breaking and trailing commas; there are **no** optional-bracket short
forms; group order is free, each group at most once.

---

## `component`

```
component Health {
    hp:  f32,
    max: f32,
    link last_attacker: Entity,        // -> #[entities] (load-time remap opt-in)
} with {
    requires (Regen, Mass(1.0), Transform = Transform::from_translation(SPAWN))
    flags    (Visible = on, Stunned = off)     // initial enable-bit states (needs kernel FLAGS_DIRECT)
    hooks    (on_add = f, on_insert = g, on_replace = h, on_remove = k, on_despawn = d)
    kernel   (bundle = off, storage = dense, clone = my_fn,
              serialize = (stable_name = "game::Health", version = 3))
    relates  (target = LikedBy, allow_self)    // XOR related (source = ..., linked_despawn)
}
```

- Groups: `requires` (all three engine ctor forms, closing the v1 bare-path-only gap) · `flags`
  (initial enable-bit state on attach — needs the kernel `FLAGS_DIRECT` twin of `REQUIRES_DIRECT`;
  `requires` of a `flag` is refused at parse, since the kernel path panics on a poolless id) ·
  `hooks` (five keys once `on_despawn` is unlocked in the derive) · `kernel` (negatives are values:
  `bundle = off`, `clone = off | auto | path`, `serialize = off | auto | (stable_name, version)`,
  `storage = table | dense`) · `relates`/`related` (mutually exclusive; the relationship owns its
  hook slots — a user hook in an owned slot stays a loud refusal).
- Field-head keywords: `link` (→ `#[entities]`), `key` (→ the relationship FK marker). A field may
  not be named `link`/`key`.
- `storage = bitset` does NOT exist here — that is the `flag` construct.
- Fieldless `component X {}` is refused: "a fieldless component is a `tag`; an enable bit is a
  `flag`" (keeps `grep '^tag '` a census).

## `tag` / `flag`

```
tag  Player;                       // ZST component: archetype bit, may take with { ... }
tag  Enemy with { requires (Health) }
flag Stunned;                      // enable bit: O(1) toggle, takes NOTHING
```

`flag` accepts no body and no groups — hooks are impossible (no pool), `requires` can never fire
(no insert path), the bundle is suppressed; the refusal states each reason.

## `bundle`

```
bundle Pawn(Health, Velocity, Transform);      // positional — primary
bundle Boss {                                  // named — for ..base and named errors
    health:    Health,
    velocity:  Velocity,
    transform: Transform,
}
```

Disambiguated by the first token after the name. Arity cap 16 with the error on the 17th element in
both forms. NEW refusal: a repeated component type in one bundle ("component `Health` appears
twice") — the derive never checks it and the runtime behaviour is unverified.

## `event`

```
event Damage {
    victim: entity(Health),        // participant; context checked by a router debug_assert (F6)
    amount: f32,
} with {
    lanes    32                    // 1..=64, parse-checked against MAX_EVENT_THREADS
    capacity 4096                  // 1..=16384, parse-checked against MAX_EVENT_CAPACITY
    ordered                        // opt-in run-to-run byte-stable order (see EVENTS.md)
}
```

- The sibling `plugin` **auto-registers** the lanes (`preregister_event[_default]`); a `with { }`
  block without a plugin in the block is refused.
- A flat constructor `Damage::new(victim, amount)` is generated in source field order — the
  two-lane rewrite stays an implementation detail of `#[event]`. Flat read accessors: deferred.
- `event Ping {}` (a ZST) is refused at parse with the counter-suggestion, instead of failing at
  monomorphisation.

## `system`

```
system chase(
    q:     query<&mut Velocity, with Enemy, without Stunned>,
    dev:   nonsend<GpuDevice>,             // NEW sugar: NonSendRes, mut-inferred
    tally: mut res<Tally>,
)
    schedule update
    sets     (Combat, Ai)
    order    (after integrate, before shoot, chain cleanup)   // chain: sibling name ONLY
    when     (alive, not_paused)           // each -> .run_if; EAGER fold semantics (D5)
{ ... }
```

- Four groups replace the v1 flat clause list; no `with` wrapper (the body brace terminates).
- `schedule startup` still refuses every other group.
- `when (A or B)` / `unless C` lower to the kernel combinators (`CombinedSystem`, eager fold).
- `or(...)` in **filter** position is RESERVED: refused with a span diagnostic pointing at the
  verbatim escape, until the kernel `Or`-dense fix is green and a real consumer exists (D4).
- **`gpu`** (bare group) → `.gpu()` — marks a GPU-compute system (dispatcher-solo at the apply
  window, the sound site for `!Send` RHI recording). The kernel marker is deliberately
  non-inferable from access, so this is the only ergonomic route. *(Ratified O3.)*
- **`system exclusive name(w: world) { … }`** → `fn(&mut EcsMaster)`. Exactly one param (`world`
  is contextual); any second param is refused. A plain `system` with a verbatim `&mut EcsMaster`
  is refused with a pointer at `exclusive` — accidental exclusivity (a whole-frame barrier)
  becomes unwritable. *(Ratified O2.)*

## `set` *(ratified O1)*

```
set Combat order (after Input, before Render) when (in_state(GameFlow::Playing));
```

Emits `#[derive(SystemSet)] pub struct Combat;` plus `b.configure_set(Combat)…` in the sibling
plugin — the set-level ordering and conditions (`configure_set`) that `sets (Combat)` could only
reference. A set-level `when` evaluates once per set per frame instead of once per member system.

## `relation` *(ratified O4, variant b)*

```
relation Likes -> LikedBy { linked_despawn, allow_self }
```

One declaration mints BOTH sides of a relationship: the source FK component and the target
reverse-index component with its **private** collection field (the privacy the kernel macro
enforces becomes generator-internal — no exception to the "everything pub" rule, and the
`target =`/`source =` cross-references cannot desync because nobody writes them). A single-field
source needs no `key` marker. The two-block `relates`/`related` form remains for a source with
extra fields.

## `attributes` *(ratified O7 — the foundation; `effect` layers on later)*

```
attributes Stats { attack: f32 = 10.0, armor: f32 = 5.0 }
```

Emits three POD components (`StatsBase`, `StatsMods`, `StatsCurrent`), the `#[require]` wiring,
and ONE recompute system under `Changed<StatsMods>` with `current = (base + add) * mul` unrolled
per field. What it kills: field-list desync across the three structs, and order-dependent buff
removal drift — removal is subtract-from-Mods + recompute-from-Base, exact by construction.

## `tags` — hierarchical *(ratified O6, with both gates)*

```
tags sticky { Weapon.Ranged.Rifle; Weapon.Melee.Sword; }
```

Mints prefixed unit structs (`WeaponRangedRifle` requires `WeaponRanged` requires `Weapon`), so
attaching the leaf implies the ancestors — the silent partial-attach class becomes unwritable, and
`With<Weapon>` catches every descendant as one signature bit. **Zero runtime difference** vs
attaching the tags manually — this is correctness sugar, valuable when queries span taxonomy
levels. Gates: the expander computes the archetype-count product of orthogonal branches and
refuses past the ceiling; `#[require]` is granted only to the `sticky` class, because requires
fire on attach and are never removed — a removable taxonomy would strand stale ancestors.

## `each`

```
each drift(mut Transform, Velocity, with Enemy, time: res<Time>) {
    transform.translation += velocity.linear * time.delta_secs();
}
```

- Query data = `mut Type` / `Type` prefix params; binding = `snake()` of the type (explicit
  `t: mut Transform` overrides). `e: entity` binds the row id. Everything else is an ordinary param.
- **Default driver: `iter_mut`** (change ticks work). `each soa` opts into `for_each_chunk`
  (refused when any term is non-archetypal or the data is not chunkable — the diagnostic explains
  tick-blindness); `each par` opts into the parallel driver (needs R4 for event emission).
- Refusals: no query datum ("that is a `system`"), two `entity` bindings, `return` in the body
  ("exits the per-entity closure — write `continue`, or use `system`").

## `plugin`

Unchanged, except the emitted `name()` override is removed — the trait default (fully-qualified
type name) is strictly better in duplicate diagnostics.

## `resource` *(ratified O5)*

```
resource PlayerFocus { who: option<entity>, pos: Vec3 } with { init }   // Default-constructed
resource Gravity { g: f32 } with { value Gravity { g: -9.81 } }         // explicit value
```

Emits `#[derive(Resource)] pub struct` + `app.insert_resource(...)` in the sibling plugin.
`option<entity>` sugars `Option<Entity>`; refusal R-RES: `with { init }` over a bare `entity` field
("`Entity` does not derive `Default`; use `option<entity>` or `with { value ... }`").

## `machine` (global form)

Grammar unchanged; the **codegen authority moves to `boyko_macros::state_chart!`** and Aether
becomes a front-end (R2). The route merge that lands there fixes the both-chains-run defect and
aligns arbitration to first-declared-wins (M6). The per-entity form is specified in
[`MACHINES.md`](MACHINES.md).

**Payload binding** *(ratified O8)*: `on Damage(dmg) if dmg.parameters.amount > 50.0 => T { … }` —
the drain loop binds the user's pattern, visible in the guard and the action (the global-machine
twin of the per-entity `arg` slot). Semantics after the R2 merge: the FIRST event of the frame
passing the guard wins, consistent with first-declared arbitration. Binding guards
(`if let Some(x) = …`) follow as a second step on demand.

## `material`, `scene`

Out of this campaign: `material` is parked pending the shader-policy decision (own campaign);
`scene` keeps its narrowed authored-scene role, with the world moving to the baked asset format
(own campaign). Two emission fixes to authored scenes ride independently: collapsing the per-node
`.insert` chain into one generated extras bundle (one migration instead of N), and grouping
same-shaped anonymous nodes through `spawn_batch`.
