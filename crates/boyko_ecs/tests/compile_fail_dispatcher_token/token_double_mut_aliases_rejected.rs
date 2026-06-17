//! Phase 5 Option C (M1) — two live `&mut R` from one `DispatcherToken` must be
//! rejected.
//!
//! `DispatcherToken::nonsend_resource_mut` returns a `&mut R` tied to `&mut self`,
//! NOT to the token's `'w`. So a second call while the first borrow is still live
//! needs a second `&mut self` borrow of the same token — which the borrow checker
//! forbids. If this compiled, two aliasing `&mut R` would coexist (the M1 UB hole).
//!
//! The token is passed in by value (its constructor `new` is `pub(crate)`, so an
//! out-of-crate test cannot mint one) — that isolates the failure to the
//! conflicting `&mut self` borrows, not a construction error.

use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_ecs::ecs::core::system::DispatcherToken;

struct NonSendCounter {
    value: u32,
    _not_send: *const u8,
}
impl NonSendResource for NonSendCounter {}

/// Holds the result of two `nonsend_resource_mut` calls live at once — the second
/// call needs a 2nd `&mut token` while the first `&mut R` still borrows the token.
/// Rejected.
fn double_project(mut token: DispatcherToken<'_>) {
    let a = token.nonsend_resource_mut::<NonSendCounter>().unwrap();
    let b = token.nonsend_resource_mut::<NonSendCounter>().unwrap();
    a.value += 1;
    b.value += 1;
}

fn main() {}
