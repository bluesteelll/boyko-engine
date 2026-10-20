# CI-FIX-1: the record (lane `u/ci-fix-1`, 2026-10-20)

Master's push CI run 38182709148 (on 9cd4ad6a) was red on two blocking Windows-debug tests and one informational
Linux-release test; all three were latent on the trunk 0bd68959 (orchestrator ruling 73). This lane fixes the three as
classes, value-neutral: no simulation or render value moves. Base `0bd689596565769e1d31a4c80e46591fe352938a` plus the
trunk sync 29739376 (PHYS-TOOL-1, no file of this lane).

| item | red | class | fixed in |
|---|---|---|---|
| F1 | `log_host_editor`: "a threaded shutdown must join within its deadline" | the sink thread wrote the console (an external consumer) first and under the drain role; a stalled pipe starved the durable file, kept the role, and `shutdown` gave up without closing anything | `d1cf18f4` (red-first), `eb5dc569`, `589c31a7`, `41257568` |
| F2 | `narrowphase_carry`: a moved step 6 acquisitions against a fewest of 3 | a thread's FIRST crossbeam-epoch pin allocates its `Local` (and, process-first, `Global` and the queue sentinel) inside whatever frame its first steal lands in | `dd6685a8` (red-first), `eca7f277`, `c29e1bc4`, `11276668`, `c5a9427a` |
| F3 | `view::tests::perspective_bridge_reproduces_legacy_push_constants`, Linux release, one ulp | one side's `tan` constant-folded by LLVM, the other computed by the C runtime's `tanf` | `07ac21e1` |

Raw receipts stay in the lane scratch (`D:/tmp/phys-orch/ci-fix-1/`); each is quoted here as a summary with its sha256
prefix.

## Stage 0: the captures taken before any commit

### F1: where the census pauses (ruling 73 (2), triage check W1)

Instrument (scratch, `capture/f1/make_inst.py` 26c7f793): a static `[AtomicU64; 2 x 8192]` trace of
`(kind | thread | arg, ticks)` in `boyko_log`'s sink loop, per record (console, file, binary, ECS write spans),
per pass, per backoff sleep, and around each census row on the emitting thread; `shutdown` dumps it after its wait.
`log_host_editor` with CI's features (`boyko-ecs/profiling-analysis`), 100 runs per profile, one process at a time,
stdout and stderr each an OS pipe drained by a reader thread (`capture/f1/run_capture.py` 2dcaee8c).
Reduction `capture/f1/analyze.py` 49198a31 -> `capture/f1/summary.md` 1b727abe:

| profile | runs | class (cut F1-P4) | max gap ms | median gap ms | rows 4 and 5 |
|---|---|---|---|---|---|
| debug | 100 | (f) not reproduced | 0.173 | 0.029 | same pass in every run |
| release | 100 | (f) not reproduced | 0.062 | 0.026 | same pass in every run |

The gap is `CON_E(memory) -> CON_E(components)`, what a pipe reader timestamps. Its largest piece is the text-file
write of the memory row (~0.017 ms). CI's 41-332 ms pauses do not occur on this machine with a promptly drained
pipe. **The location stays OPEN** (ruling 75 Q8); the fix below is correct under every class. C8 ("wake the sink")
was not built: ruling 76 OQ2 keeps it only for a class-(b) finding within the 51.2 ms backoff ceiling, and there is
no class (b).

### F1: the red-first end to end (cut 4.4, triage check W2)

`f1/harness.py` 1c78d3b0 runs the debug exe with its stderr on an anonymous pipe that is not read until after the
census: "quota" mode (a 1 KiB pipe, nobody reads for 2 s) and the cut's "filler" mode (the reader stops after the
session header; a 1 MiB filler fills a duplicate write end, closed after its write).

| tree | mode | exit | stdout | durable file |
|---|---|---|---|---|
| base (exe d5df20e1) | quota, 2 s | 101 | `panicked at crates\boyko_app\tests\log_host_editor.rs:68:5:` + "a threaded shutdown must join within its deadline" (dbc4ea81) | 7 of 28 census rows, no limiter line (8dc58924) |
| base | filler, 2 s | 101 | the same panic at `:68:5` (e2279ab6) | 0 of 28 (36502112) |
| base | quota, 0 s | 0 | - | removed by the passing test |
| C9 (exe 1675a5a9) | quota, 2 s | 0 | ok (5d695167) | whole; the console received 15 census lines (the ones the sink had staged) |
| C9 | filler, 2 s | 0 | ok (975f2f82) | whole; the console received all 28 |
| C9 | quota, 0 s | 0 | ok | whole |

### F2: what the +3 is (ruling 73 (3), triage check W3)

Instrument (scratch, `capture/f2/make_inst.py` 43a29008, exe 3a76ba45): `narrowphase_carry`'s allocator logs every
in-window acquisition as `(window, thread role, size, align)`. Forced arm (`capture/f2/forced/try_1.txt` 4e506071): a
1-worker pool P built before warm-up; the precondition held on the first try (no epoch object before the window);
inside cycle 2's moved window the driver's forced steal of P's worker deque (witness: 8 of 8 children ran on the
driver). The window held the step's own objects (`256/128` scope frame, `4096/64` chunk on the driver, `16/8` on the
worker), P's scope objects, and **exactly `2072/8`, `640/128`, `2304/128` on the driver**: crossbeam-epoch 0.9.21's
queue sentinel, collector `Global` and the thread's `Local`. Natural arm, 66 debug runs: 66 green, no natural +3; 65
processes never pinned at all, one pinned in warm-up (`capture/f2/natural/exits.txt` a116433f).

### R-B1: the same class on another thread (ruling 73 (3), ruling 75 Q5)

`sr_region_counters`' thread-local allocator log on the counted thread, release (exe f23ba66a), 60 runs under
affinity mask F with three busy Python threads (`capture/rb1/run_rb1.py` 1fa5a893): **2 of 60 red at `(5, 1)`**;
both times the +3 on the counted step was exactly `2072/8`, `640/128`, `2304/128` - the process-first triple,
taken by the calling thread, which is the W4 pool's builder (`capture/rb1/runs/runs.tsv` 31593843). Decision row
F2-C4: **closed by the class fix**, and since an epoch signature was in the +3, the test gained the guard.

### F3: which side folds (ruling 73 (4), ruling 75 Q6, ruling 76 OQ4)

`cargo test --release -p boyko-render --lib --no-run -vv`, then `capture/f3/f3_replay.py` d2e2fe85 re-ran cargo's own
rustc command line for the release unit-test harness (fat LTO, the dev-dependency features) with
`--emit=dep-info,link,asm` into scratch; `capture/f3/f3_analyze.py` c015540f. Target `x86_64-pc-windows-msvc`, the
proxy for the failing `x86_64-unknown-linux-gnu` leg (critique O11); `view.rs`, `camera.rs` and `perspective()`'s
body are unchanged since 9cd4ad6a.

| tree | `.s` (the only one emitted) | test body | folded `0x3F13CD3A` |
|---|---|---|---|
| lane base 29739376 | `boyko_render-158f551af76ac8e1.s` sha256 `84e76d6bb8cc5c9a76d6f24f7035272c136d02fb79648c398936a434b0faa59d` | ONE `callq tanf` (the bridged side, after the out-of-line `ViewUniform::from_camera`) | yes: the legacy side's expected `cam_forward` is the constant `__xmm@3f13cd3abf8000000000000000000000` (10 occurrences in the file) |
| C1 07ac21e1 | the same name, sha256 `df45a24ec66e0d41976a880003acaa902d741074f8f53facc1b2e5ef33990306` | TWO `callq tanf` | none anywhere in the `.s` |

The legacy side folded (the triage's inference, confirmed). UCRT's `tanf` agrees with LLVM's fold at this input;
glibc's returns `0x3F13CD3B` (0.5773503), which is the CI red.

## The fixes

**F3 (`07ac21e1`).** The test black-boxes `view` and the legacy fov, so both sides reach `tan` at run time through
the same symbol on the same f32; the shared helper is untouched. The Linux green is proved by the owner-approved CI
run (ruling 73 (5)); nothing was triggered on GitHub.

**F2 (`eca7f277`).** `ThreadPoolBuilder::build` registers its builder and every worker as crossbeam-epoch
participants before it returns, through a private probe deque whose `Worker` end is dropped inside `build` (nothing
can push to it, so a probe can never take a task: critique W1, ruling 76). The bootstrap handshake runs both ways:
each worker registers and signals boot, `build` waits for all of them before it publishes; a worker unwinding before
it signals makes `build` panic (critique O8). `ThreadPool::register_thread` is the setup API for a driver that did
not build its pool; the shipped driver is the builder (`App::new` builds on the constructing thread; the runner
drives frames on it). No pool is built in a `Drop` impl, a TLS destructor or a lazy initialiser (ruling 76 OQ1).
`narrowphase_carry` (`c29e1bc4`) and `sr_region_counters` (`11276668`) name an epoch-sized acquisition in a counted
step before their comparisons; `narrowphase_carry`'s "deterministic" claim is corrected.

**F1 (`eb5dc569`, `589c31a7`, `41257568`).** The sink thread writes file -> binary -> ECS -> console per record,
stages its console lines on its own stack and writes them after it has released the drain role. A `shutdown` whose
500 ms join expires delivers and closes the durable sinks itself when the role is free (`ClosedAfterStall`), records
`StalledInsidePass` when it is held, and names a dead sink thread (`SinkDied`, ruling 76 OQ3); the late close
refuses the synchronous channel and counts what it refused (ruling 76 W2). `log_host_editor` asserts the whole
durable census and an eventual exit (ruling 76 W3); the runner's exit path waits up to 2 s more and reports a stall
still inside a pass.

## Every red-first: predicted and observed

| gate | predicted | observed |
|---|---|---|
| C1 mutation `fov_y * (1 + EPSILON)` | red, both profiles, on `cam_forward.w` only | red in both at the assert, `0.5773504` vs `0.57735026` |
| C2 class gate, unfixed pool | arm 1 "build() registered 0 of 2" | exactly that, debug and release (a819f236) |
| C2 identity receipt | `{640/128, 2072/8, 2304/128}` on the driver | `[(true, 2072, 8), (true, 640, 128), (true, 2304, 128)]` (70e6286a) |
| C3 m1 worker registration dropped | arm 1 red; zero_alloc's driver touch reads 1 | both |
| C3 m2 builder registration dropped | arm 1 red | red, `subject [0,0,0], others [1,1,1]` (the worker became process-first) |
| C3 m3 registration after the boot signal (+20 ms) | arm 1 red | red |
| C3 m4 `register_thread` registers nothing | arm 3 red | red |
| C3 m5 `register_thread` probes `stealers[0]` | the new unit test red | red in both profiles (debug: the probe invariant, then the 30 s bound; release: the bound) |
| C4 r1 a pool built inside a counted step | the guard, naming the new worker's `Local` | the guard, naming `Local 2304/128` and `2072/8` (the dropped pool's worker pushes its last bag) |
| C4 r2 `Vec::with_capacity` in `build_jumpers` | the carry comparison, not the guard | the comparison: 4 against a fewest of 3 |
| C5 a 2304/128 object in each counted step | the guard, before the scope bound | the guard at step 0 |
| C6 t1, t2 at the unfixed lifecycle | "0 of N census rows" | "0 of 28", both profiles (2094b5e4) |
| C6 t3 at the unfixed lifecycle | red (no late pass exists) | red at its outcome assertion, `StalledInsidePass` |
| C7 m1 console back inside the role | t1, t2 red | t1, t2, t3 red |
| C7 m2 late close without its durable pass | t2 red, t1 green | exactly that (t3, t4 red too) |
| C7 m3 late close without its close | t1 red at the closed-file probe | exactly that |
| C7 m4 late pass not refused the synchronous channel | t3 red | red: "shutdown returned only because the console stall gave up" |
| C7 m5 dead sink read as a stall | t4 red | red, `ClosedAfterStall` |
| C9 harness at the base | red at `:68` with its message | red at `:68:5`, both modes |
| C9 mutation "the sink never exits" | red at the 30 s exit assertion | red, `outcome Some(ClosedAfterStall), state Exiting` |

## Moved on purpose

Rule: "a pool build registers its builder and every worker inside `build()`".

| census | old | new |
|---|---|---|
| `boyko_ui/tests/zero_alloc.rs`, `p4_bind_zero_alloc.rs`: driver touch | 1 | 2 |
| `boyko_physics/tests/alloc_frame_attribution.rs`: driver touch; `warmed_pool` target | 1; `if workers >= 2 { workers } else { 0 }` | 2; `workers` |
| `boyko_app/tests/e1_engine_ui_alloc_census.rs`: driver touch | 1 | 2 |
| UG-02 (`docs/memory/ledger/ug02-pins.tsv`, line `CI-FIX-1-C3`) | 1681 / 3020 / 1329, pool-utils-log 177 | 1682 / 3025 / 1333, 182 (five new rows); pool sites `5 7` unchanged |

Global-allocator census (`#[global_allocator]` files that build a pool or an `App`, 30): the four rows above are
class (ii). Every other hit is class (i), whose windows open after every pool build (thread-local counters on the
calling thread, or windows inside a built schedule), or class (iii), criterion benches and size-predicate receipts
that never read an epoch size.

## Residuals and open items

- **F1's pause location is OPEN** (class (f) locally in 200 runs). Ruling 75 Q8: if the owner wants it, a temporary
  capture step is a question for the CI-run request, never added to a shipping crate.
- The Linux green of F3 and the Windows greens of F1/F2 are proved by the owner-approved CI run (PC-7).
- `StalledInsidePass` with a durable sink that never recovers is reported, not closed: nothing can write safely while
  the role is held.
- After `ClosedAfterStall` the console never receives the lines the late close delivered to the durable sinks
  (counted in `console_lines_skipped`).
- At W4 a pool's `PoolInner::workers` slice is 640/128, the collector `Global`'s size (critique O4); the class gate
  asserts `Local`s only at W > 1, and the physics guards run at W1 or on one thread.

## Quiet-window commands (the orchestrator's; cut section 7)

The lane makes no performance claim. F1's location rests on timestamps, so it is re-taken with the machine idle:

```bash
python -I D:/tmp/phys-orch/ci-fix-1/capture/f1/run_capture.py D:/tmp/phys-orch/ci-fix-1/capture/exe/log_host_editor_inst_debug.exe 100 D:/tmp/phys-orch/ci-fix-1/qw/f1_debug
python -I D:/tmp/phys-orch/ci-fix-1/capture/f1/run_capture.py D:/tmp/phys-orch/ci-fix-1/capture/exe/log_host_editor_inst_release.exe 100 D:/tmp/phys-orch/ci-fix-1/qw/f1_release
python -I D:/tmp/phys-orch/ci-fix-1/capture/f1/analyze.py D:/tmp/phys-orch/ci-fix-1/qw/f1_debug D:/tmp/phys-orch/ci-fix-1/qw/f1_release > D:/tmp/phys-orch/ci-fix-1/qw/f1_summary.md
```

Claim rule: the quiet window's majority class must match Stage 0's (f); a different majority withdraws Stage 0's
reading. The instrumented exes are c1f111f3 (debug) and 3441414e (release).
