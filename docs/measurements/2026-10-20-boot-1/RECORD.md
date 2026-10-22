# BOOT-1: the record (lane `u/boot-1`, 2026-10-20 to 2026-10-22)

The owner, 2026-10-20: games start too slowly behind a long white window. Orchestrator ruling 89 item (1) scoped
this rung: F-A (a persisted `VkPipelineCache` plus one boot telemetry line), F1 (a dark window from the first
moment), F4 (the window answers during boot and closes during boot), and F-E (parallel pipeline creation, built
only past a pre-stated threshold). Lazy / per-config pipeline creation is out (owner V3), and so are dev-profile
changes (ruling 89 (2)). Base `b25df72405322826ed0d79ed15089b9c071408eb`, trunk-synced over PX1a (`290f38b3`) and
ci-fix-1 (`0af80c78`).

**Every timing number below is DIAGNOSTIC: the machine was not quiet** (other lanes queued on the heavy lock, the
owner at the desktop). The gates are not timing gates. They assert counts and states: hits, pumps, hung samples,
visibility. Raw receipts stay in the lane scratch `D:/tmp/boot-orch/boot1/`; each is quoted here with its sha256
prefix.

| commit | step | what it proves |
|---|---|---|
| `8c91170f` | C1: the boot-window observer (`boot_window_responsive.rs`) | an external, message-free observer of a real boot: white pixels, `IsHungAppWindow`, DWM ghost, `WM_NULL` round trips, a close at first visible |
| `bbacf08b` | C2: the stock black class brush (F1) | no pure-white client sample between window creation and the first present |
| `be4424a7` | C3: the pipeline-cache file format (host side) | a corrupt, truncated or foreign file never reaches the driver |
| `74f3b8dd` | C4: one `VkPipelineCache` per context at all five create sites, with creation feedback | a second create of the same pipeline is an application-cache hit at every site |
| `16ce7185` | C5: the per-user persisted cache and the `boot:` telemetry line | a second boot hits every pipeline, also from a copied exe (check I2) |
| `ef1f73ba` | C6: the boot pump and the deferred close (F4) | never hung during boot, a close during boot hides at once, a long startup system can pump through the seam |
| this commit | C7: feature rows, gap C9, this record | — |

F-E (C8) was **not built**: FE-0 missed its threshold (below).

## F1 and F4: the observer

`boot_window_responsive` spawns its own worker, a room-class scene through `EnginePlugins::window`, with a cold
pipeline path (`BOYKO_PIPELINE_CACHE=off`) and `BOYKO_WINDOW_FRAMES=30`. It samples every 20 ms from outside the
process.

| gate | red (before) | green (after) |
|---|---|---|
| `boot_client_area_never_white` | C1: `WHITE WINDOW: 1257 of 1258 boot samples had >= 95 % of the client grid at (255,255,255); first at t=9.205 s` | C2: 1 passed (56 s); the first visible sample is `grid=dark (black 25 of 25)` |
| `boot_window_never_hung` | C5 content: `398 IsHungAppWindow samples (first at 5.059 s), 397 ghost samples, 3 WM_NULL timeouts, max round trip 4509 ms` (`b65f60f3`) | C6: `WM_NULL: 30 round trips, max 1511 ms, 0 timeouts`, no hung or ghost sample (`35dd5e75`) |
| `close_during_boot_hides_at_once_and_exits_clean` | C5 content: `WM_CLOSE posted at 2.599 s, the window was still visible until 9.840 s` (`51fa4da2`) | C6: posted 3.214 s, hidden 3.602 s, child exit 0 at 9.390 s with `frames=0` (`c12fff6f`) |
| `startup_system_pumping_through_the_seam_never_hangs` (a startup system busy 6 s, pumping through `boyko_app::pump_boot_messages` every 100 ms) | C6 with the seam made a no-op: `31 IsHungAppWindow samples, 31 ghost samples, 1 WM_NULL timeouts, max round trip 4512 ms` (`2ebf0cba`) | C6: `94 round trips, max 975 ms, 0 timeouts` (`d8b2ed9f`) |

The longest unpumped stretch is now one pipeline compile or one fence wait. The slowest `WM_NULL` round trip seen
during a cold boot was 1.5 s.

**Not credited (cut D-F4b, check I1).** A game's own startup system that runs one uninterrupted CPU stretch longer
than 5 s still ghosts the window, unless it calls the seam between its work items. Moving that last piece needs an
asynchronous startup decode on the idle pool (ruling 90 Q8), a later kernel lane.

At C6 the white gate re-ran **observer-blind**: zero foreground samples, because another application held the
foreground for the whole run. The C2 exe, run as a control, was blind the same way. C6 does not touch the brush.
The re-run on a free desktop is part of the quiet-window rows below.

## F-A: the persisted pipeline cache

The runner's worker (`boot_pipeline_cache`, a room scene, three frames, its own cache directory) prints one
`BOOT1-STATS` line per boot (`8f28729d`, C6):

| boot | pipelines | app-cache hits | saved bytes | load |
|---|---|---|---|---|
| child 1, no file | 60 | 18 (in-process duplicates) | 1,959,403 to 1,959,416 | `Absent` |
| child 2, same exe | 60 | **60** | 0 (nothing missed, no write) | `Loaded` |
| child 2b, the exe copied to another path (check I2) | 60 | **60** | 0 | `Loaded` |
| `BOYKO_PIPELINE_CACHE=off` | 60 | 0 | 0, file untouched | `Off` |

Every boot reported `created_total=60`, so the frame loop created no pipeline. It also reported
`create_pumps=60 wait_pumps=12 stage_pumps=5`: one pump per pipeline creation.

Each of these was shown able to fail on a scratch mutant, restored by content afterwards. Never seeding the cache
from the file left 18 of 60 hits. Always saving made the warm boot write 1.96 MB. A file name carrying the exe's
stem left child 2b at 18 hits. Recording the stats before the late pipelines gave `created_total` 60 against 57.
A missing pump tick at one create site gave 56 pumps for 60 pipelines.

A file is rejected **before** the driver sees a byte when it is truncated, has one payload byte flipped, carries
`driver_version + 1`, or is 4 KiB of garbage or empty. The next boot then rewrites a valid file. Device test
`foreign_or_corrupt_file_is_rejected_then_rewritten`; the byte-level cases are host-proptested in
`pipeline_cache.rs`.

## F-E: FE-0, the pre-stated threshold, and the verdict

The threshold was stated before measuring (cut D-E). F-E ships only if the median wall time to compile the nine
heaviest boot compute pipelines on 4 threads is at most 0.50 x the median on 1 thread.

Protocol:
- cold: NULL cache, and a fresh `%LOCALAPPDATA%` per run, so the driver's own disk cache is cold too;
- one fresh process per run, order 1 2 4 8 repeated 3x, 30 s idle before each run;
- `RTX 3060 Laptop GPU`; every one of 108 creations returned `VK_SUCCESS` with `VALID` feedback and no hit.

Probe `fe/vk_pipeline_probe_mt.py` (`6d771a25`), driver `tools/fe0.sh` (`55dea2b3`), summary `fe/out/summary.txt`
(`1463abf3`).

| threads | wall ms (3 runs) | median | median of summed driver-reported ms |
|---|---|---|---|
| 1 | 7337.7, 6512.1, 4874.2 | 6512.1 | 6507.2 |
| 2 | 4945.6, 5474.4, 3330.3 | 4945.6 | 9765.2 |
| 4 | 4333.9, 4953.1, 2052.1 | 4333.9 | 15849.7 |
| 8 | 3870.4, 4070.8, 2125.4 | 3870.4 | 25733.1 |

**median wall(4) / median wall(1) = 0.666 > 0.50: F-E is not built.** The driver-reported time per pipeline grows
with the thread count: the summed time goes from 6.5 s at 1 thread to 15.8 s at 4. The work is not spread; it gets
longer.

The third repetition alone was faster at every N and gives 0.42. A quiet-window repeat of `tools/fe0.sh` is
therefore the only thing that could reopen F-E, and only by an orchestrator decision; the rule above stands as
measured.

## Pending: the quiet-window rows (orchestrator; cut section 5)

Each row runs under the heavy lock, in a quiet window, at the lane HEAD. Until BOOT-1 merges, every runner-based
run keeps the owner's per-user cache untouched (ruling 91 Q1):
- `BOYKO_PIPELINE_CACHE_DIR=D:/tmp/boot-orch/boot1/pc/`;
- for the drivers that scrub `BOYKO_*` (`golden.ps1`, `boot_validation_clean`), `LOCALAPPDATA` pointed into the lane
  scratch, or the per-user directory relocated (cut Q4).

The rows:
- **W1 / W2**: every golden leg cold, then warm, byte-identical.
- **W3**: the device-needing suite, plus BOOT-1's `pipeline_cache_persist`, `boot_pipeline_cache`,
  `boot_window_responsive` (on a **free desktop**, `BOYKO_WIN_HIDDEN` unset) and the `solo` leg `window_boot_pump`.
- **W4**: launch to first rendered frame and the white / dark intervals, room_smoke, before / head-cold /
  head-warm, 3 runs each.
- **W5**: the `playground` asset-decode number (gap G3), recorded once, with no fix.

Their numbers land in the next revision of this record.
