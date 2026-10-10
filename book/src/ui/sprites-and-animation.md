# Sprites & Animation

> Images, nine-slice borders, sprite-sheet flipbooks and tweens for `boyko_ui` nodes, all driven by
> one UI clock.

> **Status:** the windowed host (`boyko_app`) does not composite UI yet. These components build and
> animate the UI world; drawing it needs a host that runs the UI render pass. See
> [UI Overview](overview.md).

## Images

`UiImage` gives a node a texture:

| Field | Meaning |
|-------|---------|
| `texture` | A bindless texture slot (`0` = untextured). |
| `uv_min`, `uv_max` | The sub-rectangle of the texture to show. |
| `tint` | Straight (not premultiplied) RGBA8, multiplied into the image. |

The default tint has alpha `0`, so an image you forget to tint is invisible rather than a white
box. `ImageBundle` spawns a layout node with an image.

## Nine-slice borders

`UiNineSlice` keeps a frame's corners at a fixed size while its edges and centre stretch or tile:

| Field | Meaning |
|-------|---------|
| `border_px` | The corner size on screen, in logical pixels, as `[left, top, right, bottom]`. |
| `border_uv` | The same insets in the source image, as fractions of the node's `UiImage` UV rectangle. |
| `mode` | `Stretch` (default) scales the edges and centre; `Tile` repeats them. |
| `fill_center` | `false` leaves the centre empty, for a frame around other content. |

The engine does not store texture sizes, so the tile count is derived from the ratio of the two
borders. A frame with a picture inside it is two nodes: a nine-sliced parent and an imaged child.

## Sprite sheets and flipbooks

A sprite sheet is a grid of frames in one texture. Register it once, at setup, in the
`UiSheetTable` resource; `register` returns a dense `SheetId`. A zero `cols` or `rows` is raised to
`1`, and `frame_count` is clamped to the grid:

```rust,ignore
use boyko_ui::sprite::{SheetId, UiSheet, UiSheetTable};

/// `atlas_slot` is the bindless slot the sheet's texture was registered in.
fn register_explosion(sheets: &mut UiSheetTable, atlas_slot: u32) -> SheetId {
    sheets.register(UiSheet {
        slot: atlas_slot,
        cols: 8,
        rows: 4,
        frame_count: 30,     // the last two cells are empty
        _pad: [0; 2],
        inset_uv: [0.0; 2],  // a half-texel inset avoids bilinear bleed between frames
    })
}
```

A node shows a frame with `UiSpriteSheet { sheet, index }`. To animate it, add `UiSpriteAnim`:

| Field | Meaning |
|-------|---------|
| `first`, `last` | The frame range, inclusive. |
| `fps` | Frames per second. |
| `mode` | `Forward` (default), `Reverse`, `PingPong` or `Once` (play once, then hold `last`). |
| `repeats` | How many cycles to play before holding the frame the last cycle ended on; `0` loops forever. |

Adding `UiSpriteAnim` also adds the flipbook's private state, `UiSpriteCursor`, through an
`on_add` hook. `AnimatedSpriteBundle` spawns the layout, image, sheet, animation and cursor in one
go. Each frame `ui_sprite_flipbook` advances the cursor and writes the new frame into
`UiSpriteSheet::index` with `set_if_neq`, so a 12 fps flipbook on a 60 Hz display changes the
component only on the frames where the picture changes.

## Tweens

A tween animates one visual channel of a node from a start value to an end value over a duration:

| Channel | Value | Helper pair |
|---------|-------|-------------|
| `TweenTint` | RGBA8 colour multiplier | `start_tween_tint` / `stop_tween_tint` |
| `TweenOpacity` | `0.0..=1.0` | `start_tween_opacity` / `stop_tween_opacity` |
| `TweenOffset` | `[x, y]` offset in logical pixels | `start_tween_offset` / `stop_tween_offset` |
| `TweenScale` | `[x, y]` scale about the node's centre | `start_tween_scale` / `stop_tween_scale` |

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_ui::prelude::*;

/// Fade a panel in over 250 ms.
fn show_panel(mut commands: Commands, panel: Res<PanelEntity>) {
    start_tween_opacity(&mut commands, panel.0, 0.0, 1.0, 250.0, EasingId::LINEAR, 0);
}
```

The arguments are the commands, the entity, `from`, `to`, the duration in milliseconds, the
easing curve and per-tween flags.

- Each running channel writes into the node's `UiVisual` component (tint, opacity, offset, scale),
  which the UI render pack applies. A node gets a `UiVisual` automatically when its first tween
  starts.
- When a tween reaches its duration, the end value is assigned exactly and the channel is removed.
  `stop_tween_*` removes it early and leaves the node at its current value.
- A duration that is not finite, or negative, starts no tween. A duration of `0` jumps straight to
  the end value.
- Only `EasingId::LINEAR` is evaluated today.
- The tween channels use [dense storage](../concepts/dense-components.md), so starting and finishing
  animations does not move nodes between archetypes.

## The UI clock

`UiAnimationPlugin` inserts the `UiClock` resource and registers, in `UiAnimationSet` on the main
schedule, the clock tick (`ui_clock_tick`), the tween update (`ui_visual_tick`) and the removal of
finished tweens (`ui_tween_reap`).

`UiClock` holds two deltas, both clamped to `max_delta` (0.1 s by default) so that a stall such as
an alt-tab does not skip whole animation cycles:

| Delta | Follows pause and time scale | Used by |
|-------|------------------------------|---------|
| `dt_real` | no | tweens, by default, so a pause menu can still fade in while the game is paused |
| `dt_virtual` | yes | the flipbook, and tweens started with `TWEEN_FLAG_VIRTUAL_CLOCK` |

## Ordering

`UiAnimationPlugin` does not register the flipbook; add it yourself, with two ordering edges:

```rust,ignore
use boyko_render::ui_render_discovery;
use boyko_ui::prelude::UiAnimationSet;
use boyko_ui::sprite::ui_sprite_flipbook;

app.add_systems_cfg(|b| {
    // The host that draws UI registers discovery; ordering edges take its key.
    let discovery = b.add_system(ui_render_discovery).key();
    b.add_system(ui_sprite_flipbook)
        .after_set(UiAnimationSet) // read this frame's clock
        .before(discovery);        // the render pass must see this frame's write
});
```

Both edges matter. A flipbook that runs before the clock tick advances by the previous frame's
delta. A flipbook that runs after `ui_render_discovery` loses the repaint rather than delaying it:
the write carries this frame's change tick, which lies outside the window the discovery system checks
on its next run, so the frame change is never drawn.

## See also

- [UI Overview](overview.md): layout, markup and the render pass.
- [Text & MSDF](text-msdf.md): text nodes.
- [Change Detection](../change_detection.md): why the ordering edge decides whether a write is seen.
- [System Ordering & Sets](../scheduling/ordering-and-sets.md): `.before`, `.after` and sets.
- Source: [`sprite.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ui/src/sprite.rs),
  [`animation.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ui/src/animation.rs),
  [`components.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ui/src/components.rs).
