# Measure the Claude desktop app's CPU (the session's UI host, NOT claude-code) over 10 s, optionally minimize its window, measure again.
param([switch]$Minimize)
$ErrorActionPreference='SilentlyContinue'
function Sample([int]$s) {
  $a=@{}; foreach ($p in Get-Process claude) { $a[$p.Id]=$p.CPU }
  Start-Sleep -Seconds $s
  $out=@()
  foreach ($p in Get-Process claude) { if ($a.ContainsKey($p.Id)) { $d=$p.CPU-$a[$p.Id]; if ($d -gt 0.05) { $out += ("{0}={1:N2}s" -f $p.Id,$d) } } }
  "$(Get-Date -Format HH:mm:ss) over ${s}s: " + ($out -join ' ')
}
Sample 10
if ($Minimize) {
  $sig='[DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);'
  Add-Type -MemberDefinition $sig -Name W -Namespace U
  foreach ($p in Get-Process claude) { if ($p.MainWindowHandle -ne 0) { "minimize $($p.Id) '$($p.MainWindowTitle)' -> " + [U.W]::ShowWindowAsync($p.MainWindowHandle, 6) } }
  Start-Sleep -Seconds 3
  Sample 10
}
