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

set -u
set -o pipefail

PASS="${1:-}"
if [ -z "$PASS" ]; then
    echo "usage: bash scripts/ke16_measure.sh <pass-number> [variant-key ...]" >&2
    exit 2
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
# Step C is deliberately ABSENT: `c1` is defined as `A*+B*+W*+c1`, so it cannot
# be built until the W verdict names W*. Adding it here would be a guess.
VARIANT_KEYS=(w0 wg wc wgc)
variant_string() {
    case "$1" in
        w0)  echo 'a3+b0+w0+c0'  ;;
        wg)  echo 'a3+b0+wg+c0'  ;;
        wc)  echo 'a3+b0+wc+c0'  ;;
        wgc) echo 'a3+b0+wgc+c0' ;;
        *)   echo '' ;;
    esac
}
variant_features() {
    case "$1" in
        w0)  echo 'ke16-a3' ;;
        wg)  echo 'ke16-a3,ke16-w-gate' ;;
        wc)  echo 'ke16-a3,ke16-w-count' ;;
        wgc) echo 'ke16-a3,ke16-w-gate,ke16-w-count' ;;
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

failures=0

for key in "${VARIANT_KEYS[@]}"; do
    variant="$(variant_string "$key")"
    features="$(variant_features "$key")"
    if [ -z "$variant" ]; then
        echo "unknown variant key \`$key\` -- the table above lists the four this step defines" >&2
        exit 2
    fi

    baseline="$variant-r$PASS"
    echo ""
    echo "=== pass $PASS / variant $variant (features: $features) ==="

    # The build is NOT timed and is deliberately done before the load receipt:
    # a compile saturates every core, and a receipt taken with one in flight
    # describes the compile rather than the bench.
    if ! cargo build --release -p boyko-threadpool --features "$features" \
        > "$OUT_DIR/$key-build.log" 2>&1; then
        echo "  BUILD FAILED -- see $OUT_DIR/$key-build.log; variant skipped, NOT recorded as a number"
        failures=$((failures + 1))
        continue
    fi

    load_receipt "before $variant" "$OUT_DIR/$key-load-before.txt"

    export KE16_EXPECT="$variant"

    # Pool grid + the park_timeout rows.
    cargo bench -p boyko-threadpool --bench ke16_nested_scope --features "$features" \
        -- --save-baseline "$baseline" --noplot > "$OUT_DIR/$key-pool.log" 2>&1
    pool_exit=$?

    # The consumers take the feature through the pool crate, so the switch the
    # callers read (`KE16_SPAWN_BATCH`, and the W arms' own `cfg`s) is THIS
    # crate's and not a pass-through of their own.
    prefixed="$(echo "$features" | sed 's/[^,]*/boyko-threadpool\/&/g')"

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
