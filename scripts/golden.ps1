#requires -Version 5.1
<#
.SYNOPSIS
    Golden byte-identity gate (Tier-0) for boyko-engine render output.

.DESCRIPTION
    One command that encodes the hard-won anti-false-green lessons so they can no longer
    be forgotten:

      1. FORCE-COMPILE the test binary (cargo test --no-run) and ABORT on a build error
         -- kills the "cargo check skips test targets, so a stale .bmp hashes as a match"
         false-green (docs/RENDER-RUNG3B-TAA-PLAN.md:394-398).
      2. DELETE the target .bmp first and fail loud if it survives -- no stale artifact can
         masquerade as a fresh render.
      3. SET the full BOYKO_* env explicitly (never inherit) and CLEAR BOYKO_FORCE_SOFTWARE,
         so a stray shell variable cannot silently change the pipeline the pin describes.
      4. After the render, assert the .bmp was FRESHLY written (LastWriteTime > run start).
      5. SHA-256 the .bmp and compare against goldens\PINS.toml -- the single source of truth
         (replaces the hash string formerly hand-copied across ~10 docs).
      6. BUILD THIS SCRIPT'S OWN CHECKOUT, never the caller's. Every cargo call runs inside the
         repository root the script lives in (Push-Location, popped in a finally), and every run
         prints that root and cargo's own `locate-project --workspace` answer, aborting if they
         differ. Until 2026-09-18 PINS.toml was read next to the script while cargo ran in the
         CALLER's working directory, so a worktree lane's sweep launched from a shell parked in
         the main checkout built and rendered the main checkout and compared it against the
         lane's pins: six "moved" TAA pins were blamed on the lane's change and were a
         measurement of another tree (the log's own receipt: `Compiling boyko-app ...
         (D:\claude\BoykoEngine\crates\boyko_app)`). --manifest-path is NOT the fix: cargo finds
         .cargo\config.toml (the target-cpu rustflags) and rustup finds rust-toolchain.toml from
         the working directory, so it would still build one tree with the other tree's settings.

    Windows / single RTX-3060 / windows-msvc (the workstation's build host since 2026-09-10;
    every sha256 in PINS.toml was blessed on the preceding windows-gnu host and none has been
    re-run under msvc -- PINS.toml's header states what to do with a first mismatch, and it is
    NOT -Bless). Windowed dumps require --test-threads=1 and are #[ignore]d, so this is a
    human/orchestrator command -- it never runs on CI (no GPU there).

.PARAMETER Pin
    Which pin in goldens\PINS.toml to gate. Default: grand_showcase.

.PARAMETER Hwrt
    Use the --features hwrt leg and the sha256_hwrt pin (instead of the software leg).

.PARAMETER Bless
    On success, WRITE the freshly-computed hash back into PINS.toml for the chosen leg.
    Do this ONLY after a visual owner sign-off on the dumped BMP. Without -Bless the script
    only CHECKS and reports PASS/FAIL.

.PARAMETER ValidationOn
    Validation-audit mode: render the SAME pin with the Vulkan validation layer ACTIVE
    (the pin's BOYKO_DISABLE_VALIDATION is stripped instead of set) and the window HIDDEN
    (BOYKO_WIN_HIDDEN=1 -- audits must not park windows on the desktop). The test output is
    captured and scanned for [vk-validation] lines; ANY message fails the run (exit 3), in
    addition to the normal byte-identity gate (validation never changes pixels, so the hash
    must still match). Incompatible with -Bless (never bless from an audit run).

.EXAMPLE
    scripts\golden.ps1                         # check grand_showcase (software leg)
    scripts\golden.ps1 -Hwrt                    # check the hwrt leg
    scripts\golden.ps1 -Bless                   # re-pin after an intentional, verified change
    scripts\golden.ps1 -Pin vb_mesh -ValidationOn   # validation-layer audit of the same frame
#>
[CmdletBinding()]
param(
    [string]$Pin = 'grand_showcase',
    [switch]$Hwrt,
    [switch]$Bless,
    [switch]$ValidationOn
)

if ($ValidationOn -and $Bless) {
    throw "[golden] -ValidationOn is an AUDIT mode; blessing from it is forbidden (run -Bless separately)."
}

$ErrorActionPreference = 'Stop'

# Run a native executable WITHOUT letting its stderr trip `$ErrorActionPreference='Stop'`.
# In Windows PowerShell 5.1 a native command's stderr lines (e.g. cargo's normal "Finished
# ..." status, which cargo writes to stderr) are wrapped as NativeCommandError records; under
# 'Stop' that terminates the script even though the exe returned exit code 0. Relaxing the
# preference ONLY around the native call keeps the cmdlet-level 'Stop' intact, and the caller
# still gates on `$LASTEXITCODE` afterwards, so no failure is masked.
# -WorkDir is mandatory so no call can inherit the caller's directory (.DESCRIPTION item 6);
# the Push-Location fails the script before the call if the directory is missing.
function Invoke-Native {
    param([Parameter(Mandatory)][string]$Exe, [string[]]$Args, [Parameter(Mandatory)][string]$WorkDir)
    Push-Location -LiteralPath $WorkDir -ErrorAction Stop
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $Exe $Args } finally { $ErrorActionPreference = $prev; Pop-Location }
}

# --- minimal, section-scoped TOML-subset reader (PS 5.1 has no TOML parser) ------------
function Read-Pins([string]$path) {
    if (-not (Test-Path -LiteralPath $path)) { throw "[golden] pins file not found: $path" }
    $map = @{}
    $section = ''
    foreach ($line in Get-Content -LiteralPath $path) {
        $t = $line.Trim()
        if ($t -eq '' -or $t.StartsWith('#')) { continue }
        if ($t -match '^\[(.+)\]$') { $section = $Matches[1].Trim(); continue }
        if ($t -match '^([A-Za-z0-9_]+)\s*=\s*(.+?)\s*$') {
            $key = $Matches[1]
            $val = $Matches[2].Trim()
            if ($val.StartsWith('"') -and $val.EndsWith('"')) {
                $val = $val.Substring(1, $val.Length - 2)
            }
            $val = $val -replace '\\\\', '\'
            $map["$section.$key"] = $val
        }
    }
    return $map
}

$repoRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
Write-Host "[golden] repo root = $repoRoot (pins are read and every cargo call runs here)" -ForegroundColor Cyan
$pinsPath = Join-Path $repoRoot 'goldens\PINS.toml'
$pins = Read-Pins $pinsPath

$bmp   = $pins["$Pin.bmp"]
$crate = $pins["$Pin.crate"]
$bin   = $pins["$Pin.test_binary"]
$name  = $pins["$Pin.test_name"]
if (-not $bmp -or -not $crate -or -not $bin -or -not $name) {
    throw "[golden] pin '$Pin' is missing one of bmp/crate/test_binary/test_name in $pinsPath"
}

$leg      = if ($Hwrt) { 'hwrt' } else { 'software' }
$shaKey   = "$Pin.sha256_$leg"
$expected = $pins[$shaKey]
$featArgs = if ($Hwrt) { @('--features', 'hwrt') } else { @() }

Write-Host "[golden] pin=$Pin leg=$leg bmp=$bmp" -ForegroundColor Cyan

# --- (3) explicit env: WIPE every stray BOYKO_*, then SET from the pin -------------------
#
# The pin's [<pin>.env] block is the WHOLE truth about the pipeline: any BOYKO_* the ambient
# shell happens to carry and the pin does NOT name is wiped here, not inherited.
#
# This used to clear BOYKO_FORCE_SOFTWARE and nothing else, while PINS.toml's header promised
# "a stray shell env cannot silently change the pipeline". It could, and it did: running
# `golden.ps1 -Pin taa_armed` (whose env sets BOYKO_AA=taa) and then `-Pin grand_showcase_2mat`
# in the SAME PowerShell process leaked BOYKO_AA into the second pin -- Set-Item Env: persists
# for the life of the process -- so an AA-OFF pin rendered with TAA armed and reported a
# MISMATCH. The trap is that the "fix" for a MISMATCH is -Bless, which would have frozen a
# TAA-armed frame as an AA-off pin's golden, silently and permanently.
#
# Note the failure was invisible to the RHI-level pins (window_present_gbuffer builds its own
# scene and never reads BOYKO_AA) and only bit the boyko-app ones -- i.e. it presents as ONE
# arbitrary pin drifting, which reads exactly like a real regression.
$pinEnvNames = @()
foreach ($k in $pins.Keys) {
    $prefix = "$Pin.env."
    if ($k.StartsWith($prefix)) { $pinEnvNames += $k.Substring($prefix.Length) }
}
Get-ChildItem Env: |
    Where-Object { $_.Name -like 'BOYKO_*' -and ($pinEnvNames -notcontains $_.Name) } |
    ForEach-Object {
        Write-Host ("  env WIPED (stray, not named by the pin): {0}={1}" -f $_.Name, $_.Value) -ForegroundColor Yellow
        Remove-Item -Path ("Env:" + $_.Name) -ErrorAction SilentlyContinue
    }
foreach ($k in $pins.Keys) {
    $prefix = "$Pin.env."
    if ($k.StartsWith($prefix)) {
        $envName = $k.Substring($prefix.Length)
        Set-Item -Path ("Env:" + $envName) -Value $pins[$k]
        Write-Host ("  env {0}={1}" -f $envName, $pins[$k]) -ForegroundColor DarkGray
    }
}

# -ValidationOn overrides (AFTER the pin env is applied): strip the pin's
# BOYKO_DISABLE_VALIDATION, SET BOYKO_ENABLE_VALIDATION so the layer + messenger oracle are
# actually live, and hide the window so the audit never parks a window on the desktop.
# Everything else stays the pin's truth.
#
# BOYKO_ENABLE_VALIDATION is not redundant with the strip, and the difference was measured. The backend gates the
# layer on `enable_validation && BOYKO_DISABLE_VALIDATION unset`: `fn validation_requested`,
# boyko_rhi_vulkan/src/device.rs:2543 (re-derived 2026-09-18 -- this cited :2362 until then, a `message_severity:`
# field inside the debug-messenger create-info initializer, not a gate). boyko_app's runner USED TO hardcode the
# first conjunct to `false`, so stripping alone enabled NOTHING on all 22 boyko-app pins -- a "VALIDATION: clean
# (0 messages)" verdict that could not fail (an illegal `mip_levels: 12` drew zero; docs/OPEN-QUESTIONS.md). The
# runner is ENV-GATED now (crates/boyko_app/src/runner.rs:221), so SETTING the variable -- not the strip -- arms it.
if ($ValidationOn) {
    Remove-Item Env:BOYKO_DISABLE_VALIDATION -ErrorAction SilentlyContinue
    $env:BOYKO_ENABLE_VALIDATION = '1'
    $env:BOYKO_WIN_HIDDEN = '1'
    Write-Host "  env OVERRIDE (-ValidationOn): BOYKO_DISABLE_VALIDATION stripped, BOYKO_ENABLE_VALIDATION=1, BOYKO_WIN_HIDDEN=1" -ForegroundColor Yellow
}

# Ensure the dump directory exists (D:\tmp is wiped by a Windows crash -- recreate it).
$bmpDir = Split-Path -Parent $bmp
if ($bmpDir -and -not (Test-Path -LiteralPath $bmpDir)) {
    New-Item -ItemType Directory -Force -Path $bmpDir | Out-Null
}

# --- (6) cargo's own answer to WHICH workspace it builds, asked from where it will build ----
# Asked after the pin env is applied, so it runs under the pin's RUSTUP_TOOLCHAIN like the build.
$located = "$(Invoke-Native -Exe 'cargo' -Args @('locate-project', '--workspace', '--message-format', 'plain') -WorkDir $repoRoot)".Trim()
$rootManifest = Join-Path $repoRoot 'Cargo.toml'
if ($LASTEXITCODE -ne 0 -or $located -eq '' -or [System.IO.Path]::GetFullPath($located) -ne $rootManifest) {
    throw "[golden] cargo resolves the workspace to '$located', not '$rootManifest' - aborting: this run would not build the checkout whose PINS.toml it reads."
}
Write-Host "[golden] cargo workspace = $located" -ForegroundColor Cyan

# --- (1) force-compile the test binary; ABORT on build error ---------------------------
Write-Host "[golden] compiling test binary '$bin' (--no-run) ..." -ForegroundColor Cyan
$buildArgs = @('test', '-p', $crate) + $featArgs + @('--test', $bin, '--no-run')
Invoke-Native -Exe 'cargo' -Args $buildArgs -WorkDir $repoRoot
if ($LASTEXITCODE -ne 0) {
    throw "[golden] test binary '$bin' FAILED TO COMPILE - aborting. This is the cargo-check-skips-tests false-green: fix the build before trusting any hash."
}

# --- (2) delete the stale artifact; fail loud if it survives ---------------------------
if (Test-Path -LiteralPath $bmp) { Remove-Item -LiteralPath $bmp -Force }
if (Test-Path -LiteralPath $bmp) { throw "[golden] could not delete stale BMP '$bmp' - aborting." }
$start = Get-Date

# --- render (windowed, single-threaded, #[ignore]d) ------------------------------------
Write-Host "[golden] rendering '$name' ..." -ForegroundColor Cyan
$runArgs = @('test', '-p', $crate) + $featArgs + @('--test', $bin, $name, '--', '--ignored', '--test-threads=1')
$valLog = $null
if ($ValidationOn) {
    # Capture the FULL merged test output to a log so the [vk-validation] scan below sees
    # every messenger line (routed through cmd: PS 5.1 wraps native stderr into
    # NativeCommandError records, which a plain 2>&1 capture would mangle).
    $valLog = Join-Path $env:TEMP ("golden_valon_{0}_{1}.log" -f $Pin, $leg)
    if (Test-Path -LiteralPath $valLog) { Remove-Item -LiteralPath $valLog -Force }
    $cmdLine = 'cargo ' + ($runArgs -join ' ') + ' --nocapture > "' + $valLog + '" 2>&1'
    Invoke-Native -Exe 'cmd' -Args @('/c', $cmdLine) -WorkDir $repoRoot
    if ($LASTEXITCODE -ne 0) {
        if ($valLog -and (Test-Path -LiteralPath $valLog)) { Get-Content -LiteralPath $valLog -Tail 25 | Write-Host }
        throw "[golden] render test '$name' returned non-zero under -ValidationOn - aborting (log: $valLog)."
    }
} else {
    Invoke-Native -Exe 'cargo' -Args $runArgs -WorkDir $repoRoot
    if ($LASTEXITCODE -ne 0) {
        throw "[golden] render test '$name' returned non-zero - aborting."
    }
}

# --- (4) fresh-artifact guard ----------------------------------------------------------
if (-not (Test-Path -LiteralPath $bmp)) {
    throw "[golden] the test did not produce '$bmp' - aborting (nothing to hash)."
}
$mtime = (Get-Item -LiteralPath $bmp).LastWriteTime
if ($mtime -le $start) {
    throw "[golden] '$bmp' was NOT freshly written (mtime $mtime <= run start $start) - STALE artifact, aborting."
}

# --- (4b) -ValidationOn: the messenger-line scan ----------------------------------------
$valCount = 0
if ($ValidationOn) {
    $valLines = @(Select-String -LiteralPath $valLog -Pattern '\[vk-validation\]')
    $valCount = $valLines.Count
    if ($valCount -gt 0) {
        Write-Host "[golden] VALIDATION: $valCount [vk-validation] message(s) - log: $valLog" -ForegroundColor Red
        $valLines | Select-Object -First 10 | ForEach-Object { Write-Host ("    " + $_.Line) -ForegroundColor Red }
    } else {
        Write-Host "[golden] VALIDATION: clean (0 messages)." -ForegroundColor Green
    }
}

# --- (5) hash + compare / bless --------------------------------------------------------
$actual = (Get-FileHash -LiteralPath $bmp -Algorithm SHA256).Hash.ToLower()
Write-Host "[golden] $Pin/$leg actual = $actual"

if ($Bless) {
    if ($expected -eq $actual) {
        Write-Host "[golden] already blessed - hash unchanged, nothing to write." -ForegroundColor Green
        exit 0
    }
    $keyName = "sha256_$leg"
    $lines = Get-Content -LiteralPath $pinsPath
    $sec = ''
    $wrote = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $t = $lines[$i].Trim()
        if ($t -match '^\[(.+)\]$') { $sec = $Matches[1].Trim(); continue }
        if ($sec -eq $Pin -and $t -match "^$keyName\s*=") {
            $lines[$i] = ('{0,-15} = "{1}"' -f $keyName, $actual)
            $wrote = $true
            break
        }
    }
    if (-not $wrote) { throw "[golden] could not find '$keyName' under [$Pin] in $pinsPath to bless." }
    Set-Content -LiteralPath $pinsPath -Value $lines -Encoding UTF8
    Write-Host "[golden] BLESSED $Pin/${leg}: $expected -> $actual" -ForegroundColor Yellow
    Write-Host "[golden] goldens\PINS.toml updated. Review the diff and commit it deliberately." -ForegroundColor Yellow
    exit 0
}

# -Check (default)
if ([string]::IsNullOrWhiteSpace($expected) -or $expected -eq 'PENDING') {
    $hwrtFlag = if ($Hwrt) { '-Hwrt ' } else { '' }
    Write-Host "[golden] NO PIN recorded for $Pin/$leg (value='$expected'). actual=$actual." -ForegroundColor Yellow
    Write-Host "[golden] After a visual sign-off on '$bmp', run: scripts\golden.ps1 -Pin $Pin $hwrtFlag-Bless" -ForegroundColor Yellow
    exit 2
}
if ($actual -eq $expected) {
    Write-Host "[golden] PASS - $Pin/$leg byte-identical." -ForegroundColor Green
    if ($ValidationOn -and $valCount -gt 0) {
        Write-Host "[golden] ...but the -ValidationOn scan found $valCount message(s) - FAILING the audit." -ForegroundColor Red
        exit 3
    }
    exit 0
} else {
    Write-Host "[golden] FAIL - $Pin/$leg MISMATCH" -ForegroundColor Red
    Write-Host "  expected: $expected" -ForegroundColor Red
    Write-Host "  actual:   $actual"   -ForegroundColor Red
    Write-Host "[golden] If this change is INTENTIONAL and visually verified, re-run with -Bless." -ForegroundColor Red
    exit 1
}
