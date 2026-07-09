//! R0b — constructing `FrameWriteToken { slot: 0 }` from OUTSIDE the crate must
//! NOT compile: the field is private, so the only mints are the fence proof
//! (`Renderer::wait_frame_in_flight`) and the audited `unsafe` setup-seeding
//! hatch (`FrameWriteToken::forge_unfenced`).

use boyko_rhi_vulkan::swapchain::FrameWriteToken;

/// Forges the write proof via a struct literal — rejected (private field).
fn forge() -> FrameWriteToken {
    FrameWriteToken { slot: 0 }
}

fn main() {
    // Keeps `forge` live so the ONLY diagnostic is the private-field rejection
    // (E0451 does not abort before the dead-code lint pass, unlike E0599/E0382).
    let _ = forge();
}
