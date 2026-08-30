# Gaia — language sketch (research-stage, NOT a spec)

> ⚠️ **ratified-stale.** The syntax shown below is **pre-R1 on essentially every line** and does
> **not** match the ratified rulings ([`PENDING-SYNTAX-PLAN.md`](PENDING-SYNTAX-PLAN.md) finding
> **M6**; [`../AETHER-GAIA-REVISION-2026-08-29.md`](../AETHER-GAIA-REVISION-2026-08-29.md)
> §per-file table). **Do not generate Gaia from this file** — an agent that reads it as the spec
> emits wrong-on-every-line output. Only interim drift-reduction annotations have landed here; the
> rewrite into the reworked syntax is owner-gated. This banner is the marking G0's own gate
> requires in the file's head: *a file that RESOLVES but is STALE does not satisfy a
> cross-reference*, and an index that knows is a file the reader never opened
> (gated by [`tests/gaia_g0_citation_census.rs`](../../tests/gaia_g0_citation_census.rs)).

The shape of the language as recommended by the research synthesis. Every ruling behind a line here
is in [`DECISIONS.md`](DECISIONS.md); the grammar itself is written at rung G2 and gated there.
This file exists so the *feel* of the language is on record before the spec — nothing in it is
byte-final.

## Header and node model

```
gaia 1 profile=scene|ui|data
asset "pack:path/name"
```

KDL-shaped nodes: `name positional key=value { children }`, `/-` slashdash to comment out one
component or child. Values are engine-native literals; the target Rust field dictates the type
(type-directed parsing — the `.ui` rule). Comments are first-class and survive the formatter
(AIR-14). Whitespace is never load-bearing (AIR-16, its "delimited whitespace-insensitive text"
clause — AIR-16 is a single row with no lettered sub-clauses).

## Scene profile

```
gaia 1 profile=scene
asset "levels/crypt/cell_07"

abstract template Torch(power: f32) {
    Transform pos=(0,0,0)
    MeshRef "props/torch"            // stable asset id — never a slot index
    PointLight power=$power range=12.0 color=#FFB35C
}

entity @gate_01 {
    Transform pos=(4,0,-2) rot=euler(0,90,0)
    MeshRef "props/gate"
    link opened_by = @crypt_logic/lever_03    // cross-file entity ref: remapped, loud on miss
}

instance "prefabs/torch_wall" @wall_east {
    patch @sconce_a.PointLight power=2.5 layer=tuning
}
```

> **Interim annotations (drift-reduction only).** This file is **ratified-stale**: it is pre-R1 on
> essentially every line — R1 being the first of the Gaia **syntax rulings R1–R7** in
> [`PENDING-SYNTAX-PLAN.md`](PENDING-SYNTAX-PLAN.md) (finding M6 there), not an Aether campaign
> rung — and the rewrite into the reworked syntax is owner-gated. The
> notes below are the ONLY corrections applied; the rest of the sketch has not been swept.
>
> - **`PointLight`'s field is `power`, not `intensity`** — corrected above at all three sites (the
>   template hole, the key, and the `patch` line). Verified against the engine: `PointLight`
>   (`boyko_render/src/light.rs`) is the unique struct of that name in the workspace and carries
>   `position: [f32; 3]`, `color: [f32; 3]`, `power: f32` (luminous power Φ in lumens; the baked
>   intensity is `Φ / (4π)`) and `range: f32`. Nothing in the engine is called `intensity` on a
>   point light, so every line that used the old spelling was a bake error waiting to happen.
> - **`range` has no default and is now supplied.** `PointLight` derives no `Default` (the bundle
>   doc in `boyko_render/src/bundles.rs` says so outright), so a typed constructor REQUIRES
>   `range`; the sketch omitted it. The `12.0` above is illustrative, not ruled. This is the same
>   hole PENDING M7 records at FIELD granularity — the closed evaluator has no way to spell a
>   missing field's neutral.
> - **Colour — BOTH halves settled, and the literal above is corrected.** Ballot **GB-2** RESOLVED
>   2026-08-30 by standing rule ([`DECISIONS.md`](DECISIONS.md) §Language shape; body and
>   measurements in [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29).
>   - **Arity:** checked against the target's arity, mirroring Aether's shipped `ColorLit`
>     (`crates/aether_lang/src/parse.rs:1177-1213`). Widening 3 → 4 supplies alpha `1.0`; narrowing
>     4 → 3 is a coded refusal, never a silent alpha drop. `PointLight::color` is `[f32; 3]`, so the
>     RGBA-4 `#FFB35CFF` this sketch used to carry was a bake error; the 6-digit form **`#FFB35C`**
>     is now written above. This is a drift-reduction correction of the same kind as `power`.
>   - **Transfer function:** a property of the destination field, not the literal. `PointLight::color`
>     is documented LINEAR (`boyko_render/src/light.rs:309-310`), so the bake applies the **sRGB
>     EOTF**; a `u32` STRAIGHT-RGBA8 UI field gets the **identity**. Carrying raw bytes into the
>     linear field — what this sketch's earlier form implied — is wrong by **1.557× on green and
>     3.371× on blue** for this very colour.
> - ⚠ **Open ballot GB-5 — `position` on `PointLight`.** `light_reconcile` DERIVES `position` from
>   the entity's `GlobalTransform`, so whatever a scene authors into that field is overwritten. The
>   **ui** profile already rules that engine outputs are undeclarable (bake error); GB-5 rules
>   whether the same rule extends to engine-derived fields in the **scene** profile. It must land
>   as a DECISIONS line **before G1 freezes the GK-4 field table**, since that table is where such
>   fields would be marked. Not settled here.
> - **`MeshRef` is an UNLANDED carrier**, at both sites above. No `MeshRef` type exists in the
>   workspace; what the engine has is `MeshHandle(pub u32)`
>   (`boyko_scene/src/render_caps.rs`) — the raw process-local slot form that the ratified **GN1**
>   lint exists to BAN, and substituting it would falsify this line's own comment ("never a slot
>   index"). The name therefore stands as a placeholder pointing at DECISIONS **GN1** / §Identity
>   and at CAMPAIGN **G6** / ballot **F4**; the stable-asset-id carrier is named by the G6 design
>   pass, not here.
> - **Every other component head in this sketch is unswept against the engine.** `MeshRef` was
>   found by looking; nothing here establishes that the rest are real. Sweep them all before any
>   rewrite.

`@name` — the file-local stable object id (author-visible, assignable by `gaia fmt --assign-ids`,
never minted by bake). `instance` + `patch` — the single composition construct; `extends` (live)
vs `copy` (snapshot) name the two linkage forms. `remove <component|@id>` is first-class.

## UI profile

```
gaia 1 profile=ui
asset "ui/hud"

@health_bar bar {                     // sugar: {UiLayout, UiBackground, Bar, …}, erased by bake
    UiLayout width=Px(240) height=Px(18)
    UiBackground color=#202020FF corner_radius=(4,4,4,4)
    bind value from=@player/unit comp=Health num=current den=max
}
@pause_btn button {
    UiText size_px=16 color=#FFFFFFFF
    text "Пауза"
    OnClick action=PauseGame          // name → hash; unresolvable = bake error, never NO_ACTION
}
```

`bind` bakes to a POD bind-record (all name references as hashes, resolved once at load); a
provably-constant expression emits bytes and no record; structure never reacts — a subtree swap is
an action. Engine outputs (`ComputedRect`, `Interaction`, bitset tags) are undeclarable — bake
error.

## Data profile

```
gaia 1 profile=data
asset "items/swords"

table SwordDef {                      // N rows of one set → a dense column + a generated const per row
    row @iron_sword  { damage=12 weight=3.5 rarity=common }
    row @flame_sword extends @iron_sword {
        damage=18 rarity=rare
        damage_by_level = scalable(curve="curves/level_scale", mul=1.2)
    }
}
contract SwordDef { damage > 0, weight in 0.1..50.0 }
```

`table` (pending fork F3) bakes rows into one dense column and emits a Rust const per row name —
renaming a row becomes a COMPILE error at every use site. `scalable` is the curve valve; `contract`
is the closed predicate vocabulary with blame on the violating value's span.

## Evaluation summary

Total (no recursion form), eager (every contract every bake), closed (no fallback). The allowed
list is exhaustive and lives in DECISIONS.md §The logic line; everything else is refused with a
coded diagnostic pointing at the Aether-side idiom. All engine references travel as names in text
and name hashes in the binary; raw build-local ordinals are unrepresentable.
