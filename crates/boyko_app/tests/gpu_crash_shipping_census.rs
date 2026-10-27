//! Render rung GB — **zero cost in a player build, shown on the linked image.**
//!
//! Every GB site is gated `GpuCrashDiagLevel::COMPILED_IN && <armed byte>`, and `COMPILED_IN` is a
//! `const false` under `BOYKO_PROFILE=shipping` (`GPU_CRASH_DIAG_CEILING = 0`). The claim is that
//! the sites, their `#[cold]` bodies and the extension entry points they would load are then NOT
//! IN THE BINARY. This test builds the `clear` example — the smallest program that links the
//! windowed runner and `render_gbuffer_frame` — twice, under `BOYKO_PROFILE=dev` and `=shipping`,
//! each into its own target directory, and byte-scans each image for two kinds of witness:
//!
//! - the NUL-terminated names only GB loads (its entry points and device extensions);
//! - a string literal only a GB per-frame `#[cold]` BODY references (the frame scope's label,
//!   reached only through the funnels' and host steps' scope entry).
//!
//! The second kind exists because the first cannot see the sites (GB review r1 F3): the loads fold
//! through `GpuCrashDiagLevel::effective()`'s own `const` clamp, so with `COMPILED_IN` forced
//! `true` every loader name still reads `shipping 0` while the per-frame sites and their bodies
//! come back (+46 KB of image). A body literal comes back with its body.
//!
//! `COMPILED_IN` itself is pinned at compile time (`boyko_rhi_vulkan/src/gpu_crash.rs`'s two
//! `const` assertions), which the nested `shipping` build below evaluates like CI's `profile-legs`.
//!
//! # Why a byte scan of the image
//!
//! The MSVC image carries no symbol table (`profile_fixture/tests/profile_axis_census.rs`'s second
//! finding), but a `vkGetInstanceProcAddr` / `vkGetDeviceProcAddr` name is a C-string literal in
//! `.rdata`, and the release profile is fat LTO, so a literal survives exactly when some reachable
//! code references it. `VK_EXT_debug_utils` itself is not scanned: validation enables it too.
//!
//! # The verdict
//!
//! - the `dev` image holds EVERY name and EVERY body literal — the positive control; a zero there
//!   means the scanner (or the subject) went blind, which is RED, never a pass;
//! - the `shipping` image holds NONE of them.
//!
//! RED shown at the rung: replace `boot_with_instance`'s `COMPILED_IN && level != Off` gate on the
//! `arm_gpu_crash` call, and the `labels_enabled` it passes, with an opaque
//! `core::hint::black_box(true)` — a loader whose gate the ceiling no longer folds — and the
//! shipping image holds every entry-point name. (Replacing the call's gate alone stays green: the
//! instance-side `want_labels` is folded too, so the loads stay dead. Two folds, each sufficient.)
//!
//! RED shown at review round 1 (F3): drop `COMPILED_IN &&` from `Renderer::crumbs_armed` and the
//! shipping image holds the per-frame body literal while every loader name still reads 0; force
//! `COMPILED_IN` to `true` and the nested `shipping` build fails on the `const` assertion.
//!
//! # Running it
//!
//! ```text
//! cargo test -p boyko-app --test gpu_crash_shipping_census -- --ignored --exact \
//!     gb_sites_are_absent_from_shipping --test-threads=1 --nocapture
//! ```
//!
//! Two nested release builds (minutes each on a cold directory), no device. The target
//! directories are `$CARGO_TARGET_DIR`'s siblings `gb-census-dev` / `gb-census-shipping` (or
//! `target/gb-census-*` without the variable).
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The NUL-terminated names only GB references: its entry points and its device extensions.
const GB_ONLY_NAMES: &[&str] = &[
    "vkCmdBeginDebugUtilsLabelEXT",
    "vkCmdEndDebugUtilsLabelEXT",
    "vkSetDebugUtilsObjectNameEXT",
    "vkCmdWriteBufferMarkerAMD",
    "vkCmdSetCheckpointNV",
    "vkGetQueueCheckpointDataNV",
    "vkGetDeviceFaultInfoEXT",
    "VK_AMD_buffer_marker",
    "VK_NV_device_diagnostic_checkpoints",
    "VK_EXT_device_fault",
];

/// String literals only a GB `#[cold]` body references. Rust literals carry no NUL, so these are
/// counted as plain byte runs; each is unique in the tree.
/// - `frame_forward_plus`: `present/gpu_crumbs.rs`'s `FRAME_LABELS`, read only by the frame
///   scope's opening inside `enter_scope` — the per-frame bodies (`crumb_enter_pass`,
///   `crumb_enter_host`). An indexed table, so the literal stays a byte run in `.rdata`.
///
/// A literal whose only use is a copy at a known index is not a witness: `CrumbState::new`'s
/// command-buffer name `frame_cmd_0` is copied into a fixed label buffer, and the measured `dev`
/// image holds no `frame_cmd_0` run at all (review r1, F3's first green run, where this positive
/// control caught it; the copy of a constant most likely became immediate stores).
const GB_BODY_LITERALS: &[&str] = &["frame_forward_plus"];

/// Every per-knob override the profile axis honours only under `custom`: a stray one beside a
/// named profile is a `compile_error!`, so the nested builds must not inherit any.
const PROFILE_KNOBS: &[&str] = &[
    "BOYKO_PROFILING_TIER",
    "BOYKO_PROFILING_REGION_CAPACITY",
    "BOYKO_PROFILING_DYN_CAP",
    "BOYKO_LOG_MAX_LEVEL",
    "BOYKO_GPU_CRASH_DIAG_MAX",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn target_dir(profile: &str) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(t) => PathBuf::from(t).join("..").join(format!("gb-census-{profile}")),
        None => workspace_root().join("target").join(format!("gb-census-{profile}")),
    }
}

/// Builds `boyko-app`'s `clear` example in release under `BOYKO_PROFILE=<profile>` and returns
/// the image's path.
fn build_clear(profile: &str) -> PathBuf {
    let dir = target_dir(profile);
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(workspace_root())
        .args(["build", "--release", "--locked", "-p", "boyko-app", "--example", "clear"])
        .env("BOYKO_PROFILE", profile)
        .env("CARGO_TARGET_DIR", &dir);
    for knob in PROFILE_KNOBS {
        cmd.env_remove(knob);
    }
    let status = cmd.status().unwrap_or_else(|e| panic!("test setup: spawn cargo: {e}"));
    assert!(status.success(), "the {profile} build of the clear example failed: {status:?}");
    let image = dir.join("release").join("examples").join("clear.exe");
    assert!(image.is_file(), "the {profile} build produced no {}", image.display());
    image
}

/// How many times `name` followed by a NUL occurs in `bytes`.
fn count_cstr(bytes: &[u8], name: &str) -> usize {
    let mut needle = name.as_bytes().to_vec();
    needle.push(0);
    count_bytes(bytes, &needle)
}

/// How many times `needle` occurs in `bytes` (overlapping runs counted).
fn count_bytes(bytes: &[u8], needle: &[u8]) -> usize {
    bytes.windows(needle.len()).filter(|w| *w == needle).count()
}

#[test]
#[ignore = "slow: two nested release builds of the boyko-app clear example (BOYKO_PROFILE=dev and =shipping, own target dirs); cargo test -p boyko-app --test gpu_crash_shipping_census -- --ignored --exact gb_sites_are_absent_from_shipping --test-threads=1 --nocapture"]
fn gb_sites_are_absent_from_shipping() {
    let dev = std::fs::read(build_clear("dev")).expect("test setup: read the dev image");
    let shipping = std::fs::read(build_clear("shipping")).expect("test setup: read the shipping image");
    let mut red = Vec::new();
    for name in GB_ONLY_NAMES {
        let (d, s) = (count_cstr(&dev, name), count_cstr(&shipping, name));
        println!("[GB-census] {name}: dev {d}, shipping {s}");
        if d == 0 {
            red.push(format!("{name}: absent from the DEV image -- the positive control is blind"));
        }
        if s != 0 {
            red.push(format!("{name}: present {s}x in the SHIPPING image -- a GB site survived the ceiling"));
        }
    }
    for lit in GB_BODY_LITERALS {
        let (d, s) = (count_bytes(&dev, lit.as_bytes()), count_bytes(&shipping, lit.as_bytes()));
        println!("[GB-census] body literal {lit}: dev {d}, shipping {s}");
        if d == 0 {
            red.push(format!("body literal {lit}: absent from the DEV image -- the positive control is blind"));
        }
        if s != 0 {
            red.push(format!(
                "body literal {lit}: present {s}x in the SHIPPING image -- a GB #[cold] body survived the ceiling"
            ));
        }
    }
    println!("[GB-census] dev image {} B, shipping image {} B", dev.len(), shipping.len());
    assert!(red.is_empty(), "GB zero-cost census:\n{}", red.join("\n"));
}

#[test]
fn the_scanner_counts_nul_terminated_names_only() {
    let bytes = b"xxvkCmdEndDebugUtilsLabelEXT\0vkCmdEndDebugUtilsLabelEXTZ\0";
    assert_eq!(count_cstr(bytes, "vkCmdEndDebugUtilsLabelEXT"), 1);
    assert_eq!(count_cstr(b"", "vkCmdEndDebugUtilsLabelEXT"), 0);
}

#[test]
fn the_scanner_counts_body_literals_without_a_nul() {
    let bytes = b"frame_deferredframe_forward_plusframe_vbframe_forward_plus";
    assert_eq!(count_bytes(bytes, b"frame_forward_plus"), 2);
    assert_eq!(count_bytes(bytes, b"frame_vb"), 1);
    assert_eq!(count_bytes(b"frame_forward_plu", b"frame_forward_plus"), 0);
}
