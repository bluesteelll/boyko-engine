#!/usr/bin/env bash
#
# KE16 -- the timed-step driver. One PASS over the variants, interleaved.
#
# =============================================================================
# Why a script rather than the commands from the design document
# =============================================================================
#
# `docs/threadpool/KE16-DESIGN-MEASUREMENT.md` §2 already gives the three
# commands, and they are correct. This exists for the three things AROUND them
# that the campaign has been bitten by, each of which is a written finding rather
# than a tidiness preference:
#
# 1. INTERLEAVING IS THE METHOD, NOT AN OPTIMISATION. `KE16-RESULTS.md` §A.2 is
#    explicit that what closed axis A was a pass-by-pass comparison inside ONE
#    session, and §Method records that the same arm measured in two sessions
#    disagreed by more than the effect being ranked. So the loop is
#    `for pass { for variant { ... } }` and never the transpose: a variant that
#    owns a contiguous half-hour of wall clock owns that half-hour's drift too.
#
# 2. THE LOAD RECEIPT IS TAKEN BEFORE AND AFTER EVERY TIMED REGION, and it is a
#    CPU percentage plus a resident-set list -- never a process count.
#    `tasklist` reports ZERO processes on this box while PowerShell reports 268
#    (`KE16-RESULTS.md` §Instruments 3), so a receipt built on it is green from
#    an emptiness. A pass whose before/after receipts disagree is re-taken, per
#    §"Precondition on every timed step" item 2.
#
# 3. THE WITNESS IS SET FROM THE SAME STRING THAT NAMES THE BASELINE. Passing
#    `--features` does not prove the features took: `KE16_EXPECT` makes the build
#    itself refuse a mislabelled run (`src/lib.rs::ke16_check_expected_variant`).
#    Deriving both from one table entry is what keeps the criterion baseline
#    directory and the witness from ever disagreeing.
#
# =============================================================================
# Usage
# =============================================================================
#
#   bash scripts/ke16_measure.sh <pass-number> [variant-key ...]
#
# One PASS = one run number, every variant, in table order. Run it once per pass
# rather than looping here, so a pass is a checkpoint: the machine can be
# re-verified idle between passes and a contaminated pass discarded whole.
#
# Output lands in `target/ke16-measure/pass-<N>/`, one log per harness per
# variant plus the two load receipts. Criterion baselines are saved under the
# variant string, exactly as §2 prescribes.
#
# ⚠ THIS SCRIPT TIMES THINGS. Nothing else may run on the box while it does --
# including the agent that started it. That is a rule about the MACHINE, not
# about the script.
#
# ⚠⚠ AND THE SCRIPT ITSELF IS THE THING MOST LIKELY TO BREAK THAT RULE. Each
# variant is a different `--features` line, so cargo rebuilds the pool crate and
# its dependents the first time an arm is used, and this loop then measures in
# the wake of a full-core compile. MEASURED 2026-09-08: in pass 1 three of the
# four arms compiled SEVEN CRATES immediately before their timed region while
# passes 2 and 3 compiled nothing, and the reference's `bench_thread_install`
# read 36.24 / 13.36 / 8.12 ms across the three -- a monotone "speed-up" that was
# entirely this script's own compiles ending. Moving the build ahead of the load
# receipt makes the RECEIPT honest; it does not make the MACHINE quiet.
#
# ⇒ RUN `--prebuild` ONCE BEFORE PASS 1, and only then take the passes:
#
#     bash scripts/ke16_measure.sh --prebuild
#     bash scripts/ke16_measure.sh 1
#     ...
#
# A pass whose logs show any `Compiling` line is a pass taken in a compile's
# wake; the driver prints a warning when it sees one.

set -u
set -o pipefail

PASS="${1:-}"
if [ -z "$PASS" ]; then
    echo "usage: bash scripts/ke16_measure.sh --prebuild" >&2
    echo "       bash scripts/ke16_measure.sh <pass-number> [variant-key ...]" >&2
    exit 2
fi
PREBUILD=no
if [ "$PASS" = "--prebuild" ]; then
    PREBUILD=yes
    PASS=prebuild
fi
shift || true

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT_DIR="$REPO_ROOT/target/ke16-measure/pass-$PASS"
mkdir -p "$OUT_DIR"

# ---------------------------------------------------------------------------
# The variant table
# ---------------------------------------------------------------------------
#
# key | variant string (what `ke16_variant()` returns, and what the baseline is
#     | named) | the pool crate's feature list
#
# A* = `a3` and B* = `b0` are the axis verdicts already on record
# (`KE16-RESULTS.md` §A.4, §B). The W step measures `wg`, `wc` and `wgc` against
# a reference RE-TAKEN IN THE SAME PASS -- the preserved 49-row `a3+b0+w0+c0`
# table belongs to code state CS-2 and this tree is past it (stage 3b moved the
# task cell into a per-scope bump block), and §CS forbids comparing across code
# states. Re-taking the reference is what makes the W rows mean anything.
#
# STEP C IS PARAMETERISED ON W*, not hard-coded, because `c1` is defined as
# `A*+B*+W*+c1`. `KE16_WSTAR` selects it; the default is the arm the W step left
# standing. Passing the wrong one is caught by the witness, which is built from
# the same table entry as the baseline name.
WSTAR="${KE16_WSTAR:-wc}"
case "$WSTAR" in
    w0) WSTAR_FEATURES='' ;;
    wc) WSTAR_FEATURES=',ke16-w-count' ;;
    *)  echo "KE16_WSTAR must be w0 or wc -- \`wg\`/\`wgc\` were eliminated at CS-4 (KE16-RESULTS.md §W)" >&2
        exit 2 ;;
esac

# ⚠⚠ `c1` AND `c1f` ARE MEASURED AS A PAIR, and that DEPARTS from §7 Steps W/C
# rule 3 ("`c1` not kept => `c1f` is not measured"). The departure is a decision
# taken on a number, and it is recorded rather than silently applied.
#
# Rule 3's premise was that `c1` is App-4's mechanism and `c1f` a width
# refinement on top of it. `src/scope.rs::Scope::wake_for_wave` states the
# opposite about a build with `ke16-c-batch` ALONE: the wave's ONE wake decision
# activates the spawner plus ONE sibling, and what turns that into a wave's worth
# of lanes is `worker::wake_after_residue` -- which is `ke16-w-gate`'s and a no-op
# without it. So `c1` alone is not "App-4's accounting"; it is App-4's accounting
# MINUS up to W/2 lanes.
#
# The design escalated the resulting fork ("measured on top of a fan-out
# mechanism, or fan out unconditionally -- a DESIGN call"), offering two ways to
# make a `c1` row interpretable: over `ke16-w-gate`'s cascade, or over
# `ke16-w-fanout`. **THE W STEP DELETED THE FIRST**: `wg` is eliminated at CS-4,
# capping the pool at 5-6 of 16 lanes for a 2-6x loss, so a `c1` row taken over
# it would describe a configuration that will never ship. Only `c1f` remains as a
# wake mechanism a `c1` row can be read against.
#
# Applying rule 3 literally would therefore file a REJECTION OF APP-4 that is
# really a rejection of the wake collapse, and forbid measuring the one arm that
# could separate them. Both are taken; the C verdict is read off `c1f`, and `c1`
# is recorded as the isolated cost of the wake collapse rather than as a
# candidate.
VARIANT_KEYS=(w0 wg wc wgc)
variant_string() {
    case "$1" in
        w0)  echo 'a3+b0+w0+c0'  ;;
        wg)  echo 'a3+b0+wg+c0'  ;;
        wc)  echo 'a3+b0+wc+c0'  ;;
        wgc) echo 'a3+b0+wgc+c0' ;;
        c0)  echo "a3+b0+$WSTAR+c0"  ;;
        c1)  echo "a3+b0+$WSTAR+c1"  ;;
        c1f) echo "a3+b0+$WSTAR+c1f" ;;
        # The axis-A RE-JUDGEMENT that fixing defect B forces. See the block below.
        ship)  echo "a3+b0+$WSTAR+c0"    ;;
        a1f)   echo "a1f+b0+$WSTAR+c0"   ;;
        a1fb1) echo "a1f+b1+$WSTAR+c0"   ;;
        a1fb3) echo "a1f+b3+$WSTAR+c0"   ;;
        *)   echo '' ;;
    esac
}
variant_features() {
    case "$1" in
        w0)  echo 'ke16-a3' ;;
        wg)  echo 'ke16-a3,ke16-w-gate' ;;
        wc)  echo 'ke16-a3,ke16-w-count' ;;
        wgc) echo 'ke16-a3,ke16-w-gate,ke16-w-count' ;;
        c0)  echo "ke16-a3$WSTAR_FEATURES" ;;
        c1)  echo "ke16-a3$WSTAR_FEATURES,ke16-c-batch" ;;
        # `ke16-w-fanout` implies `ke16-c-batch` in the manifest; both are spelled
        # so the feature line and the witness `c1f` are legible side by side.
        c1f) echo "ke16-a3$WSTAR_FEATURES,ke16-c-batch,ke16-w-fanout" ;;

        # === The axis-A re-judgement `KE16-DESIGN-B4.md` BLOCKING 3 owes =======
        #
        # The register gives `b1`/`b3` a SECOND return trigger
        # (`KE16-REJECTED.md:359-362`): they return "if defect B is fixed by some
        # other route that gives the worker joiner a registered destination
        # deque" — which is exactly what B4-1 would be. They build only over
        # `a1`/`a1f`, so what returns with them is `a1f+b1`, and the axis-A
        # closure was decided against `a1f+b0`.
        #
        # !! THE MEASUREMENT DOES NOT NEED B4 TO EXIST. `a1f` and `b1` are both
        # already implemented arms, so "does `a1f+b1` beat the shipped `a3`?" is
        # answerable today, with no new code. B4's design treats this re-run as a
        # price paid AFTER writing the remedy; taking it first turns a design
        # blocker into a measured fact, and decides whether B4-1 is a local fix
        # (return row discharged with a number) or whether the axis-A closure is
        # void (a finding larger than the remedy).
        #
        # `wc` rides on all three arms because it is the shipped W and measured a
        # tie everywhere, so it is neutral to the comparison and keeps every arm
        # at the configuration that would actually ship. `a1f` alone is carried to
        # separate `b1`'s contribution from `a1f`'s.
        ship)  echo "ke16-a3$WSTAR_FEATURES" ;;
        a1f)   echo "ke16-a1-fifo$WSTAR_FEATURES" ;;
        a1fb1) echo "ke16-a1-fifo,ke16-b1$WSTAR_FEATURES" ;;
        # Step B's remaining arm: `b1`'s worker joiner plus an EXTERNAL joiner
        # that never helps — it snoozes and parks. Reachable only since `a1f`
        # returned; `b1`/`b3` are mutually exclusive and neither builds over `a3`.
        #
        # §7 Step B rule 2 decides `b3` vs `b1` on the DISPATCHER-route 100 us and
        # 1 ms x 4W cells, calling them "the fontbake shape", and that citation is
        # sound: `boyko_fontbake/src/msdf/distance.rs::pick_band_rows` sets
        # `target_bands = workers * 4`, so the MSDF bake dispatches exactly 4W
        # tasks. This is the case Step A rule 3's 64W column was NOT — a width a
        # production dispatcher really emits, named in the source rather than
        # assumed — so the rule stands as written and this arm is judged by it.
        a1fb3) echo "ke16-a1-fifo,ke16-b3$WSTAR_FEATURES" ;;
        *)   echo '' ;;
    esac
}

if [ "$#" -gt 0 ]; then
    VARIANT_KEYS=("$@")
fi

# ---------------------------------------------------------------------------
# Load receipt
# ---------------------------------------------------------------------------
#
# CPU percentage over three one-second samples plus every process holding more
# than 200 MB, which is §"Precondition" item 2's "cheapest sufficient form", and
# `available_parallelism` beside it. Written to a file rather than echoed so the
# before/after pair can be diffed mechanically.
load_receipt() {
    local label="$1" file="$2"
    {
        echo "=== load receipt: $label ==="
        date -u +"%Y-%m-%dT%H:%M:%SZ"
        powershell -NoProfile -Command '
            $s = Get-Counter "\Processor(_Total)\% Processor Time" -SampleInterval 1 -MaxSamples 3 -ErrorAction SilentlyContinue
            if ($s) { "cpu_percent: " + (($s.CounterSamples | ForEach-Object { [math]::Round($_.CookedValue,1) }) -join ", ") }
            "logical_processors: " + [Environment]::ProcessorCount
            "process_count: " + (Get-Process).Count
            "resident_over_200MB:"
            Get-Process | Where-Object { $_.WorkingSet -gt 200MB } |
                Sort-Object WorkingSet -Descending |
                ForEach-Object { "  {0,-24} {1,6} MB" -f $_.Name, [math]::Round($_.WorkingSet/1MB) }
        ' 2>/dev/null
    } > "$file"
    cat "$file"
}

# ---------------------------------------------------------------------------
# Drive
# ---------------------------------------------------------------------------

export PATH="$HOME/.cargo/bin:$PATH"
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable-x86_64-pc-windows-gnu}"

# ⚠ RUSTFLAGS IS NEVER SET HERE. `.cargo/config.toml` carries the ISA baseline on
# `[target.x86_64-pc-windows-gnu] rustflags`, and the environment variable
# REPLACES that value rather than extending it -- which silently drops the
# `-C target-cpu=x86-64-v3` every number in this campaign was taken under.

echo "=== KE16 pass $PASS : ${VARIANT_KEYS[*]} ==="
echo "toolchain: $(rustc --version)"
echo "output:    $OUT_DIR"

# ---------------------------------------------------------------------------
# Prebuild: every arm, every harness, before ANY pass is taken
# ---------------------------------------------------------------------------
#
# The whole point is that after this returns, no `cargo bench` in any later pass
# has anything left to compile, so no timed region sits in a compile's wake.
if [ "$PREBUILD" = yes ]; then
    rc=0
    for key in "${VARIANT_KEYS[@]}"; do
        features="$(variant_features "$key")"
        prefixed="$(echo "$features" | sed 's/[^,]*/boyko-threadpool\/&/g')"
        echo "--- prebuilding $key ($features) ---"
        cargo bench -p boyko-threadpool --bench ke16_nested_scope      --features "$features" --no-run || rc=1
        cargo bench -p boyko-ecs        --bench ke16_par_iter_in_system --features "$prefixed" --no-run || rc=1
        cargo bench -p boyko-physics    --bench ke16_solve_in_system    --features "$prefixed" --no-run || rc=1
    done
    if [ "$rc" -eq 0 ]; then
        echo ""
        echo "prebuild complete. Let the box settle before pass 1 -- a compile's aftermath (indexer,"
        echo "AV scan of the fresh binaries, page-cache churn) outlives the compile itself."
    fi
    exit "$rc"
fi

failures=0

for key in "${VARIANT_KEYS[@]}"; do
    variant="$(variant_string "$key")"
    features="$(variant_features "$key")"
    if [ -z "$variant" ]; then
        echo "unknown variant key \`$key\` -- the table above lists the four this step defines" >&2
        exit 2
    fi

    # ⚠⚠ THE CODE STATE IS PART OF THE BASELINE NAME, AND IT WAS ADDED AFTER A
    # LOSS. `KE16-DESIGN-MEASUREMENT.md` §2 prescribes `<V>-r<N>`, keyed on the
    # variant string ALONE -- and the variant string says nothing about which
    # code state produced it. Criterion's `--save-baseline` overwrites in place,
    # so re-taking the `a3+b0+w0+c0` reference at a new code state SILENTLY
    # DESTROYS the old state's raw samples under the identical directory name.
    # MEASURED 2026-09-08: passes 1-3 of the post-3b re-take overwrote
    # `a3+b0+w0+c0-r1`, `-r2` and `-r3` of the preserved CS-2 reference; `-r4`
    # and `-r5` survived only because that pass stopped at three. The medians and
    # bands live on in `KE16-RESULTS.md` §W, so the FINDINGS survive -- the raw
    # per-sample data behind three of the five runs does not, and criterion
    # baselines are not rebuildable from anything in the tree.
    #
    # This is §CS's rule ("numbers from different code states must not be
    # compared") appearing one layer down, in STORAGE rather than in analysis:
    # the naming scheme could not express the distinction the document forbids
    # collapsing. The short HEAD hash makes the distinction structural, so two
    # code states can never address one directory.
    # ⚠ DERIVED FROM THE LAST COMMIT THAT TOUCHED `crates/`, not from HEAD. HEAD
    # moves on documentation commits too, and this campaign writes a lot of them:
    # labelling by HEAD splits ONE code state across several directory names,
    # which is the opposite defect to the collision this field was added to fix
    # and just as misleading — a reader comparing two labels would think the code
    # differed. `-- crates/` is the whole build input for every bench here.
    CODE_STATE="${KE16_CODE_STATE:-$(git -C "$REPO_ROOT" log -1 --format=%h -- crates/)}"
    baseline="$variant-$CODE_STATE-r$PASS"
    echo ""
    echo "=== pass $PASS / variant $variant (features: $features) ==="

    # The consumers take the feature through the pool crate, so the switch the
    # callers read (`KE16_SPAWN_BATCH`, and the W arms' own `cfg`s) is THIS
    # crate's and not a pass-through of their own.
    prefixed="$(echo "$features" | sed 's/[^,]*/boyko-threadpool\/&/g')"

    # ALL THREE HARNESSES ARE COMPILED BEFORE THE LOAD RECEIPT IS TAKEN, and
    # that ordering is the point rather than an optimisation. A compile
    # saturates every core; a receipt taken with one still draining describes
    # the compile, and the protocol's before/after pair would then disagree for
    # a reason that has nothing to do with the machine's other occupants. Build
    # failures are counted and the variant is SKIPPED -- a configuration that
    # does not build produces no number, and must not silently produce one from
    # a stale binary.
    build_failed=no
    cargo bench -p boyko-threadpool --bench ke16_nested_scope --features "$features" \
        --no-run > "$OUT_DIR/$key-build.log" 2>&1 || build_failed=yes
    cargo bench -p boyko-ecs --bench ke16_par_iter_in_system --features "$prefixed" \
        --no-run >> "$OUT_DIR/$key-build.log" 2>&1 || build_failed=yes
    cargo bench -p boyko-physics --bench ke16_solve_in_system --features "$prefixed" \
        --no-run >> "$OUT_DIR/$key-build.log" 2>&1 || build_failed=yes
    if [ "$build_failed" = yes ]; then
        echo "  BUILD FAILED -- see $OUT_DIR/$key-build.log; variant skipped, NOT recorded as a number"
        failures=$((failures + 1))
        continue
    fi

    # A `Compiling` line here means this arm's timed region is about to run in the
    # wake of a full-core compile. That is not fatal -- the numbers are still
    # taken -- but it is the difference between a pass that measures the arm and
    # one that measures the machine recovering, so it is said out loud and
    # written into the pass directory rather than left in a log nobody re-reads.
    if grep -q '^ *Compiling' "$OUT_DIR/$key-build.log"; then
        n=$(grep -c '^ *Compiling' "$OUT_DIR/$key-build.log")
        echo "  ⚠ COMPILED $n crate(s) just now -- this timed region follows a full-core build."
        echo "    Run \`bash scripts/ke16_measure.sh --prebuild\` before pass 1 and let the box settle."
        echo "$key: compiled $n crate(s) immediately before its timed region" >> "$OUT_DIR/DIRTY-PASS.txt"
    fi

    load_receipt "before $variant" "$OUT_DIR/$key-load-before.txt"

    export KE16_EXPECT="$variant"

    # Pool grid + the park_timeout rows.
    cargo bench -p boyko-threadpool --bench ke16_nested_scope --features "$features" \
        -- --save-baseline "$baseline" --noplot > "$OUT_DIR/$key-pool.log" 2>&1
    pool_exit=$?

    cargo bench -p boyko-ecs --bench ke16_par_iter_in_system --features "$prefixed" \
        -- --save-baseline "$baseline" --noplot > "$OUT_DIR/$key-ecs.log" 2>&1
    ecs_exit=$?

    cargo bench -p boyko-physics --bench ke16_solve_in_system --features "$prefixed" \
        -- --save-baseline "$baseline" --noplot > "$OUT_DIR/$key-physics.log" 2>&1
    phys_exit=$?

    unset KE16_EXPECT

    load_receipt "after $variant" "$OUT_DIR/$key-load-after.txt"

    # The witness is read off the logs rather than trusted: a bench that never
    # printed its variant line did not run the arm the baseline is named after.
    for harness in pool ecs physics; do
        if ! grep -q "$variant" "$OUT_DIR/$key-$harness.log"; then
            echo "  ⚠ $harness log does not name \`$variant\` -- the witness did not print, so this is NOT a recorded number"
            failures=$((failures + 1))
        fi
    done

    echo "  exits: pool=$pool_exit ecs=$ecs_exit physics=$phys_exit"
    if [ "$pool_exit" -ne 0 ] || [ "$ecs_exit" -ne 0 ] || [ "$phys_exit" -ne 0 ]; then
        failures=$((failures + 1))
    fi
done

echo ""
echo "=== pass $PASS complete; $failures problem(s) ==="
echo "Load receipts must be compared before/after per variant: a pass whose pair disagrees is re-taken."
[ "$failures" -eq 0 ]
