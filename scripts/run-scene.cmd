@echo off
rem Wrapper so the launcher runs regardless of the PowerShell execution policy
rem (a .cmd is not blocked by it; it invokes PowerShell with -ExecutionPolicy Bypass
rem for THIS call only -- no system/security setting is changed).
rem   scripts\run-scene.cmd -Scene paradigm_lab -Path vb -Legs both
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-scene.ps1" %*
