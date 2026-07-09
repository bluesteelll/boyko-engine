//! Shared type-erased element buffer for the `#[event]` attribute path.
//!
//! The `parameters` and `participants` channels of an event each need a
//! type-erased byte buffer that stores a run of identically-typed `Copy`
//! elements (one per dispatched event) and hands them back on demand. The two
//! channels differ only in the element trait they accept (`Parameters` vs
//! `Participants`) and in the names used in diagnostics; the storage logic is
//! identical. This module holds that single logic as [`ErasedKindBuffer<K>`],
//! parameterised by a zero-sized [`BufferKind`] marker.
//!
//! The public per-channel names — [`ParametersBuffer`] and [`ParticipantBuffer`]
//! — are preserved as type aliases so existing paths keep compiling without any
//! change to the `#[event]` macro.

use std::any::TypeId;
use std::marker::PhantomData;
use std::mem::MaybeUninit;

use crate::ecs::core::events::event::EventId;
use crate::ecs::core::events::parameters::parameters::Parameters;
use crate::ecs::core::events::participants::participants::Participants;

/// Zero-sized marker selecting which event channel a buffer serves.
///
/// A `BufferKind` supplies the human-readable names used in the debug-only
/// type-confusion assertions. Each marker is paired with the element trait it
/// accepts through a [`KindOf`] impl.
pub trait BufferKind: 'static {
    /// Name of the concrete buffer type, used in assertion messages
    /// (e.g. `"ParametersBuffer"`).
    const BUFFER_NAME: &'static str;

    /// Name of the element trait, used in assertion messages
    /// (e.g. `"Parameters"`).
    const ELEMENT_NAME: &'static str;
}

/// Bridges a [`BufferKind`] marker to the concrete element type `P` it accepts.
///
/// The two impls target **different** marker types, so their blanket bounds
/// (`P: Parameters` and `P: Participants`) never overlap — coherence is
/// satisfied while each channel keeps its original element trait bound.
///
/// # Safety
/// Implementers guarantee that `element_size()` returns exactly
/// `size_of::<P>()`, so the raw byte copies in [`ErasedKindBuffer::push`] and
/// [`ErasedKindBuffer::get`] address the correct number of bytes.
pub unsafe trait KindOf<P: 'static + Copy>: BufferKind {
    /// Size in bytes of a single `P` element, as reported by its layout.
    fn element_size() -> usize;
}

/// Marker for the event **parameters** channel.
pub struct ParametersKind;

/// Marker for the event **participants** channel.
pub struct ParticipantsKind;

impl BufferKind for ParametersKind {
    const BUFFER_NAME: &'static str = "ParametersBuffer";
    const ELEMENT_NAME: &'static str = "Parameters";
}

impl BufferKind for ParticipantsKind {
    const BUFFER_NAME: &'static str = "ParticipantBuffer";
    const ELEMENT_NAME: &'static str = "Participants";
}

// SAFETY: `P::layout()` is the trait default `Layout::new::<P>()`, whose `size()`
// equals `size_of::<P>()`; the two blanket impls target distinct marker types so
// they cannot overlap.
unsafe impl<P: Parameters> KindOf<P> for ParametersKind {
    #[inline]
    fn element_size() -> usize {
        P::layout().size()
    }
}

// SAFETY: same invariant as the `ParametersKind` impl above; `Participants` also
// defaults `layout()` to `Layout::new::<P>()`.
unsafe impl<P: Participants> KindOf<P> for ParticipantsKind {
    #[inline]
    fn element_size() -> usize {
        P::layout().size()
    }
}

/// Type-erased buffer storing a run of identically-typed `Copy` elements for one
/// event channel `K`.
///
/// The buffer carries a [`TypeId`] recording the concrete `P` used at
/// construction. Every typed read (`get`) and write (`push`) asserts, in debug
/// builds, that the caller passes the same `P`, catching bugs in
/// macro-generated glue or test helpers that bypass the `#[event]` macro. In
/// release builds the assertion compiles away; the invariant is upheld by the
/// macro, which always ties a buffer to a single concrete element type.
pub struct ErasedKindBuffer<K: BufferKind> {
    /// Event ID this buffer is for.
    event_id: EventId,

    /// `TypeId` of the concrete element type this buffer was constructed for.
    /// Used in debug builds to detect type confusion (Q-019).
    type_id: TypeId,

    /// Raw data storage. `MaybeUninit<u8>` makes padding-byte writes sound
    /// under Miri — the buffer never interprets the bytes as initialized `u8`.
    data: Vec<MaybeUninit<u8>>,

    /// Number of element sets stored.
    count: usize,

    /// Size of each element set in bytes.
    element_size: usize,

    /// Zero-sized channel marker; carries no runtime data.
    _kind: PhantomData<K>,
}

impl<K: BufferKind> ErasedKindBuffer<K> {
    /// Creates a new buffer for the concrete element type `P`.
    ///
    /// The `TypeId` of `P` is recorded and checked on every `get`/`push` call in
    /// debug builds, defending against type confusion when buffers are accessed
    /// through type-erased code paths.
    pub fn new<P>(event_id: EventId) -> Self
    where
        K: KindOf<P>,
        P: 'static + Copy,
    {
        Self {
            event_id,
            type_id: TypeId::of::<P>(),
            data: Vec::new(),
            count: 0,
            element_size: K::element_size(),
            _kind: PhantomData,
        }
    }

    /// Creates a new buffer with pre-allocated capacity for `capacity` elements.
    pub fn with_capacity<P>(event_id: EventId, capacity: usize) -> Self
    where
        K: KindOf<P>,
        P: 'static + Copy,
    {
        let element_size = K::element_size();
        Self {
            event_id,
            type_id: TypeId::of::<P>(),
            data: Vec::with_capacity(capacity * element_size),
            count: 0,
            element_size,
            _kind: PhantomData,
        }
    }

    /// Adds an element to the buffer. Returns the index of the inserted entry.
    pub fn push<P>(&mut self, element: &P) -> usize
    where
        K: KindOf<P>,
        P: 'static + Copy,
    {
        debug_assert_eq!(
            TypeId::of::<P>(),
            self.type_id,
            "{}::push called with wrong {} type (Q-019)",
            K::BUFFER_NAME,
            K::ELEMENT_NAME,
        );
        debug_assert_eq!(std::mem::size_of::<P>(), self.element_size);
        let index = self.count;
        let old_len = self.data.len();
        self.data
            .resize(old_len + self.element_size, MaybeUninit::uninit());
        // SAFETY:
        // (1) `element` is a valid `&P` for `size_of::<P>()` bytes (borrowed reference).
        // (2) The destination is the freshly-reserved tail region of `self.data` with exactly
        //     `element_size` bytes (the `resize` above extended it); `element_size == size_of::<P>()`
        //     by the `KindOf` safety invariant, re-checked by the debug_assert above.
        // (3) `MaybeUninit<u8>` accepts any byte pattern including padding, so copying
        //     padding bytes from `*element` is sound.
        // (4) Regions cannot overlap: `element` is borrowed externally, the destination
        //     is inside the internal Vec allocation.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (element as *const P).cast::<u8>(),
                self.data.as_mut_ptr().add(old_len).cast::<u8>(),
                self.element_size,
            );
        }
        self.count += 1;
        index
    }

    /// Gets the element at the given index.
    ///
    /// # Safety
    /// `P` must be the exact type supplied when the buffer was constructed (i.e.
    /// the same `P` passed to [`new`](Self::new) or
    /// [`with_capacity`](Self::with_capacity)). In debug builds a [`TypeId`]
    /// assertion enforces this; in release builds the invariant is upheld by the
    /// `#[event]` macro, which always binds the buffer to a single concrete type.
    pub unsafe fn get<P>(&self, index: usize) -> Option<P>
    where
        K: KindOf<P>,
        P: 'static + Copy,
    {
        debug_assert_eq!(
            TypeId::of::<P>(),
            self.type_id,
            "{}::get called with wrong {} type (Q-019)",
            K::BUFFER_NAME,
            K::ELEMENT_NAME,
        );
        if index >= self.count {
            return None;
        }
        debug_assert_eq!(std::mem::size_of::<P>(), self.element_size);
        let offset = index * self.element_size;
        // SAFETY: `offset + element_size <= self.data.len()` by construction —
        // `count` is incremented only after a successful `push` that grew `data`.
        // Caller's invariant ensures the bytes at this offset are a valid bit-pattern
        // for `P`. `read_unaligned` does not require alignment of the source pointer.
        unsafe {
            let src = self.data.as_ptr().add(offset).cast::<P>();
            Some(std::ptr::read_unaligned(src))
        }
    }

    /// Clears all elements.
    pub fn clear(&mut self) {
        self.data.clear();
        self.count = 0;
    }

    /// Returns the number of element sets stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` if the buffer contains no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the event ID this buffer belongs to.
    #[inline]
    pub fn event_id(&self) -> EventId {
        self.event_id
    }
}
