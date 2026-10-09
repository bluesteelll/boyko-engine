# Framegraph

> The renderer's synchronisation authority: each frame's passes declare what they read and write,
> and the framegraph derives the minimal set of Vulkan pipeline barriers from those declarations.

## What it is

Vulkan makes the application order every GPU access by hand: image layout transitions, memory
visibility after a write, ordering between a read and a later write. Hand-placed barriers rot as
passes are added. The framegraph in
[`boyko_rhi_vulkan::framegraph`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_rhi_vulkan/src/framegraph)
replaces them with declarations:

1. **Declare.** Each pass names the images and buffers it touches, with the pipeline stage, access
   mask and (for images) the layout it needs.
2. **Compile.** A per-resource state machine (in the style of Granite's) walks the passes in order
   and emits a barrier only where a real hazard exists.
3. **Record.** Before each pass, its barriers are grouped by `(src_stage, dst_stage)` and recorded
   as one `vkCmdPipelineBarrier` call per group.

Every render path declares its own graph (`declare_deferred_graph`, `declare_forward_graph`,
`declare_vb_graph` in
[`present/graph_bridge.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/present/graph_bridge.rs)),
and re-declares it every frame, so a pass that is off this frame contributes no barriers.

## Using it

The framegraph is plain CPU code, so it runs without a GPU. This program declares a compute write
followed by a fragment read and counts the barriers the compile derives:

```rust,ignore
use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
};
use boyko_rhi_vulkan::framegraph::{BarrierSink, BufBarrier, FrameGraph, ImgBarrier, SubRange};

/// A sink that only counts; the engine's real sinks record `vkCmdPipelineBarrier`.
struct Count(usize);

impl BarrierSink for Count {
    fn image_barriers(&mut self, _src: u32, _dst: u32, group: &[ImgBarrier]) {
        self.0 += group.len();
    }
    fn buffer_barriers(&mut self, _src: u32, _dst: u32, group: &[BufBarrier]) {
        self.0 += group.len();
    }
}

fn main() {
    // Allocated once; `reset` + re-declare every frame allocates nothing.
    let mut g = FrameGraph::with_capacity(16, 16, 64);

    let ao = g.add_image("ao");

    // Pass 1: a compute shader writes the image.
    let write = g.add_pass("ao_compute");
    g.image_access(ao, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT,
                   VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);

    // Pass 2: a fragment shader samples it.
    let read = g.add_pass("composite");
    g.image_access(ao, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT,
                   VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::COLOR);

    g.compile(); // derives the barriers

    let mut sink = Count(0);
    g.record_pass(write, &mut sink); // before pass 1: the initial layout transition
    let before_read = sink.0;
    g.record_pass(read, &mut sink);  // before pass 2: write -> read, GENERAL -> READ_ONLY
    // Prints "write pass barriers: 1, read pass barriers: 1".
    println!("write pass barriers: {before_read}, read pass barriers: {}", sink.0 - before_read);
}
```

A second read of `ao` at the same stage would get no barrier: the data is already visible there.

| Call | Purpose |
|------|---------|
| `add_image(name)`, `add_buffer(name)` | A resource that starts undefined. |
| `add_image_seeded`, `add_buffer_seeded` | A resource whose state carries over from the previous frame (`ResSync`). |
| `add_image_mipped(name, mips, seed)` | An image tracked per mip level, such as the HZB pyramid. |
| `add_pass(name)` | Starts a pass; the accesses that follow belong to it. |
| `image_access`, `buffer_access` | One access: stage, access mask, and layout plus subresource range for images. |
| `compile()` | Derives the barrier plan. |
| `record_pass(pass, &mut sink)` | Feeds the pass's barriers to a `BarrierSink`, grouped by stage pair. |

## How the compile works

Each resource carries a `ResSync` state: its current layout, the pending write (access and stages)
and what has already been made visible. For every access, `transition` emits a barrier only when
one of these holds:

- the layout differs from the one the pass needs;
- the access reads data a previous pass wrote and has not yet been flushed and made visible;
- the access writes over data a previous pass read or wrote, so the two must be ordered;
- the read happens at a stage or access the last flush did not cover.

Otherwise the access is free. State is keyed by `(resource, mip)`, so a pass that writes mip *k*
while reading mip *k − 1* (the HZB build) gets exact barriers, and adjacent mips in the same state
share one barrier.

Barriers name resources by logical `ResId`, not by `VkImage`. The sink binds the physical handle at
record time, so one plan serves either frame-in-flight slot.

## Frame writes: `FrameWriteToken`

The host writes some GPU buffers from the CPU every frame (the instance rings, the camera block,
the UI instances). Two frames are in flight (`FRAMES_IN_FLIGHT = 2`), so a write is only safe to a
slot whose fence the CPU has waited on.

`FrameWriteToken` turns that rule into a type. Waiting on the slot's fence produces the token, and
per-slot write functions take `&FrameWriteToken` instead of a slot index. The token cannot be
cloned, and the calls that submit the frame consume it by value, so a write after submit, or a
token kept across frames, does not compile. It costs one `usize`. `forge_unfenced` exists for
setup code that writes before any frame was submitted; it is `unsafe`.

## Design decisions

- **Linear order.** Passes compile in declaration order. Each path's frame is authored in
  dependency order, so a topological sort would return the same order.
- **Preallocated storage.** `FrameGraph::with_capacity` allocates the resource, pass and access
  arrays once; `reset` clears them and keeps the capacity.
- **Vulkan-valued barriers.** The graph lives in `boyko_rhi_vulkan` and stores raw `Vk*` stages,
  access masks and layouts. A backend-neutral layer in `boyko_rhi` would need the whole graphics
  stage and layout vocabulary for a single backend.

## See also

- [RHI & Vulkan Backend](rhi.md): the device, handles and the dispatcher token.
- [Render Paths & Visibility Buffer](render-paths.md): which passes each path declares.
- [GPU-Resident Columns](gpu-columns.md): the per-frame uploads the write token guards.
- Source: [`framegraph/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_rhi_vulkan/src/framegraph)
  and [`present/frame_driver.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/present/frame_driver.rs).
