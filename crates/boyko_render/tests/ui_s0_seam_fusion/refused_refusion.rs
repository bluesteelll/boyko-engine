//! UI-ADVANCED S0 gate G0-5 — a call site that RE-FUSES the two-phase seam
//! (Phase 1's `WorldView` live while Phase 2's `&mut RhiContext` is projected
//! from the same token) must be rejected by borrowck.
//!
//! This is the exact shape of the deleted `host_upload_frame_from_world`: a
//! signature demanding a live view AND the `!Send` context at one call site.
//! `world()` borrows the token `&self`; `nonsend_resource_mut` borrows it
//! `&mut self`; the view is USED after the projection, so the shared borrow is
//! live across the mutable one — E0502 (M1 working as designed).
//!
//! The token is passed in by value (its constructor is `pub(crate)`, so an
//! out-of-crate test cannot mint one) — that isolates the failure to the
//! conflicting borrows, not a construction error.

use boyko_ecs::ecs::core::system::DispatcherToken;
use boyko_render::{RhiContext, UiUploadSystem};

/// Holds Phase 1's view live across Phase 2's projection. Rejected.
fn refused_refusion(sys: &mut UiUploadSystem, mut token: DispatcherToken<'_>) -> usize {
    let view = token.world();
    let rhi = token.nonsend_resource_mut::<RhiContext>();
    let n = sys.gather_into_staging(&view);
    drop(rhi);
    n
}

fn main() {}
