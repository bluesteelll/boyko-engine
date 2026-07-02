@echo off
REM RDG framegraph visual gate — launches the interactive viewer (A/B the graph vs hand barriers).
REM Controls: WASD + Space/Ctrl fly, mouse look, Esc quit, G = toggle framegraph barriers.
REM The frame must look IDENTICAL in both modes. A .cmd batch file sidesteps the PowerShell
REM script-execution policy that blocks the .ps1. Run from the repo root: .\run_framegraph_viewer.cmd
set RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu
set CARGO_INCREMENTAL=0
set BOYKO_DISABLE_VALIDATION=1
cargo test -p boyko_rhi_vulkan --test window_present_gbuffer engine_interactive_viewer -- --ignored --nocapture
