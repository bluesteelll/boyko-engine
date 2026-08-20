//! Rung A5 end-to-end: an `aether!` block's `material` builders mint REAL assets through a REAL
//! `App` — `Assets<Material>::add` on a startup system, the handles resolved back out of the
//! world, and every lane of the 48-byte `MaterialGpu` checked against §3.6's numbers.
//!
//! # What only this test can catch
//!
//! The unit snapshots pin the TOKENS Aether emits. They cannot tell whether those tokens name a
//! constructor that exists, in the argument order it actually has: `Material::new`'s six
//! parameters are four `f32`-ish scalars in a row (`metallic, roughness, reflectance`), so a
//! transposition type-checks perfectly and only a VALUE assertion exposes it. That is the §8 R4
//! anti-drift gate's real content — hence `roughness: 0.14` and `reflectance` defaulted to `0.5`
//! on `gold`: two DIFFERENT numbers in adjacent slots, so a swap fails here.
//!
//! # The textured half
//!
//! `MATERIAL_FLAG_TEXTURED` is derived by the engine, never by Aether. The `textures:` escape is
//! therefore asserted by its OBSERVABLE: the flag bit is set in `gpu.mrr[3]` for the textured
//! material and clear for the two that omit the key — which holds only if the emission really
//! routed through `Material::with_textures` (the sole constructor that sets it) rather than
//! `Material::new` (which never does).

use aether::aether;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::{Assets, Handle};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{MATERIAL_FLAG_TEXTURED, Material, MaterialTextures};

aether! {
    material gold { base: (1.0, 0.72, 0.30), metallic: 1.0, roughness: 0.14 }

    material lamp { base: (0.02, 0.02, 0.02), roughness: 0.6, emissive: (1.6, 0.9, 0.3) }

    material shipping_crate {
        base: (0.8, 0.8, 0.8),
        roughness: 0.4,
        textures: MaterialTextures { albedo: ALBEDO_SLOT, ..MaterialTextures::NONE },
    }
}

/// A bindless slot the `textures:` escape carries verbatim — an EXPRESSION, not a literal, so the
/// test also covers §2's "values are verbatim Rust fragments" rule for that key.
const ALBEDO_SLOT: u32 = 7;

/// The mint channel: aether materials are plain fns (no captures), so the handles a startup system
/// mints reach the assertions through a resource, exactly as a game would carry them.
#[derive(boyko_macros::Resource)]
struct Minted {
    gold: Option<Handle<Material>>,
    lamp: Option<Handle<Material>>,
    shipping_crate: Option<Handle<Material>>,
}

#[test]
fn aether_materials_mint_handles_that_resolve_through_assets() {
    let mut app = App::new();
    // The world-global CPU authority the host inserts at boot (`runner.rs`'s
    // `Assets::<Material>::with_reserved`) — a plain `Resource`, no GPU device involved.
    app.insert_resource(Assets::<Material>::with_reserved(8));
    app.insert_resource(Minted { gold: None, lamp: None, shipping_crate: None });

    app.add_startup_system(|mut materials: ResMut<Assets<Material>>, mut out: ResMut<Minted>| {
        out.gold = Some(materials.add(gold()));
        out.lamp = Some(materials.add(lamp()));
        out.shipping_crate = Some(materials.add(shipping_crate()));
    });

    app.update();

    let minted = app.world().resource::<Minted>();
    let (h_gold, h_lamp, h_crate) = (
        minted.gold.expect("invariant: the startup system minted `gold`"),
        minted.lamp.expect("invariant: the startup system minted `lamp`"),
        minted.shipping_crate.expect("invariant: the startup system minted `shipping_crate`"),
    );

    let materials = app.world().resource::<Assets<Material>>();

    // --- `gold`: the §3.6 before/after pair's first row, lane by lane.
    let gold_asset = *materials.get(h_gold).expect("the minted `gold` handle resolves");
    assert_eq!(gold_asset.gpu.base_color, [1.0, 0.72, 0.30, 1.0], "rgb + the synthesized alpha");
    // `mrr` = [metallic, roughness, reflectance, bitcast(flags)]: two given, one defaulted.
    assert_eq!(gold_asset.gpu.mrr[0], 1.0, "metallic, as written");
    assert_eq!(gold_asset.gpu.mrr[1], 0.14, "roughness, as written (NOT reflectance's default)");
    assert_eq!(gold_asset.gpu.mrr[2], 0.5, "reflectance defaults to the standard 4% F0 scale");
    assert_eq!(gold_asset.gpu.emissive, [0.0, 0.0, 0.0, 0.0], "emissive defaults to [0.0; 3]");
    assert_eq!(gold_asset.textures, MaterialTextures::NONE, "no `textures:` key ⇒ no slots bound");
    // The WHOLE `flags` lane, not just the textured bit: `mrr[3]` is `bitcast(flags)`, so a
    // drifted default (0 → 2) leaves the mask below satisfied and every other assertion green.
    assert_eq!(gold_asset.gpu.mrr[3].to_bits(), 0, "`flags` defaults to 0, whole lane");
    assert_eq!(
        gold_asset.gpu.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED,
        0,
        "`Material::new` never sets MATERIAL_FLAG_TEXTURED — the pre-T5 byte-identity contract"
    );

    // --- `lamp`: the second row — an emissive color, and metallic left to its default.
    let lamp_asset = *materials.get(h_lamp).expect("the minted `lamp` handle resolves");
    assert_eq!(lamp_asset.gpu.base_color, [0.02, 0.02, 0.02, 1.0], "rgb + the synthesized alpha");
    assert_eq!(lamp_asset.gpu.mrr[0], 0.0, "metallic defaults to 0.0");
    assert_eq!(lamp_asset.gpu.mrr[1], 0.6, "roughness, as written");
    assert_eq!(lamp_asset.gpu.mrr[2], 0.5, "reflectance defaults");
    assert_eq!(lamp_asset.gpu.mrr[3].to_bits(), 0, "`flags` defaults to 0, whole lane");
    // `MaterialGpu::emissive` is a vec4 lane whose `w` the constructor zeroes.
    assert_eq!(lamp_asset.gpu.emissive, [1.6, 0.9, 0.3, 0.0], "the emissive color, as written");

    // --- the `textures:` escape: the engine's ONLY textured constructor ran.
    let crate_asset = *materials.get(h_crate).expect("the minted `shipping_crate` handle resolves");
    assert_eq!(crate_asset.textures.albedo, ALBEDO_SLOT, "the verbatim slot expression");
    assert_eq!(crate_asset.textures.normal, 0, "the struct-update tail left the rest unbound");
    assert_ne!(
        crate_asset.gpu.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED,
        0,
        "`Material::with_textures` derived MATERIAL_FLAG_TEXTURED — Aether never mints that bit"
    );
    assert_eq!(crate_asset.gpu.base_color, [0.8, 0.8, 0.8, 1.0], "the textured path keeps the base");
    assert_eq!(crate_asset.gpu.mrr[1], 0.4, "the textured path keeps the scalar parameters");

    // Three distinct rows, three distinct handles — the builders are values, not a shared table.
    assert_ne!(h_gold.index(), h_lamp.index(), "each mint takes its own asset row");
    assert_ne!(h_lamp.index(), h_crate.index(), "each mint takes its own asset row");
}
