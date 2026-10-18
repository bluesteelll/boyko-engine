//! Render wave PX0 (`px0/cut.md` PX0-9; AD-20) — **`scripts/golden.ps1`'s tolerance branch, run
//! for real** on a scratch pins file and synthetic BMPs, inside `cargo test --workspace`.
//!
//! Every case goes through `golden.ps1 -PinsFile <scratch> -CompareOnly <bmp>`: no build, no
//! render, the same verdict code the sweeps read. The scratch file lives under
//! `CARGO_TARGET_TMPDIR`, outside `goldens\`, which is the only place `-CompareOnly` accepts; the
//! last case proves the refusal against the real `goldens\PINS.toml`.
//!
//! Windows only (the script is Windows PowerShell 5.1); spawned `-NoProfile -ExecutionPolicy
//! Bypass` (`px0/critique.md` W6). The tolerance branch needs Python >= 3.11, found by the same
//! rule as `golden_diff_selftest` — none is a RED, never a skip.

#![cfg(windows)]

#[path = "tool_runner/mod.rs"]
mod tool_runner;

use std::path::{Path, PathBuf};

/// A 4x4 32-bpp bottom-up BMP, every pixel `(10, 20, 30)` but pixel 3's red, which is `10 + delta`.
fn write_bmp(path: &Path, delta: u8) {
    let (w, h) = (4u32, 4u32);
    let mut px = [[10u8, 20, 30]; 16];
    px[3][0] = 10 + delta;
    let data_len = w * h * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b] = px[(y * w + x) as usize];
            out.extend_from_slice(&[b, g, r, 255]);
        }
    }
    std::fs::write(path, out).expect("write the synthetic BMP");
}

/// The scratch directory of one case.
fn scratch(case: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden_tolerance_selftest").join(case);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create the scratch dir");
    d
}

/// A scratch pins file with one tolerance pin `t` (reference `ref`) and one hash pin `h`.
fn pins(dir: &Path, reference: &Path, max_delta: &str, bound: &str, sha: &str) -> PathBuf {
    let toml = format!(
        "[t]\ncrate = \"x\"\ntest_binary = \"x\"\ntest_name = \"x\"\nbmp = \"x.bmp\"\nsha256_software = \"PENDING\"\n\n\
         [t.tolerance]\nreason = \"self-test\"\nreference_software = \"{}\"\nmax_delta = {max_delta}\n\
         max_changed_frac = 0.5\nmax_changed_frac_lit = 0.5\norder_noise_steps = 1\nmax_delta_bound = {bound}\n\n\
         [h]\ncrate = \"x\"\ntest_binary = \"x\"\ntest_name = \"x\"\nbmp = \"x.bmp\"\nsha256_software = \"{sha}\"\n",
        reference.display().to_string().replace('\\', "/")
    );
    let p = dir.join("PINS.toml");
    std::fs::write(&p, toml).expect("write the scratch pins");
    p
}

/// Runs `golden.ps1 -Pin <pin> -PinsFile <pins> -CompareOnly <bmp> [-Bless]`; the exit code and
/// the merged output.
fn golden(pin: &str, pins: &Path, bmp: &Path, bless: bool) -> (i32, String) {
    let script = tool_runner::repo_root().join("scripts").join("golden.ps1");
    let mut c = tool_runner::powershell_file(&script);
    c.args(["-Pin", pin, "-PinsFile"]).arg(pins).arg("-CompareOnly").arg(bmp);
    if bless {
        c.arg("-Bless");
    }
    let out = c.current_dir(tool_runner::repo_root()).output().expect("powershell spawns");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn sha256_of(path: &Path) -> String {
    let o = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-FileHash -LiteralPath '{}' -Algorithm SHA256).Hash.ToLower()", path.display())])
        .output()
        .expect("powershell spawns");
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

fn python_or_red() {
    if let Err(why) = tool_runner::find_python() {
        panic!("the tolerance branch needs Python >= 3.11: {why}");
    }
}

#[test]
fn tolerance_verdicts_bless_and_refusals() {
    python_or_red();
    let d = scratch("verdicts");
    let reference = d.join("ref.bmp");
    write_bmp(&reference, 0);
    let p = pins(&d, &reference, "1", "3", "0");
    let leg = |delta: u8| {
        let b = d.join(format!("leg{delta}.bmp"));
        write_bmp(&b, delta);
        b
    };

    let (rc, out) = golden("t", &p, &leg(0), false);
    assert_eq!(rc, 0, "an identical dump passes:\n{out}");
    let (rc, out) = golden("t", &p, &leg(2), false);
    assert_eq!(rc, 1, "+2 at max_delta = 1 fails:\n{out}");
    let (rc, out) = golden("t", &p, &leg(5), false);
    assert_eq!(rc, 4, "an observed delta past max_delta_bound is a DEFECT (exit 4):\n{out}");

    // -Bless writes the reference.
    let (rc, out) = golden("t", &p, &leg(2), true);
    assert_eq!(rc, 0, "bless:\n{out}");
    assert_eq!(std::fs::read(&reference).expect("reference"), std::fs::read(leg(2)).expect("leg"));

    // A PENDING measured field refuses bless and leaves the reference alone.
    let pending = pins(&d, &reference, "PENDING", "3", "0");
    let before = std::fs::read(&reference).expect("reference");
    let (rc, out) = golden("t", &pending, &leg(0), true);
    assert_ne!(rc, 0, "bless with a PENDING field must be refused:\n{out}");
    assert!(out.contains("PENDING"), "the refusal names the PENDING field:\n{out}");
    assert_eq!(std::fs::read(&reference).expect("reference"), before, "a refused bless wrote the reference");
}

#[test]
fn a_hash_pin_still_compares_sha256() {
    let d = scratch("hash");
    let bmp = d.join("leg.bmp");
    write_bmp(&bmp, 1);
    let sha = sha256_of(&bmp);
    assert_eq!(sha.len(), 64, "Get-FileHash answered `{sha}`");
    let reference = d.join("ref.bmp");
    write_bmp(&reference, 0);
    let p = pins(&d, &reference, "1", "3", &sha);
    let (rc, out) = golden("h", &p, &bmp, false);
    assert_eq!(rc, 0, "a matching hash passes:\n{out}");
    let other = d.join("other.bmp");
    write_bmp(&other, 2);
    let (rc, out) = golden("h", &p, &other, false);
    assert_eq!(rc, 1, "a differing hash fails:\n{out}");
}

#[test]
fn compare_only_is_refused_against_the_real_pins() {
    let d = scratch("refusal");
    let bmp = d.join("leg.bmp");
    write_bmp(&bmp, 0);
    let real = tool_runner::repo_root().join("goldens").join("PINS.toml");
    let (rc, out) = golden("grand_showcase", &real, &bmp, false);
    assert_ne!(rc, 0, "-CompareOnly against goldens\\PINS.toml must be refused:\n{out}");
    assert!(out.contains("refused"), "the refusal says so:\n{out}");
}
