# RDG framegraph visual gate — launches the interactive viewer so you can A/B the
# auto-derived framegraph barrier path against the hand-authored one.
#
# Controls:  WASD + Space/Ctrl = fly,  mouse = look,  Esc = quit,
#            G = TOGGLE the framegraph-driven barriers (OFF = hand path, ON = graph).
#
# What to check: fly around a bit, then tap G repeatedly. The image MUST look
# IDENTICAL in both modes (the graph is sound-superset: same dependencies, same
# pixels). Any flicker / wrong shadow / garbage / crash on G = a real defect —
# tell Claude. The console prints "use_framegraph = true/false" on each toggle.
#
# Validation layer is OFF here (it is crash-prone on this box); the pixel A/B is the
# gate. For the STRONGER check, delete the BOYKO_DISABLE_VALIDATION line below and
# re-run — if the validation layer stays quiet with G ON, the barriers are proven.

$env:RUSTUP_TOOLCHAIN     = "stable-x86_64-pc-windows-gnu"
$env:CARGO_INCREMENTAL    = "0"
$env:BOYKO_DISABLE_VALIDATION = "1"

cargo test -p boyko_rhi_vulkan --test window_present_gbuffer engine_interactive_viewer `
    -- --ignored --nocapture
