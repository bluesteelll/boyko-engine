# rustc 1.98 on `x86_64-pc-windows-gnu`: every `thread_local!` read costs two contended RMWs and a Win32 call

*Found 2026-09-10 while re-taking the KE16 Step App numbers. Established by disassembling the two
benchmark binaries, opening the std sources and the two upstream PRs, and having every claim re-opened
by a separate refuter. Nothing in this file is a timing; the one timing that would close it is named
in §5 and has not been run.*

## 1. The symptom

`crates/boyko_threadpool/benches/ke16_nested_scope.rs`, cell `worker/body_1us_tasks_64W` (1,024 tasks
of a 1 µs spin body spawned from inside a worker), commit `5863b041`, features
`ke16-a1-fifo,ke16-b1,ke16-w-count` (the shipped `a1f+b1+wc+c0`), one box, one hour, arm witness printed
per run:

| compiler | `worker/body_1us_tasks_64W` | `worker/body_10us_tasks_4W` |
|---|---:|---:|
| rustc 1.97.1 (8bab26f4f, LLVM 22.1.6) | **99.3 / 105.0 / 132.7 µs** | 65.8 / 138.4 / 76.3 µs |
| rustc 1.98.1 (48a229cea, LLVM 22.1.8) | 1,170–1,215 µs (9 of 11 runs), 444 / 542 µs (2) | 72.5–74.7 µs |
| `a3+b0+wc+c0`, 1.97.1 | 165.8 / 128.1 µs | 182.8 / 157.9 µs |
| `a3+b0+wc+c0`, 1.98.1 | 99.9 / 100.6 µs | 175.7 / 178.9 µs |

The `a1f` placement path is ~11× slower under 1.98.1; `a3` is not slower at all; the effect is confined
to 1 µs bodies. The stable toolchain on this box was rewritten to 1.98.1 on 2026-09-09 02:52, so every
binary built since — including everything the Step App take measured — carries it. Full measurement
trail: `KE16-RESULTS.md` §A-RE (replay finding) and `docs/OPEN-QUESTIONS.md` (2026-09-10).

## 2. The mechanism, in the binaries

Binaries: `ke16_nested_scope-37cb3c70d2bfc95f.exe` (1.97.1) and `-5e324654af0d966a.exe` (1.98.1), both
in `D:/wt/ke16-cs4/target/release/deps/`; disassembled with `llvm-objdump -d --demangle`. The diffs of
every hot function are on disk under the session scratchpad (`asm/`); the numbers below were re-derived
by the refuter from fresh objdumps.

**Under 1.97.1** a `thread_local!` read is one call to an out-of-line per-key closure:
`mov esi,[KEY+0x18]; test; dec; call TlsGetValue`. `std::sys::thread_local::guard::windows::enable`
(0x1401c0980) is a four-instruction stub (three loads, `ret`) called from **10** sites, all in
`thread_local::os::destroy_value` — thread exit only. Zero `Fls*` imports.

**Under 1.98.1** the read is inlined and gated on the key's destructor slot:
`cmp qword [KEY],0; je; call guard::windows::enable; mov ebx,[KEY+0x18]; …; call TlsGetValue`, and the
key **always** has a destructor (`os::Storage::new` passes `Some(destroy_value)`), so the `je` never
skips. `enable` (0x1401c2840) is now, per call:

```
lock inc dword [ACTIVE_ENABLE_CALLS]     ; process-global static, 0x1403566a8
lock or  dword [rsp],0                   ; fence(SeqCst)
movzx    eax,[AT_EXIT_HOOK_CALLED]
mov      ecx,[KEY]
…
mov      edx,1 ; call kernel32!FlsSetValue
lock dec dword [ACTIVE_ENABLE_CALLS]
ret
```

Two lock-prefixed RMWs on **one cache line shared by every thread in the process**, a full fence, and a
kernel32 call — **on every `thread_local!` access**. Call-site census: `enable` is called from **192**
sites in the 1.98.1 binary; `TlsGetValue` call sites 169 → 362.

Where the `a1f+b1` hot path pays it (1.98.1 addresses):

| site | TLS reads per … | why |
|---|---|---|
| `push_task` → `tls::worker_lane_for` (0x1400332c9, 0x14003330b) | **2 per spawned task** | `WORKER_DEQUE.with` then `current_worker_id()` — the placement predicate |
| `try_steal_random` (0x14003354a + `with_handle::<pin>`) | **2 per victim probed**, inside the loop, inside the thief's `front.load → CAS` window | crossbeam-epoch `HANDLE`: `is_pinned()` then `pin()` in `Stealer::steal_batch_and_pop` (crossbeam-deque 0.8.8 `deque.rs:1021,1025`), before the `len <= 0` check |
| `Scope::drop` → `join_on_worker` (0x1400341b0, 0x140034661) | 2 at entry + 2 per victim in its own sweep | same two reads, plus the joiner's steal sweep |

Why `a3` is untouched, at the instruction level: its `push_task` and `Scope::drop` contain **zero** TLS
reads under either compiler (`place_task` returns `push_global_no_wake` before the predicate), and
`Injector::steal_batch_with_limit_and_pop` — where `a3`'s thieves drain at stage 2 — has **zero**
`enable` calls; only `Stealer::steal_batch_with_limit_and_pop`, the `a1f` thief's path, has one.

Everything else on the path is relocation-only identical between the two binaries: the spin body,
`Scope::spawn`, `Worker::pop`, `drain_one`, `pop_global_injector`, `unpark_one_idle_excluding`, every
`lock`/`xchg`/`pause` site (counts identical per function), `Instant::now`'s hot path (one QPC call);
`mfence` appears in neither. LLVM went 22.1.6 → 22.1.8, a patch bump. The back-end is not a candidate.

## 3. Where it came from

* **rust-lang/rust #148799** — *Switch the destructors implementation for thread locals on Windows to use
  FLS* (ohadravid; merged 2026-06-03; milestone 1.98.0; listed in `RELEASES.md` 1.98.0 Compatibility
  Notes). Replaced the `.CRT$XLB` `tls_callback` guard with FLS and gave `enable()` its `EnableGuard`
  (`fetch_add` + `fence(SeqCst)` … `fetch_sub`) and the `FlsSetValue(KEY, 1)` call.
* **rust-lang/rust #157483** — *fix windows-gnu TLS leak* (RalfJung; merged 2026-06-07; milestone
  1.98.0). Fixed a std-test memory leak where `LazyKey::init` called `guard::enable` only on the first
  thread to touch a key: the diff adds `if self.dtor.is_some() { guard::enable(); }` inside
  `LazyKey::force()` — i.e. **every access** — and removes it from `register_dtor`.
* **Why windows-gnu only.** `target_thread_local` is unset for `x86_64-pc-windows-gnu` and set for
  `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` (verified with
  `nightly rustc -Zunstable-options --print cfg`; the cfg is hidden on stable, which produced one false
  "msvc = 0 too" reading during the hunt). On gnu, (i) every `thread_local!` uses the OS-key `os::Storage`
  backend, whose key always has a dtor — `const {}` initialisation does **not** help — and (ii) `enable`'s
  per-thread `#[thread_local] REGISTERED` short-circuit is compiled out. std's own comment at
  `guard/windows.rs:116-118`: *"`#[thread_local]` is unavailable on windows-gnu … so we don't bother
  tracking registration separately"*, with a cost model that reads *"about as expensive as TlsGet"* and
  omits the two contended RMWs.
* No upstream report found: three GitHub issue searches (`windows-gnu thread_local FlsSetValue`,
  `thread_local windows-gnu regression 1.98`, `FlsSetValue guard enable performance`) returned zero.
  Keyword-bound; UNVERIFIED as a negative.

## 4. What is proven and what is not

**Proven (binaries + sources, refuted-and-confirmed):** the per-access cost exists under 1.98.x on
windows-gnu, is absent under 1.97.1, sits on the `a1f` spawn / thief / join paths at the multiplicities
above, and is absent from `a3`'s paths. Nothing else on the path changed.

**Not proven:** that this cost is the *whole* of the 11×, and the reading of the 444 / 542 µs mode as
"the same cost with fewer thieves on the line". The asm cannot decide the dynamics — whether the
~1.15 µs/task floor is the spawner serialised by its own 4 RMWs + 2 `FlsSetValue` per spawn against 15
sweeping thieves, or the thieves losing every `front` CAS to the FIFO owner's `fetch_add` because their
window now contains two `enable` calls (`drain_one`'s unbounded `Retry`). Both are downstream of the same
construct and neither changes whose defect it is. The cost of `kernel32!FlsSetValue` itself (whether
ntdll takes a lock) is UNVERIFIED.

**Whose defect.** (a) **Upstream std**: a legitimate leak fix whose per-access price on windows-gnu is
two contended RMWs plus a Win32 call, unaccounted for in its own comment. Our code relies on
`thread_local!` being O(one load), which is std's contract; there is nothing accidental to repair.
Minimal report shape: any `thread_local!` read on `x86_64-pc-windows-gnu` (a `Cell<u32>` with `const {}`
init suffices); toolchains 1.97.1 vs 1.98.0/1.98.1; sequence A vs sequence B from §2; the 1.98.0
milestone of #148799 + #157483. Filing it is the owner's call — it publishes.
(b) **Ours to do regardless**, because a TLS read per spawned task was never a good idea:
1. ~~**`Scope` captures the lane once.**~~ **SUPERSEDED (2026-09-10, the fix commit): the two slots are
   MERGED instead.** A `Scope`-cached lane would help exactly one call site, while the identity predicate
   is asked at five (`Scope::spawn`, `ThreadPool::spawn`, `joiner_wake_target`, the join dispatch, the
   `install` frame), and it would make Chase-Lev's owner-only `push` depend on `Scope` happening to be
   `!Sync`. `WORKER_DEQUE` and `CURRENT_WORKER_ID` are now one `#[repr(C)] LaneDeposit` cell, so
   `worker_lane_for` is ONE `thread_local!` access instead of two at every site (`tls.rs`, invariants
   D6–D8; `tests/tls_lane_merge.rs` counts the accesses in the source because no behavioural test can
   see the difference). The cached-lane design stays a candidate for a later pass, behind a
   compile-fail pin that `Scope` is `!Sync`.
2. **Do not pin on an empty victim.** `Stealer::len()` / `is_empty()` load `front`/`back` without pinning;
   probing 14 empty deques per sweep and pinning on each is 28 TLS reads for nothing. Check emptiness
   before `steal_batch_and_pop`. The pin-before-`len` order is crossbeam-deque's and is also worth
   reporting upstream. **Done in the same commit**, in both sweeps (`worker::try_steal_random`,
   `scope::steal_one_random`). ⚠ This tree compiles **crossbeam-deque 0.8.7** (`Cargo.lock`), not 0.8.8:
   the pin sits at `deque.rs:1002/1006` before the length check at `:1013`; the 0.8.8 line numbers in
   §2 describe the same defect one release later.
Both are hot-path changes and are **not** to be shipped without the bench in the gate (§5).

## 5. The falsification run (owed; a timing — not before the owner says the box is free)

Build-only, single variable, no toolchain edit: copy
`$(rustc +stable-x86_64-pc-windows-gnu --print sysroot)/lib/rustlib/src/rust/library` to a scratch dir,
delete the `if self.dtor.is_some() { guard::enable(); }` block from its
`std/src/sys/thread_local/key/windows.rs::force()` and re-add `guard::enable();` as the first statement
of `os::Storage::try_initialize`, then in `D:/wt/ke16-cs4`:
`RUSTC_BOOTSTRAP=1 __CARGO_TESTS_ONLY_SRC_ROOT=<copy> RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu cargo bench -p boyko-threadpool --bench ke16_nested_scope --features ke16-a1-fifo,ke16-b1,ke16-w-count -- worker/body_1us_tasks_64W`
(with `-Zbuild-std`). Expected ≈ 100 µs. Whether this cargo honours `__CARGO_TESTS_ONLY_SRC_ROOT` is
UNVERIFIED (the string is present in the binary; not tried). If it does not, the alternative is the two
local fixes of §4(b) measured against the unpatched 1.98.1 build — a weaker test, because it changes two
variables.

The bracket "1.98.0 or the 1.98.1 point release" is answered by the sources: both PRs carry the 1.98.0
milestone, and the 1.98.0 `libstd` rlib on this box already has the RMW `enable` at 0x44840.

~~The MSVC control ("build the same source for `x86_64-pc-windows-msvc`, where `#[thread_local]` is native")
is not executable on this box: the installed MSVC toolchain is rustc 1.92.0 and there is no MSVC linker.~~
**STALE, and it was stale within a day.** The owner installed VS Build Tools 2022 + Windows SDK
10.0.26100 on 2026-09-10 and `stable-x86_64-pc-windows-msvc` is **rustc 1.98.1 (48a229cea), LLVM
22.1.8 — the same commit and the same LLVM as the gnu toolchain**. That makes the MSVC control the
cleanest falsification available here (one variable: the host env, hence whether
`target_thread_local` is set and `LazyKey::force` compiles at all), strictly better than the
`-Zbuild-std` route above, which patches std. It is queued as the four-arm A/B in §5b.

## 5a. The fix, measured: two thread-local reads per spawn cost exactly twice one (2026-09-10)

`e6115223` merged `WORKER_DEQUE` and `CURRENT_WORKER_ID` into one `LaneDeposit` slot, so
`worker_lane_for` performs **one** `thread_local!` read per spawn instead of two. If §2's mechanism
is right the saving must be **half the excess** — not a fraction of the runtime, half of the *gap*
between 1.97.1 and 1.98.1 — and it must vanish as the body grows, because the cost is a constant
per spawn and not a rate.

A/B in ONE session, two trees that differ only in `crates/boyko_threadpool/src/{tls,worker,scope,
thread_pool,lib}.rs`: **A** = `02325b01` (the fix), **B** = `8d13115e` (its parent). Same box, same
`rustc 1.98.1 (48a229cea)`, `available_parallelism = 16`, features already removed (`67563d3b`), so
no `--features` and no `KE16_EXPECT`. Both bench binaries were built BEFORE the window; the window
opened only after three consecutive 20 s samples saw zero `cargo`/`rustc`/`link` processes; a load
receipt was taken immediately before and after every timed region; the arms were **interleaved**
A,B,A,B rather than A,A,B,B. The `r1` pair is clean on both receipts (0 build processes); the `r2`
pair ran with 2 build processes and is reported only as corroboration, never as the datum.

| cell (`ke16_nested_scope`) | A: fix | B: pre-fix | B/A |
|---|---:|---:|---:|
| `dispatcher/body_1us_tasks_W` | 4.5 µs | 25.9 µs | **5.75×** |
| `dispatcher/body_1us_tasks_4W` | 10.4 µs | 43.4 µs | **4.17×** |
| `dispatcher/body_1us_tasks_64W` | 120.2 µs | 325.5 µs | **2.71×** |
| `worker/body_1us_tasks_W` | 17.1 µs | 31.0 µs | **1.81×** |
| `worker/body_1us_tasks_4W` | 39.8 µs | 81.4 µs | **2.05×** |
| **`worker/body_1us_tasks_64W`** (§1's cell) | **531.6 µs** | **978.7 µs** | **1.84×** |
| `body_10us_*` (6 cells) | — | — | 1.03–1.50× |
| `body_100us_*` (6 cells) | — | — | 1.02–1.11× |
| `body_1ms_*` (6 cells) | — | — | 0.90–1.04× (noise, both directions) |

**The prediction and the number.** Take §1's 1.97.1 figure on the deciding cell as the
guard-free baseline, ≈ 110 µs (99.3 / 105.0 / 132.7). Then the pre-fix excess is
978.7 − 110 = 868.7 µs for TWO reads, i.e. ≈ 434 µs per read per wave (1,024 tasks ⇒ ≈ 424 ns per
read per task, which is the right order for two `lock`-RMWs on a contended global line plus an
`FlsSetValue` call). One read predicts 110 + 434 = **544 µs**. Measured **531.6 µs** — **2.3 %
below the prediction**. The mechanism is not merely consistent with the effect; it gets its SIZE
right.

**And the regression is NOT closed, which the ratio alone would hide.** 531.6 µs against ≈ 110 µs
is still ≈ 4.8×. The fix removes the read it could remove; the residual is the cost of the ONE
remaining read on the spawn path, and no further merging can remove that one — it is what a
`thread_local!` costs on this host under this compiler. The exits are §3's upstream fix, a host
without the guard (§5b), or a spawn path that reads no thread-local at all.

**The decay across the body axis is the second, independent confirmation.** 5.75× at 1 µs, ~1.2× at
10 µs, ~1.05× at 100 µs, 1.00× at 1 ms: a constant per spawn amortises exactly like that, and a
rate would not. Nothing in the 1 ms row moves outside the band in either direction.

## 5b. The MSVC control (queued): if this story is right, the fix must buy NOTHING there

Four arms in one session — `{fix, pre-fix} × {gnu, msvc}` — same source, same rustc commit, same
LLVM, differing only in the host env. `x86_64-pc-windows-msvc` sets `target_thread_local`, so the
`REGISTERED` short-circuit compiles and `LazyKey::force` does not exist on the path at all.

**Stated before the run so it could fail:** on msvc the two arms must land on top of each other,
both near the 1.97.1 gnu figure, while the gnu pair stays ≈ 1.8× apart on the deciding cell.

**RUN 2026-09-10. The mechanism is confirmed decisively, and the prediction was HALF WRONG in a way
that changed what ships.** All eight passes receipt-clean, `available_parallelism = 16`.

| `worker/body_1us_tasks_64W` | pre-fix | with the fix commit |
|---|---:|---:|
| gnu 1.98.1 | **1116 µs** | 615 µs |
| msvc 1.98.1 | **121.5 µs** | 234.5 µs |

The left column is the whole story of this document: the same source, the same rustc commit
(48a229cea) and the same LLVM 22.1.8, differing only in the host env, and **9.2× disappears** —
landing at 121.5 µs, inside §1's 1.97.1 band of 99–133 µs. On the DISPATCHER route the prediction
held to the digit (msvc 1.04× / 0.98× / 1.01× — the fix buys exactly nothing where the guard does
not exist).

**On the WORKER route the fix commit made msvc ~2× WORSE, and that was not predicted.** It is also
not what the title of the commit says it is, because the commit carries TWO changes: the TLS merge
and FIX B (`Stealer::is_empty()` before `steal_batch_and_pop`). A third arm — `8d13115e` plus FIX B
alone — separates them, three arms × two hosts × two runs, every region receipt-clean:

| `worker/body_1us_tasks_64W` (µs) | neither | FIX B alone | FIX B + TLS merge |
|---|---:|---:|---:|
| gnu, `dispatcher` route | 520.1 | **140.7** | 125.7 |
| gnu, `worker` route | 1134.3 | 1089.2 | **594.8** |
| msvc, `dispatcher` route | 88.9 | 90.4 | 89.4 |
| msvc, `worker` route | **124.5** | **274.8** | 266.5 |

So the two changes do different jobs and the attribution is clean: **FIX B** carries the gnu
dispatcher win (3.7×, i.e. this document's TLS cost removed from the steal sweep) and is worth
almost nothing on the gnu worker route; **the TLS merge** carries the gnu worker win (1.83×) and is
neutral on msvc. The msvc regression is **entirely FIX B** (274.8 alone vs 266.5 with both, against
124.5 with neither).

**Why FIX B costs anything at all where TLS is free:** it is not only a cost-saving, it is a POLICY
change. A thief that skips a victim which is empty for an instant gives up a steal that
`steal_batch_and_pop`'s own `Retry` loop would have completed, and at 1 µs bodies the wave is short
enough that the lost steals become parks. On gnu the guard's TLS saving is larger than that loss;
on msvc there is no saving left and only the loss.

**Consequence, shipped 2026-09-10:** the guard rides `worker::STEAL_EMPTY_GATE`, a `cfg!` constant
(`all(windows, target_env = "gnu")`) rather than a `#[cfg]` block, so both arms typecheck and lint
on every host and neither can go dark. Confirmed on both hosts after the change, receipt-clean:
msvc `worker/1us/64W` back to **103.9 µs** (un-gated 124.5, un-conditional 266.5) and gnu keeping
its win at **528.2 µs** (594.8 before the constant; that cell's own run-to-run spread is wide —
455–601 — so "kept", not "improved"). msvc's dispatcher row is unmoved at 88.7.

⚠ This is the second time a KE16 number turned out to be a property of the HOST rather than of the
design (the first was §1 itself). Any absolute in `KE16-RESULTS.md` that was taken on gnu is a gnu
number, and the msvc line is a THIRD compiler line, not a continuation of either.

## 6. What this changes for the record

* Every KE16 absolute taken since 2026-09-09 02:52 was taken under this cost; every absolute before it
  was not. The **ratios** inside one session remain comparable — the decisive cell moved 0.47 → 0.42 —
  but the 1 µs cells and `empty_schedule_control` (one `Schedule::run`, TLS on the dispatcher path) are
  compiler-dependent and must be published with the compiler that produced the bytes.
* A measurement receipt must therefore name **the rustc that built the binary** (`rustc -vV`'s commit
  hash), not the channel: `stable` named two compilers a day apart and a load receipt cannot see the
  difference. `MEASUREMENT-QUEUE.md` / `KE16-DESIGN-MEASUREMENT.md` §0 should carry the requirement.
* The design's own §1.4 prediction for `a1f` — "same-end contention … a CAS storm" — was the wrong
  storm: the contended line is std's `ACTIVE_ENABLE_CALLS`, not the deque's `front`.
