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

**Findings (verified against primary sources — Microsoft Learn, man7, kernel.org,
LWN; see Sources below).**
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
- **Huge pages (2 MB)** on the `Arena` → ~512× fewer TLB entries for the same span,
  and the only mainstream userspace way to *guarantee* a contiguous, locked physical
  block. Windows: `VirtualAlloc(MEM_LARGE_PAGES)` — requires `SeLockMemoryPrivilege`,
  must reserve+commit in a SINGLE op (no commit-into-reserved), and the docs warn to
  allocate them all at startup because contiguous physical space is hard to find once
  RAM fragments. Linux: `madvise(MADV_HUGEPAGE)` (transparent, unprivileged) /
  `MAP_HUGETLB`; the kernel actively *compacts* physical memory to assemble them
  (`khugepaged` collapse + direct / `vm.compact_memory` compaction — "compaction to
  copy data around memory", per kernel docs). *Primary candidate.*
- **Page-locking** (`VirtualLock` / `mlock`) pins our working set resident (never
  swapped) — but the docs are explicit it guarantees RESIDENCY only, NOT placement or
  contiguity. `mlock` is bounded by `RLIMIT_MEMLOCK` (CAP_IPC_LOCK for unlimited);
  `VirtualLock` needs no privilege but has a small per-process cap. Secondary.
- *(Overkill — noted only for completeness, not for the foundation:)* AWE
  (`AllocateUserPhysicalPages`) gives a process locked frame *ownership*, but the
  docs state plainly there is NO contiguity and NO physical-address control
  ("physical pages can reside at any physical address"); Windows **Memory Partitions**
  are a system-level page-partitioning facility, NOT a publicly-documented per-process
  "dedicated RAM" guarantee (do not rely on it); Linux **hugetlbfs** reserves
  contiguous pages at boot. All too heavy/privileged for a game-engine core.

**Decision / next step.** No code now. If `perf`/VTune (`dtlb_load_misses`) ever
shows a TLB bottleneck on a large-world workload, add an opt-in huge-page flag to
`Arena` and validate A/B with the project's standard p-value methodology. Until
then this is a non-issue for cache locality.

**Sources (verified).** No headline claim was contradicted by its cited source.
- Linux "processes normally have no way of knowing (and no need to know) where their
  memory is located in physical memory"; the physical-page mover is an unmerged RFC —
  [LWN 944115](https://lwn.net/Articles/944115/).
- Windows large pages (contiguous, nonpageable, single reserve+commit,
  `SeLockMemoryPrivilege`, allocate at startup) — [Large-Page Support](https://learn.microsoft.com/en-us/windows/win32/memory/large-page-support).
- AWE: locked ownership, "physical pages can reside at any physical address … make no
  assumptions about contiguity" — [AllocateUserPhysicalPages](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-allocateuserphysicalpages).
- `VirtualLock` residency-only, no privilege, small cap — [VirtualLock](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtuallock).
- `mlock` residency-only, `RLIMIT_MEMLOCK`/`CAP_IPC_LOCK` — [mlock(2)](https://man7.org/linux/man-pages/man2/mlock.2.html).
- Kernel compacts physical memory to form huge pages — [Transparent Hugepage Support](https://docs.kernel.org/admin-guide/mm/transhuge.html)
  + [vm sysctl `compact_memory`](https://docs.kernel.org/admin-guide/sysctl/vm.html).
- Cache-line-within-a-page locality is a textbook architecture fact (64 B line ⊂ 4 KB
  page; L1 VIPT index within the page offset, L2/L3 PIPT), uncontested but not from a
  fetched primary; the TLB-reach corollary is supported by the large-page doc above.
