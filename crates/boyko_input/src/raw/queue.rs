//! The raw-event ring buffer ([`RawInputQueue`], §5.4) and the per-frame
//! physical snapshot ([`PhysicalInput`], §5.5).

use boyko_macros::Resource;
use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::constants::RAW_QUEUE_CAP;
use crate::raw::event::RawInputEvent;
use crate::raw::keycode::{ButtonState, MouseButton, ScrollDelta};

/// Fixed-capacity SPSC ring buffer of raw input events (plan §5.4).
///
/// One allocation at plugin build (`with_capacity`); the per-frame ingest path
/// only reads/writes the existing buffer — **zero per-frame heap allocation**.
/// The capacity is forced to a power of two so head/tail wrap is a branchless
/// mask.
///
/// # Overflow policy: drop-oldest
/// On a slow frame with a key-repeat storm the *newest* events (the player's
/// latest intent) must survive; the oldest stale repeats are evicted. The
/// `dropped` counter records evictions this frame and a `debug_assert!` fires
/// in debug so the cap is tuned, never silently lossy in dev. The ring is
/// drained fully each frame, so overflow can only occur within one frame's
/// burst.
///
/// # Seam (Decision 3)
/// [`RawInputQueue::push_raw`] is the single load-bearing seam: every frontend
/// translates its native events into [`RawInputEvent`] and pushes them here
/// before the scheduler window. No `dyn`, no allocation, no atomics (v1 is
/// pump-then-update, serial — §11). The layout is SPSC-ready for a future
/// threaded pump (head/tail promotable to atomics).
#[derive(Resource)]
pub struct RawInputQueue {
    /// One-time allocation; `cap` is a power of two.
    buf: Box<[RawInputEvent]>,
    /// `cap - 1`, the wrap mask. Stored to avoid recomputing per push/pop.
    mask: u32,
    /// Index of the oldest live event (next to read).
    tail: u32,
    /// Number of live events in the ring (`0..=cap`).
    len: u32,
    /// High-water mark of `len` observed since construction (debug
    /// observability for cap tuning).
    high_water: u32,
    /// Count of drop-oldest evictions in the current frame; reset by
    /// [`RawInputQueue::begin_frame`].
    dropped: u32,
}

impl RawInputQueue {
    /// Allocates a ring with capacity rounded up to the next power of two
    /// (minimum [`RAW_QUEUE_CAP`]). Cold path — called once at plugin build.
    pub fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(1).next_power_of_two();
        debug_assert!(cap <= u32::MAX as usize, "RawInputQueue capacity overflows u32");
        // `RawInputEvent` is `Copy`; a filler value initializes the unused
        // slots. They are never read while `len` says they are empty.
        let filler = RawInputEvent::MouseMotion { dx: 0.0, dy: 0.0 };
        let buf = vec![filler; cap].into_boxed_slice();
        Self {
            buf,
            mask: (cap - 1) as u32,
            tail: 0,
            len: 0,
            high_water: 0,
            dropped: 0,
        }
    }

    /// Pushes a raw event (the seam, Decision 3).
    ///
    /// Called O(events/frame) from the runner thread *before* the scheduler
    /// window. No allocation. On overflow the oldest event is evicted
    /// (drop-oldest) and `dropped` is incremented.
    #[inline]
    pub fn push_raw(&mut self, ev: RawInputEvent) {
        let cap = self.buf.len() as u32;
        if self.len == cap {
            // Full: evict the oldest event (drop-oldest), keep the newest.
            self.tail = (self.tail + 1) & self.mask;
            self.len -= 1;
            self.dropped += 1;
            debug_assert!(
                self.dropped == 0,
                "RawInputQueue overflow — raise RAW_QUEUE_CAP"
            );
        }
        let head = (self.tail + self.len) & self.mask;
        // `head <= mask < cap == buf.len()`, so the indexed write is in bounds;
        // the bounds check is provably elidable.
        self.buf[head as usize] = ev;
        self.len += 1;
        if self.len > self.high_water {
            self.high_water = self.len;
        }
    }

    /// Pops the oldest event, or `None` if empty. Used by the drain loop.
    #[inline]
    pub fn pop(&mut self) -> Option<RawInputEvent> {
        if self.len == 0 {
            return None;
        }
        let ev = self.buf[self.tail as usize];
        self.tail = (self.tail + 1) & self.mask;
        self.len -= 1;
        Some(ev)
    }

    /// Resets the per-frame `dropped` counter. Called by the ingest system at
    /// the start of each frame before draining.
    #[inline]
    pub fn begin_frame(&mut self) {
        self.dropped = 0;
    }

    /// Number of live events currently in the ring.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns `true` if the ring holds no events.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Total capacity (a power of two).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Drop-oldest evictions in the current frame (debug observability).
    #[inline]
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// High-water mark of `len` since construction (cap-tuning observability).
    #[inline]
    pub fn high_water(&self) -> u32 {
        self.high_water
    }
}

impl Default for RawInputQueue {
    #[inline]
    fn default() -> Self {
        Self::with_capacity(RAW_QUEUE_CAP)
    }
}

/// Per-frame physical input snapshot (plan §5.5).
///
/// Keyboard state is held as [`BitSet256`] level + edge sets indexed by
/// [`KeyCode::dense_index`](crate::raw::keycode::KeyCode::dense_index); mouse
/// buttons as `u8` bitmasks. Edges (`just_pressed`/`just_released`) are
/// accumulated from the **event stream** (W4) so a same-frame down+up tap is
/// not lost. `#[repr(C, align(64))]` keeps the hot bitsets cache-line aligned.
#[derive(Resource, Clone)]
#[repr(C, align(64))]
pub struct PhysicalInput {
    /// Level: keys held this frame.
    pub keys_pressed: BitSet256,
    /// Edge: keys whose first `Pressed` arrived this frame (event-stream).
    pub keys_just_pressed: BitSet256,
    /// Edge: keys whose `Released` arrived this frame (event-stream).
    pub keys_just_released: BitSet256,
    /// Level: mouse buttons held this frame (bit `i` = canonical button `i`).
    pub mouse_pressed: u8,
    /// Edge: mouse buttons pressed this frame.
    pub mouse_just_pressed: u8,
    /// Edge: mouse buttons released this frame.
    pub mouse_just_released: u8,
    /// Summed raw relative motion this frame (`[dx, dy]`).
    pub mouse_delta: [f64; 2],
    /// Last absolute cursor position seen this frame (`[x, y]`).
    pub cursor_pos: [f64; 2],
    /// Summed wheel delta this frame (`[x, y]`), pixels (lines folded in via
    /// `LINE_TO_PIXEL`).
    pub wheel: [f64; 2],
    /// Level: whether the cursor is currently inside the window surface (GUI P4
    /// Decision 12). Set/cleared from the OS `CursorEntered`/`CursorLeft` events
    /// and PERSISTS across `begin_frame` (a level, not an edge). Defaults `true`
    /// so a host that never routes cursor-enter/leave (e.g. a synthetic test
    /// stream) sees the cursor as inside and the UI hit-test runs.
    pub cursor_inside: bool,
    /// Level: whether the window currently holds keyboard/input focus (GUI P4
    /// Decision 12). Set/cleared from the OS `WindowFocus` event and PERSISTS
    /// across `begin_frame`. Defaults `true` for the same reason as
    /// `cursor_inside`.
    pub window_focused: bool,
}

impl PhysicalInput {
    /// Returns an empty snapshot (all bits clear, all accumulators zero).
    #[inline]
    pub fn new() -> Self {
        Self {
            keys_pressed: BitSet256::new(),
            keys_just_pressed: BitSet256::new(),
            keys_just_released: BitSet256::new(),
            mouse_pressed: 0,
            mouse_just_pressed: 0,
            mouse_just_released: 0,
            mouse_delta: [0.0; 2],
            cursor_pos: [0.0; 2],
            wheel: [0.0; 2],
            // Levels default "inside / focused" so a host that never routes the
            // cursor-enter/window-focus events still hit-tests (GUI P4 D12).
            cursor_inside: true,
            window_focused: true,
        }
    }

    /// Clears the per-frame edge/accumulator state at the start of a frame,
    /// preserving the persistent level state (`keys_pressed`, `mouse_pressed`,
    /// `cursor_pos`).
    ///
    /// Edge sets and the frame-summed accumulators reset to empty; held levels
    /// and the last cursor position carry over (they are level state, mutated
    /// by the event stream, not re-derived each frame).
    #[inline]
    pub fn begin_frame(&mut self) {
        self.keys_just_pressed = BitSet256::new();
        self.keys_just_released = BitSet256::new();
        self.mouse_just_pressed = 0;
        self.mouse_just_released = 0;
        self.mouse_delta = [0.0; 2];
        self.wheel = [0.0; 2];
        // `cursor_pos` and the held levels persist across frames.
    }

    /// Applies one raw event to the snapshot, accumulating edges from the event
    /// stream (W4). OS auto-repeat key events update nothing (a repeat is not a
    /// new edge and the level is already held).
    #[inline]
    pub fn apply(&mut self, ev: &RawInputEvent) {
        match *ev {
            RawInputEvent::Key { code, state, repeat } => {
                if repeat {
                    return;
                }
                if let Some(idx) = code.dense_index() {
                    match state {
                        ButtonState::Pressed => {
                            self.keys_pressed.set(idx);
                            self.keys_just_pressed.set(idx);
                        }
                        ButtonState::Released => {
                            self.keys_pressed.clear(idx);
                            self.keys_just_released.set(idx);
                        }
                    }
                }
            }
            RawInputEvent::MouseButton { button, state } => {
                if let Some(idx) = button.dense_index() {
                    let bit = 1u8 << idx;
                    match state {
                        ButtonState::Pressed => {
                            self.mouse_pressed |= bit;
                            self.mouse_just_pressed |= bit;
                        }
                        ButtonState::Released => {
                            self.mouse_pressed &= !bit;
                            self.mouse_just_released |= bit;
                        }
                    }
                }
            }
            RawInputEvent::MouseMotion { dx, dy } => {
                self.mouse_delta[0] += dx;
                self.mouse_delta[1] += dy;
            }
            RawInputEvent::CursorMoved { x, y } => {
                self.cursor_pos = [x, y];
            }
            RawInputEvent::Wheel(delta) => {
                let (x, y) = match delta {
                    ScrollDelta::Lines { x, y } => (
                        x as f64 * crate::constants::LINE_TO_PIXEL,
                        y as f64 * crate::constants::LINE_TO_PIXEL,
                    ),
                    ScrollDelta::Pixels { x, y } => (x, y),
                };
                self.wheel[0] += x;
                self.wheel[1] += y;
            }
            // Text is for text fields only — never gameplay; the physical
            // snapshot ignores it.
            RawInputEvent::Text(_) => {}
            // Window-surface level state (GUI P4 D12) — drives the UI blur/leave
            // short-circuit. Persisted across `begin_frame` like other levels.
            RawInputEvent::CursorEntered => {
                self.cursor_inside = true;
            }
            RawInputEvent::CursorLeft => {
                self.cursor_inside = false;
            }
            RawInputEvent::WindowFocus(focused) => {
                self.window_focused = focused;
            }
        }
    }

    /// Tests whether a canonical mouse button is held this frame.
    #[inline]
    pub fn mouse_held(&self, button: MouseButton) -> bool {
        match button.dense_index() {
            Some(idx) => (self.mouse_pressed >> idx) & 1 == 1,
            None => false,
        }
    }
}

impl Default for PhysicalInput {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::keycode::KeyCode;

    fn key(code: KeyCode, state: ButtonState) -> RawInputEvent {
        RawInputEvent::Key {
            code,
            state,
            repeat: false,
        }
    }

    #[test]
    fn capacity_rounds_up_to_power_of_two() {
        let q = RawInputQueue::with_capacity(1000);
        assert_eq!(q.capacity(), 1024);
        let q = RawInputQueue::with_capacity(1024);
        assert_eq!(q.capacity(), 1024);
    }

    #[test]
    fn fifo_order_and_wrap() {
        let mut q = RawInputQueue::with_capacity(4);
        // Fill, drain partially, refill to force a wrap, then verify FIFO order.
        for _ in 0..3 {
            q.push_raw(key(KeyCode::KeyA, ButtonState::Pressed));
        }
        assert_eq!(q.len(), 3);
        // Drain 2.
        assert!(q.pop().is_some());
        assert!(q.pop().is_some());
        // Push 2 more (head wraps past the end).
        q.push_raw(key(KeyCode::KeyB, ButtonState::Pressed));
        q.push_raw(key(KeyCode::KeyC, ButtonState::Pressed));
        assert_eq!(q.len(), 3);
        // Remaining order: A (1 left from first batch), B, C.
        assert_eq!(q.pop(), Some(key(KeyCode::KeyA, ButtonState::Pressed)));
        assert_eq!(q.pop(), Some(key(KeyCode::KeyB, ButtonState::Pressed)));
        assert_eq!(q.pop(), Some(key(KeyCode::KeyC, ButtonState::Pressed)));
        assert_eq!(q.pop(), None);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn overflow_drops_oldest() {
        // The debug_assert fires in debug; this exercises the release policy.
        let mut q = RawInputQueue::with_capacity(2);
        q.push_raw(key(KeyCode::KeyA, ButtonState::Pressed));
        q.push_raw(key(KeyCode::KeyB, ButtonState::Pressed));
        // Full — this evicts the oldest (A).
        q.push_raw(key(KeyCode::KeyC, ButtonState::Pressed));
        assert_eq!(q.len(), 2);
        assert_eq!(q.dropped(), 1);
        assert_eq!(q.pop(), Some(key(KeyCode::KeyB, ButtonState::Pressed)));
        assert_eq!(q.pop(), Some(key(KeyCode::KeyC, ButtonState::Pressed)));
    }

    #[test]
    fn high_water_tracks_peak() {
        let mut q = RawInputQueue::with_capacity(8);
        for _ in 0..5 {
            q.push_raw(key(KeyCode::KeyA, ButtonState::Pressed));
        }
        q.pop();
        q.pop();
        assert_eq!(q.high_water(), 5);
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn physical_same_frame_tap_sets_both_edges() {
        // W4: a key that goes down and up in one frame must set both edges even
        // though the end-of-frame level is "not held".
        let mut p = PhysicalInput::new();
        p.begin_frame();
        p.apply(&key(KeyCode::Space, ButtonState::Pressed));
        p.apply(&key(KeyCode::Space, ButtonState::Released));
        let i = KeyCode::Space.dense_index().unwrap();
        assert!(p.keys_just_pressed.get(i), "rising edge must survive");
        assert!(p.keys_just_released.get(i), "falling edge must survive");
        assert!(!p.keys_pressed.get(i), "level is not held at frame end");
    }

    #[test]
    fn physical_repeat_is_not_an_edge() {
        let mut p = PhysicalInput::new();
        p.begin_frame();
        p.apply(&RawInputEvent::Key {
            code: KeyCode::KeyW,
            state: ButtonState::Pressed,
            repeat: true,
        });
        let i = KeyCode::KeyW.dense_index().unwrap();
        assert!(!p.keys_just_pressed.get(i));
        assert!(!p.keys_pressed.get(i));
    }

    #[test]
    fn physical_begin_frame_clears_edges_keeps_level() {
        let mut p = PhysicalInput::new();
        p.apply(&key(KeyCode::KeyW, ButtonState::Pressed));
        let i = KeyCode::KeyW.dense_index().unwrap();
        assert!(p.keys_just_pressed.get(i));
        p.begin_frame();
        assert!(!p.keys_just_pressed.get(i), "edge cleared on new frame");
        assert!(p.keys_pressed.get(i), "held level persists across frames");
    }

    // --- I1 gate: ring buffer edge cases (plan §5.4) ---

    #[test]
    fn pop_on_empty_returns_none() {
        let mut q = RawInputQueue::with_capacity(4);
        assert!(q.is_empty());
        assert_eq!(q.pop(), None, "empty pop is None");
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn capacity_minimum_is_one() {
        // `with_capacity(0)` is clamped to 1 (a 0-cap power-of-two ring is
        // degenerate); cap is always a power of two ≥ 1.
        let q = RawInputQueue::with_capacity(0);
        assert_eq!(q.capacity(), 1, "zero capacity clamps to 1");
    }

    #[test]
    fn fills_exactly_to_capacity_without_dropping() {
        let mut q = RawInputQueue::with_capacity(4);
        for _ in 0..4 {
            q.push_raw(key(KeyCode::KeyA, ButtonState::Pressed));
        }
        assert_eq!(q.len(), 4, "ring holds exactly cap events");
        assert_eq!(q.dropped(), 0, "no eviction at exactly-full");
        assert_eq!(q.high_water(), 4);
    }

    #[test]
    fn single_capacity_wraps_and_keeps_newest() {
        // A degenerate 1-slot ring: push/pop must still maintain FIFO of one.
        let mut q = RawInputQueue::with_capacity(1);
        assert_eq!(q.capacity(), 1);
        q.push_raw(key(KeyCode::KeyA, ButtonState::Pressed));
        assert_eq!(q.pop(), Some(key(KeyCode::KeyA, ButtonState::Pressed)));
        q.push_raw(key(KeyCode::KeyB, ButtonState::Pressed));
        assert_eq!(q.pop(), Some(key(KeyCode::KeyB, ButtonState::Pressed)));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn begin_frame_resets_dropped_not_high_water() {
        let mut q = RawInputQueue::with_capacity(8);
        for _ in 0..5 {
            q.push_raw(key(KeyCode::KeyA, ButtonState::Pressed));
        }
        q.begin_frame();
        assert_eq!(q.dropped(), 0, "begin_frame zeroes dropped");
        assert_eq!(q.high_water(), 5, "high_water is a lifetime peak, not reset");
        assert_eq!(q.len(), 5, "begin_frame does not drain live events");
    }

    #[test]
    fn default_uses_raw_queue_cap() {
        let q = RawInputQueue::default();
        assert_eq!(q.capacity(), RAW_QUEUE_CAP);
    }

    // --- I1 gate: PhysicalInput event-stream model (W4) ---

    #[test]
    fn physical_mouse_button_edges_and_level() {
        let mut p = PhysicalInput::new();
        p.begin_frame();
        p.apply(&RawInputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        });
        assert!(p.mouse_held(MouseButton::Left), "left held after press");
        let bit = 1u8 << MouseButton::Left.dense_index().unwrap();
        assert_eq!(p.mouse_just_pressed & bit, bit, "rising edge set");
        p.apply(&RawInputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Released,
        });
        assert!(!p.mouse_held(MouseButton::Left), "released level cleared");
        assert_eq!(p.mouse_just_released & bit, bit, "falling edge set");
    }

    #[test]
    fn physical_mouse_other_button_is_ignored() {
        // `Other(n)` has no dense index; it must not corrupt the u8 bitmask.
        let mut p = PhysicalInput::new();
        p.apply(&RawInputEvent::MouseButton {
            button: MouseButton::Other(99),
            state: ButtonState::Pressed,
        });
        assert_eq!(p.mouse_pressed, 0, "Other button leaves the mask untouched");
        assert!(!p.mouse_held(MouseButton::Other(99)));
    }

    #[test]
    fn physical_mouse_motion_accumulates() {
        let mut p = PhysicalInput::new();
        p.begin_frame();
        p.apply(&RawInputEvent::MouseMotion { dx: 1.5, dy: -2.0 });
        p.apply(&RawInputEvent::MouseMotion { dx: 0.5, dy: 1.0 });
        assert_eq!(p.mouse_delta, [2.0, -1.0], "relative motion sums per frame");
        p.begin_frame();
        assert_eq!(p.mouse_delta, [0.0, 0.0], "delta resets each frame");
    }

    #[test]
    fn physical_cursor_pos_is_last_seen() {
        let mut p = PhysicalInput::new();
        p.begin_frame();
        p.apply(&RawInputEvent::CursorMoved { x: 10.0, y: 20.0 });
        p.apply(&RawInputEvent::CursorMoved { x: 30.0, y: 40.0 });
        assert_eq!(p.cursor_pos, [30.0, 40.0], "cursor holds the latest position");
        p.begin_frame();
        assert_eq!(p.cursor_pos, [30.0, 40.0], "cursor persists across frames (level)");
    }

    #[test]
    fn physical_wheel_lines_fold_to_pixels() {
        let mut p = PhysicalInput::new();
        p.begin_frame();
        p.apply(&RawInputEvent::Wheel(ScrollDelta::Lines { x: 0.0, y: 1.0 }));
        assert_eq!(
            p.wheel[1],
            crate::constants::LINE_TO_PIXEL,
            "one line folds into LINE_TO_PIXEL pixels"
        );
        p.apply(&RawInputEvent::Wheel(ScrollDelta::Pixels { x: 5.0, y: 3.0 }));
        assert_eq!(p.wheel[0], 5.0);
        assert_eq!(p.wheel[1], crate::constants::LINE_TO_PIXEL + 3.0, "mixed units sum");
    }

    #[test]
    fn physical_text_event_is_ignored_by_snapshot() {
        // Text is for text fields, never gameplay — the physical snapshot must
        // not react to it at all.
        let mut p = PhysicalInput::new();
        p.begin_frame();
        p.apply(&RawInputEvent::Text('q'));
        assert!(p.keys_pressed.is_empty(), "Text must not touch the key level");
        assert!(p.keys_just_pressed.is_empty(), "Text must not set any edge");
    }

    #[test]
    fn physical_unidentified_key_does_not_panic_or_set_bits() {
        // An exotic key with no dense index must be a safe no-op on the bitsets.
        let mut p = PhysicalInput::new();
        p.apply(&key(KeyCode::Unidentified(0x1234), ButtonState::Pressed));
        assert!(p.keys_pressed.is_empty(), "Unidentified leaves the level empty");
        assert!(p.keys_just_pressed.is_empty(), "Unidentified sets no edge");
    }

    #[test]
    fn physical_repeat_release_is_filtered_too() {
        // OS auto-repeat only fires Pressed-with-repeat, but guard the symmetric
        // case: a repeat-flagged event of any state is a no-op.
        let mut p = PhysicalInput::new();
        p.apply(&RawInputEvent::Key {
            code: KeyCode::KeyW,
            state: ButtonState::Released,
            repeat: true,
        });
        let i = KeyCode::KeyW.dense_index().unwrap();
        assert!(!p.keys_just_released.get(i), "repeat events never produce edges");
    }

    #[test]
    fn physical_clone_is_independent() {
        // PhysicalInput is Clone (used to snapshot); a clone must not alias.
        let mut p = PhysicalInput::new();
        p.apply(&key(KeyCode::KeyW, ButtonState::Pressed));
        let snapshot = p.clone();
        p.begin_frame();
        p.apply(&key(KeyCode::KeyW, ButtonState::Released));
        let i = KeyCode::KeyW.dense_index().unwrap();
        assert!(snapshot.keys_pressed.get(i), "the clone is frozen at press time");
        assert!(!p.keys_pressed.get(i), "the original moved on independently");
    }
}
