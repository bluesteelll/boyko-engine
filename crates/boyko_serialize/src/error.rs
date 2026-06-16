//! Save / load error types (Phase S1 + S2).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.10 (public API). [`SaveError`] is the
//! save side (S1); [`LoadError`] is the load side (S2) — a separate type because
//! the load path has a categorically richer failure surface (it ingests untrusted
//! bytes: bad magic, unsupported version, foreign endianness/ptr-width, a
//! truncated/corrupt stream, a layout-fingerprint mismatch on a blittable column).

use std::fmt;
use std::io;

use boyko_ecs::ecs::core::serialize::DecodeError;

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

/// A failure during [`load_world`](crate::load_world) /
/// [`load_world_from_file`](crate::load_world_from_file).
///
/// Load ingests **untrusted bytes** (plan §3.11 step 1 / C3), so every variant is
/// a RELEASE-level rejection (not a `debug_assert`): the loader never transmutes a
/// malformed stream into a value or blits a layout-incompatible column. On any
/// error the destination world is left CONSISTENT — a partially-loaded archetype is
/// rolled back to empty before the error returns (the W5 contract).
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// The file did not start with the `b"BOYKOSAV"` magic (not a boyko snapshot,
    /// or a truncated header).
    BadMagic,
    /// The on-disk `format_version` is not supported by this build. Carries the
    /// file's version.
    UnsupportedVersion(u32),
    /// The file's endianness marker is not the build's native endianness (v1 has
    /// no byteswap path — plan O2). Carries the file's marker byte.
    EndiannessMismatch(u8),
    /// The file's pointer width is not 8 (v1 supports only 64-bit — plan O2).
    /// Carries the file's `ptr_width`.
    PtrWidthMismatch(u8),
    /// A header / body offset, count, or region length is inconsistent with the
    /// byte slice (a truncated file, a region pointing past the end, or an
    /// arithmetic overflow). Carries a short static reason.
    Truncated(&'static str),
    /// A `PlainOldBytes` column's `layout_fingerprint` did not match the running
    /// type's, and no `deserialize_fn` is installed for the file's
    /// `format_version` (the C2 hard error — never a silent garbage blit). Carries
    /// the offending component's stable name.
    FingerprintMismatch(&'static str),
    /// A per-element `deserialize_fn` rejected a malformed/truncated column run
    /// (the C3 validate-on-read obligation). Carries the underlying
    /// [`DecodeError`]. The partially-loaded archetype was rolled back.
    Decode(DecodeError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::BadMagic => {
                write!(f, "load: bad magic (not a BOYKOSAV snapshot, or truncated header)")
            }
            LoadError::UnsupportedVersion(v) => {
                write!(f, "load: unsupported on-disk format version {v}")
            }
            LoadError::EndiannessMismatch(e) => write!(
                f,
                "load: endianness mismatch (file marker {e}, this build is native-endian only)"
            ),
            LoadError::PtrWidthMismatch(w) => {
                write!(f, "load: pointer-width mismatch (file {w}, this build requires 8)")
            }
            LoadError::Truncated(why) => write!(f, "load: malformed/truncated snapshot ({why})"),
            LoadError::FingerprintMismatch(name) => write!(
                f,
                "load: layout-fingerprint mismatch for blittable component '{name}' and no \
                 deserialize_fn for the file's format_version (bump format_version on a \
                 layout change)"
            ),
            LoadError::Decode(e) => write!(f, "load: malformed component stream ({e:?})"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl From<DecodeError> for LoadError {
    #[inline]
    fn from(e: DecodeError) -> Self {
        LoadError::Decode(e)
    }
}
