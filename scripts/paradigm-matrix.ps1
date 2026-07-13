#requires -Version 5.1
<#
.SYNOPSIS
    Render ONE scene across the render-paradigm matrix to BMPs for side-by-side comparison.

.DESCRIPTION
    Sweeps a scene through the render paths (and optionally every geometry-leg set), capturing one
    settled frame per cell to `<OutDir>\<scene>_<path>_<legs>.bmp` via the host BOYKO_HOST_DUMP
    channel + the `BOYKO_RENDER_PATH` / `BOYKO_GEOMETRY_LEGS` env seam
    (crates/boyko_app/src/plugins.rs). Non-interactive -- each cell renders and exits, so you end
    up with a folder of images to diff visually.

    Default: the four render paths at `-Legs both`. Pass -Full to sweep all 12 cells
    (4 paths x 3 legs). Default scene `paradigm_lab` is the only one whose meshes render under the
    VisibilityBuffer path (see run-scene.ps1's help) -- a warning is emitted for others.

.PARAMETER Scene
    Example basename under crates/boyko_app/examples/ (default: paradigm_lab).

.PARAMETER Legs
    Geometry legs for the path sweep when NOT -Full: both | mesh | sdf (default: both).

.PARAMETER Full
    Sweep all 12 cells (every path x every leg) instead of the 4 paths at a fixed -Legs.

.PARAMETER OutDir
    Output directory for the BMPs (default: D:\tmp\paradigm-matrix).

.PARAMETER Hwrt
    Build/run with --features hwrt.

.PARAMETER Release
    Build/run --release.

.EXAMPLE
    scripts\paradigm-matrix.ps1                          # paradigm_lab, 4 paths @ both -> 4 BMPs
    scripts\paradigm-matrix.ps1 -Full                    # paradigm_lab, all 12 cells
    scripts\paradigm-matrix.ps1 -Scene sdf_room -Legs sdf
#>
[CmdletBinding()]
param(
    [string]$Scene = 'paradigm_lab',
    [ValidateSet('both', 'mesh', 'sdf')]
    [string]$Legs = 'both',
    [switch]$Full,
    [string]$OutDir = 'D:\tmp\paradigm-matrix',
    [switch]$Hwrt,
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$examplesDir = Join-Path $repo 'crates\boyko_app\examples'

if (-not (Test-Path -LiteralPath (Join-Path $examplesDir "$Scene.rs"))) {
    throw "[matrix] unknown scene '$Scene' (looked in crates\boyko_app\examples\). Try scripts\run-scene.ps1 -List."
}
if (-not (Test-Path -LiteralPath $OutDir)) { New-Item -ItemType Directory -Force -Path $OutDir | Out-Null }

$paths = @('deferred', 'forward', 'forwardplus', 'vb')
$legsSet = if ($Full) { @('both', 'mesh', 'sdf') } else { @($Legs) }

$env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'
$env:BOYKO_DISABLE_VALIDATION = '1'

$results = @()
foreach ($p in $paths) {
    if ($p -eq 'vb' -and $Scene -ne 'paradigm_lab') {
        Write-Warning "[matrix] '$Scene' meshes will not render under vb (no VB geometry-table slot); capturing anyway (SDF/sky only)."
    }
    foreach ($l in $legsSet) {
        $bmp = Join-Path $OutDir ("{0}_{1}_{2}.bmp" -f $Scene, $p, $l)
        Remove-Item -LiteralPath $bmp -ErrorAction SilentlyContinue
        $env:BOYKO_RENDER_PATH = $p
        $env:BOYKO_GEOMETRY_LEGS = $l
        $env:BOYKO_HOST_DUMP = $bmp

        $cargoArgs = @('run', '-p', 'boyko-app', '--example', $Scene)
        if ($Release) { $cargoArgs += '--release' }
        if ($Hwrt) { $cargoArgs += @('--features', 'hwrt') }

        Write-Host ("[matrix] {0} x {1} -> {2}" -f $p, $l, $bmp) -ForegroundColor Green
        Push-Location $repo
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { & cargo @cargoArgs 2>&1 | Out-Null } finally { $ErrorActionPreference = $prev; Pop-Location }

        $ok = Test-Path -LiteralPath $bmp
        $results += [pscustomobject]@{ Path = $p; Legs = $l; Bmp = $bmp; Rendered = $ok }
        if (-not $ok) { Write-Warning "[matrix] no BMP produced for $p x $l (build/run error?)" }
    }
}

Remove-Item Env:\BOYKO_HOST_DUMP -ErrorAction SilentlyContinue
Write-Host "`n[matrix] done -> $OutDir" -ForegroundColor Cyan
$results | Format-Table -AutoSize
