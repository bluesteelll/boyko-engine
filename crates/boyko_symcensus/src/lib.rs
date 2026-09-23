//! `boyko_symcensus` — UG-15's instrument (unification plan rung B3; 03 §6).
//!
//! A dev-only crate. It builds a subject binary under a named profile with `cargo rustc … --
//! --emit=obj`, pairs the post-LTO object with the image the linker wrote from it (or REDs), reads
//! both with the LLVM binutils, and normalises what it reads so that a rebuild is not a move.
//!
//! **Why the object.** On the msvc gate host `link.exe` writes no COFF symbol table into the image
//! and the PDB carries publics only, so a fat-LTO local is invisible in both; the post-LTO object
//! is the linker's input and carries every symbol (U-22;
//! `crates/profile_fixture/tests/profile_axis_census.rs`, SECOND FINDING). The two existing copies
//! of that instrument stay as they are; this crate is a third, general one.
//!
//! **RED, never skip.** An absent tool, an empty census, a missing object, an unpaired object, an
//! unset target dir or a host mismatch is a [`Red`](red::Red) with its own kind.
//!
//! The heavy legs and the probes run through the `ug15` binary (`src/bin/ug15.rs`), invoked
//! directly after `cargo build -p boyko-symcensus --bin ug15`, so no outer cargo holds the build
//! lock while the nested `cargo rustc` runs in the same target dir.

pub mod candidates;
pub mod contain;
pub mod host;
pub mod json;
pub mod leg7b;
pub mod llvm;
pub mod maps;
pub mod normalize;
pub mod objbuild;
pub mod objview;
pub mod pe;
pub mod pins;
pub mod probe;
pub mod red;
pub mod seam;
pub mod sha256;
pub mod snapshot;
pub mod source;
pub mod tools;
