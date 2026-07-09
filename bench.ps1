<#
.SYNOPSIS
    Phase X.E reproducible benchmark protocol for boyko-engine.

.DESCRIPTION
    Runs a single criterion benchmark target N times back-to-back with the
    machine nudged toward a steady state (High process priority + affinity
    pinned to every logical core on the spawned cargo process). The goal is a
    median-of-N protocol on this noisy Windows box, where any single run can
    swing +/- 20-30%.

    This script does NOT aggregate results itself. Save a criterion baseline per
    run with -Baseline and compare with `critcmp` (cargo install critcmp), then
    take the MEDIAN of the deltas. See docs/BENCHMARKING.md for the full A/B
    protocol and the manual stabilization steps (turbo/SpeedStep, background
    load) that this script intentionally leaves to the operator.

.PARAMETER Bench
    The [[bench]] target name (e.g. "query_iter", "comparison"). Required.

.PARAMETER Package
    The cargo package that owns the bench. Defaults to "boyko-ecs".
    Use "bench-bevy-vs-boyko" for the cross-engine benches.

.PARAMETER Runs
    How many times to run the bench. Defaults to 3 (median-of-3).

.PARAMETER Filter
    Optional criterion benchmark-name filter, forwarded after `--`.

.PARAMETER Baseline
    Optional criterion baseline tag. Each run is saved as
    "<Baseline>_run<i>" via --save-baseline, so the N runs do not overwrite
    each other and can be diffed with critcmp.

.PARAMETER BenchAlloc
    Switch. Adds `--features bench-alloc`, swapping in the low-variance
    mimalloc global allocator (OFF by default; see docs/BENCHMARKING.md).

.EXAMPLE
    ./bench.ps1 -Bench query_iter

.EXAMPLE
    ./bench.ps1 -Bench comparison -Package bench-bevy-vs-boyko -Runs 5 -BenchAlloc

.EXAMPLE
    ./bench.ps1 -Bench query_iter -Baseline before -Filter query_ref_iter
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Bench,

    [string] $Package = 'boyko-ecs',

    [int] $Runs = 3,

    [string] $Filter,

    [string] $Baseline,

    [switch] $BenchAlloc
)

$ErrorActionPreference = 'Stop'

if ($Runs -lt 1) {
    throw "Runs must be >= 1 (got $Runs)."
}

# Build the static portion of the cargo argument list once. The per-run
# --save-baseline tag (if any) is appended inside the loop.
$baseArgs = @('bench', '-p', $Package, '--bench', $Bench)
if ($BenchAlloc) {
    $baseArgs += @('--features', 'bench-alloc')
}

# Criterion-side args go after the `--` separator: the optional name filter and
# the optional per-run baseline tag.
$allCoresMask = [IntPtr](([Int64]1 -shl [System.Environment]::ProcessorCount) - 1)

Write-Host ''
Write-Host '===========================================================================' -ForegroundColor Cyan
Write-Host " boyko-engine bench protocol (Phase X.E)" -ForegroundColor Cyan
Write-Host '===========================================================================' -ForegroundColor Cyan
Write-Host (" package      : {0}" -f $Package)
Write-Host (" bench        : {0}" -f $Bench)
Write-Host (" runs         : {0} (median-of-{0})" -f $Runs)
Write-Host (" allocator    : {0}" -f $(if ($BenchAlloc) { 'mimalloc (bench-alloc)' } else { 'system heap (production-honest)' }))
if ($Filter)   { Write-Host (" filter       : {0}" -f $Filter) }
if ($Baseline) { Write-Host (" baseline tag : {0}_run1..{0}_run{1}" -f $Baseline, $Runs) }
Write-Host (" cores        : {0} logical (affinity pinned on the cargo child)" -f [System.Environment]::ProcessorCount)
Write-Host ''
Write-Host ' NOTE: this script does NOT lock turbo/SpeedStep or close background' -ForegroundColor Yellow
Write-Host '       load. For repeatable clocks set the power plan max processor' -ForegroundColor Yellow
Write-Host '       state to 99% (or disable turbo in BIOS) and run nothing else.' -ForegroundColor Yellow
Write-Host '       NEVER run two bench/Miri jobs at once. See docs/BENCHMARKING.md.' -ForegroundColor Yellow
Write-Host ''

for ($i = 1; $i -le $Runs; $i++) {
    $runArgs = $baseArgs

    # Assemble criterion args (after `--`).
    $criterionArgs = @()
    if ($Filter) {
        $criterionArgs += $Filter
    }
    if ($Baseline) {
        $criterionArgs += @('--save-baseline', "$($Baseline)_run$i")
    }
    if ($criterionArgs.Count -gt 0) {
        $runArgs = $runArgs + @('--') + $criterionArgs
    }

    Write-Host '---------------------------------------------------------------------------' -ForegroundColor Green
    Write-Host (" run {0}/{1}  ->  cargo {2}" -f $i, $Runs, ($runArgs -join ' ')) -ForegroundColor Green
    Write-Host '---------------------------------------------------------------------------' -ForegroundColor Green

    # Spawn cargo as a child so we can pin priority/affinity on IT (not this
    # whole shell). -PassThru without -Wait returns the Process object before it
    # exits; we set the knobs, then block on WaitForExit. Output inherits this
    # console (no redirection), so criterion's live progress is preserved.
    $proc = Start-Process -FilePath 'cargo' -ArgumentList $runArgs -NoNewWindow -PassThru

    try {
        $proc.PriorityClass = [System.Diagnostics.ProcessPriorityClass]::High
    } catch {
        Write-Host (" (could not raise priority: {0})" -f $_.Exception.Message) -ForegroundColor Yellow
    }
    try {
        # Pin to the full set of logical cores. This keeps the scheduler from
        # migrating the bench across packages mid-sample; it does not reserve
        # cores from other processes.
        $proc.ProcessorAffinity = $allCoresMask
    } catch {
        Write-Host (" (could not set affinity: {0})" -f $_.Exception.Message) -ForegroundColor Yellow
    }

    $proc.WaitForExit()
    $code = $proc.ExitCode

    if ($code -ne 0) {
        throw "cargo bench (run $i/$Runs) failed with exit code $code."
    }

    Write-Host (" run {0}/{1} complete." -f $i, $Runs) -ForegroundColor Green
    Write-Host ''
}

Write-Host '===========================================================================' -ForegroundColor Cyan
Write-Host " all $Runs run(s) complete." -ForegroundColor Cyan
if ($Baseline) {
    Write-Host ''
    Write-Host ' Aggregate with critcmp (cargo install critcmp):' -ForegroundColor Cyan
    Write-Host ("   critcmp {0}_run1 {0}_run2 {0}_run3" -f $Baseline)
    Write-Host ''
    Write-Host ' For an A/B comparison, run this script once per side with distinct'
    Write-Host ' -Baseline tags (e.g. -Baseline before, then -Baseline after), then:'
    Write-Host '   critcmp before_run1 after_run1   # (repeat per run; take the median delta)'
}
Write-Host ''
Write-Host ' Full methodology + manual stabilization: docs/BENCHMARKING.md' -ForegroundColor Cyan
Write-Host '===========================================================================' -ForegroundColor Cyan
