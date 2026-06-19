//! Compile-time constants for `boyko_input` (engine principle: all constants in
//! one place, `pub const`, no wrappers).

/// Default capacity of the [`RawInputQueue`](crate::raw::queue::RawInputQueue)
/// ring buffer (plan §5.4). Must be a power of two so the head/tail wrap is a
/// branchless mask. 1024 events comfortably absorbs a single frame's burst even
/// under a key-repeat storm; overflow is drop-oldest with a debug assert.
pub const RAW_QUEUE_CAP: usize = 1024;

/// The maximum number of distinct actions a single [`Actionlike`] enum may
/// declare (V8). Set by the `BitSet256` capacity of
/// [`ActionState`](crate::action::state::ActionState) — well beyond any real
/// input map (real games run 10–60). A `COUNT > 256` enum is a cold exotic
/// case handled by a future `BitSetN`-generic fallback (plan §13).
///
/// [`Actionlike`]: crate::action::actionlike::Actionlike
pub const MAX_ACTIONS: usize = 256;

/// Maximum number of keys in a chord binding (V9) — covers
/// `Ctrl+Shift+Alt+key`.
pub const MAX_CHORD_KEYS: usize = 4;

/// Conversion factor from one scroll *line* to *pixels* when accumulating a
/// mixed `Lines`/`Pixels` wheel stream into the single `f64` accumulator
/// (plan §12). A conventional one-notch line ≈ this many pixels.
pub const LINE_TO_PIXEL: f64 = 50.0;

/// Debug-only guard on the clash-resolution active-chord set (plan §6
/// Decision 8). The clash pass is O(active²) over only *active* bindings
/// (typically < 10); this catches a pathological binding map in dev.
pub const CLASH_LIMIT: usize = 64;
