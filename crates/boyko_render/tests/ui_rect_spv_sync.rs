//! The UI-rect shaders' `.hlsl` ↔ `.spv` BYTE GATE (`docs/UI-PLAN-SPRITES.md` rung S1, gate
//! G1-2; architecture D30) — the `particle_edsl_sync` layer-2 idiom applied to
//! `shaders/ui_rect.{vs,fs}.hlsl`.
//!
//! Each committed `.spv` is the re-DXC of its own source under the frozen recipe pinned in
//! that source's header (`-spirv -T {vs,ps}_6_0 -E main -fspv-target-env=vulkan1.3`, no
//! `-O`, no `-D` — the UI family has NO variant axis; both rows in
//! `docs/SHADER-VARIANT-MANIFEST.md` state that). Before this rung the only pin on these two
//! binaries was the const-generic byte LENGTH (`SpirvBlob<2368>` / `SpirvBlob<7060>`,
//! `src/ui/mod.rs`), which catches a size change but NOT a re-compile drift at the same size
//! — this file is the gate D30 found missing.
//!
//! SKIPS (with an `eprintln`) when no `dxc` resolves; **a skipped run is not a pass** (the
//! PARTICLES-PLAN F15 rule, restated by S1's gate table): the rung is not called done on a
//! host without the pinned VulkanSDK 1.4.350.0 toolchain, and the skip line exists so the
//! person reporting has something to quote.

use std::path::PathBuf;
use std::process::Command;

// ---- Shared plumbing (the `particle_edsl_sync` idioms) --------------------------------------

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and
/// `.spv` live (and where DXC runs so any future `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: the pinned Vulkan-SDK path, then `$VULKAN_SDK/Bin`, then
/// `PATH`. `None` ⇒ the byte-identity tests SKIP.
fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// One `ui_rect` artifact row: `<hlsl_stem>.hlsl` compiled at `profile` into
/// `<spv_stem>.spv`. NO `-D` axis (each source compiles to exactly one artifact).
#[derive(Clone, Copy)]
struct UiArtifact {
    hlsl_stem: &'static str,
    profile: &'static str,
    spv_stem: &'static str,
}

/// The complete `ui_rect` artifact census — two sources, two binaries, no defines. The
/// manifest's UI section mirrors these rows.
const UI_ARTIFACTS: [UiArtifact; 2] = [
    UiArtifact {
        hlsl_stem: "ui_rect.vs",
        profile: "vs_6_0",
        spv_stem: "ui_rect.vs",
    },
    UiArtifact {
        hlsl_stem: "ui_rect.fs",
        profile: "ps_6_0",
        spv_stem: "ui_rect.fs",
    },
];

/// Re-DXCs one [`UiArtifact`] under the EXACT frozen recipe its header pins into a fresh temp
/// `.spv` and returns the bytes. Never overwrites a committed artifact.
fn redxc(dxc: &PathBuf, dir: &PathBuf, a: UiArtifact) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{}.spv.redxc.spv", a.spv_stem));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir)
        .args(["-spirv", "-T", a.profile, "-E", "main"]);
    cmd.arg("-fspv-target-env=vulkan1.3");
    cmd.arg(format!("{}.hlsl", a.hlsl_stem)).arg("-Fo").arg(&out_spv);
    let status = cmd
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(
        status.success(),
        "dxc failed re-compiling {}.hlsl under the frozen recipe for {}.spv",
        a.hlsl_stem,
        a.spv_stem
    );
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

/// Asserts one artifact's committed `.spv` byte-equals its own re-DXC.
fn assert_spv_byte_identical(spv_stem: &str) {
    let a = UI_ARTIFACTS
        .into_iter()
        .find(|a| a.spv_stem == spv_stem)
        .expect("invariant: every byte gate names a row of UI_ARTIFACTS");
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP {spv_stem}_spv_byte_identical: no dxc on this host");
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join(format!("{spv_stem}.spv"));
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc(&dxc, &dir, a);
    assert!(
        committed == fresh,
        "{spv_stem}.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of {}.hlsl \
         under the frozen recipe — either the committed .spv is stale (re-run the recipe in \
         the shader's header and commit it) or the host dxc is not the pinned VulkanSDK \
         1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
        a.hlsl_stem,
    );
}

// ---- The byte gates (gate G1-2) -------------------------------------------------------------

#[test]
fn ui_rect_vs_spv_byte_identical() {
    assert_spv_byte_identical("ui_rect.vs");
}

#[test]
fn ui_rect_fs_spv_byte_identical() {
    assert_spv_byte_identical("ui_rect.fs");
}

/// The census is closed in the direction that leaks (the `particle_edsl_sync` lesson): every
/// gate above walks `UI_ARTIFACTS`, so it can only check artifacts it was told about — a
/// third `ui_rect_*.spv` dropped into `shaders/` tomorrow (a `-D` variant, a hand-compiled
/// experiment) would ship ungated while passing every test here by being invisible to them.
/// Discovery is by file-name prefix, exact for this family: `emit_ui` owns every `ui_rect_*`
/// source in the directory.
#[test]
fn every_committed_ui_rect_artifact_has_a_row() {
    let dir = shaders_dir();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot enumerate the shader directory {}: {e}", dir.display()));
    let mut found: Vec<String> = entries
        .map(|e| e.expect("invariant: the shader directory is readable entry by entry"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("ui_rect") && n.ends_with(".spv"))
        .map(|n| n.trim_end_matches(".spv").to_string())
        .collect();
    found.sort();

    let mut enumerated: Vec<String> =
        UI_ARTIFACTS.iter().map(|a| a.spv_stem.to_string()).collect();
    enumerated.sort();

    assert_eq!(
        found, enumerated,
        "the committed ui_rect `.spv` set and UI_ARTIFACTS have diverged. An artifact on the \
         LEFT and not the right is UNGATED — nothing re-DXCs it and a stale copy would ship \
         silently. One on the RIGHT and not the left means a row names an artifact nobody \
         builds."
    );
}
