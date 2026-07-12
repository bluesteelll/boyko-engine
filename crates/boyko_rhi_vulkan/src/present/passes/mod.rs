//! The four `Renderer::record_*` command-buffer bodies, one per frame skeleton:
//! `clear` (UNDEFINED→COLOR→PRESENT), `scene` (depth-tested draw), `present_blit`
//! (fullscreen composite sample), and `gbuffer` (the raster→march→resolve→blit
//! 3-pass). Each is an `impl Renderer` block; the shared acquire/submit/present
//! skeleton lives in [`super::frame_driver`]. Split out of the former monolithic
//! `swapchain.rs` (audit W4).

mod clear;
mod fxaa;
mod gbuffer;
mod present_blit;
mod scene;
mod smaa;
mod ssaa;
mod taa;
