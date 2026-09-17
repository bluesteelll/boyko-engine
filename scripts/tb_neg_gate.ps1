#Requires -Version 5.1
#
# KE16 M2w -- the negative-control driver. Its success is FOUR ABORTS.
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
# which performs the completion. Both are in `crates/boyko_threadpool/src/task/scoped.rs`.
# Tree Borrows protects a reference-typed argument for the whole call, so
# `finish_neg`'s frame holds a strong protector over chunk memory across the
# release RMW, and the first reclamation after that RMW is `free_all`.
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
#    into "at least 1/4 red" and call it a pass. One process per seed is the only
#    way to observe four independent aborts.
#
# 2. `-Zmiri-tree-borrows` is not optional. The protector this gate hunts is a
#    Tree-Borrows object; Stacked Borrows does not install it on this shape, and
#    a Stacked-Borrows run of the arm is GREEN.
#
# 3. `-Zmiri-preemption-rate=0` is not optional either. At the DEFAULT rate the
#    equivalent defect is caught on a MINORITY of seeds -- the receiver x seed
#    table in `tests/miri_scope_completion_protector.rs`'s module header measures
#    1/4 at rate 0.01 against 4/4 at rate 0. A gate that fires on one seed in four
#    cannot tell an unarmed recipe from a sound one.
#
# 4. MIRIFLAGS IS SPELLED IN FULL HERE, on purpose. `.cargo/config.toml` sets
#    `[env] MIRIFLAGS = "-Zmiri-tree-borrows"`, and an environment MIRIFLAGS
#    REPLACES that value rather than extending it -- the same asymmetry `RUSTFLAGS`
#    has against `[target.*.rustflags]`. Every flag the run needs, including
#    tree-borrows, is therefore re-stated below. The ISA baseline in
#    `[target.x86_64-pc-windows-gnu].rustflags` is untouched because this script
#    never sets RUSTFLAGS.
#
# =============================================================================
# Why the exit code is not the verdict
# =============================================================================
#
# A build or link failure exits 1 exactly like a red gate does -- indistinguishable
# from "the gate is red" if the exit code is all you read. Two defences: the
# toolchain is spelled in full (`+nightly-x86_64-pc-windows-gnu`), and a seed only
# counts as red once its receipt shows cargo's own
# `Running ...tb_neg_m2w_block_reference` line, i.e. the binary was built AND
# launched. A receipt without that line is reported as LAUNCH-FAILED and is NOT
# counted as red.
#
# When this comment was written the concrete failure was that `cargo +nightly`
# resolved to `nightly-x86_64-pc-windows-msvc`, for which this box had no linker
# (MEASURED 2026-09-04). That is gone twice over: rustup's default_host_tuple was
# rewritten to gnu on 2026-09-07 13:56 (file mtime), so a bare `+nightly` now
# picks the gnu nightly, and msvc links since Build Tools 2022 was installed --
# but the toolchain stays spelled `+nightly-x86_64-pc-windows-gnu` on purpose: the
# committed receipts were produced by that nightly's miri (2026-08-20) and the
# msvc nightly's is 2026-05-29 (both MEASURED 2026-09-10). Moving the build host
# must not silently move the CHECKER under a Tree-Borrows gate.
#
# =============================================================================
# Usage
# =============================================================================
#
#   pwsh -NoProfile -File scripts/tb_neg_gate.ps1
#
# Receipts are written to `docs/threadpool/receipts/tb-neg-m2w-<seed>.stderr` and
# are meant to be COMMITTED: `crates/boyko_threadpool/tests/tb_neg_m2w_arm_present.rs`
# censuses their existence and content, so landing the block without the arm is
# red at census time rather than at review time.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# Configuration. The seed list's SPELLING is pinned by
# `tests/tb_neg_m2w_arm_present.rs` -- that census reads this file as text, so it
# can only see the seeds if they are written exactly like this. Keep the literal
# `@(0, 1, 7, 15)` intact when editing.
# ---------------------------------------------------------------------------

# The four seeds of the measured receiver x seed table in
# `crates/boyko_threadpool/tests/miri_scope_completion_protector.rs` (module
# header). Same seeds, so a red here and a green there are comparable.
$Seeds = @(0, 1, 7, 15)

$Feature   = 'tb-neg-m2w'
$Package   = 'boyko-threadpool'
$TestName  = 'tb_neg_m2w_block_reference'
$Toolchain = '+nightly-x86_64-pc-windows-gnu'

$RepoRoot   = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$ReceiptDir = Join-Path $RepoRoot 'docs/threadpool/receipts'

# Matches the shipped positive-gate recipe flag for flag, plus the seed.
# `-Zmiri-ignore-leaks` is carried for parity with that recipe: this run is
# expected to abort long before any leak check, and its absence would only add a
# second, unrelated red to a receipt whose whole value is one attributable red.
$MiriFlagsBase = @(
    '-Zmiri-tree-borrows'
    '-Zmiri-disable-isolation'
    '-Zmiri-permissive-provenance'
    '-Zmiri-ignore-leaks'
    '-Zmiri-preemption-rate=0'
)

# ---------------------------------------------------------------------------
# Receipt predicates
# ---------------------------------------------------------------------------

# Does some 3-line window following a line matching $Anchor contain EVERY regex
# in $Needles?
#
# THREE LINES, and the number is rustc's rendering rather than a guess: a spanned
# `help:` prints the header, then `-->` with the file and line, then a `|` gutter,
# then the source snippet. So the file lands at +1 and the snippet at +3, and a
# 3-line window is exactly what is needed to pin BOTH the file and the item
# without ever reading a line number -- which is the rule this repository adopted
# after a citation rotted inside a file whose content had not changed.
function Test-Window {
    param(
        [string[]] $Lines,
        [string]   $Anchor,
        [string[]] $Needles
    )

    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -notmatch $Anchor) { continue }
        $hi = [Math]::Min($i + 3, $Lines.Count - 1)
        $window = if ($hi -ge ($i + 1)) { $Lines[($i + 1)..$hi] } else { @() }
        $all = $true
        foreach ($needle in $Needles) {
            if (-not ($window -match $needle)) { $all = $false; break }
        }
        if ($all) { return $true }
    }
    return $false
}

function Test-AnyLine {
    param([string[]] $Lines, [string] $Pattern)
    return [bool]($Lines -match $Pattern)
}

# ---------------------------------------------------------------------------
# The arm's protector site, DERIVED from the source on every run
# ---------------------------------------------------------------------------
#
# !! THIS REPLACES A PREDICATE THAT COULD NEVER MATCH, and the first four
# receipts are what proved it. The original grep 3 required the arm's function
# NAME inside the 3-line window after `the protected tag ... was created here`,
# on the rendering argument documented above `Test-Window`. MEASURED 2026-09-08,
# all four seeds: Miri renders these two `help:` sub-diagnostics with the
# location line ONLY -- no gutter, no snippet -- so `finish_neg` appears NOWHERE
# in the receipt while the property is reported exactly as designed. The gate
# read `protector=no` on 4/4 seeds that were red for precisely the declared
# reason.
#
# The location IS in the receipt, so the check becomes an identity check on it,
# with the line DERIVED here and never written down -- which keeps the property
# the "no line numbers" rule above exists to protect. A citation rots when a file
# moves under a number frozen elsewhere; a number recomputed from that same file
# on every run cannot.
function Get-UniqueLine {
    param([string] $Path, [string] $Needle, [string] $What)

    if (-not (Test-Path -LiteralPath $Path)) {
        Write-Host "tb-neg-m2w: ABORT -- $What's source ``$Path`` does not exist, so the receipt rule cannot be built" -ForegroundColor Red
        exit 2
    }
    $hits = @(Select-String -LiteralPath $Path -SimpleMatch -Pattern $Needle)
    if ($hits.Count -ne 1) {
        Write-Host "tb-neg-m2w: ABORT -- ``$Needle`` occurs $($hits.Count) times in ``$Path``, and $What must be UNIQUE for the receipt rule to identify it. Zero means the arm was renamed or deleted; more than one means the rule would accept a protector born at the wrong site." -ForegroundColor Red
        exit 2
    }
    return $hits[0].LineNumber
}

$ArmSource   = Join-Path $RepoRoot 'crates/boyko_threadpool/src/task/scoped.rs'
$ChunkSource = Join-Path $RepoRoot 'crates/boyko_threadpool/src/block.rs'
$ArmLine   = Get-UniqueLine -Path $ArmSource   -Needle '_cell: &ScopedCell<'    -What 'the arm protector site'
$ChunkLine = Get-UniqueLine -Path $ChunkSource -Needle 'unsafe { alloc(layout) }' -What 'the chunk allocation site'

Write-Host "tb-neg-m2w: receipt rule derived from source -- protector must be born at scoped.rs:$ArmLine, accessed tag at block.rs:$ChunkLine"

# ---------------------------------------------------------------------------
# Drive
# ---------------------------------------------------------------------------

if (-not (Test-Path -LiteralPath $ReceiptDir)) {
    New-Item -ItemType Directory -Force -Path $ReceiptDir | Out-Null
}

$savedMiriFlags = $env:MIRIFLAGS
$results = @()

try {
    foreach ($seed in $Seeds) {
        $receipt = Join-Path $ReceiptDir "tb-neg-m2w-$seed.stderr"
        # stdout goes to a scratch file and is echoed afterwards: the receipt is
        # stderr ONLY, because that is the stream Miri's diagnostics and cargo's
        # `Running` line use, and mixing libtest's stdout into it would make the
        # committed receipt depend on interleaving.
        $stdoutTmp = Join-Path ([System.IO.Path]::GetTempPath()) "tb-neg-m2w-$seed.stdout"

        $env:MIRIFLAGS = ($MiriFlagsBase + "-Zmiri-seed=$seed") -join ' '

        $cargoArgs = @(
            $Toolchain, 'miri', 'test',
            '-p', $Package,
            '--features', $Feature,
            '--test', $TestName,
            '--', '--include-ignored', '--nocapture'
        )

        Write-Host ""
        Write-Host "=== tb-neg-m2w seed $seed ===" -ForegroundColor Cyan
        Write-Host "MIRIFLAGS=$($env:MIRIFLAGS)"
        Write-Host "cargo $($cargoArgs -join ' ')"

        $proc = Start-Process -FilePath 'cargo' `
            -ArgumentList $cargoArgs `
            -WorkingDirectory $RepoRoot `
            -NoNewWindow -PassThru `
            -RedirectStandardError $receipt `
            -RedirectStandardOutput $stdoutTmp
        $proc.WaitForExit()
        $exitCode = $proc.ExitCode

        if (Test-Path -LiteralPath $stdoutTmp) {
            Get-Content -LiteralPath $stdoutTmp | ForEach-Object { Write-Host $_ }
            Remove-Item -LiteralPath $stdoutTmp -Force -ErrorAction SilentlyContinue
        }

        $lines = @(Get-Content -LiteralPath $receipt -ErrorAction SilentlyContinue)

        # The launch discriminator: cargo prints `     Running tests/<name>.rs (...)`
        # to stderr once the binary exists and is being executed. Without it the
        # process died in resolution, compilation or the linker, and its exit 1
        # says nothing about the property.
        $launched = Test-AnyLine $lines "Running\s+.*$TestName"

        # A linker death is named explicitly so the report says WHICH failure it
        # was rather than only that the receipt was unattributable.
        $linkDeath = Test-AnyLine $lines 'error: linking with|link\.exe|LNK\d{4}|undefined reference|error: could not compile'

        # Receipt grep 1 -- a non-zero exit AND a UB report. Both, because either
        # alone is satisfied by something that is not this property: a panic exits
        # non-zero without a UB report, and a UB report cannot appear without one.
        $ub = Test-AnyLine $lines 'error: Undefined Behavior:'

        # Receipt grep 2 -- the DECLARED diagnostic kind. `deallocation through
        # <TAG> ... is forbidden` is the protector rule; any other UB kind (a data
        # race, a use-after-free, an alignment fault) would be a red for a
        # different reason and must not discharge this gate.
        $kind = Test-AnyLine $lines 'deallocation through <[^>]+>.*is forbidden'

        # Receipt grep 3 -- the protected tag was born at THE ARM'S SIGNATURE.
        #
        # A Tree-Borrows protector is installed by the fn-entry retag of a
        # reference-typed PARAMETER, so the span rustc prints for "the protected
        # tag ... was created here" is the parameter of the CALLEE --
        # `finish_neg`'s `_cell` -- not the `&*ptr.cast()` in `run_scoped_neg`
        # that produced the value. The name is not in the receipt to match on
        # (see the derivation block above), so the location is what identifies
        # it, and $ArmLine was derived from the arm's own source. `scoped.rs`
        # alone would be too weak: that file carries the shipped scoped path as
        # well, and a protector born in it anywhere would discharge a gate that
        # is about ONE function.
        $protector = Test-Window -Lines $lines `
            -Anchor 'the (strongly )?protected tag <[^>]+> was created here' `
            -Needles @("scoped[.]rs:${ArmLine}:")

        # Receipt grep 4 -- the ACCESSED tag was born at the CHUNK ALLOCATION.
        # This identifies the freed allocation's CLASS as a bump CHUNK rather
        # than the `ScopeShared` box: both are freed inside the same window since
        # 3b, and only the chunk's creation site is `ScopeBlock::grow`.
        # `block.rs` alone was the weaker form of the same idea -- that file also
        # allocates in its own test harness -- so the line is derived and pinned
        # exactly as the arm's is.
        $accessed = Test-Window -Lines $lines `
            -Anchor 'the accessed tag <[^>]+> was created here' `
            -Needles @("block[.]rs:${ChunkLine}:")

        # Receipt grep 5 -- the DEALLOCATING FRAME is the post-join release.
        # Not redundant with grep 4: that says WHAT was freed, this says WHO
        # freed it. Stage 3b's replacement soundness argument is precisely that
        # the chunk dies in `free_all` called from `Scope::drop`, inside the
        # window `ScopeShared`'s own `Box::from_raw` sits in.
        $freedBy = (Test-AnyLine $lines 'boyko_threadpool::block::ScopeBlock::free_all') `
            -and (Test-AnyLine $lines 'as std::ops::Drop>::drop')

        $red = ($exitCode -ne 0) -and $launched -and $ub -and $kind -and $protector -and $accessed -and $freedBy

        $results += [pscustomobject]@{
            Seed      = $seed
            Exit      = $exitCode
            Launched  = $launched
            LinkDeath = $linkDeath
            Ub        = $ub
            Kind      = $kind
            Protector = $protector
            Accessed  = $accessed
            FreedBy   = $freedBy
            Red       = $red
            Receipt   = $receipt
        }

        if ($red) {
            Write-Host "seed $seed : RED, attributed (exit=$exitCode)" -ForegroundColor Green
        } elseif (-not $launched) {
            Write-Host "seed $seed : LAUNCH-FAILED -- the binary never ran; exit=$exitCode is NOT a verdict$(if ($linkDeath) { ' (linker death detected)' })" -ForegroundColor Yellow
        } else {
            Write-Host "seed $seed : NOT RED FOR THE DECLARED REASON (exit=$exitCode ub=$ub kind=$kind protector=$protector accessed=$accessed freed_by=$freedBy)" -ForegroundColor Red
        }
    }
}
finally {
    if ($null -eq $savedMiriFlags) {
        Remove-Item Env:MIRIFLAGS -ErrorAction SilentlyContinue
    } else {
        $env:MIRIFLAGS = $savedMiriFlags
    }
}

# ---------------------------------------------------------------------------
# Verdict
# ---------------------------------------------------------------------------

Write-Host ""
Write-Host "=== tb-neg-m2w receipt table ===" -ForegroundColor Cyan
$results | Format-Table Seed, Exit, Launched, Ub, Kind, Protector, Accessed, Red -AutoSize

$redCount = @($results | Where-Object { $_.Red }).Count
$total    = $Seeds.Count

Write-Host "red on $redCount/$total seeds"

if ($redCount -eq $total) {
    Write-Host "tb-neg-m2w: PASS -- $total/$total seeds red for the declared reason. KE16 exit condition 6 discharged; receipts in $ReceiptDir" -ForegroundColor Green
    exit 0
}

# A partial result FAILS. It does not pass with a note: the value of a negative
# control is that the deciding configuration is UB on every seed, and "red on 2
# of 4" is exactly the signature of an unarmed recipe (the free landing outside
# the release window on the other two), which is the state this gate exists to
# make visible.
Write-Host "tb-neg-m2w: FAIL -- red on $redCount/$total seeds, and a partial result is a FAILURE, not a pass with a note." -ForegroundColor Red
Write-Host "A seed that is not red for the declared reason means one of: the arm is not built (read the receipt for 'arm not built'); the recipe is unarmed, so the chunk free never landed inside a completer's release window (read the KE16-TB-NEG-M2W-CENSUS line's block_overlaps=); the run never launched (LAUNCH-FAILED -- check the toolchain triple and the linker); or M2w itself is refuted, which is the finding this campaign would most want to hear about."
exit 1
