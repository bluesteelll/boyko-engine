//! GUI P5b — the text EMIT/MEASURE no-realloc gate (Decision T5-A), via a counting
//! global allocator.
//!
//! Decision T5-A: "emit/measure scratch is sized once at setup to the hard
//! `UiTextBuffer::CAP` bound and asserted never to realloc." The plan's M5/T5 gate
//! requires a counting-allocator bench asserting the emit path NEVER reallocs in steady
//! state. The existing P5a `ui_no_realloc` test covers only the RECT pack lane
//! (`text_uv: None`); this test extends the no-realloc proof to the GLYPH emit path.
//!
//! The emit driver is [`emit_glyphs`](boyko_ui::text::emit_glyphs) — the live emit core
//! the host folds into the P5a instance stream. It shapes the content via the streaming
//! [`shape_into`](boyko_ui::text::shape_into) (a single pass, no buffering) and pushes
//! one [`GlyphInstance`](boyko_ui::text::GlyphInstance) per visible glyph into a reused
//! [`TextEmitScratch`](boyko_ui::text::TextEmitScratch). Once warmed to the
//! `UiTextBuffer::CAP` worst case, a steady-state frame is `clear()` + `push` only, so
//! the global allocator must observe ZERO allocations across repeated frames at the same
//! glyph count.
//!
//! A counting global allocator (installed for THIS test binary only) makes the invariant
//! a hard assert. The armed window brackets the allocation counter around the warmed
//! frames so the first-frame growth (legitimately excluded from the steady-state
//! guarantee) does not mask a regression.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use boyko_ui::components::ComputedRect;
use boyko_ui::text::{
    emit_glyphs, measure_one, AtlasKind, AtlasMeta, BakedFont, FontId, FontTable, GlyphInstance,
    GlyphMetrics, TextAlign, TextEmitScratch, UiText,
};

// ───────────────────────── counting allocator ─────────────────────────────

struct CountingAlloc;

// Per-thread counting: only allocations made on the ARMED measuring thread count, so the
// test harness's own threads (result collection, output, the thread pool) and any other
// test running concurrently in this binary CANNOT pollute the window — which is why no
// cross-test lock is needed. A process-global flag would count the harness's one-off
// allocations during an armed window as a false non-zero (the original flake: a single
// stray `alloc` on a harness thread tripping `left: 1, right: 0`).
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    static REALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// Arms THIS thread (zeroing its counters) and returns a guard that disarms on drop, so
/// the measured window is exactly the guard's lifetime. Call AFTER warm-up.
fn arm() -> ArmGuard {
    ALLOC_COUNT.with(|c| c.set(0));
    REALLOC_COUNT.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
    ArmGuard
}

struct ArmGuard;
impl Drop for ArmGuard {
    fn drop(&mut self) {
        ARMED.with(|a| a.set(false));
    }
}

fn alloc_count() -> usize {
    ALLOC_COUNT.with(|c| c.get())
}
fn realloc_count() -> usize {
    REALLOC_COUNT.with(|c| c.get())
}

// SAFETY: every method forwards verbatim to the system allocator (same ptr/layout
// contract); the only added work is a per-thread counter bump when this thread is armed.
// The thread-locals are `const`-initialized (inline TLS storage — no lazy heap init, so
// no re-entrant allocation) and read via `try_with`, so a thread tearing down its TLS
// simply does not count instead of panicking inside the allocator.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ARMED.try_with(|a| {
            if a.get() {
                let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ARMED.try_with(|a| {
            if a.get() {
                let _ = REALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
            }
        });
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

// ───────────────────────── a tiny ASCII font ──────────────────────────────

/// Builds a tiny `BakedFont` with a non-empty (visible) quad for every printable ASCII
/// codepoint `0x20..0x7F`, so `emit_glyphs` emits one [`GlyphInstance`] per source char.
/// Slot 0 is `.notdef`; slots 1.. map the ASCII codepoints in sorted order.
fn ascii_font() -> BakedFont {
    use boyko_fontbake::atlas::MappedCodepoint;

    // A visible glyph: a 1×1 atlas cell with a non-degenerate plane quad (so the emitter
    // does not skip it as an empty/space quad).
    let visible = GlyphMetrics {
        advance_em: 0.5,
        plane: [0.0, 0.0, 0.5, 0.7], // non-zero area ⇒ emitted
        atlas: [0.0, 1.0, 1.0, 0.0],
    };
    let mut glyphs = vec![GlyphMetrics {
        advance_em: 0.0,
        plane: [0.0; 4],
        atlas: [0.0; 4],
    }]; // slot 0 = .notdef
    let mut cmap: Vec<MappedCodepoint> = Vec::new();
    for (slot, cp) in (1u16..).zip(0x20u32..0x7F) {
        glyphs.push(visible);
        cmap.push(MappedCodepoint { codepoint: cp, slot });
    }

    BakedFont {
        meta: AtlasMeta {
            distance_range_texels: 6.0,
            pixels_per_em: 48.0,
            atlas_w: 1,
            atlas_h: 1,
            ascender_em: 0.8,
            descender_em: -0.2,
            line_gap_em: 0.0,
            kind: AtlasKind::Mtsdf,
        },
        glyphs,
        cmap,
        kern: Vec::new(),
        atlas: boyko_fontbake::atlas::AtlasImage {
            width: 1,
            height: 1,
            pixels: vec![0u8; 4],
        },
    }
}

/// A long single-word content of `n` printable ASCII glyphs (no whitespace ⇒ no wrap
/// split, a single line, exactly `n` emitted glyphs). Bounded by `UiTextBuffer::CAP`.
fn content(n: usize) -> String {
    "A".repeat(n)
}

/// One steady-state emit frame: `clear()` the scratch, then emit the node's glyphs into
/// it (the exact per-frame host call). No wrap (`rect.w == 0` ⇒ single line).
fn emit_frame(scratch: &mut TextEmitScratch, fonts: &FontTable, text: &UiText, body: &str) {
    scratch.clear();
    let rect = ComputedRect { x: 4.0, y: 4.0, w: 0.0, h: 0.0 };
    emit_glyphs(text, &rect, body, boyko_ui::components::StackIndex(0), None, fonts, &mut scratch.glyphs);
}

/// THE GATE: the reused emit scratch does NOT reallocate frame-to-frame in steady state
/// (Decision T5-A). Once warmed to the per-node glyph high-water, `clear()` + `push`
/// reuse the storage and the capacity is byte-stable.
#[test]
fn text_emit_scratch_does_not_realloc_in_steady_state() {
    // Glyph count well under UiTextBuffer::CAP (247) but large enough to exercise growth.
    const N: usize = 200;
    let mut fonts = FontTable::new();
    let id = fonts.load(&ascii_font());
    assert_eq!(id, FontId(0), "single resident font is slot 0 (Decision T4-E)");

    let text = UiText { color: 0xFFFF_FFFF, size_px: 16.0, font: id, align: TextAlign::Left, _pad: 0 };
    let body = content(N);
    let mut scratch = TextEmitScratch::new();

    // Warm-up: the first frames legitimately allocate + grow the scratch to high-water.
    for _ in 0..3 {
        emit_frame(&mut scratch, &fonts, &text, &body);
    }
    assert_eq!(scratch.glyphs.len(), N, "every visible glyph is emitted (no wrap)");
    let cap = scratch.glyphs.capacity();
    assert!(cap >= N, "warmed to >= N capacity");

    // Steady state: arm THIS thread and run many frames at the SAME glyph count.
    let _arm = arm();

    for _ in 0..64 {
        emit_frame(&mut scratch, &fonts, &text, &body);
    }

    let allocs = alloc_count();
    let reallocs = realloc_count();

    assert_eq!(
        scratch.glyphs.capacity(),
        cap,
        "the emit scratch capacity must not change in steady state (no hidden grow)"
    );
    assert_eq!(
        reallocs, 0,
        "the glyph emit scratch must NOT realloc frame-to-frame in steady state (got {reallocs})"
    );
    assert_eq!(
        allocs, 0,
        "the glyph emit path must allocate nothing per frame in steady state \
         (clear() + push reuse the warmed buffer) — got {allocs}"
    );
}

/// Smaller steady-state frames keep capacity (clear() retains it; push stays under it),
/// so a frame whose glyph count is below the warmed high-water is fully alloc-free.
#[test]
fn text_emit_scratch_smaller_frame_reuses_capacity() {
    const WARM_N: usize = 200;
    const STEADY_N: usize = 12;
    let mut fonts = FontTable::new();
    let id = fonts.load(&ascii_font());

    let text = UiText { color: 0xFFFF_FFFF, size_px: 16.0, font: id, align: TextAlign::Left, _pad: 0 };
    let warm = content(WARM_N);
    let steady = content(STEADY_N);
    let mut scratch = TextEmitScratch::new();

    for _ in 0..3 {
        emit_frame(&mut scratch, &fonts, &text, &warm);
    }

    let _arm = arm();

    for _ in 0..64 {
        emit_frame(&mut scratch, &fonts, &text, &steady);
    }

    assert_eq!(
        realloc_count(),
        0,
        "smaller steady-state emit frames must not realloc"
    );
    assert_eq!(
        alloc_count(),
        0,
        "smaller steady-state emit frames must not allocate (clear keeps capacity)"
    );
}

/// `measure_one` (the change-gated measure core) shapes through the SAME streaming
/// `shape_into` to a NO-OP sink and returns the extent directly, so a measure is fully
/// allocation-free per call (no scratch at all).
#[test]
fn text_measure_is_alloc_free_per_call() {
    const N: usize = 200;
    let mut fonts = FontTable::new();
    let id = fonts.load(&ascii_font());

    let text = UiText { color: 0xFFFF_FFFF, size_px: 16.0, font: id, align: TextAlign::Left, _pad: 0 };
    let body = content(N);
    let rect = ComputedRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    // Warm any one-time lazy init (none expected; measure touches no heap).
    for _ in 0..3 {
        let _ = measure_one(&text, &body, &rect, &fonts);
    }

    let _arm = arm();

    let mut last = 0.0f32;
    for _ in 0..64 {
        let size = measure_one(&text, &body, &rect, &fonts);
        last = size.width; // keep the result live so the call is not optimized away
    }

    assert!(last > 0.0, "a non-empty run has a positive measured width");
    assert_eq!(
        alloc_count(),
        0,
        "measure_one must allocate nothing per call (streaming shape to a no-op sink)"
    );
}

/// A compile-time + value witness that the emit lane really produces the renderable
/// descriptor the P5a pack consumes (`GlyphInstance` carries the atlas UV + premul-able
/// color + z key) — the CPU half of "text is not stubbed".
#[test]
fn emit_produces_renderable_glyph_instances() {
    let mut fonts = FontTable::new();
    let id = fonts.load(&ascii_font());
    let text = UiText { color: 0xFF00_00FF, size_px: 24.0, font: id, align: TextAlign::Left, _pad: 0 };
    let mut out: Vec<GlyphInstance> = Vec::new();
    let rect = ComputedRect { x: 10.0, y: 20.0, w: 0.0, h: 0.0 };
    emit_glyphs(&text, &rect, "Hi", boyko_ui::components::StackIndex(7), None, &fonts, &mut out);

    assert_eq!(out.len(), 2, "two visible glyphs emitted for \"Hi\"");
    for g in &out {
        assert_eq!(g.color, 0xFF00_00FF, "the glyph carries the node's foreground color");
        assert_eq!(g.stack, 7, "the glyph carries the node's StackIndex z key");
        assert!(g.uv.iter().all(|v| (0.0..=1.0).contains(v)), "the glyph UV is normalized");
        assert!(g.rect[2] > 0.0 && g.rect[3] > 0.0, "the glyph quad has a positive footprint");
    }
    // The two glyphs advance horizontally (distinct pen x), proving real shaping.
    assert!(out[1].rect[0] > out[0].rect[0], "the second glyph pen-advances past the first");
}
