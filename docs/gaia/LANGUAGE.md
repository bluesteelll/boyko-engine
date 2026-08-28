# Gaia — language sketch (research-stage, NOT a spec)

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
(AIR-14). Whitespace is never load-bearing (AIR-16a).

## Scene profile

```
gaia 1 profile=scene
asset "levels/crypt/cell_07"

abstract template Torch(intensity: f32) {
    Transform pos=(0,0,0)
    MeshRef "props/torch"            // stable asset id — never a slot index
    PointLight intensity=$intensity color=#FFB35CFF
}

entity @gate_01 {
    Transform pos=(4,0,-2) rot=euler(0,90,0)
    MeshRef "props/gate"
    link opened_by = @crypt_logic/lever_03    // cross-file entity ref: remapped, loud on miss
}

instance "prefabs/torch_wall" @wall_east {
    patch @sconce_a.PointLight intensity=2.5 layer=tuning
}
```

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
