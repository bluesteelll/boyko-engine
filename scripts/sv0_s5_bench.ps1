<#
.SYNOPSIS
    VB-SV0 rung S5: runs the 10 GPU sessions of the VB lit-producer cost A/B and prints the
    transcription block.

.DESCRIPTION
    docs\VB-SV0-SDF-SHADOW-PLAN.md, "S5 -- measure" requires an interleaved paired A/B of the VB
    lit-producer dispatch -- SV0 armed vs sv0_mode = 0 -- on the `vb_both_sdf` fixture at 512x512,
    over matrix row 1 (`vb_resolve`, fused) and row 7 (`vb_shade_split`), with >= 30 quadruples,
    warm-up discarded, the median paired delta as the statistic, and >= 3 SESSIONS AS SEPARATE
    PROCESSES per row plus a null control.

    Warm-up is discarded at TWO levels, and the second one is why this is 10 sessions and not 8:

        row 1: WARM-UP x1 (discarded), armed x3, null x1     (BOYKO_SV0_SSAO unset)
        row 7: WARM-UP x1 (discarded), armed x3, null x1     (BOYKO_SV0_SSAO = 1)

    The first eight-session run produced a `row1_armed1` median of 549376 ns against 12800 / 13312
    for the two sessions after it -- 42x, on a protocol whose gate is 10%. The 20-frame IN-SESSION
    warm-up did not touch it and could not: sessions 2 and 3 ran the same 20 frames and agreed with
    each other, so the cold start is at PROCESS level. This script therefore burns one whole
    session per row before the three it transcribes. That session runs the SAME command line as an
    armed one -- it is discarded by BOOKKEEPING, not by a different configuration -- so it
    exercises exactly the code path the kept sessions do.

    Three properties make the discard a design rather than a widened threshold:

      1. It is discarded, NOT down-weighted. A robust statistic over more sessions would absorb the
         cold session and produce a clean-looking number; this finding exists only because the
         harness refused to absorb a disagreement it could not explain.
      2. It stays VISIBLE. Its median is transcribed into `SV0_S5_<R>_WARMUP_DELTA_NS` and its
         ratio to the kept sessions is printed here and by the Rust gate.
      3. Its TICKS still count. A cold session is invalid evidence about the TERM and perfectly
         valid evidence about the INSTRUMENT, so its `quantum_max_ns` pools with the rest -- and in
         the first run it was the cold session, with its far wider range of durations, that
         supplied the tightest lattice bound.

    Each session is its own `cargo test` process -- the failure mode the repetition guards against
    is per-process GPU clock/power state, which one process cannot resample.

    THE RESOLUTION LINE STATES BOUNDS, NOT DEVICE READINGS. `quantum_max_ns` is
    `G * gcd(observed multipliers)` for a hardware step `G`, i.e. a MULTIPLE of `G` whenever a
    session's durations cluster -- which is what a fixed-workload dispatch produces. Ten sessions
    therefore yield ten UPPER BOUNDS on one device property, and the strongest statement they
    jointly support is their GCD. This script pools them that way and prints which session backed
    the result, together with the `distinct_ticks` the bound rests on. Sessions that report
    different bounds are NOT contradicting each other; sessions that report different
    `timestamp_period_ns` / `timestamp_valid_bits` are, and that is checked separately.

    The assertions live in the CPU test `crates\boyko_app\tests\sv0_vb_term_bench.rs`, whose
    MEASURED block is `f64::NAN` until a human transcribes what this prints. This script does NOT
    write those literals. That is deliberate: a script that edits the constants a gate reads can
    manufacture a passing gate, which is the exact defect class this campaign keeps finding one
    level down. It prints a ready-to-paste block; a person pastes it.

.PARAMETER OutDir
    Where the session logs go (default D:\tmp\sv0_s5).

.PARAMETER Quads
    Per-session TIMED quadruple budget (BOYKO_SV0_S5_BENCH_QUADS). Default = the runner's own 200.
    Lower it only to smoke-test the plumbing -- the gate's floor is 30 and a short session is not
    a valid sample of this protocol.

.PARAMETER Rows
    Optional subset of {1,7}, for re-running one row.

.EXAMPLE
    powershell -File scripts\sv0_s5_bench.ps1
    cargo test -p boyko-app --test sv0_deferred_term_bench -- --nocapture
    cargo test -p boyko-app --test sv0_vb_term_bench       -- --nocapture

.EXAMPLE
    # Plumbing smoke test (NOT a valid measurement -- 8 quadruples is below the protocol floor)
    powershell -File scripts\sv0_s5_bench.ps1 -Quads 8
#>
[CmdletBinding()]
param(
    [string]$OutDir = 'D:\tmp\sv0_s5',
    [int]$Quads = 200,
    [int[]]$Rows = @(1, 7)
)

$ErrorActionPreference = 'Stop'

# Run a native executable WITHOUT letting its stderr trip `$ErrorActionPreference='Stop'`.
# Verbatim `golden.ps1` / `sv0_arm_matrix.ps1`'s helper, and for the same PowerShell 5.1 reason:
# cargo writes its normal status lines to stderr, which 'Stop' would turn into a script-terminating
# NativeCommandError even on exit code 0. The caller still gates on $LASTEXITCODE.
function Invoke-Native {
    param([Parameter(Mandatory)][string]$Exe, [string[]]$Args)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $Exe $Args } finally { $ErrorActionPreference = $prev }
}

# Pull `key=value` out of a whitespace-separated engine summary line.
function Get-Field {
    param([Parameter(Mandatory)][string]$Line, [Parameter(Mandatory)][string]$Key)
    if ($Line -match ("(?:^|\s)" + [regex]::Escape($Key) + "=([^\s]+)")) { return $Matches[1] }
    return $null
}

# Euclidean GCD. Used to POOL the per-session lattice bounds: each session's `tick_gcd` is a
# multiple of the hardware step, so the GCD of ten of them is the tightest bound the set supports.
# This is the meet of ten upper bounds -- not an average, not a vote, and not "pick one".
function Get-Gcd {
    param([Parameter(Mandatory)][long]$A, [Parameter(Mandatory)][long]$B)
    $x = [math]::Abs($A)
    $y = [math]::Abs($B)
    while ($y -ne 0) {
        $t = $x % $y
        $x = $y
        $y = $t
    }
    return $x
}

# Emit an f64 literal that COMPILES. `"$([double]1)"` renders as `1` in PowerShell, and `1` in an
# `f64` slot is a Rust type error -- a defect this script shipped once already, caught only when a
# human pasted the block.
function Format-F64 {
    param([Parameter(Mandatory)][AllowNull()]$Value)
    return ("{0:0.0}" -f [double]$Value)
}

# The row table. `Producer` is documentation here -- the Rust gate is what asserts it against the
# run log -- carried so a failure in this script names the same row the gate does.
$rowTable = @(
    @{ Index = 1; Producer = 'vb_resolve';     Label = 'fused'; Env = @{} }
    @{ Index = 7; Producer = 'vb_shade_split'; Label = 'split'; Env = @{ BOYKO_SV0_SSAO = '1' } }
)

# Every BOYKO_* this bench ever sets or could be poisoned by. Wiped before EVERY session and
# re-set from the row, so a knob left over from the previous session (or from the ambient shell)
# cannot silently select a different variant. BOYKO_SV0_FROXEL and BOYKO_VB_FORCE_CLASSIFIED are
# in the list precisely because they would select rows 2/5/6 -- rows the plan does not name, whose
# numbers would then be transcribed under row 1's constants.
$ownedEnv = @(
    'BOYKO_SV0_S5_BENCH',
    'BOYKO_SV0_S5_BENCH_NULL',
    'BOYKO_SV0_S5_BENCH_QUADS',
    'BOYKO_SV0_BENCH',
    'BOYKO_SV0_BENCH_NULL',
    'BOYKO_VB_BENCH',
    'BOYKO_SV0_MODE',
    'BOYKO_SV0_FROXEL',
    'BOYKO_SV0_SSAO',
    'BOYKO_VB_FORCE_CLASSIFIED',
    'BOYKO_HOST_DUMP',
    'BOYKO_DISABLE_VALIDATION',
    'BOYKO_SHADOW_DENOISE',
    'BOYKO_AA',
    'BOYKO_RENDER_PATH',
    'BOYKO_WIN_HIDDEN',
    'BOYKO_WINDOW_FRAMES',
    'BOYKO_FORCE_SOFTWARE',
    'BOYKO_PBR_TEXTURE_DIR'
)

if (-not (Test-Path -LiteralPath $OutDir)) {
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
}

$env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'

# --- compile ONCE, up front; ABORT on build error ---------------------------------------
# The cargo-check-skips-tests false-green is a standing trap in this repo: a fixture that fails to
# compile would otherwise surface as "the session printed nothing", which reads like a GPU problem.
#
# NO --release. Plan section 7 clause 3 divides this rung's number by S1.5's, and S1.5's runbook is
# a plain dev-profile `cargo test`; the summary line prints `debug_assertions=` and the gate pins
# it, so a release run here fails loudly rather than silently comparing two host conditions.
Write-Host "[s5] compiling test binary 'sv0_vb_term_bench' (--no-run, dev profile) ..." -ForegroundColor Cyan
Invoke-Native -Exe 'cargo' -Args @('test', '-p', 'boyko-app', '--test', 'sv0_vb_term_bench', '--no-run', '-j2')
if ($LASTEXITCODE -ne 0) {
    throw "[s5] test binary FAILED TO COMPILE - aborting before any session runs."
}

$sessions = @()
$failures = @()

foreach ($rowNo in $Rows) {
    $row = $rowTable | Where-Object { $_.Index -eq $rowNo }
    if (-not $row) { throw "[s5] no such row: $rowNo (plan section 6 S5 names rows 1 and 7)" }

    # ONE discarded warm-up session, then three ARMED, then ONE null control.
    #
    # The warm-up runs FIRST and on the armed configuration, because the whole point is that it
    # absorbs whatever the FIRST process of a set pays. Nothing about it is special-cased in the
    # engine: `Mode = 'warmup'` differs from `'armed'` only in this script's bookkeeping and in
    # which constant its number is transcribed into. Making it a distinct configuration would have
    # made it warm a code path the kept sessions do not run.
    #
    # The null runs LAST so its threshold -- a fraction of the armed median -- is already knowable
    # when it is read.
    $plan = @(
        @{ Mode = 'warmup'; Index = 1 }
        @{ Mode = 'armed';  Index = 1 }
        @{ Mode = 'armed';  Index = 2 }
        @{ Mode = 'armed';  Index = 3 }
        @{ Mode = 'null';   Index = 1 }
    )

    foreach ($s in $plan) {
        $log = Join-Path $OutDir ("row{0}_{1}{2}.log" -f $rowNo, $s.Mode, $s.Index)

        foreach ($name in $ownedEnv) { Remove-Item -Path ("Env:" + $name) -ErrorAction SilentlyContinue }
        $env:BOYKO_DISABLE_VALIDATION = '1'      # the GPU validation layer is crash-prone on this box
        $env:BOYKO_SV0_S5_BENCH = '1'
        $env:BOYKO_SV0_S5_BENCH_QUADS = "$Quads"
        if ($s.Mode -eq 'null') { $env:BOYKO_SV0_S5_BENCH_NULL = '1' }
        foreach ($k in $row.Env.Keys) { Set-Item -Path ("Env:" + $k) -Value $row.Env[$k] }

        if (Test-Path -LiteralPath $log) { Remove-Item -LiteralPath $log -Force }
        if (Test-Path -LiteralPath $log) { throw "[s5] could not delete stale '$log' - aborting." }
        $start = Get-Date

        $note = ''
        if ($s.Mode -eq 'warmup') { $note = ' [DISCARDED - process-level cold start]' }
        Write-Host ("[s5] row {0} ({1}) {2} session {3}{4} ... " -f $rowNo, $row.Producer, $s.Mode, $s.Index, $note) -ForegroundColor Cyan

        # Routed through cmd so the merged stdout+stderr lands in the log verbatim: PS 5.1 wraps
        # native stderr into ErrorRecords, which would mangle the `VB lit producer = ...` line the
        # gate reads. `--nocapture` is what lets the engine's own println!/eprintln! through.
        $cmdLine = 'cargo test -p boyko-app --test sv0_vb_term_bench sv0_vb_term_bench' +
                   ' -j2 -- --ignored --test-threads=1 --nocapture > "' + $log + '" 2>&1'
        Invoke-Native -Exe 'cmd' -Args @('/c', $cmdLine)
        if ($LASTEXITCODE -ne 0) {
            if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Tail 25 | Write-Host }
            $failures += ("row {0} {1}{2}: the session returned {3} (log: {4})" -f $rowNo, $s.Mode, $s.Index, $LASTEXITCODE, $log)
            continue
        }

        # --- fresh-artifact guard (the campaign's false-fresh trap, at a new surface) ------
        if (-not (Test-Path -LiteralPath $log)) {
            $failures += ("row {0} {1}{2}: no log at {3}" -f $rowNo, $s.Mode, $s.Index, $log)
            continue
        }
        if ((Get-Item -LiteralPath $log).LastWriteTime -le $start) {
            $failures += ("row {0} {1}{2}: '{3}' was NOT freshly written - STALE" -f $rowNo, $s.Mode, $s.Index, $log)
            continue
        }

        $summary = @(Select-String -LiteralPath $log -Pattern '^VB-SV0-S5 mode=')
        if ($summary.Count -eq 0) {
            $failures += ("row {0} {1}{2}: the session printed no summary line - it was truncated or the bench never armed (log: {3})" -f $rowNo, $s.Mode, $s.Index, $log)
            continue
        }
        $resolution = @(Select-String -LiteralPath $log -Pattern '^VB-SV0-S5 RESOLUTION:')
        if ($resolution.Count -eq 0) {
            $failures += ("row {0} {1}{2}: no RESOLUTION line (log: {3})" -f $rowNo, $s.Mode, $s.Index, $log)
            continue
        }
        $sum = $summary[-1].Line
        $res = $resolution[-1].Line

        # --- the checks the runbook asks a reader to make, made here so they cannot be skipped ---
        # These apply to the WARM-UP session too. It is discarded as a measurement of the term, not
        # excused from running the configuration it is supposed to warm: a warm-up that bound the
        # wrong producer warmed the wrong thing.
        $producerLines = @(Select-String -LiteralPath $log -Pattern 'VB lit producer = ')
        $producer = if ($producerLines.Count -gt 0) { ($producerLines[-1].Line -split 'VB lit producer = ')[-1].Trim() } else { '<none>' }
        if ($producer -ne $row.Producer) {
            $failures += ("row {0} {1}{2}: expected producer '{3}' but the run bound '{4}'" -f $rowNo, $s.Mode, $s.Index, $row.Producer, $producer)
            Write-Host ("    producer MISMATCH: {0}" -f $producer) -ForegroundColor Red
            continue
        }
        # A clamp line means sync_sv0_light_gate could not honour the request, i.e. BOTH phases
        # rendered unarmed and this "armed" session is a second null control -- a zero delta that
        # reads as "SV0 is free". The runner asserts vb_sdf_mesh_armable at boot, so this should be
        # unreachable; it is checked anyway because its symptom is a plausible number.
        if (@(Select-String -LiteralPath $log -Pattern 'VB-SV0 was requested').Count -gt 0) {
            $failures += ("row {0} {1}{2}: the SV0 request was CLAMPED - this session measured nothing (log: {3})" -f $rowNo, $s.Mode, $s.Index, $log)
            Write-Host "    SV0 request CLAMPED - the boot could not carry it" -ForegroundColor Red
            continue
        }
        $extent = Get-Field -Line $sum -Key 'extent'
        if ($extent -ne '512x512') {
            $failures += ("row {0} {1}{2}: extent was '{3}', not 512x512 - the OS clamped the window and this measures a different per-pixel workload" -f $rowNo, $s.Mode, $s.Index, $extent)
            continue
        }
        $rowLabel = Get-Field -Line $sum -Key 'row'
        if ($rowLabel -ne $row.Label) {
            $failures += ("row {0} {1}{2}: summary says row={3}, expected {4}" -f $rowNo, $s.Mode, $s.Index, $rowLabel, $row.Label)
            continue
        }
        $q = [int](Get-Field -Line $sum -Key 'quads')
        $n = [int](Get-Field -Line $sum -Key 'samples')
        # A large shortfall against 4*quads means the stream was dropping frames, which orphans
        # quadruples; a session that quietly lost much of its stream must not read as clean.
        if ($n -lt (3.6 * $q)) {
            $failures += ("row {0} {1}{2}: samples={3} is far below 4*quads={4} - the frame stream was dropping" -f $rowNo, $s.Mode, $s.Index, $n, (4 * $q))
            continue
        }

        $sessions += [pscustomobject]@{
            Row        = $rowNo
            Mode       = $s.Mode
            Index      = $s.Index
            Log        = $log
            Producer   = $producer
            Label      = $rowLabel
            Quads      = $q
            Delta      = Get-Field -Line $sum -Key 'median_delta_ns'
            Bias       = Get-Field -Line $sum -Key 'median_order_bias_ns'
            Dispatch   = Get-Field -Line $sum -Key 'median_armed_ns'
            FirstHalf  = Get-Field -Line $sum -Key 'median_delta_first_half_ns'
            SecondHalf = Get-Field -Line $sum -Key 'median_delta_second_half_ns'
            Dbg        = Get-Field -Line $sum -Key 'debug_assertions'
            Period     = Get-Field -Line $res -Key 'timestamp_period_ns'
            TickGcd    = Get-Field -Line $res -Key 'tick_gcd'
            Distinct   = Get-Field -Line $res -Key 'distinct_ticks'
            MinGap     = Get-Field -Line $res -Key 'min_tick_gap'
            TickSpan   = Get-Field -Line $res -Key 'tick_span'
            QuantumMax = Get-Field -Line $res -Key 'quantum_max_ns'
            ValidBits  = Get-Field -Line $res -Key 'timestamp_valid_bits'
        }
        $last = $sessions[-1]
        if ($null -eq $last.TickGcd) {
            $failures += ("row {0} {1}{2}: the RESOLUTION line has no tick_gcd/distinct_ticks - this log came from a harness older than the bound-reporting one, and its quantum_ns cannot be pooled (log: {3})" -f $rowNo, $s.Mode, $s.Index, $log)
            continue
        }
        Write-Host ("    ok  delta={0}ns bias={1}ns quads={2} producer={3}" -f $last.Delta, $last.Bias, $q, $producer) -ForegroundColor DarkGray
        Write-Host ("        halves: first={0}ns second={1}ns | lattice bound <= {2}ns from {3} distinct ticks (min gap {4})" -f $last.FirstHalf, $last.SecondHalf, $last.QuantumMax, $last.Distinct, $last.MinGap) -ForegroundColor DarkGray
    }
}

foreach ($name in $ownedEnv) { Remove-Item -Path ("Env:" + $name) -ErrorAction SilentlyContinue }

if ($failures.Count -gt 0) {
    Write-Host "[s5] SESSION SET INCOMPLETE:" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host ("    " + $_) -ForegroundColor Red }
    throw "[s5] $($failures.Count) session(s) failed - the gate cannot be run against a partial set."
}

# --- the FLAT device properties: a disagreement here IS a contradiction ------------------
# `timestamp_period_ns` and `timestamp_valid_bits` are read straight off VkPhysicalDeviceLimits.
# Unlike the lattice bound below, they are not estimated from a sample, so two sessions cannot
# legitimately differ: that would mean two different devices, or two different builds.
$distinctPeriod = $sessions | ForEach-Object { $_.Period } | Sort-Object -Unique
$distinctBits = $sessions | ForEach-Object { $_.ValidBits } | Sort-Object -Unique
if ($distinctPeriod.Count -gt 1 -or $distinctBits.Count -gt 1) {
    Write-Host "[s5] FINDING: flat device properties DISAGREE across sessions:" -ForegroundColor Red
    Write-Host ("    timestamp_period_ns: {0}" -f ($distinctPeriod -join ', ')) -ForegroundColor Red
    Write-Host ("    timestamp_valid_bits: {0}" -f ($distinctBits -join ', ')) -ForegroundColor Red
    throw "[s5] these are read from VkPhysicalDeviceLimits, not estimated - two values means two devices or two builds. Do NOT pool them."
}

$distinctDbg = $sessions | ForEach-Object { $_.Dbg } | Sort-Object -Unique
if ($distinctDbg.Count -gt 1) {
    Write-Host "[s5] WARNING: sessions ran under DIFFERENT build profiles: $($distinctDbg -join ', ')" -ForegroundColor Yellow
}

# --- the lattice BOUND: sessions that differ are not contradicting each other -------------
# `quantum_max_ns` is `G * gcd(observed multipliers)`, so each session states `G <= its own value`.
# The strongest statement the set jointly supports is the GCD of those bounds -- the meet of ten
# upper bounds. A previous revision of this script called these "device properties", warned that
# they disagreed, and refused to pick one; refusing was right (it is what surfaced the finding) but
# the framing was wrong, and "pick one" was never the only alternative to "pick one".
#
# The warm-up sessions pool in DELIBERATELY. A cold session is invalid evidence about the TERM and
# valid evidence about the INSTRUMENT: its durations are integer tick counts on the same counter
# whatever the clocks were doing, and it is precisely its wider range of durations that makes a
# tighter bound recoverable.
$period = [double]$sessions[0].Period
$pooledGcd = [long]0
foreach ($sn in $sessions) { $pooledGcd = Get-Gcd -A $pooledGcd -B ([long]$sn.TickGcd) }
$pooledQuantumNs = [double]$pooledGcd * $period

$distinctGcds = $sessions | ForEach-Object { $_.TickGcd } | Sort-Object -Unique
Write-Host ""
Write-Host "[s5] LATTICE BOUND (pooled over $($sessions.Count) sessions, warm-up included):" -ForegroundColor Cyan
foreach ($sn in ($sessions | Sort-Object Row, Mode, Index)) {
    Write-Host ("    row {0} {1}{2}: tick_gcd={3} distinct={4} min_gap={5} span={6}" -f $sn.Row, $sn.Mode, $sn.Index, $sn.TickGcd, $sn.Distinct, $sn.MinGap, $sn.TickSpan) -ForegroundColor DarkGray
}
if ($distinctGcds.Count -gt 1) {
    Write-Host ("    the per-session bounds DIFFER ({0}) - that is sample homogeneity, NOT a device disagreement." -f ($distinctGcds -join ', ')) -ForegroundColor Yellow
    Write-Host "    A GCD over clustered durations is a MULTIPLE of the counter's step, so a narrow session states a weaker bound." -ForegroundColor Yellow
}
Write-Host ("    POOLED: quantum <= {0} ns ({1} ticks x {2} ns period)" -f (Format-F64 $pooledQuantumNs), $pooledGcd, $period) -ForegroundColor Green

# The evidence behind the pooled bound. Prefer the session that actually achieved it; if the pooled
# GCD is finer than every individual bound (no single session states it), fall back to the largest
# distinct count in the set and SAY SO, because the evidence is then the combination rather than
# any one session.
$backing = @($sessions | Where-Object { [long]$_.TickGcd -eq $pooledGcd } | Sort-Object -Property { [int]$_.Distinct } -Descending)
if ($backing.Count -gt 0) {
    $backingDistinct = [int]$backing[0].Distinct
    Write-Host ("    backed by row {0} {1}{2} ({3} distinct tick values, min gap {4})" -f $backing[0].Row, $backing[0].Mode, $backing[0].Index, $backingDistinct, $backing[0].MinGap) -ForegroundColor Green
} else {
    $backingDistinct = [int](($sessions | ForEach-Object { [int]$_.Distinct } | Measure-Object -Maximum).Maximum)
    Write-Host ("    NOTE: no single session states the pooled bound - it is the COMBINATION of {0} sessions." -f $sessions.Count) -ForegroundColor Yellow
    Write-Host ("    Transcribing the largest single-session distinct count ({0}) as its evidence, which UNDERSTATES it." -f $backingDistinct) -ForegroundColor Yellow
}

# The reported median lands on quantum/2, halved again to quantum/4 when the quadruple count is
# even and the median averages two order statistics. Take /4 when ANY transcribed session has an
# even count: a /2-lattice value also lies on the /4 lattice, so the FINER divisor is the correct
# one for differences between medians -- and it is the stricter one downstream.
$kept = @($sessions | Where-Object { $_.Mode -ne 'warmup' })
$anyEven = @($kept | Where-Object { ([int]$_.Quads % 2) -eq 0 }).Count -gt 0
if ($anyEven) { $latticeDiv = 4.0 } else { $latticeDiv = 2.0 }
$pooledLatticeNs = $pooledQuantumNs / $latticeDiv
Write-Host ("    median lattice <= {0} ns (quantum / {1}; the transcribed sessions' quadruple counts are {2})" -f (Format-F64 $pooledLatticeNs), $latticeDiv, (($kept | ForEach-Object { $_.Quads }) -join ', ')) -ForegroundColor Green

# --- the cold-start disclosure ------------------------------------------------------------
# Two separate readings, and they select opposite remedies:
#   * the WARM-UP session vs the three kept ones  -> a PROCESS-level cold start (already handled by
#     discarding it; the ratio says whether the discard was load-bearing on this run).
#   * a session's own two halves                  -> an IN-SESSION ramp, which the discard cannot
#     reach and which would call for a longer SV0_BENCH_WARMUP.
# No threshold is applied to either: both are reported, and a pass/fail line invented here would be
# fitted to whichever run happened to be in front of the author.
Write-Host ""
Write-Host "[s5] COLD-START DISCLOSURE:" -ForegroundColor Cyan
foreach ($rowNo in $Rows) {
    $armed = @($sessions | Where-Object { $_.Row -eq $rowNo -and $_.Mode -eq 'armed' } | Sort-Object Index)
    $warm = @($sessions | Where-Object { $_.Row -eq $rowNo -and $_.Mode -eq 'warmup' })
    if ($armed.Count -lt 1 -or $warm.Count -lt 1) { continue }
    $armedVals = @($armed | ForEach-Object { [double]$_.Delta } | Sort-Object)
    $central = $armedVals[[int]([math]::Floor($armedVals.Count / 2))]
    $warmDelta = [double]$warm[0].Delta
    if ($central -ne 0) { $ratio = $warmDelta / $central } else { $ratio = [double]::NaN }
    Write-Host ("    row {0}: DISCARDED warm-up={1}ns vs kept central={2}ns -> {3:0.00}x" -f $rowNo, (Format-F64 $warmDelta), (Format-F64 $central), $ratio) -ForegroundColor DarkGray
}
foreach ($sn in ($sessions | Sort-Object Row, Mode, Index)) {
    $h1 = [double]$sn.FirstHalf
    $h2 = [double]$sn.SecondHalf
    if ($h2 -ne 0) { $hr = $h1 / $h2 } else { $hr = [double]::NaN }
    Write-Host ("    row {0} {1}{2} halves: {3}ns / {4}ns -> {5:0.00}x" -f $sn.Row, $sn.Mode, $sn.Index, (Format-F64 $h1), (Format-F64 $h2), $hr) -ForegroundColor DarkGray
}
Write-Host "    Halves that disagree = that session was still settling while it recorded (remedy: raise SV0_BENCH_WARMUP)." -ForegroundColor DarkGray
Write-Host "    Halves that agree while the SESSION disagrees with its siblings = a process-level cold start (already discarded)." -ForegroundColor DarkGray

# --- the transcription block ------------------------------------------------------------
# PRINTED, never written. See this script's .DESCRIPTION for why: a script that edits the literals
# a gate reads can manufacture a passing gate.
function Format-Triple {
    param([object[]]$Values)
    return '[' + (($Values | ForEach-Object { Format-F64 $_ }) -join ', ') + ']'
}

Write-Host ""
Write-Host "[s5] TRANSCRIBE THE FOLLOWING into crates\boyko_app\tests\sv0_vb_term_bench.rs" -ForegroundColor Green
Write-Host "     (the MEASURED block; nothing here is written for you, and nothing should be)" -ForegroundColor Green
Write-Host ""
foreach ($rowNo in $Rows) {
    $armed = @($sessions | Where-Object { $_.Row -eq $rowNo -and $_.Mode -eq 'armed' } | Sort-Object Index)
    $null_ = @($sessions | Where-Object { $_.Row -eq $rowNo -and $_.Mode -eq 'null' })
    $warm = @($sessions | Where-Object { $_.Row -eq $rowNo -and $_.Mode -eq 'warmup' })
    if ($armed.Count -lt 3 -or $null_.Count -lt 1 -or $warm.Count -lt 1) {
        Write-Host ("     row {0}: incomplete session set, no block emitted" -f $rowNo) -ForegroundColor Yellow
        continue
    }
    $R = "ROW$rowNo"
    Write-Host ("const SV0_S5_{0}_SESSION_MEDIAN_DELTA_NS: [f64; SV0_S5_BENCH_SESSIONS] = {1};" -f $R, (Format-Triple ($armed | ForEach-Object { $_.Delta })))
    Write-Host ("const SV0_S5_{0}_SESSION_QUADS: [usize; SV0_S5_BENCH_SESSIONS] = [{1}];" -f $R, (($armed | ForEach-Object { $_.Quads }) -join ', '))
    Write-Host ("const SV0_S5_{0}_SESSION_ORDER_BIAS_NS: [f64; SV0_S5_BENCH_SESSIONS] = {1};" -f $R, (Format-Triple ($armed | ForEach-Object { $_.Bias })))
    Write-Host ("const SV0_S5_{0}_MEDIAN_DISPATCH_NS: f64 = {1};" -f $R, (Format-F64 $armed[0].Dispatch))
    Write-Host ("const SV0_S5_{0}_NULL_MEDIAN_DELTA_NS: f64 = {1};" -f $R, (Format-F64 $null_[0].Delta))
    Write-Host ("const SV0_S5_{0}_NULL_ORDER_BIAS_NS: f64 = {1};" -f $R, (Format-F64 $null_[0].Bias))
    Write-Host ("const SV0_S5_{0}_WARMUP_DELTA_NS: f64 = {1};" -f $R, (Format-F64 $warm[0].Delta))
    Write-Host ("const SV0_S5_{0}_ROW_LABEL: &str = `"{1}`";" -f $R, $armed[0].Label)
    Write-Host ("const SV0_S5_{0}_PRODUCER: &str = `"{1}`";" -f $R, $armed[0].Producer)
    Write-Host ""
}
$first = $sessions[0]
Write-Host ("const SV0_S5_DEBUG_ASSERTIONS: Option<bool> = Some({0});" -f $first.Dbg)
Write-Host ("const SV0_S5_TIMESTAMP_PERIOD_NS: f64 = {0};" -f (Format-F64 $first.Period))
Write-Host ("const SV0_S5_QUANTUM_MAX_NS: f64 = {0};" -f (Format-F64 $pooledQuantumNs))
Write-Host ("const SV0_S5_MEDIAN_LATTICE_MAX_NS: f64 = {0};" -f (Format-F64 $pooledLatticeNs))
Write-Host ("const SV0_S5_QUANTUM_DISTINCT_TICKS: Option<usize> = Some({0});" -f $backingDistinct)
Write-Host ""
Write-Host "[s5] logs in $OutDir. Then run the gates (BOTH -- S1.5's is what keeps S5's reference honest):" -ForegroundColor Green
Write-Host "    cargo test -p boyko-app --test sv0_deferred_term_bench -- --nocapture" -ForegroundColor Green
Write-Host "    cargo test -p boyko-app --test sv0_vb_term_bench       -- --nocapture" -ForegroundColor Green
