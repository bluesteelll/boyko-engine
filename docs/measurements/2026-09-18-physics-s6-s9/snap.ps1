# Per-process CPU-time snapshot (excluding this PowerShell itself), one JSON object on stdout.
$h = @{}
foreach ($p in Get-Process) {
    if ($p.Id -eq $PID) { continue }
    try { $c = $p.TotalProcessorTime.TotalSeconds } catch { continue }
    if ($c -ne $null) { $h["$($p.Id)"] = @($p.ProcessName, [double]$c) }
}
$h | ConvertTo-Json -Compress
