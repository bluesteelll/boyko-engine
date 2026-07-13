#requires -Version 5.1
<#
.SYNOPSIS
    Launch any windowed boyko-engine example scene in any render paradigm.

.DESCRIPTION
    Sets the `BOYKO_RENDER_PATH` / `BOYKO_GEOMETRY_LEGS` env vars that `EnginePlugins::build`
    reads (crates/boyko_app/src/plugins.rs -- the dev/test launch seam) and runs the chosen
    example, so you can eyeball the SAME scene across all four render paths and all three geometry
    legs without editing any code.

    The default scene `paradigm_lab` is purpose-built to render correctly in EVERY cell of the
    RenderPath x GeometryLegs matrix (its meshes are VB-aware-registered AND it carries a live SDF
    primitive). The OTHER examples register meshes without a VB geometry-table slot, so they render
    meshes only under Deferred / Forward / ForwardPlus (their meshes vanish under -Path vb) -- use
    them for the raster paradigms, and `paradigm_lab` for VisibilityBuffer.

    Interactive by default (a live window stays open until you close it). Pass -Dump <path.bmp> to
    capture ONE settled frame and exit instead (the host BOYKO_HOST_DUMP channel).

.PARAMETER Scene
    The example basename under crates/boyko_app/examples/ (default: paradigm_lab). -List enumerates.

.PARAMETER Path
    Render path: deferred | forward | forwardplus | vb (default: deferred).

.PARAMETER Legs
    Geometry legs: both | mesh | sdf (default: both).

.PARAMETER Dump
    If set, capture one settled frame to this BMP path and exit (non-interactive).

.PARAMETER Hwrt
    Build/run with --features hwrt (hardware ray tracing leg).

.PARAMETER Release
    Build/run --release (default is a debug build).

.PARAMETER List
    Print the available example scenes and exit.

.EXAMPLE
    scripts\run-scene.ps1                                        # paradigm_lab, Deferred x Both, interactive
    scripts\run-scene.ps1 -Path vb -Legs both                   # paradigm_lab in Visibility Buffer
    scripts\run-scene.ps1 -Scene sdf_room -Path forward -Legs sdf
    scripts\run-scene.ps1 -Path forwardplus -Dump D:\tmp\fp.bmp # one-shot capture
    scripts\run-scene.ps1 -List
#>
[CmdletBinding()]
param(
    [string]$Scene = 'paradigm_lab',
    [ValidateSet('deferred', 'forward', 'forwardplus', 'vb')]
    [string]$Path = 'deferred',
    [ValidateSet('both', 'mesh', 'sdf')]
    [string]$Legs = 'both',
    [string]$Dump = '',
    [switch]$Hwrt,
    [switch]$Release,
    [switch]$List
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$examplesDir = Join-Path $repo 'crates\boyko_app\examples'

function Get-Scenes {
    Get-ChildItem -LiteralPath $examplesDir -Filter '*.rs' | ForEach-Object { $_.BaseName } | Sort-Object
}

if ($List) {
    Write-Host "[run-scene] available scenes (crates\boyko_app\examples\):" -ForegroundColor Cyan
    foreach ($s in Get-Scenes) {
        $tag = if ($s -eq 'paradigm_lab') { '  <- all 4 paradigms (VB-aware + SDF)' } else { '' }
        Write-Host ("  {0}{1}" -f $s, $tag)
    }
    Write-Host "`n[run-scene] paths: deferred | forward | forwardplus | vb   legs: both | mesh | sdf"
    return
}

$scenes = Get-Scenes
if ($scenes -notcontains $Scene) {
    throw "[run-scene] unknown scene '$Scene'. Run with -List to see the available scenes."
}
if ($Path -eq 'vb' -and $Scene -ne 'paradigm_lab') {
    Write-Warning "[run-scene] '$Scene' registers meshes WITHOUT a VB geometry-table slot -- its meshes will not appear under -Path vb (SDF still shows). Use -Scene paradigm_lab for the Visibility Buffer path."
}

# Explicit env (never inherit a stray value); RUSTUP_TOOLCHAIN pins the windows-gnu toolchain.
$env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'
$env:BOYKO_RENDER_PATH = $Path
$env:BOYKO_GEOMETRY_LEGS = $Legs
$env:BOYKO_DISABLE_VALIDATION = '1'
if ($Dump -ne '') {
    $env:BOYKO_HOST_DUMP = $Dump
    $dumpDir = Split-Path -Parent $Dump
    if ($dumpDir -and -not (Test-Path -LiteralPath $dumpDir)) { New-Item -ItemType Directory -Force -Path $dumpDir | Out-Null }
}
else {
    Remove-Item Env:\BOYKO_HOST_DUMP -ErrorAction SilentlyContinue
}

$cargoArgs = @('run', '-p', 'boyko-app', '--example', $Scene)
if ($Release) { $cargoArgs += '--release' }
if ($Hwrt) { $cargoArgs += @('--features', 'hwrt') }

$mode = if ($Dump -ne '') { "dump -> $Dump" } else { 'interactive' }
Write-Host "[run-scene] $Scene | $Path x $Legs | $mode$(if($Hwrt){' | hwrt'})$(if($Release){' | release'})" -ForegroundColor Green
Write-Host "[run-scene] cargo $($cargoArgs -join ' ')"

# Relax the Stop preference ONLY around the native cargo call: in PS 5.1 cargo's normal status
# lines go to stderr and are wrapped as NativeCommandError records, which under 'Stop' would
# terminate the script even on a clean exit. We gate on $LASTEXITCODE afterwards instead.
Push-Location $repo
$prev = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try { & cargo @cargoArgs }
finally { $ErrorActionPreference = $prev; Pop-Location }
if ($LASTEXITCODE -ne 0) { Write-Warning "[run-scene] cargo exited $LASTEXITCODE" }
