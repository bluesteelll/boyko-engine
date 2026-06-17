# boyko-engine — Backlog (open questions / ideas not yet scheduled)

> Forward-looking "think about later" list. Items here are **not** committed
> phases — they are open questions, speculative optimizations, and ideas pending
> measurement or design. An item graduates to a PLAN doc + phase only when it is
> decided. This is distinct from the historical [TODOI.md](TODOI.md) and
> [ROADMAP-PHASE-2-PLUS.md](ROADMAP-PHASE-2-PLUS.md) (both superseded).
>
> Status legend: ❓ open question · 🔬 needs measurement · 💡 idea · ⏸ deferred.

## Memory / physical placement

### ❓🔬 BL-1 — Physical-memory placement & TLB hardening (huge pages, page-locking)

**Status: OPEN — do nothing until a profiler shows a real TLB bottleneck
(principle 7: measure before optimizing). Recorded for later thought.**

**Origin.** Question: our `Arena` reserves a large contiguous *virtual* range, but
the OS hands out *physical* frames lazily and other processes also allocate
physical memory while the game runs — do the resulting scattered physical frames
("holes") break our cache locality? And could we ask the OS to place other
processes' memory "far from ours" as insurance?

**Findings so far (analysis; specific OS-API semantics being verified separately).**
- **Cache-line locality (L1/L2/L3) is NOT affected by physical fragmentation.** A
  64 B cache line always lies within a single 4 KB page, and within one page the
  virtual and physical addresses are contiguous (a page = one contiguous frame).
  Our sequential SoA hot loops are immune by construction.
- **The only real effect is TLB reach** (random access over a working set far
  larger than TLB coverage — roughly ~6 MB on 4 KB pages) and the ability to form
  huge pages. This is a gradual percentage tax, not a cliff or crash; sequential
  access amortizes it to ~one TLB miss per page (negligible).
- **"Push other processes' physical memory away from ours" is not possible and
  would not help.** There is no userspace API on Windows or Linux to influence
  *another* process's physical placement (it would break isolation), the physical
  allocators are non-linear free-list pools (not low→high bump allocators, so
  "allocate from the end" is not a meaningful concept), and even if it worked it
  would not touch cache-line locality (see above).

**Candidate levers — all act on OUR OWN memory, opt-in, by measurement only.**
- **Huge pages (2 MB)** on the `Arena` → ~512× fewer TLB entries for the same span.
  Windows: `VirtualAlloc(MEM_LARGE_PAGES)` (+ `SeLockMemoryPrivilege`). Linux:
  `madvise(MADV_HUGEPAGE)` / `MAP_HUGETLB`; the kernel (`khugepaged`/`kcompactd`)
  compacts physical memory to form them. *This is the primary candidate.*
- **Page-locking** (`VirtualLock` / `mlock`) to pin our working set resident (never
  swapped) and grab stable frames early at startup, before the system fragments.
- *(Overkill — noted only for completeness, not intended for the foundation:)*
  Windows Memory Partitions, AWE (`AllocateUserPhysicalPages`), Linux hugetlbfs —
  direct physical reservation; too heavy/privileged for a game-engine core.

**Decision / next step.** No code now. If `perf`/VTune (`dtlb_load_misses`) ever
shows a TLB bottleneck on a large-world workload, add an opt-in huge-page flag to
`Arena` and validate A/B with the project's standard p-value methodology. Until
then this is a non-issue for cache locality.
