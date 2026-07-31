<#
.SYNOPSIS
  Fetches the VG-R0 density-census corpus described by assets/vg_corpus/CORPUS.toml.

.DESCRIPTION
  The corpus is fetched + gitignored + pinned by content hash (plan section 4.3). This script is the
  producer of that payload:

      download source_url -> verify archive_sha256 -> extract -> verify each glb_sha256

  A hash mismatch is a HARD STOP, never a warning. The pin is what makes "the corpus" a fixed object
  rather than whatever a URL happened to serve today, and a census run against unpinned content
  measures nothing reproducible.

  Nothing is downloaded when CORPUS.toml lists no assets, which is its state until the owner has
  approved the selection: fetching writes hundreds of megabytes from third-party URLs, so the
  candidate list and each candidate's licence are surfaced for approval before this runs.

.PARAMETER Force
  Re-download and re-verify even when an extracted .glb is already present and matches its pin.
#>
[CmdletBinding()]
param([switch]$Force)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$corpusDir = Join-Path $repo 'assets\vg_corpus'
$manifestPath = Join-Path $corpusDir 'CORPUS.toml'

if (-not (Test-Path $manifestPath)) {
    throw "manifest not found: $manifestPath"
}

# Flat, section-scoped TOML subset -- the same shape the campaign's other readers use. Comments are
# stripped FIRST so a commented-out example block is never read as a live entry.
function Read-Assets([string]$Path) {
    $assets = @()
    $current = $null
    foreach ($raw in Get-Content $Path) {
        $line = $raw
        $hash = $line.IndexOf('#')
        if ($hash -ge 0) { $line = $line.Substring(0, $hash) }
        $line = $line.Trim()
        if ($line -eq '') { continue }
        if ($line -eq '[[asset]]') {
            if ($null -ne $current) { $assets += $current }
            $current = @{}
            continue
        }
        if ($line.StartsWith('[')) { $current = $null; continue }
        if ($null -eq $current) { continue }
        $eq = $line.IndexOf('=')
        if ($eq -lt 0) { continue }
        $key = $line.Substring(0, $eq).Trim()
        $val = $line.Substring($eq + 1).Trim().Trim('"')
        $current[$key] = $val
    }
    if ($null -ne $current) { $assets += $current }
    return $assets
}

function Assert-Sha256([string]$File, [string]$Expected, [string]$What) {
    $actual = (Get-FileHash -Algorithm SHA256 -Path $File).Hash.ToLower()
    $want = $Expected.ToLower()
    if ($want -eq 'pending') {
        throw "$What is pinned as PENDING in CORPUS.toml. An unblessed pin is not a pin -- record the real sha256 (actual: $actual) in the same commit that adds the entry."
    }
    if ($actual -ne $want) {
        throw "$What sha256 MISMATCH`n  expected $want`n  actual   $actual`nThis is a hard stop: a census run against unpinned content measures nothing reproducible."
    }
    Write-Host "  ok  $What  $actual"
}

$assets = Read-Assets $manifestPath
if ($assets.Count -eq 0) {
    Write-Host "[corpus] CORPUS.toml lists no assets -- nothing to fetch."
    Write-Host "[corpus] That is the manifest's authored state, not an error: selection and fetching are"
    Write-Host "[corpus] separate acts, and the fetch writes third-party payload to this machine."
    exit 0
}

$required = @('id','source_url','licence','licence_url','attribution','archive_sha256','glb','glb_sha256','published_triangles')
foreach ($a in $assets) {
    foreach ($k in $required) {
        if (-not $a.ContainsKey($k) -or [string]::IsNullOrWhiteSpace([string]$a[$k])) {
            throw "asset '$($a['id'])' is missing required manifest field '$k'"
        }
    }
}

Write-Host "[corpus] $($assets.Count) asset(s) named by the manifest."
foreach ($a in $assets) {
    Write-Host "[corpus] $($a['id'])  --  $($a['licence'])  --  $($a['attribution'])"
}

$work = Join-Path $corpusDir '.archives'
New-Item -ItemType Directory -Force -Path $work | Out-Null

foreach ($a in $assets) {
    $id = $a['id']
    $glbPath = Join-Path $corpusDir $a['glb']
    if ((Test-Path $glbPath) -and -not $Force) {
        $have = (Get-FileHash -Algorithm SHA256 -Path $glbPath).Hash.ToLower()
        if ($have -eq $a['glb_sha256'].ToLower()) {
            Write-Host "[corpus] $id already present and pinned-clean; skipping (use -Force to redo)."
            continue
        }
    }

    $archive = Join-Path $work ("{0}{1}" -f $id, [System.IO.Path]::GetExtension($a['source_url']))
    Write-Host "[corpus] fetching $id from $($a['source_url'])"
    Invoke-WebRequest -Uri $a['source_url'] -OutFile $archive -UseBasicParsing
    Assert-Sha256 $archive $a['archive_sha256'] "$id archive"

    Write-Host "[corpus] extracting $id"
    Expand-Archive -Path $archive -DestinationPath $corpusDir -Force

    if (-not (Test-Path $glbPath)) {
        throw "asset '$id': the archive did not contain '$($a['glb'])' as the manifest claims"
    }
    Assert-Sha256 $glbPath $a['glb_sha256'] "$id glb"
}

Write-Host "[corpus] all assets fetched and pinned-verified."
