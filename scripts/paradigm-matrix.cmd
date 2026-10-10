@echo off
rem Wrapper so the matrix sweep runs regardless of the PowerShell execution policy
rem (a .cmd is not blocked by it; it invokes PowerShell with -ExecutionPolicy Bypass
rem for THIS call only -- no system/security setting is changed).
rem   scripts\paradigm-matrix.cmd -Scene paradigm_lab -Legs both
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0paradigm-matrix.ps1" %*
