#!/usr/bin/env bash
#
# KE16 M2w -- the negative-control driver. Its success is FOUR ABORTS.
#
# POSIX sibling of `scripts/tb_neg_gate.ps1`; the two must stay in step, and
# `crates/boyko_threadpool/tests/tb_neg_m2w_arm_present.rs` censuses BOTH so a
# change to one that skips the other is caught.
#
# =============================================================================
# What this gate decides
# =============================================================================
#
# Stage 3b deleted the strongest clause of the scoped path's soundness argument:
# the cell used to be freed BEFORE the body ran, so at the instant the release
# RMW committed the payload allocation did not exist at all. Since 3b the cell
# lives in a `ScopeBlock` chunk that `Scope::drop` frees with `free_all`,
# immediately after the join -- inside the same window `ScopeShared`'s own
# `Box::from_raw` sits in. What replaces the lost clause is D1 (placement),
# D2/D3 (type-system facts), D5 (a reduction leaving exactly ONE function able to
# hold a protector across the release) and an EXECUTION GATE over that one
# function. This script is the negative half of that gate.
#
# The arm: under `--features tb-neg-m2w`, `Task::new_scoped` stores
# `run_scoped_neg::<F>` in `Task::execute`; `run_scoped_neg` forms
# `let cell: &ScopedCell<F> = &*ptr.cast()` and passes it as an ARGUMENT to
# `#[inline(never)] fn finish_neg<F>(_cell: &ScopedCell<F>, shared: *const ScopeShared)`,
# which performs the completion. Both are in
# `crates/boyko_threadpool/src/task/scoped.rs`. Tree Borrows protects a
# reference-typed argument for the whole call, so `finish_neg`'s frame holds a
# strong protector over chunk memory across the release RMW, and the first
# reclamation after that RMW is `free_all`.
#
# A negative control that goes GREEN has stopped being negative, so every seed
# must be red AND red for the declared reason. `4/4 red with receipts` is KE16
# exit condition 6.
#
# =============================================================================
# Four recipe decisions, each of which is load-bearing
# =============================================================================
#
# 1. ONE PROCESS PER SEED, and `-Zmiri-many-seeds` is deliberately NOT used.
#    That flag exists to demonstrate the SEED-INDEPENDENCE OF A GREEN: it runs
#    every seed in one process and reports whether all of them passed. Here
#    success is an abort, so the first seed that reports UB ends the process and
#    the remaining seeds are never interpreted -- the flag would turn "4/4 red"
#    into "at least 1/4 red" and call it a pass.
#
# 2. `-Zmiri-tree-borrows` is not optional. The protector this gate hunts is a
#    Tree-Borrows object; Stacked Borrows does not install it on this shape, and
#    a Stacked-Borrows run of the arm is GREEN.
#
# 3. `-Zmiri-preemption-rate=0` is not optional either. At the DEFAULT rate the
#    equivalent defect is caught on a MINORITY of seeds -- the receiver x seed
#    table in `tests/miri_scope_completion_protector.rs`'s module header measures
#    1/4 at rate 0.01 against 4/4 at rate 0.
#
# 4. MIRIFLAGS IS SPELLED IN FULL HERE, on purpose. `.cargo/config.toml` sets
#    `[env] MIRIFLAGS = "-Zmiri-tree-borrows"`, and an environment MIRIFLAGS
#    REPLACES that value rather than extending it -- the same asymmetry RUSTFLAGS
#    has against `[target.*.rustflags]`. This script never sets RUSTFLAGS, so the
#    ISA baseline in `.cargo/config.toml` survives.
#
# =============================================================================
# Why the exit code is not the verdict
# =============================================================================
#
# On the Windows workstation this campaign runs on, `cargo +nightly` resolves to
# `nightly-x86_64-pc-windows-MSVC` and dies in the LINKER with exit 1 --
# indistinguishable from "the gate is red" if the exit code is all you read. Two
# defences: the toolchain is spelled with its full triple on Windows hosts, and a
# seed counts as red only once its receipt shows cargo's own
# `Running ...tb_neg_m2w_block_reference` line, i.e. the binary was built AND
# launched. A receipt without that line is reported LAUNCH-FAILED and is NOT
# counted as red.
#
# =============================================================================
# Usage
# =============================================================================
#
#   bash scripts/tb_neg_gate.sh
#
# Override the toolchain with TB_NEG_TOOLCHAIN if the host needs a different
# triple. Receipts are written to
# `docs/threadpool/receipts/tb-neg-m2w-<seed>.stderr` and are meant to be
# COMMITTED: `tests/tb_neg_m2w_arm_present.rs` censuses their existence and
# content, so landing the block without the arm is red at census time rather than
# at review time.

set -u
set -o pipefail

# ---------------------------------------------------------------------------
# Configuration. The seed list's SPELLING is pinned by
# `tests/tb_neg_m2w_arm_present.rs` -- that census reads this file as text, so it
# can only see the seeds if they are written exactly like this. Keep the literal
# `(0 1 7 15)` intact when editing.
# ---------------------------------------------------------------------------

# The four seeds of the measured receiver x seed table in
# `crates/boyko_threadpool/tests/miri_scope_completion_protector.rs` (module
# header). Same seeds, so a red here and a green there are comparable.
SEEDS=(0 1 7 15)

FEATURE='tb-neg-m2w'
PACKAGE='boyko-threadpool'
TEST_NAME='tb_neg_m2w_block_reference'

# A bare `+nightly` is unambiguous on Linux; on a Windows host (Git Bash / MSYS)
# it resolves to MSVC and dies in the linker, so the triple is spelled in full
# there -- the same defence `tb_neg_gate.ps1` hard-codes.
case "$(uname -s 2>/dev/null || echo unknown)" in
    MINGW* | MSYS* | CYGWIN*) DEFAULT_TOOLCHAIN='+nightly-x86_64-pc-windows-gnu' ;;
    *) DEFAULT_TOOLCHAIN='+nightly' ;;
esac
TOOLCHAIN="${TB_NEG_TOOLCHAIN:-$DEFAULT_TOOLCHAIN}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RECEIPT_DIR="$REPO_ROOT/docs/threadpool/receipts"

# Matches the shipped positive-gate recipe flag for flag, plus the seed.
# `-Zmiri-ignore-leaks` is carried for parity with that recipe: this run is
# expected to abort long before any leak check, and its absence would only add a
# second, unrelated red to a receipt whose whole value is one attributable red.
MIRIFLAGS_BASE='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-preemption-rate=0'

mkdir -p "$RECEIPT_DIR"

# ---------------------------------------------------------------------------
# Receipt predicates
# ---------------------------------------------------------------------------

# any_line <receipt> <extended-regex>
any_line() {
    grep -Eq -- "$2" "$1"
}

# window_has <receipt> <anchor-regex> <needle1-regex> <needle2-regex>
#
# The needles spell a literal dot as `[.]` rather than `\.` because awk's `-v`
# assignment processes string escapes BEFORE the value becomes a regex: `\.`
# arrives as a bare `.` (awk even warns about it), which matches ANY character
# and silently loosens the receipt rule. `[.]` needs no escape in either awk or
# .NET, so `tb_neg_gate.ps1` spells it the same way and the two stay comparable.
#
# True iff some 3-line window following a line matching <anchor> contains BOTH
# needles. Pass '.' as <needle2> when only one is wanted.
#
# THREE LINES, and the number is rustc's rendering rather than a guess: a spanned
# `help:` prints the header, then `-->` with the file and line, then a `|` gutter,
# then the source snippet. So the file lands at +1 and the snippet at +3, and a
# 3-line window is exactly what is needed to pin BOTH the file and the item
# without ever reading a line number -- the rule this repository adopted after a
# citation rotted inside a file whose content had not changed.
window_has() {
    awk -v anchor="$2" -v n1="$3" -v n2="$4" '
        $0 ~ anchor { w = 3; a = 0; b = 0; next }
        w > 0 {
            if ($0 ~ n1) { a = 1 }
            if ($0 ~ n2) { b = 1 }
            w--
            if (a && b) { ok = 1 }
        }
        END { exit(ok ? 0 : 1) }
    ' "$1"
}

# ---------------------------------------------------------------------------
# The arm's protector site, DERIVED from the source on every run
# ---------------------------------------------------------------------------
#
# !! THIS REPLACES A PREDICATE THAT COULD NEVER MATCH, and the first four
# receipts are what proved it. The original grep 3 required the arm's function
# NAME inside the 3-line window after `the protected tag ... was created here`,
# on the reasoning documented above: a spanned `help:` renders header, `-->`,
# gutter, snippet, so the snippet carrying `finish_neg` lands at +3. MEASURED
# 2026-09-08, all four seeds: Miri renders these two `help:` sub-diagnostics with
# the location line ONLY -- no gutter and no snippet -- so `finish_neg` appears
# NOWHERE in the receipt (`grep -c finish_neg` = 0) while the property itself is
# reported exactly as designed. The gate read `protector=no` on 4/4 seeds that
# were red for precisely the declared reason.
#
# What is available in the receipt is the location: `scoped.rs:303:17`, the
# column of `_cell` in the arm's signature. So the check becomes an IDENTITY
# check on that location -- and the line number is DERIVED from the source here,
# never written down. That keeps the property the "no line numbers" rule above
# exists to protect: a citation rots when the file moves under a number frozen in
# another file, and a number recomputed from the file on every run cannot. If the
# signature moves, this derivation moves with it; if it stops being unique, the
# gate refuses to run rather than quietly matching the wrong one.
ARM_SOURCE="$REPO_ROOT/crates/boyko_threadpool/src/task/scoped.rs"
ARM_SIGNATURE='_cell: &ScopedCell<'
CHUNK_SOURCE="$REPO_ROOT/crates/boyko_threadpool/src/block.rs"
CHUNK_ALLOC='unsafe { alloc(layout) }'

derive_unique_line() {
    # derive_unique_line <file> <fixed-string> <what> -> prints the line number
    local file="$1" needle="$2" what="$3" hits
    if [ ! -f "$file" ]; then
        echo "tb-neg-m2w: ABORT -- $what's source \`$file\` does not exist, so the receipt rule cannot be built" >&2
        exit 2
    fi
    hits=$(grep -c -F -- "$needle" "$file")
    if [ "$hits" -ne 1 ]; then
        echo "tb-neg-m2w: ABORT -- \`$needle\` occurs $hits times in \`$file\`, and $what must be UNIQUE for the receipt rule to identify it. Zero means the arm was renamed or deleted; more than one means the rule would accept a protector born at the wrong site." >&2
        exit 2
    fi
    grep -n -F -- "$needle" "$file" | cut -d: -f1
}

ARM_LINE="$(derive_unique_line "$ARM_SOURCE" "$ARM_SIGNATURE" 'the arm protector site')"
CHUNK_LINE="$(derive_unique_line "$CHUNK_SOURCE" "$CHUNK_ALLOC" 'the chunk allocation site')"

echo "tb-neg-m2w: receipt rule derived from source -- protector must be born at scoped.rs:$ARM_LINE (\`$ARM_SIGNATURE\`), accessed tag at block.rs:$CHUNK_LINE (\`$CHUNK_ALLOC\`)"

# ---------------------------------------------------------------------------
# Drive
# ---------------------------------------------------------------------------

red_count=0
total=${#SEEDS[@]}
summary=()

for seed in "${SEEDS[@]}"; do
    receipt="$RECEIPT_DIR/tb-neg-m2w-$seed.stderr"

    echo ""
    echo "=== tb-neg-m2w seed $seed ==="
    echo "MIRIFLAGS=$MIRIFLAGS_BASE -Zmiri-seed=$seed"
    echo "cargo $TOOLCHAIN miri test -p $PACKAGE --features $FEATURE --test $TEST_NAME -- --include-ignored --nocapture"

    # stderr to the receipt, stdout to the console: the receipt is stderr ONLY,
    # because that is the stream Miri's diagnostics and cargo's `Running` line
    # use, and mixing libtest's stdout into it would make the committed receipt
    # depend on interleaving.
    (
        cd "$REPO_ROOT" || exit 127
        MIRIFLAGS="$MIRIFLAGS_BASE -Zmiri-seed=$seed" \
            cargo "$TOOLCHAIN" miri test \
            -p "$PACKAGE" \
            --features "$FEATURE" \
            --test "$TEST_NAME" \
            -- --include-ignored --nocapture
    ) 2>"$receipt"
    exit_code=$?

    # The launch discriminator: cargo prints `     Running tests/<name>.rs (...)`
    # to stderr once the binary exists and is being executed. Without it the
    # process died in resolution, compilation or the linker, and its exit 1 says
    # nothing about the property.
    launched=no
    any_line "$receipt" "Running[[:space:]]+.*$TEST_NAME" && launched=yes

    # A linker death is named explicitly so the report says WHICH failure it was
    # rather than only that the receipt was unattributable.
    link_death=no
    any_line "$receipt" 'error: linking with|link\.exe|LNK[0-9]{4}|undefined reference|error: could not compile' && link_death=yes

    # Receipt grep 1 -- a non-zero exit AND a UB report. Both, because either
    # alone is satisfied by something that is not this property: a panic exits
    # non-zero without a UB report, and a UB report cannot appear without one.
    ub=no
    any_line "$receipt" 'error: Undefined Behavior:' && ub=yes

    # Receipt grep 2 -- the DECLARED diagnostic kind. `deallocation through <TAG>
    # ... is forbidden` is the protector rule; any other UB kind (a data race, a
    # use-after-free, an alignment fault) would be a red for a different reason
    # and must not discharge this gate.
    kind=no
    any_line "$receipt" 'deallocation through <[^>]+>.*is forbidden' && kind=yes

    # Receipt grep 3 -- the protected tag was born at THE ARM'S SIGNATURE.
    #
    # A Tree-Borrows protector is installed by the fn-entry retag of a
    # reference-typed PARAMETER, so the span rustc prints for "the protected tag
    # ... was created here" is the parameter of the CALLEE -- `finish_neg`'s
    # `_cell` -- not the `&*ptr.cast()` in `run_scoped_neg` that produced the
    # value. The name is not in the receipt to grep for (see the derivation
    # block above), so the location is what identifies it, and `$ARM_LINE` was
    # derived from the arm's own source a few lines ago. `scoped.rs` alone would
    # be too weak: that file carries the shipped scoped path as well, and a
    # protector born in it anywhere would discharge a gate that is about ONE
    # function.
    protector=no
    window_has "$receipt" 'the (strongly )?protected tag <[^>]+> was created here' \
        "scoped[.]rs:$ARM_LINE:" '.' && protector=yes

    # Receipt grep 4 -- the ACCESSED tag was born at the CHUNK ALLOCATION.
    #
    # This identifies the freed allocation's CLASS as a bump CHUNK rather than
    # the `ScopeShared` box: both are freed inside the same window since 3b, and
    # only the chunk's creation site is `ScopeBlock::grow`. `block.rs` alone was
    # the weaker form of the same idea -- that file also allocates in its own
    # test harness -- so the line is derived and pinned exactly as the arm's is.
    accessed=no
    window_has "$receipt" 'the accessed tag <[^>]+> was created here' \
        "block[.]rs:$CHUNK_LINE:" '.' && accessed=yes

    # Receipt grep 5 -- the DEALLOCATING FRAME is the post-join release.
    #
    # New in the tightening, and it is not redundant with grep 4: grep 4 says
    # WHAT was freed, this says WHO freed it. The whole claim of stage 3b's
    # replacement soundness argument is that the chunk dies in `free_all` called
    # from `Scope::drop` -- immediately after the join, inside the window
    # `ScopeShared`'s own `Box::from_raw` sits in. A UB report naming the right
    # allocation from some other reclamation path would be a different finding
    # wearing this gate's receipt.
    freed_by=no
    if any_line "$receipt" 'boyko_threadpool::block::ScopeBlock::free_all' \
        && any_line "$receipt" 'as std::ops::Drop>::drop'; then
        freed_by=yes
    fi

    if [ "$exit_code" -ne 0 ] && [ "$launched" = yes ] && [ "$ub" = yes ] \
        && [ "$kind" = yes ] && [ "$protector" = yes ] && [ "$accessed" = yes ] \
        && [ "$freed_by" = yes ]; then
        red_count=$((red_count + 1))
        summary+=("seed $seed : RED, attributed (exit=$exit_code)")
        echo "seed $seed : RED, attributed (exit=$exit_code)"
    elif [ "$launched" = no ]; then
        note=""
        [ "$link_death" = yes ] && note=" (linker death detected)"
        summary+=("seed $seed : LAUNCH-FAILED$note")
        echo "seed $seed : LAUNCH-FAILED -- the binary never ran; exit=$exit_code is NOT a verdict$note"
    else
        summary+=("seed $seed : NOT RED FOR THE DECLARED REASON (exit=$exit_code ub=$ub kind=$kind protector=$protector accessed=$accessed freed_by=$freed_by)")
        echo "seed $seed : NOT RED FOR THE DECLARED REASON (exit=$exit_code ub=$ub kind=$kind protector=$protector accessed=$accessed freed_by=$freed_by)"
    fi
done

# ---------------------------------------------------------------------------
# Verdict
# ---------------------------------------------------------------------------

echo ""
echo "=== tb-neg-m2w receipt table ==="
for row in "${summary[@]}"; do
    echo "  $row"
done
echo "red on $red_count/$total seeds"

if [ "$red_count" -eq "$total" ]; then
    echo "tb-neg-m2w: PASS -- $total/$total seeds red for the declared reason. KE16 exit condition 6 discharged; receipts in $RECEIPT_DIR"
    exit 0
fi

# A partial result FAILS. It does not pass with a note: the value of a negative
# control is that the deciding configuration is UB on every seed, and "red on 2
# of 4" is exactly the signature of an unarmed recipe (the free landing outside
# the release window on the other two), which is the state this gate exists to
# make visible.
echo "tb-neg-m2w: FAIL -- red on $red_count/$total seeds, and a partial result is a FAILURE, not a pass with a note."
echo "A seed that is not red for the declared reason means one of: the arm is not built (read the receipt for 'arm not built'); the recipe is unarmed, so the chunk free never landed inside a completer's release window (read the KE16-TB-NEG-M2W-CENSUS line's block_overlaps=); the run never launched (LAUNCH-FAILED -- check the toolchain triple and the linker); or M2w itself is refuted, which is the finding this campaign would most want to hear about."
exit 1
