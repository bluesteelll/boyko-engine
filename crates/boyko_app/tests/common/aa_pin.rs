//! Render wave PX0 (H3 Scope 7; `px0/cut.md` D-A1) — the `BOYKO_AA` reading of the six AA
//! REFERENCE pins (`grand_showcase_2mat_aa_pin_dump`, `vb_mesh_tex_aa_pin_dump`).
//!
//! The variable is REQUIRED and closed: unset or an unknown word panics naming it. The eval dumps
//! these pins sit beside fall back to AA-off on a mistyped value (`grand_showcase_2mat.rs`'s knob),
//! which is right for an eval knob and wrong for a pin — an "FXAA reference" would be blessed as
//! the AA-off image.
//!
//! Pulled in with `#[path = "common/aa_pin.rs"] mod aa_pin;`.

#![allow(dead_code)] // each binary uses a subset

use boyko_app::EnginePlugins;
use boyko_render::{AaConfig, AaMode};

/// The AA mode an AA reference pin renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AaPin {
    /// `BOYKO_AA=fxaa`.
    Fxaa,
    /// `BOYKO_AA=smaa`.
    Smaa,
    /// `BOYKO_AA=ssaa` — the boot-fixed 2× supersample (`EnginePlugins::with_ssaa_scale(2)`).
    Ssaa,
}

/// The pin's mode from the value of `BOYKO_AA`.
///
/// # Panics
/// On `None` or a word outside `fxaa | smaa | ssaa`, naming `BOYKO_AA`.
pub fn parse_aa_pin(v: Option<&str>) -> AaPin {
    match v.map(str::trim) {
        Some("fxaa") => AaPin::Fxaa,
        Some("smaa") => AaPin::Smaa,
        Some("ssaa") => AaPin::Ssaa,
        other => panic!(
            "BOYKO_AA must be fxaa | smaa | ssaa for an AA reference pin (got {other:?}); an AA pin never renders AA-off"
        ),
    }
}

/// The pin's mode from the environment.
pub fn aa_pin_from_env() -> AaPin {
    parse_aa_pin(std::env::var("BOYKO_AA").ok().as_deref())
}

/// The plugins with the pin's boot-time arming (SSAA's scale), at `w × h`.
pub fn plugins(title: &'static str, w: u32, h: u32, pin: AaPin) -> EnginePlugins {
    let p = EnginePlugins::window(title, w, h);
    if pin == AaPin::Ssaa { p.with_ssaa_scale(2) } else { p }
}

/// The run-time arming (FXAA / SMAA; SSAA's `AaConfig` is the runner's own, host-authoritative).
pub fn config(pin: AaPin) -> Option<AaConfig> {
    match pin {
        AaPin::Fxaa => Some(AaConfig { mode: AaMode::Fxaa }),
        AaPin::Smaa => Some(AaConfig { mode: AaMode::Smaa }),
        AaPin::Ssaa => None,
    }
}
