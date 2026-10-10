# Load receipt for MEASUREMENT-QUEUE timed runs: process count, 10 s average of
# \Processor(_Total)\% Processor Time (1 s interval, 10 samples), and the top 5
# processes by CPU-time delta over that same window. Emits one JSON object on stdout.
$ErrorActionPreference = 'Stop'
$ncpu = [Environment]::ProcessorCount
function Snap {
    $h = @{}
    foreach ($p in Get-Process) {
        $cpu = $null
        try { $cpu = $p.TotalProcessorTime.TotalSeconds } catch { $cpu = $null }
        if ($cpu -ne $null) { $h[$p.Id] = @{ name = $p.ProcessName; cpu = [double]$cpu } }
    }
    return $h
}
$start = Get-Date
$procCount = (Get-Process).Count
$s1 = Snap
$t1 = [Diagnostics.Stopwatch]::StartNew()
$samples = (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 10).CounterSamples | ForEach-Object { [double]$_.CookedValue }
$elapsed = $t1.Elapsed.TotalSeconds
$s2 = Snap
$deltas = @()
foreach ($id in $s2.Keys) {
    $prev = 0.0
    $isNew = $true
    if ($s1.ContainsKey($id)) { $prev = $s1[$id].cpu; $isNew = $false }
    $d = $s2[$id].cpu - $prev
    $deltas += [pscustomobject]@{ pid = $id; name = $s2[$id].name; cpu_s = [math]::Round($d, 3); pct_of_machine = [math]::Round(100.0 * $d / ($elapsed * $ncpu), 2); new = $isNew }
}
$top = $deltas | Sort-Object -Property cpu_s -Descending | Select-Object -First 5
$avg = ($samples | Measure-Object -Average).Average
[pscustomobject]@{
    time = $start.ToString('yyyy-MM-ddTHH:mm:ss.fffK')
    process_count = $procCount
    cpu_samples = @($samples | ForEach-Object { [math]::Round($_, 2) })
    cpu_avg = [math]::Round($avg, 2)
    window_s = [math]::Round($elapsed, 2)
    logical_cpus = $ncpu
    top5 = @($top)
} | ConvertTo-Json -Depth 4 -Compress
