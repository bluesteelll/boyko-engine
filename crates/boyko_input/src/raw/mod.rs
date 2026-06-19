//! The source-agnostic raw-input layer (plan §5).
//!
//! Nothing here depends on a windowing library. The canonical physical enums
//! ([`keycode`]), the single seam event ([`event`]), the ring buffer + per-frame
//! snapshot ([`queue`]), and the static scancode tables ([`scancode`]) are all
//! plain data. Frontends translate into [`event::RawInputEvent`] and push
//! through [`queue::RawInputQueue::push_raw`].

pub mod event;
pub mod keycode;
pub mod queue;
pub mod scancode;
