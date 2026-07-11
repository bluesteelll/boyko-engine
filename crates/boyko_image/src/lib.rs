//! In-house PNG decoder — RFC 1950 (zlib) + RFC 1951 (DEFLATE) decompression
//! and PNG container parsing, written from the spec text with **zero
//! third-party dependencies** (`std` only).
//!
//! A pure-CPU, `Send` leaf utility crate: it depends on nothing else in the
//! workspace and nothing else in the workspace depends on it yet, mirroring
//! `boyko_utils`'s decoupled role. The single entry point is [`decode_png`].
//!
//! # Scope
//!
//! Supported: color types 0 (grayscale), 2 (truecolor/RGB), 4
//! (grayscale+alpha), 6 (truecolor+alpha/RGBA); bit depths 8 and 16;
//! non-interlaced images. This covers every real PBR texture channel layout
//! (albedo, normal, roughness/metallic, AO, masks).
//!
//! Deliberately rejected (see [`PngError`]): color type 3 (palette / `PLTE`),
//! sub-byte bit depths (1/2/4), and Adam7 interlacing (interlace method 1).
//! The asset pipeline controls authoring — supporting the full PNG matrix is
//! a large surface for zero payoff here.
//!
//! # Example
//!
//! ```
//! use boyko_image::{decode_png, PngError};
//!
//! match decode_png(&[0u8; 4]) {
//!     Err(PngError::Signature) => {} // too short to even be a PNG
//!     _ => unreachable!(),
//! }
//! ```

mod error;
mod inflate;
mod png;

pub use error::PngError;
pub use png::{DecodedImage, decode_png};
