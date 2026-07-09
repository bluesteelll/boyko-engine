//! The `.keys` human-editable persistence format (plan §9).
//!
//! A line-based, in-house text format — **no** serde/toml/ron (`boyko_serialize`
//! has no resource path, and keybinds are user-editable/shareable, so binary
//! codegen is the wrong tool; plan §9.3). The parser and serializer here are the
//! sole readers/writers of the format.
//!
//! - [`grammar`] — the one-pass hand parser ([`load_keys`], [`ParseReport`]).
//! - [`writer`] — the canonical serializer ([`save_keys`], [`keys_to_string`]).
//! - [`keyname`] — the canonical name ↔ [`KeyCode`](crate::raw::keycode::KeyCode)
//!   tables shared by both.
//!
//! Both directions are **cold, load-time** paths and may allocate; the per-frame
//! input path never touches this module.
//!
//! The canonical form is a fixed point of the parser — `parse ∘ serialize` is
//! byte-identical on canonical output (plan §9.3 round-trip).

pub mod grammar;
pub mod keyname;
pub mod writer;

pub use grammar::{load_keys, ParseReport, KEYS_FORMAT_VERSION};
pub use writer::{keys_to_string, save_keys};
