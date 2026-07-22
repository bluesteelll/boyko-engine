#requires -Version 5.1
<#
.SYNOPSIS
    Launch the comprehensive Visibility-Buffer test scene (examples/vb_lab.rs), fly-around.

.DESCRIPTION
    A one-command launcher for the `vb_lab` scene: varied mesh + SDF geometry, a spread of PBR
    materials, CSM sun + spot/point shadows, and the full post stack (SSAO, DDGI GI, TAA + RCAS)
    -- all ON by default and each toggleable here WITHOUT a rebuild. Defaults to
    `RenderPath::VisibilityBuffer`; switch paradigm with -Path to eyeball the SAME scene across
    Deferred / Forward / Forward+.

    Fly controls: W/S/A/D fly -- Space/E up -- Left Ctrl/Q down -- mouse look -- Esc quit.

    Interactive by default (a live window stays open). Pass -Dump <path.bmp> to capture ONE
    settled frame and exit instead.

.PARAMETER Path       Render path: vb (default) | deferred | forward | forwardplus.
.PARAMETER Legs       Geometry legs: both (default) | mesh | sdf.
.PARAMETER Aa         Anti-aliasing: taa (default) | off | fxaa | smaa.
.PARAMETER Sharpen    RCAS sharpen (TAA only): rcas (default) | none.
.PARAMETER Ssao       SSAO ambient occlusion: on (default) | off.
.PARAMETER Gi         DDGI global illumination: on (default) | off.
.PARAMETER Csm        Cascade sun shadows: on (default) | off.
.PARAMETER Dump       Capture one settled frame to this BMP path and exit (non-interactive).
.PARAMETER Hwrt       Build/run with --features hwrt (hardware ray tracing leg).
.PARAMETER Release    Build/run --release (default is a debug build).

.EXAMPLE
    scripts\run-vb-lab.ps1                          # VB, everything on, interactive fly-around
.EXAMPLE
    scripts\run-vb-lab.ps1 -Aa off -Gi off -Ssao off  # bare VB, no post
.EXAMPLE
    scripts\run-vb-lab.ps1 -Path deferred           # the same scene under the deferred paradigm
.EXAMPLE
    scripts\run-vb-lab.ps1 -Dump D:\tmp\vb_lab.bmp -Release   # one-shot capture
#>
[CmdletBinding()]
param(
    [ValidateSet('vb', 'deferred', 'forward', 'forwardplus')]
    [string]$Path = 'vb',
    [ValidateSet('both', 'mesh', 'sdf')]
    [string]$Legs = 'both',
    [ValidateSet('taa', 'off', 'fxaa', 'smaa')]
    [string]$Aa = 'taa',
    [ValidateSet('rcas', 'none')]
    [string]$Sharpen = 'rcas',
    [ValidateSet('on', 'off')]
    [string]$Ssao = 'on',
    [ValidateSet('on', 'off')]
    [string]$Gi = 'on',
    [ValidateSet('on', 'off')]
    [string]$Csm = 'on',
    [string]$Dump = '',
    [switch]$Hwrt,
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# Explicit env (never inherit a stray value). The scene reads BOYKO_AA / BOYKO_TAA_SHARPEN /
# BOYKO_SSAO / BOYKO_GI / BOYKO_CSM_OFF (examples/vb_lab.rs); EnginePlugins reads
# BOYKO_RENDER_PATH / BOYKO_GEOMETRY_LEGS (crates/boyko_app/src/plugins.rs).
$env:RUSTUP_TOOLCHAIN     = 'stable-x86_64-pc-windows-gnu'
$env:BOYKO_DISABLE_VALIDATION = '1'
$env:BOYKO_RENDER_PATH    = $Path
$env:BOYKO_GEOMETRY_LEGS  = $Legs
$env:BOYKO_AA             = $Aa
$env:BOYKO_TAA_SHARPEN    = $Sharpen
$env:BOYKO_SSAO           = $Ssao
$env:BOYKO_GI             = $Gi
if ($Csm -eq 'off') { $env:BOYKO_CSM_OFF = '1' } else { Remove-Item Env:\BOYKO_CSM_OFF -ErrorAction SilentlyContinue }
if ($Dump -ne '') {
    $env:BOYKO_HOST_DUMP = $Dump
    $dumpDir = Split-Path -Parent $Dump
    if ($dumpDir -and -not (Test-Path -LiteralPath $dumpDir)) { New-Item -ItemType Directory -Force -Path $dumpDir | Out-Null }
}
else {
    Remove-Item Env:\BOYKO_HOST_DUMP -ErrorAction SilentlyContinue
}

$cargoArgs = @('run', '-p', 'boyko-app', '--example', 'vb_lab')
if ($Release) { $cargoArgs += '--release' }
if ($Hwrt) { $cargoArgs += @('--features', 'hwrt') }

$mode = if ($Dump -ne '') { "dump -> $Dump" } else { 'interactive' }
Write-Host "[vb-lab] $Path x $Legs | aa=$Aa sharpen=$Sharpen ssao=$Ssao gi=$Gi csm=$Csm | $mode$(if($Hwrt){' | hwrt'})$(if($Release){' | release'})" -ForegroundColor Green
if ($Dump -eq '') {
    Write-Host "[vb-lab] fly: W/S/A/D move | Space/E up | Left Ctrl/Q down | mouse look | Esc quit" -ForegroundColor Cyan
}
Write-Host "[vb-lab] cargo $($cargoArgs -join ' ')"

# Relax Stop ONLY around the native cargo call: in PS 5.1 cargo's status lines go to stderr and are
# wrapped as NativeCommandError records, which under 'Stop' would kill the script on a clean exit.
Push-Location $repo
$prev = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try { & cargo @cargoArgs }
finally { $ErrorActionPreference = $prev; Pop-Location }
if ($LASTEXITCODE -ne 0) { Write-Warning "[vb-lab] cargo exited $LASTEXITCODE" }
