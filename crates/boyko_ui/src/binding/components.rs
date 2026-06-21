//! Data-binding components (GUI P4 Decisions 4/7/8) — the topology link, the
//! inline render-facing sinks, and the template id.
//!
//! All POD `Copy` (`Send + Sync`). The `comp`/`field` ids are resolved at
//! parse/spawn time (Decision 7), so the bind system never does a runtime string
//! compare. No heap: the text sink is an inline buffer (Decision / Principle 5).

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Component;

/// The sentinel "unused field" value for [`BindText::field2`] /
/// [`BindValue::den_field`].
pub const NO_FIELD: u8 = 0xFF;

/// A template id for the formatted text view (Decision 7). v1 supports the two
/// canonical inline forms below; a richer template table is a later extension.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TemplateId {
    /// `"{0}"` — just `field`.
    #[default]
    Value = 0,
    /// `"{0}/{1}"` — `field` then `field2` (e.g. a `current/max` HUD readout).
    Ratio = 1,
}

/// Binds a formatted text view of a source field into this widget's
/// [`UiTextBuffer`] (Decision 4/7). 16 B.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindText {
    /// The source entity hosting the bound component.
    pub source: Entity,
    /// The source component id (the accessor key; `ComponentId` == type identity).
    pub comp: ComponentId,
    /// The first bound field id (resolved from the field name at parse time).
    pub field: u8,
    /// The second field id, or [`NO_FIELD`] when unused.
    pub field2: u8,
    /// The format template.
    pub template: TemplateId,
}

/// Binds a normalized `f32` into this widget's [`UiValue`] (Decision 8). 16 B.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BindValue {
    /// The source entity hosting the bound component.
    pub source: Entity,
    /// The source component id (the accessor key).
    pub comp: ComponentId,
    /// The numerator field id.
    pub num_field: u8,
    /// The denominator field id, or [`NO_FIELD`] for a raw value (Decision 8).
    pub den_field: u8,
}

/// Inline render-facing text buffer — the [`BindText`] SINK. Mirrors the
/// `UiName` inline-buffer + `core::fmt::Write` pattern (alloc-free, POD `Copy`).
///
/// Tick-bearing so the P5 text-upload system is `Changed<UiTextBuffer>`-gated;
/// the bind system writes it set-if-changed so a steady value bumps no tick.
/// `#[repr(C, align(64))]`, 256 B (one stride of 4 cache lines).
#[repr(C, align(64))]
#[derive(Component, Clone, Copy)]
pub struct UiTextBuffer {
    /// UTF-8 bytes; only `bytes[..len]` are meaningful, the rest are zero.
    bytes: [u8; Self::CAP],
    /// Valid byte count in `bytes` (`<= CAP`).
    len: u8,
    /// Pad to 256 B total.
    _pad: [u8; 8],
}

impl UiTextBuffer {
    /// Maximum text length in bytes; keeps the struct at 256 B
    /// (`247 + 1 + 8 = 256`).
    pub const CAP: usize = 247;

    /// The formatted text as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        debug_assert!(self.len as usize <= Self::CAP, "invariant: UiTextBuffer len exceeds CAP");
        // SAFETY: the only writer is the `core::fmt::Write` impl below, which
        // appends bytes from `&str` arguments (always valid UTF-8) and truncates
        // at a char boundary on overflow, so `bytes[..len]` is always a valid
        // UTF-8 prefix and `len <= CAP`.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }

    /// The text length in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Clears the buffer (length to zero; bytes left as-is, never read past len).
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl Default for UiTextBuffer {
    #[inline]
    fn default() -> Self {
        Self {
            bytes: [0u8; Self::CAP],
            len: 0,
            _pad: [0u8; 8],
        }
    }
}

// Bit-identical equality over the live prefix only (the trailing bytes are not
// zeroed on `clear`/truncate, so a full-buffer derive would be wrong). This is
// the set-if-changed key the bind system compares against to keep the sink tick
// quiet on identical text.
impl PartialEq for UiTextBuffer {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.bytes[..self.len as usize] == other.bytes[..other.len as usize]
    }
}
impl Eq for UiTextBuffer {}

impl core::fmt::Write for UiTextBuffer {
    /// Appends `s`, saturating at [`UiTextBuffer::CAP`] on a UTF-8 char boundary
    /// (a truncated write never splits a multi-byte char). Alloc-free.
    ///
    /// Copies WHOLE `char`s only: each `char`'s UTF-8 length is known before it is
    /// written, so the buffer is grown only when the entire encoding fits. A
    /// `char` that would cross `CAP` ends the copy, leaving `bytes[..len]` a valid
    /// UTF-8 prefix BY CONSTRUCTION (never a split/incomplete sequence) — the
    /// [`UiTextBuffer::as_str`] `from_utf8_unchecked` precondition.
    #[inline]
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut len = self.len as usize;
        for ch in s.chars() {
            let n = ch.len_utf8();
            if len + n > Self::CAP {
                // This char (and therefore the rest of `s`) does not fit; stop on
                // the current boundary rather than splitting it.
                break;
            }
            ch.encode_utf8(&mut self.bytes[len..len + n]);
            len += n;
        }
        self.len = len as u8;
        Ok(())
    }
}

/// Normalized bound scalar SINK (health bars, progress, sliders). 4 B
/// (Decision 8). `#[repr(transparent)]`.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct UiValue(pub f32);
