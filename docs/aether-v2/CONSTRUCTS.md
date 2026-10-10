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
    flags    (Visible = on, Stunned = off)     // initial enable-bit states, values `on | off`
                                               //   (needs kernel FLAGS_DIRECT; vocabulary on AB-13)
    hooks    (on_add = f, on_insert = g, on_replace = h, on_remove = k, on_despawn = d)
    kernel   (bundle = off, storage = dense, clone = my_fn,
              serialize = (stable_name = "game::Health", version = 3))
    relates  (target = LikedBy, allow_self)    // XOR related (source = ..., linked_despawn)
}
```

- Groups: `requires` (all three engine ctor forms, closing the v1 bare-path-only gap) · `flags`
  (initial enable-bit state on attach; values are `on | off`, enumerated here the way the sibling
  `kernel` group's are — needs the kernel `FLAGS_DIRECT` twin of `REQUIRES_DIRECT`;
  `requires` of a `flag` is refused at parse — the refusal is **structural** (a `flag` takes no
  groups at all), and the accompanying kernel claim, that the insert path panics on a poolless id,
  is a **code-reading claim per KERNEL-BACKLOG KE11 — red test pending**, not a measured one) ·
  `hooks` (five keys once `on_despawn` is unlocked in the derive) · `kernel` (negatives are values:
  `bundle = off`, `clone = off | auto | path`, `serialize = off | auto | (stable_name, version)`,
  `storage = table | dense`) · `relates`/`related` (mutually exclusive; the relationship owns its
  hook slots — a user hook in an owned slot stays a loud refusal).
- **KNOWN-OPEN — `requires` of a `storage = dense` component.** The same poolless-id mechanism that
  grounds the `flag` refusal reaches Dense storage too (KERNEL-BACKLOG KE11 covers **both** poolless
  kinds). The parse refusal is **not** extended to `storage = dense` targets here: doing so would
  narrow the ratified `storage = table | dense` × `requires` product, and KE11's own title holds the
  refuse-vs-filter fork open. Recorded as a hole, disposition on ballot **AB-6** (see §Open
  ballots). The red tests land regardless of disposition.
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
    victim: entity(Health),        // participant; context debug_assert'd ONLY in a machine `inbox`
                                   //   router (DECISIONS M4) — unread otherwise
    amount: f32,
} with {
    lanes    32                    // a MINIMUM, not a count (ruling E4): the effective lane count is
                                   //   max(32, worker_count + 1). 1..=MAX_EVENT_THREADS is
                                   //   parse-checked; the floor is applied at boot and the raise is
                                   //   reported. On a 64-worker machine this line yields 65 lanes.
    capacity 4096                  // 1..=MAX_EVENT_CAPACITY, parse-checked (symbolic, never the
                                   //   numeric literal — DECISIONS C3)
    ordered                        // opt-in run-to-run byte-stable order (see EVENTS.md)
}
```

- The sibling `plugin` **auto-registers** the lanes (`preregister_event[_default]`); a `with { }`
  block without a plugin in the block is refused.
- A flat constructor `Damage::new(victim, amount)` is generated in source field order — the
  two-lane rewrite stays an implementation detail of `#[event]`. Flat read accessors: deferred.
- `event Ping {}` (a ZST) is refused at parse with the counter-suggestion, instead of failing at
  monomorphisation.
- The `entity(Health)` context above is illustrative only: this file's own `Damage` is consumed by
  the **global** machine's drain loop (§`machine` (global form), payload binding), which has no
  router and therefore performs no context check. The debug_assert exists on the per-entity
  `inbox` router path — see [`MACHINES.md`](MACHINES.md) §Event routing.

## `system`

```
system chase(
    q:     query<&mut Velocity, with Enemy, disabled Stunned>,   // `disabled`, NOT `without` — see below
    dev:   nonsend<GpuDevice>,             // NEW sugar: NonSendRes, mut-inferred
    tally: mut res<Tally>,
)
    schedule update
    sets     (Combat, Ai)
    order    (after integrate, before shoot, chain cleanup)   // chain: sibling name ONLY
    when     (alive, not_paused)           // each -> .run_if; EAGER fold semantics (D5)
{ ... }
```

- **`Stunned` is a `flag`, so the filter term is `disabled Stunned`, not `without Stunned`.** The
  v1 filter grammar's `enabled`/`disabled` terms (→ `Enabled<T>`/`Disabled<T>`,
  [`../AETHER-V1-SURFACE-REVIEW.md`](../AETHER-V1-SURFACE-REVIEW.md) §filter grammar) are inherited
  here: this file is a delta over v1, and absence from a delta is inheritance, not deletion. The
  polarity matters and is easy to get backwards. A `flag` is `StorageKind::Bitset`, and a bitset id
  is **filtered out of every archetype signature** (`Archetype::filtered_signature_mask`, applied by
  `ArchetypeMaster::create_archetype`) — the mask can never carry its bit. Both filters still take
  the *archetypal* path over it (`With`/`Without` set `IS_ARCHETYPAL = !C::STORAGE_IS_DENSE`, and
  `Bitset` is not `Dense`), so each tests that mask directly:
  **`with F` matches NOTHING** (the include bit is unreachable) and **`without F` excludes NOTHING —
  it matches everything** (bit-absence is always true). Two different silent wrong answers from one
  storage kind; neither is diagnosed. The sibling `with Enemy` is what
  satisfies the kernel's positive-archetypal-term requirement for this query. Whether v2
  additionally **refuses** `with`/`without` over a `flag` at parse is ballot **AB-11** (see §Open
  ballots) — undecided here.
- Four groups replace the v1 flat clause list; no `with` wrapper (the body brace terminates).
- `schedule startup` still refuses every other group.
- `when (A or B)` / `unless C` lower to the kernel combinators (`CombinedSystem`, eager fold).
- `or(...)` in **filter** position is RESERVED: refused with a span diagnostic pointing at the
  verbatim escape, until the kernel `Or`-dense fix is green and a real consumer exists (D4).
  ⚠ **Rung R0 removed the first of those two conditions (LANDED 2026-08-29)** — this reserve now
  stands on the consumer clause alone, a narrower ground than the one written here. The Gaia twin of
  this ban stood on the kernel defect *entirely*; that was ballot **GB-9**, ✅ **RULED 2026-08-30 —
  the Gaia ban is DELETED with a record**, and the ruling explicitly declined to keep it by
  borrowing D4: **D4 reserves the Aether *surface*, while that ban governed *generated code*, and
  Gaia's ratified GN2 says the baker emits no Rust.** The reserve **here** is unaffected. See
  [`DECISIONS.md`](DECISIONS.md) D4 and [`../gaia/DECISIONS.md`](../gaia/DECISIONS.md) §UI bindings,
  item 8.
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
  tick-blindness); `each par` opts into a parallel driver (needs R4 for event emission).
- **`each par` — RULED 2026-08-30 (ballot AB-8): `par_iter_mut`.** `par_for_each_chunk` is reachable
  only as `each soa par`. Measured: over 2048 rows a `Changed<>` reader sees **2048** of the
  `par_iter_mut` writes and **0** of the `par_for_each_chunk` writes; the tracked driver costs
  **1.17–1.47×** the chunked one (0.03–0.07 ns/row), which at the machine cost model's 10 000 rows
  is 0.3–0.7 µs/frame against the 34–40 µs ruling D1 measured for the jump table alone — ~1–2 % of
  the pass. The batching key is **not** author-visible in v1 (`BatchingStrategy` has three fields,
  so `parallel (batch = N)` cannot name what it appears to name). Full ruling and the rejected
  alternatives: [`DECISIONS.md`](DECISIONS.md) **C5a**. The kernel facts that decided it:
  - `Query::par_iter` / `Query::par_iter_mut`
    (`crates/boyko_ecs/src/ecs/core/iters/query/query.rs`) accept **any** `D: QueryData` — the
    bound replicates the `iter`/`iter_mut` split, so `Mut<T>` and its change ticks **survive**.
  - `Query::par_for_each_chunk` (same file) requires `D: ChunkedQueryData`, which **excludes**
    `Ref<T>`/`Mut<T>`; its `// SAFETY:` comment states that `NEEDS_CHANGE_DETECTION` const-folds to
    `false` — i.e. this driver is structurally tick-blind. It also takes a second argument,
    `BatchingStrategy` (`crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs`), which has a
    `Default` impl.
  - Both are floored by `MIN_ARCHETYPE_FOR_PARALLEL` = **1024** (same file): an archetype below it
    runs **inline on the calling thread**, `par` opt-in or not.
  - Both expose `batching_strategy(BatchingStrategy)` as a builder, so the knob stays reachable from
    the verbatim escape even though the grammar does not surface it.
- Refusals: no query datum ("that is a `system`"), two `entity` bindings, and `return` in the body:
  "`return` is not allowed in an `each` body — it would leave the generated system, not this row;
  write `continue` to skip a row, or use `system` to exit early." (True of **both** lowerings: the
  closure phrasing was false under the chunked driver. If the two lowerings emit two messages, the
  `soa` variant says **chunk**, not entity.) Trybuild goldens for both paths per AI-ORIENTATION
  AIR-03.

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

## Open ballots (owner) — this file settles none of them

These are recorded, not decided. A document that silently settles a fork is the defect this section
exists to prevent.

| Ballot | Question | Alternatives | Blocks |
|---|---|---|---|
| **AB-6** | `requires` of a `storage = dense` component | (a) parse refusal — ⚠ narrows the ratified `storage = table \| dense` × `requires` surface · (b) dense required-ctor route · (c) leave KNOWN-OPEN + hook workaround | KE11's disposition and the R3 `component` wording; the red tests land under all three |
| ~~**AB-8**~~ **RESOLVED 2026-08-30** | `each par` lowering | **`par_iter_mut`** (measured: 2048/2048 rows tracked vs 0/2048 chunked; 1.17–1.47× cost = ~1–2 % of the pass). `soa par` **exists** and is the only route to `par_for_each_chunk`. Batching key **not** author-visible in v1. Machines and `each` share the **ladder**, not the default. → [`DECISIONS.md`](DECISIONS.md) **C5a** | — |
| **AB-11** | `with`/`without` over a `flag` | (a) parse refusal with did-you-mean → `enabled`/`disabled` (dissolves the whole silent-filter class) · (b) forbidden-form list only, no refusal. ⚠ (a) adds a refusal where v1 documents non-refusal | R3 filter goldens |
| **AB-13** | `flag` initial-value vocabulary | `on \| off` reserved vs contextual; disambiguation across all three `on` positions (`machine … on entity`, `on E => T`, `flags (X = on)`); and the group's NAME — PENDING Tier 3 **withdrew** `flags → initial` (`initial` is already the machine's initial-state keyword, so the rename recreates the collision it fixes), so the question is whether that withdrawal stands or a different rename is wanted. ⚠ any rename here touches a ratified keyword | R3 |
| ~~**AB-2**~~ | ✅ **RULED 2026-08-30 → [`DECISIONS.md`](DECISIONS.md) E4**: `lanes N` is a **minimum**, effective `max(N, worker_count + 1)`, raised at boot **and reported** | *Rejected:* boot refusal (machine-dependent unbootability); dropping the knob (cleaner, but deletes a ratified surface — a SCOPE call escalated to the owner, not taken) | ~~R3 + R4~~ — the worked `lanes 32` above now reads as a floor |

## `material`, `scene`

Out of this campaign: `material` is parked pending the shader-policy decision (own campaign);
`scene` keeps its narrowed authored-scene role, with the world moving to the baked asset format
(own campaign). Two emission fixes to authored scenes ride independently: collapsing the per-node
`.insert` chain into one generated extras bundle (one migration instead of N), and grouping
same-shaped anonymous nodes through `spawn_batch`.
