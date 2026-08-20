//! The four `Renderer::record_*` command-buffer bodies, one per frame skeleton:
//! `clear` (UNDEFINED→COLOR→PRESENT), `scene` (depth-tested draw), `present_blit`
//! (fullscreen composite sample), and `gbuffer` (the raster→march→resolve→blit
//! 3-pass). Each is an `impl Renderer` block; the shared acquire/submit/present
//! skeleton lives in [`super::frame_driver`]. Split out of the former monolithic
//! `swapchain.rs` (audit W4).

mod clear;
mod forward;
mod fxaa;
mod gbuffer;
// Particles P0: the `upload → kickoff → emit → sim` block and the indirect billboard draw, shared
// verbatim by all three recorders (the barrier callback is what differs per path).
mod particles;
mod present_blit;
mod rcas;
mod scene;
mod smaa;
mod ssaa;
mod taa;
// `pub(crate)` for ONE item: VG R3 piece 2 step P2-6's `VbRecordProbe`, which `super` re-exports
// (gate G2's counts must reach `boyko_app`'s frame loop). Every other item in the module stays
// `pub(crate)`/private exactly as before.
pub(crate) mod vb;
