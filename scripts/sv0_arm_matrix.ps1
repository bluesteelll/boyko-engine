<#
.SYNOPSIS
    VB-SV0 rung S4, gate (ii): renders the 8 armable VB lit-producer rows x 3 SV0 modes.

.DESCRIPTION
    docs\VB-SV0-SDF-SHADOW-PLAN.md, "S4 -- arm", gate (ii) requires each of the eight
    SV0-armable variant rows to be rendered THREE times -- unarmed, shadow-term-only, and
    contact-AO-term-only -- so each term can be shown to move pixels ON ITS OWN. This script
    produces those 24 dumps plus the 24 captured run logs; the assertions live in the CPU test
    `crates\boyko_app\tests\sv0_arm_matrix.rs`, which reads what this writes.

    Two GPU fixtures cover the eight rows -- `vb_both_sdf` (flat) and `vb_both_sdf_tex`
    (textured) -- and the row within a fixture is selected by env knobs. Every knob defaults
    OFF, so nothing here can leak into a `golden.ps1` pin run: that script WIPES every BOYKO_*
    its pin does not name.

    Rows 9 and 10 (`vb_shade_split_hwrt`, `vb_shade_split_tex_hwrt`) are deliberately absent.
    `ShadowSources::SDF_SOFT_MARCH` requires `!hwrt_denoise_or_vis_on`, which is exactly what
    selects those two pipelines, so SV0 can never be armed while they are bound. Their gate is
    the CPU truth table `sv0_never_arms_under_hwrt` in boyko_render, not a dump.

.PARAMETER OutDir
    Where the dumps and logs go. Must match the test's BOYKO_SV0_MATRIX_DIR (default D:\tmp\sv0).

.PARAMETER Rows
    Optional subset of row numbers (1..8) to render -- for re-running one row after a mutation
    without paying for the whole matrix.

    WARNING (code-review P2-e): a partial re-run leaves the OTHER cells on disk from an earlier
    build. Each cell therefore carries a `.meta` provenance sidecar naming the SHA-256 of the test
    executable that rendered it, and the gate REFUSES a matrix whose cells for one fixture binary
    do not all share one hash. So `-Rows` stays useful for iterating on a single row, and a
    certification run has to render the whole matrix.

.EXAMPLE
    powershell -File scripts\sv0_arm_matrix.ps1
    cargo test -p boyko-app --test sv0_arm_matrix -- --ignored --nocapture

.EXAMPLE
    # Re-render only the two split rows (e.g. after reverting vb_shade_split.comp.hlsl's SV0 block)
    # -- the gate will then report a mixed-build matrix unless the edit rebuilt nothing else.
    powershell -File scripts\sv0_arm_matrix.ps1 -Rows 7,8
#>
[CmdletBinding()]
param(
    [string]$OutDir = 'D:\tmp\sv0',
    [int[]]$Rows = @(1, 2, 3, 4, 5, 6, 7, 8)
)

$ErrorActionPreference = 'Stop'

# Run a native executable WITHOUT letting its stderr trip `$ErrorActionPreference='Stop'`.
# Verbatim `golden.ps1`'s helper, and for the same PowerShell 5.1 reason: cargo writes its
# normal status lines to stderr, which 'Stop' would turn into a script-terminating
# NativeCommandError even on exit code 0. The caller still gates on $LASTEXITCODE.
function Invoke-Native {
    param([Parameter(Mandatory)][string]$Exe, [string[]]$Args)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $Exe $Args } finally { $ErrorActionPreference = $prev }
}

$repoRoot = Split-Path -Parent $PSScriptRoot

# The row table. `Producer` is documentation here (the Rust test is what asserts it against the
# run log) -- it is carried so a failure in this script names the same row the test does.
$rowTable = @(
    @{ Index = 1; Producer = 'vb_resolve';          Binary = 'vb_both_sdf';     Test = 'vb_both_sdf_screenshot_dump';     Env = @{} }
    @{ Index = 2; Producer = 'vb_resolve_froxel';   Binary = 'vb_both_sdf';     Test = 'vb_both_sdf_screenshot_dump';     Env = @{ BOYKO_SV0_FROXEL = '1' } }
    @{ Index = 3; Producer = 'vb_shade';            Binary = 'vb_both_sdf';     Test = 'vb_both_sdf_screenshot_dump';     Env = @{ BOYKO_VB_FORCE_CLASSIFIED = '1' } }
    @{ Index = 4; Producer = 'vb_shade_tex';        Binary = 'vb_both_sdf_tex'; Test = 'vb_both_sdf_tex_screenshot_dump'; Env = @{} }
    @{ Index = 5; Producer = 'vb_shade_froxel';     Binary = 'vb_both_sdf';     Test = 'vb_both_sdf_screenshot_dump';     Env = @{ BOYKO_VB_FORCE_CLASSIFIED = '1'; BOYKO_SV0_FROXEL = '1' } }
    @{ Index = 6; Producer = 'vb_shade_tex_froxel'; Binary = 'vb_both_sdf_tex'; Test = 'vb_both_sdf_tex_screenshot_dump'; Env = @{ BOYKO_SV0_FROXEL = '1' } }
    @{ Index = 7; Producer = 'vb_shade_split';      Binary = 'vb_both_sdf';     Test = 'vb_both_sdf_screenshot_dump';     Env = @{ BOYKO_SV0_SSAO = '1' } }
    @{ Index = 8; Producer = 'vb_shade_split_tex';  Binary = 'vb_both_sdf_tex'; Test = 'vb_both_sdf_tex_screenshot_dump'; Env = @{ BOYKO_SV0_SSAO = '1' } }
)

# Every BOYKO_* this matrix ever sets. Wiped before EVERY run and re-set from the row, so a knob
# left over from the previous row (or from the ambient shell) cannot silently select a different
# variant -- `golden.ps1` learned this the hard way when BOYKO_AA leaked between two pins in one
# PowerShell process, and the "fix" for the resulting MISMATCH would have been to bless it.
$ownedEnv = @(
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
    # Code-review P1-b: the textured rows' AO floor is measured on the COMMITTED `synth_bumps`
    # map, and this knob is what would silently point them at a different one. Wiped so the
    # fixture always falls back to its compiled-in default; the gate additionally asserts the
    # folder each textured run REPORTED loading, so a leak past this wipe still cannot pass.
    'BOYKO_PBR_TEXTURE_DIR'
)

if (-not (Test-Path -LiteralPath $OutDir)) {
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
}

$env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'

# --- compile both fixtures ONCE, up front; ABORT on build error -------------------------
# The cargo-check-skips-tests false-green is a standing trap in this repo: a fixture that fails
# to compile would otherwise surface as "the dump is missing", which reads like a GPU problem.
#
# The SHA-256 of each freshly-built executable is captured here and stamped into every cell's
# `.meta` sidecar (code-review P2-e). A cell's image is a deterministic function of the binary
# that rendered it plus its env, so "all cells of binary B share B's hash" is exactly "no cell of
# B predates this build" -- which is the one thing per-cell freshness cannot say.
$exeHash = @{}
foreach ($bin in @('vb_both_sdf', 'vb_both_sdf_tex')) {
    Write-Host "[sv0] compiling test binary '$bin' (--no-run) ..." -ForegroundColor Cyan
    Invoke-Native -Exe 'cargo' -Args @('test', '-p', 'boyko-app', '--test', $bin, '--no-run', '-j2')
    if ($LASTEXITCODE -ne 0) {
        throw "[sv0] test binary '$bin' FAILED TO COMPILE - aborting before any dump is taken."
    }
    # Second, fully-cached invocation purely to learn the executable PATH. Parsing it out of the
    # JSON message stream is exact; globbing target\debug\deps\<bin>-*.exe and taking the newest
    # is a guess that goes wrong the moment a stale hash-suffixed binary outlives its source.
    $json = Invoke-Native -Exe 'cargo' -Args @(
        'test', '-p', 'boyko-app', '--test', $bin, '--no-run', '--message-format=json', '-j2'
    )
    $exe = $null
    foreach ($line in $json) {
        if ($line -isnot [string] -or -not $line.StartsWith('{')) { continue }
        try { $msg = $line | ConvertFrom-Json } catch { continue }
        if ($msg.reason -eq 'compiler-artifact' -and $msg.executable -and $msg.target.name -eq $bin) {
            $exe = $msg.executable
        }
    }
    if (-not $exe -or -not (Test-Path -LiteralPath $exe)) {
        throw "[sv0] could not locate the built executable for '$bin' - the provenance stamp every cell carries cannot be computed."
    }
    $exeHash[$bin] = @{
        Path = $exe
        Sha  = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLower()
    }
    Write-Host ("[sv0]   {0} -> {1}" -f $bin, $exeHash[$bin].Sha) -ForegroundColor DarkGray
}

$modes = @(0, 1, 2)
$modeNames = @{ 0 = 'unarmed'; 1 = 'shadow-only (ii-a)'; 2 = 'contact-AO-only (ii-b)' }
$failures = @()

foreach ($rowNo in $Rows) {
    $row = $rowTable | Where-Object { $_.Index -eq $rowNo }
    if (-not $row) { throw "[sv0] no such row: $rowNo (the matrix has rows 1..8)" }

    foreach ($mode in $modes) {
        $bmp  = Join-Path $OutDir ("row{0}_mode{1}.bmp"  -f $rowNo, $mode)
        $log  = Join-Path $OutDir ("row{0}_mode{1}.log"  -f $rowNo, $mode)
        $meta = Join-Path $OutDir ("row{0}_mode{1}.meta" -f $rowNo, $mode)

        # --- explicit env: wipe every knob this matrix owns, then set this cell's ---------
        foreach ($name in $ownedEnv) { Remove-Item -Path ("Env:" + $name) -ErrorAction SilentlyContinue }
        $env:BOYKO_DISABLE_VALIDATION = '1'   # the GPU validation layer is crash-prone on this box
        $env:BOYKO_SHADOW_DENOISE = 'none'    # matches the [vb_both_sdf] / [vb_both_sdf_tex] pins
        $env:BOYKO_HOST_DUMP = $bmp
        $env:BOYKO_SV0_MODE = "$mode"
        foreach ($k in $row.Env.Keys) { Set-Item -Path ("Env:" + $k) -Value $row.Env[$k] }

        # --- delete the stale artifacts; fail loud if they survive ------------------------
        # The sidecar is deleted FIRST and written LAST, so a cell that dies mid-run leaves no
        # provenance and the gate rejects it rather than reading a stale stamp as this run's.
        foreach ($path in @($bmp, $log, $meta)) {
            if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
            if (Test-Path -LiteralPath $path) { throw "[sv0] could not delete stale '$path' - aborting." }
        }
        $start = Get-Date

        Write-Host ("[sv0] row {0} ({1}) mode {2} = {3} ..." -f $rowNo, $row.Producer, $mode, $modeNames[$mode]) -ForegroundColor Cyan

        # Routed through cmd so the merged stdout+stderr lands in the log verbatim: PS 5.1 wraps
        # native stderr into ErrorRecords, which would mangle the `VB lit producer = ...` line
        # the Rust test parses. `--nocapture` is what lets the engine's own eprintln! through.
        $cmdLine = 'cargo test -p boyko-app --test ' + $row.Binary + ' ' + $row.Test +
                   ' -j2 -- --ignored --test-threads=1 --nocapture > "' + $log + '" 2>&1'
        Invoke-Native -Exe 'cmd' -Args @('/c', $cmdLine)
        if ($LASTEXITCODE -ne 0) {
            if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Tail 25 | Write-Host }
            $failures += ("row {0} mode {1}: the render test returned {2} (log: {3})" -f $rowNo, $mode, $LASTEXITCODE, $log)
            continue
        }

        # --- fresh-artifact guard (the campaign's false-fresh trap, at a new surface) ------
        if (-not (Test-Path -LiteralPath $bmp)) {
            $failures += ("row {0} mode {1}: no dump at {2}" -f $rowNo, $mode, $bmp)
            continue
        }
        $mtime = (Get-Item -LiteralPath $bmp).LastWriteTime
        if ($mtime -le $start) {
            $failures += ("row {0} mode {1}: '{2}' was NOT freshly written (mtime {3} <= run start {4}) - STALE" -f $rowNo, $mode, $bmp, $mtime, $start)
            continue
        }

        # --- the producer line, echoed here so a wrong row is visible during the run -------
        $producerLines = @(Select-String -LiteralPath $log -Pattern 'VB lit producer = ')
        if ($producerLines.Count -eq 0) {
            $failures += ("row {0} mode {1}: the run recorded no VB frame (no producer line in {2})" -f $rowNo, $mode, $log)
            continue
        }
        $got = ($producerLines[-1].Line -split 'VB lit producer = ')[-1].Trim()
        if ($got -ne $row.Producer) {
            $failures += ("row {0} mode {1}: expected producer '{2}' but the run bound '{3}'" -f $rowNo, $mode, $row.Producer, $got)
            Write-Host ("    producer MISMATCH: {0}" -f $got) -ForegroundColor Red
            continue
        }
        $clamped = @(Select-String -LiteralPath $log -Pattern 'VB-SV0 was requested')
        if ($mode -ne 0 -and $clamped.Count -gt 0) {
            $failures += ("row {0} mode {1}: the SV0 request was CLAMPED (this dump is unarmed) - see {2}" -f $rowNo, $mode, $log)
            Write-Host "    SV0 request CLAMPED - the boot could not carry it" -ForegroundColor Red
            continue
        }
        $hash = (Get-FileHash -LiteralPath $bmp -Algorithm SHA256).Hash.ToLower()

        # --- the provenance sidecar, written LAST (code-review P2-e) ----------------------
        # Only a cell that passed every check above gets one, so its presence means "this dump
        # was produced, verified and stamped by ONE run".
        #
        # The leading `sv0-matrix-cell` line is a format marker the gate requires -- and it also
        # absorbs the BOM: PS 5.1's `-Encoding utf8` writes one, so whatever lands on line 1 is
        # prefixed with U+FEFF. A `key=value` line there would silently fail to parse. (The gate
        # strips the BOM as well; this is the belt to that pair of braces.)
        $metaLines = @(
            "sv0-matrix-cell 1",
            "row=$rowNo",
            "mode=$mode",
            "binary=$($row.Binary)",
            "producer=$got",
            "exe=$($exeHash[$row.Binary].Path)",
            "exe_sha256=$($exeHash[$row.Binary].Sha)",
            "bmp_sha256=$hash",
            "run_utc=$((Get-Date).ToUniversalTime().ToString('o'))"
        )
        Set-Content -LiteralPath $meta -Value $metaLines -Encoding utf8

        Write-Host ("    ok  producer={0}  sha256={1}" -f $got, $hash) -ForegroundColor DarkGray
    }
}

foreach ($name in $ownedEnv) { Remove-Item -Path ("Env:" + $name) -ErrorAction SilentlyContinue }

if ($failures.Count -gt 0) {
    Write-Host "[sv0] MATRIX INCOMPLETE:" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host ("    " + $_) -ForegroundColor Red }
    throw "[sv0] $($failures.Count) cell(s) failed - the gate cannot be run against a partial matrix."
}

Write-Host "[sv0] matrix complete in $OutDir. Now run the gate:" -ForegroundColor Green
Write-Host "    cargo test -p boyko-app --test sv0_arm_matrix -- --ignored --nocapture" -ForegroundColor Green
