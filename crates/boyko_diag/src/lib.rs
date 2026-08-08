//! `boyko_diag` — the diagnostics substrate: one clock, one lane topology, one loss
//! vocabulary, one never-freed-storage policy.
//!
//! # Why this crate exists at all
//!
//! It **removes** four duplications rather than adding a capability. Two planned subsystems —
//! the profiler and `boyko_log` — each need a per-thread lane index, an `rdtsc` calibration, a
//! never-freeing lane allocator and a loss accounting. Designed separately, they invented all
//! four independently and with incompatible semantics. The consequence of *not* sharing each
//! one, in a line apiece:
//!
//! | Primitive | If each subsystem keeps its own copy |
//! |---|---|
//! | [`clock`] | A suspend/resume produces a profiler window quarantined as an epoch break and, in the same seconds, log lines whose printed wall times are wrong by the suspend duration with no marker — two artifacts that disagree, neither saying why. |
//! | [`lane`] | The same worker is lane 5 to the profiler and lane 37 to the logger, so no reader can place a log line inside the zone it happened in — the one joint question the pair exists to answer becomes unanswerable by construction. |
//! | [`loss`] | The profiler reports its own drops *through* the logger, so under load — precisely when profiler drops occur — the report of the loss is itself dropped and counted as a *logger* loss. Two counters double-count one event and no rule names the authority. |
//! | [`storage`] | Two residency proofs over two statics, with two demand-zero arguments, for one toolchain behaviour — so a toolchain change reds one gate and not the other, and a reader cannot tell which is authoritative. |
//!
//! **The whole value of this crate is that it is small.** A shared bottom crate that accretes is
//! the same Principle-0 defect as two subsystems each minting their own copy, pointed the other
//! way. A thing enters here only if **both** subsystems *write* it **and** a disagreement
//! between two copies would be observable in an artifact a reader joins. Anything one writes and
//! the other only reads stays with the writer, behind a getter.
//!
//! # The mute-leaf rule
//!
//! **This crate emits no diagnostic of its own.** No `boyko-####` code, no print, no panic hook,
//! no file, no thread, no `core::fmt`. It exposes counters and status values; whoever sits above
//! it reads them and emits the codes.
//!
//! That is not tidiness — it is what keeps the dependency graph acyclic. A leaf that needed a
//! diagnostic channel would have to depend on the logger, and the logger depends on this crate.
//! The cost is real and is stated rather than hidden: a condition observed here is reported at
//! the consumer's next fold, **not** at the instant it occurs. [`loss::raise`] and
//! [`loss::take_raised`] are the mechanism.
//!
//! The three `deny`s below are the mechanical half of that rule; the rest of it — no
//! `std::process`, no `std::fs`, no `thread::spawn` — is gated by `DG9` and by
//! [`storage::section_report`] living behind the `section-gate` feature.
//!
//! # No boot work
//!
//! Nothing here is touched, calibrated, spawned or committed at process start. [`clock`] is not
//! calibrated from a static initialiser; no lane buffer is written; no spare slot is claimed; no
//! session id is minted eagerly. Every one-time cost runs on the **enable path**, which runs at
//! launch, before the game loop — so a process that never enables diagnostics never faults in a
//! page of this crate's `.bss`, and the reserved extent costs address space rather than resident
//! memory.

#![deny(clippy::print_stdout)]
#![deny(clippy::print_stderr)]
#![deny(clippy::dbg_macro)]

pub mod clock;
pub mod lane;
pub mod loss;
pub mod storage;
