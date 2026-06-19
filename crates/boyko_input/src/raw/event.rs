//! The single seam type between any input source and the action core (§5.2).
//!
//! Every frontend (native Win32 window, egui demo, synthetic test stream)
//! translates its own events into [`RawInputEvent`] and pushes them through
//! [`RawInputQueue::push_raw`](super::queue::RawInputQueue::push_raw). This is
//! the load-bearing data seam (Decision 3): a `match` over a plain enum, never
//! a `Box<dyn InputBackend>` — no virtual dispatch on the per-event path, and
//! the core never owns a windowing object.

use super::keycode::{ButtonState, KeyCode, MouseButton, ScrollDelta};

/// A single source-agnostic raw input event.
///
/// `#[repr(C)]` keeps the layout predictable for the (deferred) threaded SPSC
/// pump and any future record/replay log that blits the ring (plan §13).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RawInputEvent {
    /// A physical key transition. `repeat` marks an OS auto-repeat event (the
    /// edge accumulator ignores repeats; only the first `Pressed` is an edge).
    Key {
        code: KeyCode,
        state: ButtonState,
        repeat: bool,
    },
    /// A logical character produced under the active layout — text fields only,
    /// never gameplay. Kept distinct from `Key` so IME/composition stays out of
    /// the physical-binding path.
    Text(char),
    /// A physical mouse-button transition.
    MouseButton {
        button: MouseButton,
        state: ButtonState,
    },
    /// Raw relative pointer motion (un-accelerated) — camera look. Summed into
    /// `mouse_delta` each frame.
    MouseMotion { dx: f64, dy: f64 },
    /// Absolute cursor position in window coordinates — UI hit-testing. Stored
    /// as the last-seen position each frame.
    CursorMoved { x: f64, y: f64 },
    /// A scroll-wheel delta. Summed into the `wheel` accumulator each frame.
    Wheel(ScrollDelta),
}
