//! Profiling rung 7 — the host-side measurement channel that replaces stdout.
//!
//! Rung 7 is specified as *"the single subtractive rung"*, and measuring it showed it cannot be:
//! it must migrate six stdout consumers **to the artifact**, and no artifact writer existed. The
//! rung's own row named a channel that had never been built, and the corpus assigned the writer to
//! rung 8 in exactly one line while four other lines put it here. That is resolved in
//! `docs/diagnostics/profiling/05-LADDER-GATES.md`; this module is the channel.
//!
//! [`artifact`] is the file itself — format, writer, reader, and the header refusal `G24`'s reverse
//! RED asserts. It is deliberately the whole of rung 7's build half: the reducer that fills it and
//! the deletion of the printed lines are the halves that follow, and each needs this one to exist
//! before it can be verified against anything.
//!
//! [`contrast`] is rung 8's half: the `Floor`, the `Twin`, and the `resolve` that turns two
//! artifacts into a verdict or a stated refusal. It lives here rather than in `boyko_ecs` — where
//! the corpus placed it — for the reason rung 7 moved the reducer and the artifact here: it reads a
//! header whose `workload_tag` is derived from `boyko_render::ResolvedRenderPath`, a type the
//! kernel cannot see and must not learn to.
//!
//! [`correlate`] is rung 9's half: the rejection sampler that turns `cpu_gpu_offset` from a word
//! into a number with a measured bound. It lives beside [`contrast`] for the same reason — it
//! needs the artifact header, and the kernel cannot see one.
//!
//! [`stream`] is rung 13's half: the telemetry writer -- the window reduction, the block encode and
//! the one `write_all` per window. The FORMAT it writes lives in `boyko_diag::telemetry`, hosted
//! there for a measured graph reason: a `prof_decode` rooted at `boyko_diag` builds one crate and
//! one rooted here builds forty-four.
//!
//! [`alloc_shim`] is rung 8's opt-in allocation counter — the one place the profiler installs a
//! `#[global_allocator]`, and only when `profiling-alloc` asks. Its module doc states what a
//! process-wide counter cannot claim.
//!
//! Compiled unconditionally rather than behind `profiling-analysis`. Rung 5c measured why: features
//! unify per PACKAGE, and a `#[cfg]` on a type that another crate's construction site names makes
//! the workspace stop compiling for a reason no crate's source shows. What the feature gates is the
//! *cost* — the counter increments at the recorders' record sites — and a type nobody constructs
//! costs nothing.

pub mod alloc_shim;
pub mod artifact;
pub mod contrast;
pub mod correlate;
pub mod reduce;
pub mod stream;
