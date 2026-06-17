//! Phase 5 Option C (C1) — the public `UnsafeEcsCell::nonsend_resource_mut` is
//! DELETED; naming it must NOT compile.
//!
//! Wave C exposed a `pub unsafe fn UnsafeEcsCell::nonsend_resource_mut<R>` that
//! was reachable on the concurrent worker path. Option C removed it (the `!Send`
//! projection is now reachable only through the dispatcher-only
//! `DispatcherToken`). This case proves the worker-reachable surface is gone: if
//! the method still existed the call would type-check.
//!
//! The cell is passed in by value (its constructors are `pub(crate)`, so an
//! out-of-crate test cannot mint one) — that isolates the failure to the missing
//! `nonsend_resource_mut` method rather than a construction error.

use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_ecs::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

struct NonSendCounter {
    _value: u32,
    _not_send: *const u8,
}
impl NonSendResource for NonSendCounter {}

/// Calling the deleted method on a by-value cell — the `no method named
/// nonsend_resource_mut` error is the load-bearing failure.
unsafe fn project(cell: UnsafeEcsCell<'_>) {
    let _ = unsafe { cell.nonsend_resource_mut::<NonSendCounter>() };
}

fn main() {}
