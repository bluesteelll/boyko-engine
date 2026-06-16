//! Save-direction error type (Phase S1).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.10 (public API). The loader's
//! `LoadError` is a separate type that lands with S2 (`load.rs`); S1 ships only
//! the save side.

use std::fmt;
use std::io;

/// A failure during [`save_world`](crate::save_world) /
/// [`save_world_to_file`](crate::save_world_to_file).
///
/// Save is mostly infallible (it reads a live, already-valid world), so the
/// variants cover only the genuinely-fallible steps: arithmetic that would
/// overflow the address space while computing file offsets (a world larger than
/// `usize`), and I/O when writing to a file.
#[derive(Debug)]
#[non_exhaustive]
pub enum SaveError {
    /// Computing the file layout overflowed `usize` (a single archetype/column
    /// whose `count * stride` or whose accumulated offset cannot be represented).
    /// Practically unreachable on a 64-bit target, but checked rather than
    /// silently wrapping into a corrupt offset.
    SizeOverflow,
    /// Writing the serialized bytes to a file failed. Carries the underlying
    /// [`io::Error`]. Only produced by
    /// [`save_world_to_file`](crate::save_world_to_file).
    Io(io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::SizeOverflow => write!(
                f,
                "save: file-offset computation overflowed usize (world too large to serialize)"
            ),
            SaveError::Io(e) => write!(f, "save: I/O error writing the snapshot: {e}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveError::Io(e) => Some(e),
            SaveError::SizeOverflow => None,
        }
    }
}

impl From<io::Error> for SaveError {
    #[inline]
    fn from(e: io::Error) -> Self {
        SaveError::Io(e)
    }
}
