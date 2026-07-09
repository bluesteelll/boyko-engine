//! #30 (M1, the C1-fix `WorldView` discipline) — a live `WorldView` from
//! `DispatcherToken::world()` must exclude `nonsend_resource_mut`.
//!
//! `world()` borrows the token `&self`; `nonsend_resource_mut` borrows it
//! `&mut self`. While the `WorldView` (and a reference it handed out) is still
//! live, the borrow checker forbids the `&mut self` borrow — so the read view
//! and the mutable `!Send` projection can never coexist. If this compiled, a
//! `&R` read through the view could alias a `&mut R` projected from the token.
//!
//! The token is passed in by value (its constructor `new` is `pub(crate)`, so an
//! out-of-crate test cannot mint one) — that isolates the failure to the
//! conflicting `&self` vs `&mut self` borrows, not a construction error.

use boyko_ecs::ecs::core::resources::resource::{NonSendResource, Resource};
use boyko_ecs::ecs::core::system::DispatcherToken;
use boyko_ecs::ecs::identifiers::primitives::ResourceId;

struct NonSendCounter {
    value: u32,
    _not_send: *const u8,
}
impl NonSendResource for NonSendCounter {}

#[derive(Debug)]
struct ViewResource(u32);
impl Resource for ViewResource {
    fn resource_id() -> ResourceId {
        // An out-of-crate test cannot reach the crate-internal registry, but the
        // body never runs — the file is rejected at borrowck before any call.
        unimplemented!()
    }
}

/// Holds a `&R` read through a `WorldView` live across a `&mut self` projection
/// of the same token. The `world()` borrow (`&token`) conflicts with
/// `nonsend_resource_mut`'s (`&mut token`). Rejected.
fn world_then_mut(mut token: DispatcherToken<'_>) {
    let view = token.world();
    let r = view.resource::<ViewResource>();
    let m = token.nonsend_resource_mut::<NonSendCounter>().unwrap();
    m.value += r.0;
}

fn main() {}
