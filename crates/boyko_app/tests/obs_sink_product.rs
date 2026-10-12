//! **OBS — the sink product** (render-wave plan H0 Scope 3, device test 2).
//!
//! The (P) sink check runs on every debug golden frame, but the goldens SAMPLE the conditional
//! tails; this gate ENUMERATES them. Per declarator, every reachable combination of {interp,
//! particles, a material upload} boots [`obs_sink_worker`] once with the graph receipt armed, and
//! the receipt's sink table must be consistent: every `[[sink]]` slot names the `ResId` it
//! resolves, and every declared resource a pass touches has a slot. A drifted positional fill
//! fires the (P) assert inside the worker (it exits non-zero, before any receipt), and the
//! receipt check catches the same drift on its own (`sink_check`).
//!
//! Deferred carries all three tails (8 boots); Forward, ForwardPlus and VB declare no interp
//! buffer tail, so they enumerate {particles, upload} (4 boots each). With `upload` off the frame
//! the receipt reads is past frame 0, the only upload frame such a run has.
//!
//! **Debug builds only**: the slot metadata does not exist in a release build, and a release
//! receipt says so — the drivers are then RED with that reason, never a vacuous green.
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`,
//! `BOYKO_WINDOW_FRAMES` set (each driver also sets a per-worker backstop).

#[path = "common/graph_receipt.rs"]
mod graph_receipt;

use boyko_render::RenderPath;

use graph_receipt::{
    ENV_INTERP, ENV_PARTICLES, ENV_PATH, ENV_UPLOAD, ObsArming, Receipt, assert_obs_resolved,
    build_obs_app, fresh_out, run_worker, sink_check,
};

/// The worker's settle window: short — the tails are per-frame facts, nothing needs to converge.
const SETTLE: u32 = 4;

/// **The sink-product WORKER** — one boot of the arming its driver's env names. SKIPS without
/// `BOYKO_GRAPH_RECEIPT` (no armed capture could end its loop).
#[test]
#[ignore = "gpu-windowed: the OBS sink-product worker, spawned by the obs_sink_product_* drivers with BOYKO_GRAPH_RECEIPT set; run with --test-threads=1"]
fn obs_sink_worker() {
    if std::env::var_os("BOYKO_GRAPH_RECEIPT").is_none() {
        eprintln!("obs_sink_worker: BOYKO_GRAPH_RECEIPT is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = ObsArming::from_env();
    let mut app = build_obs_app("boyko_app OBS sink worker", arming);
    app.run();
    assert_obs_resolved(&app, arming);
}

fn flag(b: bool) -> String {
    if b { "1".to_owned() } else { "0".to_owned() }
}

/// Boots every combination of `path`'s tails and checks each sink table.
fn product(path: RenderPath, with_interp: bool) {
    let word = ObsArming::path_word(path);
    let interps: &[bool] = if with_interp { &[false, true] } else { &[false] };
    let mut failures = Vec::new();
    let mut boots = 0;
    for &interp in interps {
        for particles in [false, true] {
            for upload in [false, true] {
                let tag = format!("{word}_i{}_p{}_u{}", u8::from(interp), u8::from(particles), u8::from(upload));
                let out = fresh_out(&format!("sink_{tag}.toml"));
                run_worker(
                    "obs_sink_worker",
                    &[
                        ("BOYKO_GRAPH_RECEIPT", out.display().to_string()),
                        ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
                        (ENV_PATH, word.to_owned()),
                        (ENV_INTERP, flag(interp)),
                        (ENV_PARTICLES, flag(particles)),
                        (ENV_UPLOAD, flag(upload)),
                        ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
                    ],
                    false,
                );
                boots += 1;
                let r = Receipt::load(&out);
                // The combination must be the one asked for — otherwise this boot enumerated
                // nothing new.
                let want = [
                    ("path_has_particles", u64::from(particles)),
                    ("material_upload", u64::from(upload)),
                ];
                for (key, v) in want {
                    if r.config(key) != v {
                        failures.push(format!("{tag}: PREMISE [config] {key} = {}, the boot asked for {v}", r.config(key)));
                    }
                }
                if with_interp && r.config("interp") != u64::from(interp) {
                    failures.push(format!("{tag}: PREMISE [config] interp = {}, the boot asked for {}", r.config("interp"), u8::from(interp)));
                }
                match sink_check(&r) {
                    Ok(n) => eprintln!("{tag}: {n} sink rows consistent"),
                    Err(e) => failures.push(format!("{tag}: {e}")),
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "OBS sink product on {path:?} ({boots} boots, {} failure(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore = "gpu-windowed: OBS device test 2 on Deferred, 8 boots (interp x particles x upload) of obs_sink_worker; run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_sink_product_deferred() {
    product(RenderPath::Deferred, true);
}

#[test]
#[ignore = "gpu-windowed: OBS device test 2 on Forward, 4 boots (particles x upload) of obs_sink_worker; run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_sink_product_forward() {
    product(RenderPath::Forward, false);
}

#[test]
#[ignore = "gpu-windowed: OBS device test 2 on ForwardPlus, 4 boots (particles x upload) of obs_sink_worker; run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_sink_product_forwardplus() {
    product(RenderPath::ForwardPlus, false);
}

#[test]
#[ignore = "gpu-windowed: OBS device test 2 on VisibilityBuffer, 4 boots (particles x upload) of obs_sink_worker; run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_sink_product_vb() {
    product(RenderPath::VisibilityBuffer, false);
}
