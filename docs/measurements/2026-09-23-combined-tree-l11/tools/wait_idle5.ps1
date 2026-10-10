# Idle wait for window 5 (copy of window 4b's wait_idle4b.ps1, MaxPolls default 120 = 120 min; P0 idle rule: 0 cargo/rustc/link/miri/test-exe processes on 3 polls 60 s apart, 10-s CPU < 5 %). Adapted from window 3's wait_idle3.ps1 (+miri, cargo-miri, runner_c3, the bench exe name).
# Idle wait for window 3 (the task's step C rule). Adapted from P0's wait_idle.ps1 with ONE added condition.
# Polls every 60 s. A poll is "quiet" when ALL of:
#   - no process named cargo, rustc, link, lld-link, dxc, clippy-driver, cl, msbuild exists;
#   - no process whose executable path is under D:\wt\_targets\ or D:\wt\mq-* (the lanes' test binaries);  [ADDED]
#   - the 10-second average of '\Processor(_Total)\% Processor Time' is below 5 %;
#   - (Get-Process).Count is in the hundreds (100..999; 0 would mean the probe is dead).
# The wait ends on 3 CONSECUTIVE quiet polls.
# Exit codes: 0 idle reached, 1 timeout (never idle), 3 D: below 2 GB.
param(
    [string]$Log = "wait_idle3_log.txt",
    [int]$MaxPolls = 120,
    [int]$PollSeconds = 60,
    [double]$CpuLimit = 5.0,
    [int]$Needed = 3
)
$ErrorActionPreference = 'SilentlyContinue'
$build = @('cargo', 'rustc', 'link', 'lld-link', 'dxc', 'clippy-driver', 'cl', 'msbuild', 'miri', 'cargo-miri', 'runner_c3', 'jolt_parity_pyramid-39550833a2ead4ba')
"# wait_idle3.ps1 start $(Get-Date -Format o); poll every $PollSeconds s, max $MaxPolls polls; quiet = build procs 0 AND lane-target procs 0 AND cpu10 < $CpuLimit % AND 100 <= proc count <= 999; need $Needed consecutive" | Out-File -FilePath $Log -Append -Encoding utf8
$streak = 0
for ($i = 1; $i -le $MaxPolls; $i++) {
    $t0 = Get-Date
    $p1 = @{}
    foreach ($x in Get-Process) { $p1[$x.Id] = @($x.Name, $x.CPU) }
    $cpu = $null
    try {
        $c = Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 10 -ErrorAction Stop
        $cpu = ($c.CounterSamples | Measure-Object CookedValue -Average).Average
    } catch { $cpu = $null }
    $procs = Get-Process
    $count = $procs.Count
    $top = @()
    foreach ($x in $procs) {
        $prev = $p1[$x.Id]
        $d = if ($prev -and $prev[0] -eq $x.Name -and $x.CPU -ne $null -and $prev[1] -ne $null) { $x.CPU - $prev[1] } elseif ($x.CPU -ne $null) { $x.CPU } else { 0 }
        $top += [pscustomobject]@{ Name = $x.Name; Id = $x.Id; d = $d }
    }
    $top3 = ($top | Sort-Object d -Descending | Select-Object -First 3 | ForEach-Object { "{0}({1})={2:N2}s" -f $_.Name, $_.Id, $_.d }) -join ' '
    $bp = @(Get-Process -Name $build -ErrorAction SilentlyContinue)
    $bpn = $bp.Count
    $bpNames = ($bp | ForEach-Object { "{0}({1})" -f $_.Name, $_.Id }) -join ','
    $lp = @($procs | Where-Object { $_.Path -like 'D:\wt\_targets\*' -or $_.Path -like 'D:\wt\mq-*' })
    $lpn = $lp.Count
    $lpNames = ($lp | ForEach-Object { "{0}({1})" -f $_.Name, $_.Id }) -join ','
    $dfree = [math]::Round((Get-PSDrive D).Free / 1GB, 2)
    $cpuOk = ($cpu -ne $null) -and ($cpu -lt $CpuLimit)
    $countOk = ($count -ge 100) -and ($count -le 999)
    $quiet = ($bpn -eq 0) -and ($lpn -eq 0) -and $cpuOk -and $countOk
    if ($quiet) { $streak++ } else { $streak = 0 }
    $cpuTxt = if ($cpu -ne $null) { "{0:N2}" -f $cpu } else { "PROBE-FAILED" }
    "poll {0,3} {1} cpu10={2}% procs={3} build_procs={4} [{5}] lane_target_procs={6} [{7}] D_free_GB={8} quiet={9} streak={10} top3(cpu s in ~10s): {11}" -f $i, $t0.ToString('o'), $cpuTxt, $count, $bpn, $bpNames, $lpn, $lpNames, $dfree, $quiet, $streak, $top3 | Out-File -FilePath $Log -Append -Encoding utf8
    if ($dfree -lt 2.0) {
        "# STOP: D: free $dfree GB < 2 GB at $(Get-Date -Format o)" | Out-File -FilePath $Log -Append -Encoding utf8
        exit 3
    }
    if ($streak -ge $Needed) {
        "# IDLE reached at $(Get-Date -Format o) after $i polls" | Out-File -FilePath $Log -Append -Encoding utf8
        exit 0
    }
    $elapsed = ((Get-Date) - $t0).TotalSeconds
    $rest = $PollSeconds - $elapsed
    if ($rest -gt 0) { Start-Sleep -Milliseconds ([int]($rest * 1000)) }
}
"# TIMEOUT: never idle after $MaxPolls polls at $(Get-Date -Format o)" | Out-File -FilePath $Log -Append -Encoding utf8
exit 1
