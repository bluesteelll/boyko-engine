//! A4 — the never-freed storage policy, the `.bss`-resident cell array it is spelled with, and
//! the one section/symbol probe that checks it.
//!
//! # The policy, stated once
//!
//! > **Extent known at compile time ⇒ `.bss` static. Extent chosen at run time from config ⇒
//! > `VmReservation`, which requires the owner to sit at or above `boyko_ecs`.**
//!
//! It is stated **here, once, as a rule** — not in each subsystem plan as a per-plan exception
//! plea. A rule that each subsystem restates as its own special case is a rule that two
//! subsystems can state differently, and then a reader cannot tell which statement is
//! authoritative.
//!
//! # The boundary at `boyko_ecs` is forced, not chosen
//!
//! `VmReservation` cannot move down here. It is `pub(crate)` in `boyko_ecs`
//! (`ecs/memory/vm.rs:85`, `:109`, `:199`), it has a `Drop` (`:263`), and its unix arm calls
//! `libc::mmap` (`:149`). A std-only zero-dependency crate could host it only by taking a
//! third-party dependency — forbidden here — or by minting a **second** hand-written per-OS
//! backing implementation against that file's own single-source-of-truth clause (`:12-18`).
//! Inventing memory backing twice is a worse Principle-0 breach than the one this crate exists
//! to fix, so the run-time-extent arm of the policy simply does not live at this layer.
//!
//! # What the `.bss` argument proves, and what it does not
//!
//! A `static X: SyncCells<T, N> = SyncCells::zeroed()` whose initialiser is all zeroes is emitted
//! by the linker with a virtual size and **no raw data** — `.bss` on ELF, and on PE/COFF a
//! section whose `RawDataSize` is 0 while `VirtualSize` is non-zero. That much is mechanically
//! checkable, and [`section_report`] checks it.
//!
//! **What is UNPROVEN and is not claimed anywhere in this corpus: that the OS leaves those pages
//! uncommitted until touched.** The image tells us the bytes are not in the file; it does not
//! tell us the loader is lazy. This limit travels with every "resident ≈ 0" figure anyone states
//! about this crate, and it is stated here once so no other document has to re-derive it.

use core::cell::UnsafeCell;
use core::sync::atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
    AtomicU32, AtomicU64, AtomicUsize,
};

// ---------------------------------------------------------------------------------------------
// ZeroInit
// ---------------------------------------------------------------------------------------------

/// Marks a type for which the all-zero bit pattern is a valid, fully initialised value.
///
/// This is the crate's own marker because `bytemuck::Zeroable` is third-party and this crate has
/// an empty `[dependencies]` table by design. It is the half of the storage policy that Rust can
/// actually express; the other half — "the extent is a compile-time constant" — needs no marker
/// because array lengths are const *by construction*, so there is no broken input to reject.
///
/// # Safety
///
/// An implementor asserts that a `Self` whose every byte is `0` is a valid value that upholds
/// every invariant of the type. Consequently a type must **not** implement this trait if it
///
/// - has a `Drop` impl (a zeroed static is never dropped, and pretending otherwise hides a leak
///   behind a marker),
/// - has a niche that excludes zero (`NonZeroU32`, `&T`, `Box<T>`, `fn()`, most enums), or
/// - holds any field that is not itself `ZeroInit`.
pub unsafe trait ZeroInit {}

/// Implements [`ZeroInit`] for a list of types whose all-zero pattern is valid by definition.
macro_rules! impl_zero_init {
    ($($t:ty),+ $(,)?) => {
        $(
            // SAFETY: every listed type is a primitive integer, float, `bool`, or the atomic
            // wrapper around one. Each has no niche, no `Drop`, and no validity invariant beyond
            // its bit pattern: `0` is `0`, `0.0`, and `false` respectively, and the atomic
            // wrappers are `#[repr(transparent)]`-equivalent over their primitive with identical
            // validity. `()` is a zero-sized type with exactly one value.
            unsafe impl ZeroInit for $t {}
        )+
    };
}

impl_zero_init!(
    (),
    bool,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64,
    AtomicBool,
    AtomicU8,
    AtomicU16,
    AtomicU32,
    AtomicU64,
    AtomicUsize,
    AtomicI8,
    AtomicI16,
    AtomicI32,
    AtomicI64,
    AtomicIsize,
);

// SAFETY: an array is valid exactly when every element is, it adds no niche and no `Drop` of its
// own, and its layout is `N` contiguous `T`s with no padding between them. So all-zero bytes over
// `[T; N]` decode to `N` all-zero `T`s, each valid by `T: ZeroInit`.
unsafe impl<T: ZeroInit, const N: usize> ZeroInit for [T; N] {}

// SAFETY: `UnsafeCell<T>` is `#[repr(transparent)]` over `T` and imposes no validity invariant
// of its own — it only removes the compiler's no-aliasing-through-shared-reference guarantee.
unsafe impl<T: ZeroInit> ZeroInit for UnsafeCell<T> {}

/// Reports whether `T` may back a zero-initialised `.bss` static under the storage policy.
///
/// The answer is always `true` — **the assertion is the `T: ZeroInit` bound**, which is checked
/// at the call site, so the interesting outcome is a compile error rather than a `false`. It is
/// written to be used as a const gate:
///
/// ```
/// # use boyko_diag::storage::assert_zero_init_eligible;
/// const _: () = assert!(assert_zero_init_eligible::<u64>());
/// ```
///
/// The compile-fail counterpart cannot be a `#[test]`: a `#[test]` that fails to compile fails
/// the whole test binary's build, so it can never be a *passing* gate. It is a `trybuild`
/// `compile_fail` case, and — because [`SyncCells`] carries no bound on the struct itself — that
/// case must declare a **static**, not merely a type:
///
/// ```ignore
/// static BAD: SyncCells<core::num::NonZeroU32, 4> = SyncCells::zeroed(); // ZeroInit unsatisfied
/// ```
///
/// A case that only writes `type Bad = SyncCells<NonZeroU32, 4>;` compiles, and is a gate that
/// cannot fail.
#[must_use]
#[inline]
pub const fn assert_zero_init_eligible<T: ZeroInit>() -> bool {
    true
}

// ---------------------------------------------------------------------------------------------
// SyncCells
// ---------------------------------------------------------------------------------------------

/// A fixed-extent, never-freed array of cells that is written through raw pointers only.
///
/// This is the shape every `.bss` table in this crate uses: the lane loss cells, the spare-owner
/// words, the clock state. It exists because the alternative spellings are each wrong here — a
/// `RefCell` is banned workspace-wide (runtime borrow flags on an engine path), and an array of
/// atomics forces every field of every cell to be atomic even where the lane topology already
/// guarantees a single writer. `UnsafeCell` plus a documented `// SAFETY:` is the alternative the
/// ban's own reason line prescribes.
///
/// It hands out **no** `&T` and no `&mut T`. Every access goes through [`SyncCells::get_ptr`],
/// which carries the aliasing obligation explicitly.
#[repr(transparent)]
pub struct SyncCells<T, const N: usize>(UnsafeCell<[T; N]>);

// SAFETY: `SyncCells` grants no `&T` and no `&mut T` — the only accessor is `get_ptr`, which
// returns a raw pointer and states the single-writer obligation as its own safety contract, to
// be discharged by its caller. The type therefore adds no aliasing permission beyond what a raw
// pointer already permits, which is exactly what `Sync` asserts here. In this crate the
// obligation is discharged by the lane topology (A2): index `i` is written only by the thread
// whose `lane() == i`, and a lane is owned by at most one thread at a time via the
// `release_lane` Release / `claim_lane` Acquire pairing. `T: Send` is required because sharing
// `&SyncCells<T, N>` lets a `T` be produced and consumed on different threads. IF THE LANE
// TOPOLOGY EVER STOPS BEING SINGLE-OWNER, THIS IMPL STOPS BEING SOUND; the two facts move
// together or not at all.
unsafe impl<T: Send, const N: usize> Sync for SyncCells<T, N> {}

impl<T, const N: usize> SyncCells<T, N> {
    /// Builds the all-zero table, in `const` context, for a `static` initialiser.
    ///
    /// The `T: ZeroInit` bound is load-bearing rather than decorative: it is the premise of the
    /// `unsafe` below, and it is what makes the storage policy's compile-time half checkable
    /// ([`assert_zero_init_eligible`]).
    #[must_use]
    pub const fn zeroed() -> Self
    where
        T: ZeroInit,
    {
        // SAFETY: `T: ZeroInit` is the implementor's assertion that the all-zero bit pattern is a
        // valid, fully initialised `T`; `[T; N]: ZeroInit` follows from the array impl above, and
        // an array's layout is `N` contiguous `T`s so no padding byte is left with a meaning.
        // `mem::zeroed` therefore produces a valid `[T; N]` here and not merely plausible bytes.
        Self(UnsafeCell::new(unsafe { core::mem::zeroed() }))
    }

    /// Builds a table from an explicit array.
    ///
    /// **A static built this way from a non-all-zero array leaves `.bss` and gains raw data in
    /// the image**, which violates the storage policy — and which is precisely the showable RED
    /// for the residency gate, so the constructor exists rather than being denied.
    #[must_use]
    #[inline]
    pub const fn from_array(cells: [T; N]) -> Self {
        Self(UnsafeCell::new(cells))
    }

    /// Returns a raw pointer to cell `i`.
    ///
    /// # Safety
    ///
    /// The caller guarantees that
    ///
    /// 1. `i < N`, and
    /// 2. for the whole lifetime of the returned pointer, no other thread writes index `i`.
    ///
    /// In this crate obligation (2) is discharged **by the lane topology and by nothing else**:
    /// index `i` is written only by the thread whose `lane() == i`, and a lane is owned by at
    /// most one thread at a time. A caller outside that topology must supply its own argument.
    ///
    /// Note that a *shared* read of index `i` from another thread is not covered by (2) and is
    /// only sound when `T`'s own fields are atomics — which is why the loss cells are spelled
    /// with `AtomicU64` even though the writer is unique.
    #[must_use]
    #[inline]
    pub unsafe fn get_ptr(&self, i: usize) -> *mut T {
        debug_assert!(i < N, "invariant: SyncCells::get_ptr index is in bounds");
        // SAFETY: `UnsafeCell::get` yields a valid pointer to the whole `[T; N]`, and casting an
        // array pointer to a pointer to its first element is the documented layout of arrays.
        // `add(i)` stays inside that same allocation because the caller guarantees `i < N`, so
        // the offset is in bounds and cannot wrap the address space.
        unsafe { self.0.get().cast::<T>().add(i) }
    }

    /// The number of cells, which is a compile-time constant by construction.
    #[must_use]
    #[inline]
    pub const fn len(&self) -> usize {
        N
    }

    /// Whether the table has no cells. A zero-extent table is a configuration mistake, not a
    /// state, so this exists for the lint and for assertions.
    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        N == 0
    }
}

// ---------------------------------------------------------------------------------------------
// section-gate: the ONE image probe, feature-gated off by default
// ---------------------------------------------------------------------------------------------
//
// Everything below spawns a process and reads a file, which the mute-leaf rule forbids this crate
// to contain by default. It is therefore behind `section-gate`, declared `default = []` and
// switched on by each consumer's dev-dependency — the same shape `boyko_rhi_vulkan`'s `goldens`
// and `boyko_render`'s `test-readback` already use. A default build compiles no `std::process`
// and no `std::fs` reference at all, and that property comes from the `#[cfg]` below, NOT from
// any byte scan: the mute-leaf grep gate excludes this file by path so that it can be honest
// about the gated half, so a `std::fs` use added to the *ungated* part of this file falls outside
// every leg of it. Reviewing this one file is a human obligation the gate does not discharge.

#[cfg(feature = "section-gate")]
mod gate {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Why a probe could not produce an answer. Every variant is a **RED**, never a skip.
    ///
    /// A gate that skips when its tool is missing is green on every machine that lacks the tool,
    /// and this campaign has caught that exact vacuity repeatedly. The predicates on
    /// [`SectionReport`] and [`SymbolReport`] therefore answer `false` for every failure, so a
    /// caller that only asserts the predicate still fails rather than passing quietly.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum ProbeFailure {
        /// Neither `PATH` nor the rustup toolchain's `rustlib` bin holds the tool.
        ToolNotFound,
        /// `std::env::current_exe()` did not yield a path to inspect.
        ImageUnavailable,
        /// The tool was found but could not be executed.
        ToolSpawnFailed,
        /// The tool ran and exited non-zero.
        ToolExitedNonZero,
        /// The tool's stdout was not UTF-8.
        ToolOutputNotUtf8,
        /// The expected field was present but its value did not parse as an integer.
        FieldUnparsable,
    }

    /// The result of asking the image about one section.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum SectionReport {
        /// The section exists, with these two sizes.
        Found {
            /// Bytes the section occupies once mapped.
            virtual_size: u64,
            /// Bytes the section occupies in the file. `0` beside a non-zero `virtual_size` is
            /// the "carries a size with no raw data" property every `.bss` gate asserts.
            raw_data_size: u64,
        },
        /// The tool ran and the image has no section by that name.
        NoSuchSection,
        /// The probe could not run.
        Failed(ProbeFailure),
    }

    impl SectionReport {
        /// The section's mapped size, if the probe succeeded and found it.
        #[must_use]
        #[inline]
        pub fn virtual_size(self) -> Option<u64> {
            match self {
                Self::Found { virtual_size, .. } => Some(virtual_size),
                _ => None,
            }
        }

        /// The section's in-file size, if the probe succeeded and found it.
        #[must_use]
        #[inline]
        pub fn raw_data_size(self) -> Option<u64> {
            match self {
                Self::Found { raw_data_size, .. } => Some(raw_data_size),
                _ => None,
            }
        }

        /// Whether the section carries a size with no raw data — the checkable half of the
        /// residency argument. **A failed probe answers `false`**, per the RED-not-SKIP rule.
        ///
        /// This does **not** claim the loader leaves the pages uncommitted; see the module docs.
        #[must_use]
        #[inline]
        pub fn is_size_without_raw_data(self) -> bool {
            matches!(
                self,
                Self::Found {
                    virtual_size,
                    raw_data_size: 0
                } if virtual_size > 0
            )
        }

        /// Why the probe failed, if it did. Callers report this so a RED names its cause.
        #[must_use]
        #[inline]
        pub fn failure(self) -> Option<ProbeFailure> {
            match self {
                Self::Failed(f) => Some(f),
                _ => None,
            }
        }
    }

    /// The result of asking the image which section class one linker symbol landed in.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum SymbolReport {
        /// The symbol exists, with this `nm` class letter (`b`/`B` = uninitialised data).
        Found {
            /// The single-character class letter `nm` printed for the symbol.
            class: u8,
        },
        /// The tool ran and the image has no symbol by that name.
        NoSuchSymbol,
        /// The probe could not run.
        Failed(ProbeFailure),
    }

    impl SymbolReport {
        /// Whether the symbol landed in uninitialised data. **A failed probe answers `false`.**
        #[must_use]
        #[inline]
        pub fn is_uninitialized_data(self) -> bool {
            matches!(self, Self::Found { class } if class == b'b' || class == b'B')
        }

        /// The raw `nm` class letter, if the probe succeeded and found the symbol.
        #[must_use]
        #[inline]
        pub fn class(self) -> Option<u8> {
            match self {
                Self::Found { class } => Some(class),
                _ => None,
            }
        }

        /// Why the probe failed, if it did.
        #[must_use]
        #[inline]
        pub fn failure(self) -> Option<ProbeFailure> {
            match self {
                Self::Failed(f) => Some(f),
                _ => None,
            }
        }
    }

    /// Reports the named section of the **current executable** — under a test runner, the test
    /// binary.
    ///
    /// This is the ONE implementation of the section probe for the substrate, profiling and
    /// logging `.bss` gates, so a toolchain change reds one gate rather than splitting two that
    /// disagree about which is authoritative.
    ///
    /// **Measured limit, stated rather than guessed at:** it parses the PE/COFF field names
    /// `llvm-readobj --sections` prints on this target (`Name:`, `VirtualSize:`,
    /// `RawDataSize:`). ELF output names the same quantities `Size:` and `Type: SHT_NOBITS`, and
    /// that shape is **not** implemented — on ELF this returns [`SectionReport::NoSuchSection`]
    /// or [`ProbeFailure::FieldUnparsable`], i.e. a RED, rather than a silent pass.
    #[must_use]
    pub fn section_report(section: &str) -> SectionReport {
        let Some(tool) = resolve_tool("llvm-readobj") else {
            return SectionReport::Failed(ProbeFailure::ToolNotFound);
        };
        match run_tool(&tool, &["--sections"]) {
            Ok(text) => parse_sections(&text, section),
            Err(f) => SectionReport::Failed(f),
        }
    }

    /// Reports which section class a **linker symbol** landed in, in the current executable.
    ///
    /// # Why this exists beside [`section_report`]
    ///
    /// MEASURED this session, and it is the difference between a gate that can fail and one that
    /// cannot: a `.bss` with `RawDataSize: 0` is present in **every** Rust binary on this target
    /// whether or not any of this crate's statics survived, and LLVM does delete a private static
    /// that is only read at constant indices — a 40 KiB table moved `.bss` `VirtualSize` from
    /// `0x240` to `0xA240` only once it was forced to be kept. A residency gate phrased purely
    /// over the section table is therefore green on a binary that contains none of the statics it
    /// claims to be measuring. Naming the symbol is what makes it fail.
    #[must_use]
    pub fn symbol_report(sym: &str) -> SymbolReport {
        let Some(tool) = resolve_tool("llvm-nm") else {
            return SymbolReport::Failed(ProbeFailure::ToolNotFound);
        };
        match run_tool(&tool, &[]) {
            Ok(text) => parse_nm(&text, sym),
            Err(f) => SymbolReport::Failed(f),
        }
    }

    /// Runs `tool <args> <current_exe>` and returns its stdout.
    #[cold]
    #[inline(never)]
    fn run_tool(tool: &Path, args: &[&str]) -> Result<String, ProbeFailure> {
        let Ok(image) = env::current_exe() else {
            return Err(ProbeFailure::ImageUnavailable);
        };
        let Ok(out) = Command::new(tool).args(args).arg(&image).output() else {
            return Err(ProbeFailure::ToolSpawnFailed);
        };
        if !out.status.success() {
            return Err(ProbeFailure::ToolExitedNonZero);
        }
        String::from_utf8(out.stdout).map_err(|_| ProbeFailure::ToolOutputNotUtf8)
    }

    /// Locates an LLVM binutil.
    ///
    /// **Resolution order is `PATH`, then the rustup toolchain's `rustlib` bin, then RED** — and
    /// it deliberately does **not** go through `rustc --print sysroot`. Measured on this box: the
    /// `rustc` on `PATH` is chocolatey's 1.95.0 and its sysroot's `rustlib` bin contains zero
    /// matches for `readobj`; the tools live only under the rustup toolchains (1.97.1). A gate
    /// that resolved via that sysroot would find nothing and RED for the wrong reason.
    ///
    /// Also measured: `RUSTUP_HOME` and `RUSTUP_TOOLCHAIN` are **both unset** in the environment
    /// that runs these gates, so neither may be required — the home falls back to
    /// `%USERPROFILE%`/`$HOME` and the toolchain is chosen by scanning, preferring the one whose
    /// target directory matches this build's own arch/OS/env.
    #[cold]
    #[inline(never)]
    fn resolve_tool(stem: &str) -> Option<PathBuf> {
        let mut exe = String::from(stem);
        if cfg!(windows) {
            exe.push_str(".exe");
        }

        if let Some(path) = env::var_os("PATH") {
            for dir in env::split_paths(&path) {
                let cand = dir.join(&exe);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }

        let rustup_home = env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("USERPROFILE")
                    .or_else(|| env::var_os("HOME"))
                    .map(|h| PathBuf::from(h).join(".rustup"))
            })?;

        let toolchains = rustup_home.join("toolchains");
        let named: Vec<PathBuf> = match env::var_os("RUSTUP_TOOLCHAIN") {
            Some(t) => vec![toolchains.join(t)],
            None => std::fs::read_dir(&toolchains)
                .ok()?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .collect(),
        };

        // Every candidate is recorded, then the one whose target triple matches this build is
        // preferred: `llvm-readobj` reads any object regardless of its own host, but a gate that
        // picks arbitrarily between three installed toolchains is a gate whose answer depends on
        // directory iteration order.
        let mut fallback: Option<PathBuf> = None;
        for tc in named {
            let Ok(targets) = std::fs::read_dir(tc.join("lib").join("rustlib")) else {
                continue;
            };
            for target in targets.filter_map(|e| e.ok()) {
                let cand = target.path().join("bin").join(&exe);
                if !cand.is_file() {
                    continue;
                }
                let name = target.file_name();
                let name = name.to_string_lossy();
                let matches_host = name.contains(env::consts::ARCH)
                    && name.contains(env::consts::OS)
                    && name.ends_with(host_env_suffix());
                if matches_host {
                    return Some(cand);
                }
                if fallback.is_none() {
                    fallback = Some(cand);
                }
            }
        }
        fallback
    }

    /// The trailing environment component of this build's target triple (`gnu`, `msvc`, …).
    ///
    /// Two rustup toolchains on this box differ only in that component, and they are different
    /// binaries; picking by it is what makes the choice deterministic.
    #[inline]
    fn host_env_suffix() -> &'static str {
        if env::consts::OS == "windows" {
            if cfg!(target_env = "gnu") { "gnu" } else { "msvc" }
        } else {
            ""
        }
    }

    /// Parses `llvm-readobj --sections` output for one section.
    ///
    /// Kept separate from the process spawn so it can be unit-tested against a captured fixture,
    /// which is the only part of the probe that can be wrong without the tool present.
    fn parse_sections(text: &str, section: &str) -> SectionReport {
        let mut in_target = false;
        let mut virtual_size: Option<u64> = None;
        let mut raw_data_size: Option<u64> = None;

        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("Name: ") {
                // The tool appends the raw name bytes as ` (2E 62 73 73 …)`; the name proper is
                // everything before that.
                let name = rest
                    .split(" (")
                    .next()
                    .expect("invariant: str::split always yields at least one item")
                    .trim();
                in_target = name == section;
                virtual_size = None;
                raw_data_size = None;
            } else if in_target {
                if let Some(v) = line.strip_prefix("VirtualSize: ") {
                    let Some(n) = parse_int(v) else {
                        return SectionReport::Failed(ProbeFailure::FieldUnparsable);
                    };
                    virtual_size = Some(n);
                } else if let Some(v) = line.strip_prefix("RawDataSize: ") {
                    let Some(n) = parse_int(v) else {
                        return SectionReport::Failed(ProbeFailure::FieldUnparsable);
                    };
                    raw_data_size = Some(n);
                }
            }

            if let (Some(virtual_size), Some(raw_data_size)) = (virtual_size, raw_data_size) {
                return SectionReport::Found {
                    virtual_size,
                    raw_data_size,
                };
            }
        }
        SectionReport::NoSuchSection
    }

    /// Parses `llvm-nm` output for one symbol. Lines are `<addr> <class> <name>`, and an
    /// undefined symbol has no address at all.
    fn parse_nm(text: &str, sym: &str) -> SymbolReport {
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let (Some(a), Some(b)) = (fields.next(), fields.next()) else {
                continue;
            };
            let (class, name) = match fields.next() {
                Some(name) => (b, name),
                None => (a, b),
            };
            if name != sym {
                continue;
            }
            let bytes = class.as_bytes();
            if bytes.len() != 1 {
                return SymbolReport::Failed(ProbeFailure::FieldUnparsable);
            }
            return SymbolReport::Found { class: bytes[0] };
        }
        SymbolReport::NoSuchSymbol
    }

    /// Accepts both spellings the tool emits: `0x220` and `0`.
    fn parse_int(v: &str) -> Option<u64> {
        let v = v.trim();
        match v.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => v.parse::<u64>().ok(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Captured verbatim from `llvm-readobj --sections` over a real PE/COFF binary built by
        /// this workspace's toolchain, so the parser is pinned to output that actually exists.
        const PE_FIXTURE: &str = "\
Sections [
  Section {
    Number: 2
    Name: .data (2E 64 61 74 61 00 00 00)
    VirtualSize: 0xB60
    VirtualAddress: 0xA0000
    RawDataSize: 3072
  }
  Section {
    Number: 6
    Name: .bss (2E 62 73 73 00 00 00 00)
    VirtualSize: 0x220
    VirtualAddress: 0xD0000
    RawDataSize: 0
    PointerToRawData: 0x0
  }
]
";

        #[test]
        fn parses_the_bss_row_and_recognises_size_without_raw_data() {
            let r = parse_sections(PE_FIXTURE, ".bss");
            assert!(r.virtual_size() == Some(0x220));
            assert!(r.raw_data_size() == Some(0));
            assert!(r.is_size_without_raw_data());
        }

        #[test]
        fn a_section_with_raw_data_is_not_size_without_raw_data() {
            let r = parse_sections(PE_FIXTURE, ".data");
            assert!(r.virtual_size() == Some(0xB60));
            assert!(r.raw_data_size() == Some(3072));
            assert!(!r.is_size_without_raw_data());
        }

        #[test]
        fn an_absent_section_is_not_a_pass() {
            let r = parse_sections(PE_FIXTURE, ".nope");
            assert!(r == SectionReport::NoSuchSection);
            assert!(!r.is_size_without_raw_data());
        }

        #[test]
        fn a_failed_probe_is_red_not_skip() {
            let r = SectionReport::Failed(ProbeFailure::ToolNotFound);
            assert!(!r.is_size_without_raw_data());
            assert!(r.failure() == Some(ProbeFailure::ToolNotFound));
            let s = SymbolReport::Failed(ProbeFailure::ToolNotFound);
            assert!(!s.is_uninitialized_data());
        }

        #[test]
        fn parses_nm_classes_including_the_undefined_two_field_line() {
            let text = "1400d0040 B CELLS\n0000000140001000 T main\n                 U memcpy\n";
            assert!(parse_nm(text, "CELLS").class() == Some(b'B'));
            assert!(parse_nm(text, "CELLS").is_uninitialized_data());
            assert!(parse_nm(text, "main").class() == Some(b'T'));
            assert!(!parse_nm(text, "main").is_uninitialized_data());
            assert!(parse_nm(text, "memcpy").class() == Some(b'U'));
            assert!(parse_nm(text, "absent") == SymbolReport::NoSuchSymbol);
        }

        #[test]
        fn parses_both_integer_spellings() {
            assert!(parse_int("0x220") == Some(0x220));
            assert!(parse_int(" 0 ") == Some(0));
            assert!(parse_int("zzz").is_none());
        }

        /// The live half: the tool must resolve and the running test binary must have a `.bss`
        /// that carries a size with no raw data. **Tool absence fails this test**, which is the
        /// RED-not-SKIP rule made operative.
        ///
        /// ⚠️ **The `keep_alive` write is load-bearing and this test failed without it.**
        /// MEASURED (rustc 1.95.0, `-O`, `--edition 2024`, this box): an all-zero `static` that
        /// is never WRITTEN is eliminated whole — a 1 MiB `pub static [AtomicU64; 131072]`, read
        /// at a runtime index, produced a **byte-identical** binary (2 112 487 B both ways) with
        /// an identical section table. Add one `store` to it and `.bss` grows from `0x220` to
        /// `0x100220` — exactly the megabyte — while `RawDataSize` stays `0` and the file grows
        /// 85 bytes. The first version of this test probed a binary whose statics the test never
        /// touched, so the linker emitted **no `.bss` section at all**, and the assertion failed
        /// for a reason that had nothing to do with residency.
        ///
        /// The distinction that survives, and belongs to every `.bss` gate in the corpus: this
        /// gate proves an EXTENT IS `.bss`-eligible, which requires the linker to have emitted
        /// it, which requires the static to be live. **That is a different binary from the
        /// flag-off run whose non-residency it licenses** — and saying so is the honest form of
        /// the claim. A residency gate over untouched statics is green-or-absent for the wrong
        /// reason, which is this campaign's signature defect wearing a linker's clothes.
        #[test]
        fn live_probe_finds_bss_in_the_test_binary() {
            // Keep the crate's own zeroed statics alive so the linker emits the extent this
            // test is about. Reading is not enough — see the note above.
            crate::loss::record_here(crate::loss::LossClass::Overflow, 0);

            // MEASURED, and it is why this probes two names rather than one: the section that
            // carries the demand-zero extent DEPENDS ON THE PROFILE on this toolchain.
            //   release (`-O`): a separate `.bss`, `RawDataSize == 0`.
            //   debug (this binary): NO `.bss` at all — the zero tail lives in `.data`'s
            //   VirtualSize, measured 0xB610 virtual against 3584 raw, a 43 KiB tail.
            // Asserting `.bss` unconditionally therefore fails every debug build for a reason
            // that has nothing to do with residency. The property that holds in BOTH profiles is
            // `VirtualSize > RawDataSize` — the excess IS the demand-zero part.
            let bss = section_report(".bss");
            let data = section_report(".data");

            // The tool must have RESOLVED for at least one probe. `.data` always exists, so its
            // report is the one that proves the tool ran; `.bss` may legitimately be absent.
            assert!(
                data.failure().is_none(),
                "invariant: the image probe resolves its tool; absence is a RED, never a SKIP"
            );

            // `VirtualSize > RawDataSize` IS the demand-zero extent, in whichever section the
            // profile put it.
            let demand_zero = |r: &SectionReport| match (r.virtual_size(), r.raw_data_size()) {
                (Some(v), Some(raw)) => v > raw,
                _ => false,
            };
            assert!(
                demand_zero(&bss) || demand_zero(&data),
                "invariant: this binary carries a demand-zero extent in `.bss` or in `.data`'s \
                 tail. If BOTH are flat with the tool present, the likeliest cause is that no \
                 static in this binary is WRITTEN, so the linker eliminated them all -- see the \
                 note on this test."
            );
        }
    }
}

#[cfg(feature = "section-gate")]
pub use gate::{ProbeFailure, SectionReport, SymbolReport, section_report, symbol_report};

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// The policy's compile-time half, exercised where it would actually be written.
    const _: () = assert!(assert_zero_init_eligible::<AtomicU64>());
    const _: () = assert!(assert_zero_init_eligible::<[u8; 48]>());

    #[repr(C, align(64))]
    struct Cell {
        count: AtomicU64,
        bytes: AtomicU64,
        _pad: [u8; 48],
    }

    // SAFETY: every field is `ZeroInit` (two atomics and a byte array), the type has no `Drop`,
    // no niche, and `#[repr(C, align(64))]` only inserts trailing padding, which carries no
    // validity requirement.
    unsafe impl ZeroInit for Cell {}

    static CELLS: SyncCells<Cell, 8> = SyncCells::zeroed();

    #[test]
    fn zeroed_table_reads_zero_everywhere() {
        for i in 0..CELLS.len() {
            // SAFETY: `i < 8 == N`, and this single-threaded test is the only accessor, so no
            // other thread writes index `i` for the pointer's lifetime.
            let c = unsafe { &*CELLS.get_ptr(i) };
            assert!(c.count.load(Ordering::Relaxed) == 0);
            assert!(c.bytes.load(Ordering::Relaxed) == 0);
        }
    }

    /// Would fail if `get_ptr` dropped the index, cast the array pointer wrongly, or strided by
    /// anything other than `size_of::<T>()` — the three ways this one line can be wrong.
    #[test]
    fn get_ptr_addresses_distinct_cells_at_the_element_stride() {
        // SAFETY: index 0 is in bounds; the pointer is converted to an address and never
        // dereferenced, so no aliasing obligation outlives the statement.
        let base = unsafe { CELLS.get_ptr(0) } as usize;
        for i in 0..CELLS.len() {
            // SAFETY: `i < CELLS.len() == N`; as above, the pointer is only turned into an
            // address.
            let p = unsafe { CELLS.get_ptr(i) } as usize;
            assert!(p == base + i * size_of::<Cell>());
        }
    }

    #[test]
    fn a_write_lands_in_exactly_one_cell() {
        // A private table, because the harness runs `#[test]`s on parallel threads and two tests
        // sharing one mutable static is the same single-writer breach this type exists to make
        // explicit.
        static WRITTEN: SyncCells<Cell, 8> = SyncCells::zeroed();

        // SAFETY: index 5 is in bounds, and `WRITTEN` is named by this test alone, so no other
        // thread writes index 5 for the pointer's lifetime.
        unsafe { &*WRITTEN.get_ptr(5) }
            .count
            .store(0xDEAD_BEEF, Ordering::Relaxed);
        for i in 0..WRITTEN.len() {
            // SAFETY: `i < WRITTEN.len() == N`, and this test is the table's sole accessor.
            let got = unsafe { &*WRITTEN.get_ptr(i) }
                .count
                .load(Ordering::Relaxed);
            assert!(got == if i == 5 { 0xDEAD_BEEF } else { 0 });
        }
    }

    #[test]
    fn from_array_keeps_the_values_it_was_given() {
        static NON_ZERO: SyncCells<u32, 3> = SyncCells::from_array([7, 0, 9]);
        // SAFETY: indices 0..3 are in bounds; `NON_ZERO` is written by nothing, anywhere.
        let read = |i: usize| unsafe { *NON_ZERO.get_ptr(i) };
        assert!((read(0), read(1), read(2)) == (7, 0, 9));
    }

    #[test]
    fn extent_is_the_const_and_nothing_else() {
        assert!(CELLS.len() == 8);
        assert!(!CELLS.is_empty());
        static EMPTY: SyncCells<u8, 0> = SyncCells::zeroed();
        assert!(EMPTY.is_empty());
    }
}
