//! Asset-streaming plan "HARD PREREQ before async streaming" items (a) (b) (d) (e)
//! (branch `fix/asset-validate-prereqs`) — the CPU-only, red-first gates for
//! [`validate_asset_refs`] as a two-way (mark + clear) staleness oracle DECOUPLED
//! from user visibility, the visible stale-material substitution, and the
//! serialize version-skew orphan check.
//!
//! Harness = `asset_streaming_f5_validation.rs`'s `world_with` (the full refcount
//! pipeline's resources) + `asset_streaming_f8_material_gather.rs`'s
//! `dummy_mesh_gpu` / `spawn_drawable`, driving the REAL [`gather_mesh_draws`]
//! through `run_system` so every assertion is on what the GPU would have been
//! handed (`MeshRenderScratch::instance_count` / `material_ids`), never on a
//! stand-in. See those two files' module docs for why a `VkBuffer::NULL`-handled
//! `MeshGpu` is sound here (nothing dereferences a device handle).
//!
//! # Gate map
//!
//! - G1 (a) [`fill_re_enables_a_carrier_bound_while_loading`]
//! - G2 (b) [`stale_wins_for_the_gather_and_visibility_bit_is_untouched`]
//! - G4 (d) [`stale_material_substitutes_default_slot_0`] +
//!   [`loading_material_ships_id_0_without_validate`]
//! - G5 (e) [`schema_older_save_lacking_ref_gen_lane_is_flagged_not_drawn`]
//! - G6 (e-confirm PIN) [`load_world_fires_no_carrier_hooks`]
//!
//! # The fix-pass twins — every arm the mesh lane gates, the MATERIAL lane gates too
//!
//! The review found three arms shipped by this rung with NO gate at all: each survived a
//! deletion mutation with every lane suite green. They are the material-lane mirrors of
//! G1 / G5 and the caster mirror of G2, so they are added here as twins rather than as a
//! new concept:
//!
//! - G7 (a, material) [`material_fill_re_enables_a_carrier_bound_while_loading`] — the
//!   `q_mat_stale` CLEAR arm (`asset_refcount.rs`). Without it a carrier bound to a
//!   `reserve()`d material is drawn with the substituted default FOREVER.
//! - G8 (e, material) [`schema_older_save_lacking_material_ref_gen_lane_is_flagged`] —
//!   the `q_mat_orphan` pass. Without it a schema-older `MaterialHandle` row is
//!   AND-filtered out of BOTH material arms and drawn with a raw, unvalidatable id.
//! - G9 (b, caster) [`stale_caster_is_dropped_from_the_shadow_gather`] — the
//!   `Disabled<RenderStale>` term on `gather_shadow_casters`. Without it a caster whose
//!   mesh slot was reused under it casts the WRONG mesh's shadow while the main gather
//!   correctly drops it.

use boyko_ecs::ecs::core::asset::{AssetLoadState, Assets, GEN_UNSYNCED};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Commands;

use boyko_render::asset_refcount::{ValidateCursor, validate_asset_refs};
use boyko_render::{
    CsmCasterScratch, Material, MaterialStale, MeshBundle, MeshGpu, MeshRenderScratch, RenderEpoch,
    RenderStale, ShadowCaster, apply_refcount_deltas, gather_mesh_draws, gather_shadow_casters,
};
use boyko_scene::visibility_sync::visibility_sync;
use boyko_scene::{
    DeferredFree, MaterialHandle, MaterialRefGen, MeshHandle, MeshRefGen, RefcountDeltas,
    RenderEnabled, Transform, Visibility,
};
use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

use boyko_math::Vec3;
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;

/// The pinned default material's colour (slot 0 is `Material::default()` in every
/// world below) — the value a substituted / guarded instance must carry.
const DEFAULT_BASE_COLOR: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

/// A device-inert `MeshGpu` — see `asset_streaming_f8_material_gather.rs`'s twin.
fn dummy_mesh_gpu() -> MeshGpu {
    let dummy_buf = || BoundBuffer { buffer: VkBuffer::NULL, offset: 0, size: 0, mapped: None, block: 0 };
    MeshGpu {
        vertex_buffer: dummy_buf(),
        index_buffer: dummy_buf(),
        index_count: 36,
        index_type: IndexType::Uint16,
        vertex_count: 8,
        #[cfg(feature = "hwrt")]
        blas: None,
        geometry_slot: 0,
        local_min: [-0.5; 3],
        local_max: [0.5; 3],
    }
}

/// A non-default material with a distinctive colour, so a substitution is
/// detectable BY VALUE on the `material_ids` lane, not only by id.
fn red_material() -> Material {
    Material::new([0.9, 0.1, 0.1, 1.0], 0.0, 0.5, 0.5, [0.0, 0.0, 0.0], 0)
}

/// The full F2+F5 refcount/validation pipeline's resources (mirrors
/// `AssetRefcountPlugin::build`) PLUS the gather's `MeshRenderScratch`, so ONE
/// world runs `apply_refcount_deltas` → `validate_asset_refs` → `gather_mesh_draws`
/// exactly as the host frame does.
fn world_with(material_assets: Assets<Material>, mesh_assets: Assets<MeshGpu>) -> EcsMaster {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(RefcountDeltas::default());
    ecs.insert_resource(DeferredFree::default());
    ecs.insert_resource(ValidateCursor::default());
    ecs.insert_resource(RenderEpoch::default());
    ecs.insert_resource(MeshRenderScratch::default());
    // G9's caster gather writes its own scratch — the SAME gather core over the
    // `With<ShadowCaster>`-filtered query. Inert for every other test here.
    ecs.insert_resource(CsmCasterScratch::default());
    // The hwrt `gather_mesh_draws` variant additionally reads `Res<ShadowDenoiseConfig>`
    // (mirrors `asset_streaming_f8_material_gather.rs`'s idiom).
    #[cfg(feature = "hwrt")]
    ecs.insert_resource(boyko_render::ShadowDenoiseConfig::default());
    ecs.insert_resource(material_assets);
    ecs.insert_non_send_resource(mesh_assets);
    // Prime both ids (installs the on_insert/on_replace hooks) before any spawn.
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();
    ecs
}

/// A material store whose slot 0 is the pinned default (`Material::default()`),
/// matching the boot invariant every gather relies on.
fn materials_with_default() -> Assets<Material> {
    let mut m = Assets::<Material>::with_reserved(4);
    let default_slot = m.add(Material::default());
    assert_eq!(default_slot.index(), 0, "test precondition: the first mint is slot 0");
    m
}

/// Spawns a drawable (`MeshBundle` + `MaterialHandle` + the `RenderEnabled` bit) at a
/// unique translation-x so its ring slot can be found regardless of iteration order.
///
/// # Why the material rides the BUNDLE and is not `insert`ed over it
///
/// `MeshBundle` already carries `MaterialHandle(0)` (bundles.rs:72). Spawning the
/// bundle and THEN `insert`ing a `MaterialHandle` is a REPLACE, which fires
/// `on_replace` (a `-1` on slot 0) before `on_insert`'s `+1`. Applied in order that
/// is a zero-crossing on the PINNED DEFAULT: slot 0 goes `Retiring`, and
/// `Assets::inc_ref` then REFUSES the `+1` (it refuses an already-`Retiring` slot —
/// the resurrection guard), so slot 0 stays `Retiring` for the rest of the world's
/// life. `state_of_index(0)` is then `None`, and `validate_asset_refs`'s material arm
/// marks EVERY carrier `MaterialStale` — which silently made the G2 assertion below
/// pass for the wrong reason (the row was excluded as material-stale, not as
/// mesh-stale) and left the `Disabled<RenderStale>` gather term unproven. MEASURED:
/// with the `insert` form, deleting `Disabled<RenderStale>` from the gather filter did
/// NOT turn G2 red. Building the bundle with its material already set fires exactly
/// one `+1` per handle and keeps slot 0 `Loaded`.
fn spawn_drawable(ecs: &mut EcsMaster, mesh_id: u32, x: f32, material_raw: u16) -> Entity {
    ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(MeshBundle {
            material: MaterialHandle(material_raw),
            ..MeshBundle::new(MeshHandle(mesh_id), Transform::from_translation(Vec3::new(x, 0.0, 0.0)))
        })
        .enable::<RenderEnabled>()
        .id()
    })
}

/// The ring slot the drawable at translation-x `x` was scattered into.
fn slot_of(scratch: &MeshRenderScratch, x: f32) -> usize {
    scratch
        .ring
        .as_read_slice()
        .iter()
        .position(|c| c.rows[0][3] == x)
        .unwrap_or_else(|| panic!("a drawable at x={x} must have a ring slot"))
}

fn instance_count(ecs: &EcsMaster) -> usize {
    ecs.resource::<MeshRenderScratch>().instance_count()
}

// ════════════════════════════════════════════════════════════════════════════
// G1 (a) — fill (Loading → Loaded) must re-enable a carrier bound while Loading.
// ════════════════════════════════════════════════════════════════════════════

/// A carrier bound to a `reserve()`d (Loading) mesh slot is stale until the slot
/// is `fill`ed. `fill` bumps `install_epoch`, validate must re-run on that bump and
/// CLEAR the row's staleness, and the gather must then draw it — while the user's
/// `RenderEnabled` bit is never touched by validate at all.
#[test]
fn fill_re_enables_a_carrier_bound_while_loading() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let _slot0 = mesh_assets.add(dummy_mesh_gpu());
    // Advance `free_epoch` once BEFORE the carrier binds: the pre-fix validate ran
    // its pass only on a `free_epoch` change, so without this the first validate
    // below would early-out on the unfixed code and the test would pass vacuously
    // (never having observed the Loading row as stale in the first place).
    let throwaway = mesh_assets.add(dummy_mesh_gpu());
    let _ = mesh_assets.remove(throwaway);
    let loading = mesh_assets.reserve();
    let l = loading.index();
    assert_eq!(
        mesh_assets.state_of_index(l),
        Some(AssetLoadState::Loading),
        "test precondition: the reserved slot is Loading"
    );

    let mut ecs = world_with(materials_with_default(), mesh_assets);
    let e = spawn_drawable(&mut ecs, l, 0.0, 0);
    ecs.run_system(apply_refcount_deltas);
    let stamped = ecs.get_component::<MeshRefGen>(e).copied().expect("lane present").0;
    assert_ne!(stamped, GEN_UNSYNCED, "test precondition: the apply stamped the lane");

    // First validate: the carrier's slot is Loading (≠ Loaded) ⇒ stale.
    ecs.run_system(validate_asset_refs);
    assert!(ecs.is_enabled::<RenderStale>(e), "test precondition: a Loading-bound carrier is stale");
    assert!(ecs.is_enabled::<RenderEnabled>(e), "(b) marking stale never clears RenderEnabled");
    ecs.run_system(gather_mesh_draws);
    assert_eq!(instance_count(&ecs), 0, "test precondition: a Loading-bound carrier is not drawn");

    // The async load completes.
    ecs.non_send_resource_mut::<Assets<MeshGpu>>()
        .fill(loading, dummy_mesh_gpu())
        .unwrap_or_else(|(err, _)| panic!("fill must succeed on a Loading row: {err:?}"));

    // THE GATE: validate must observe the fill and lift the staleness; the gather
    // then draws the row; the user's visibility bit was never cleared.
    ecs.run_system(validate_asset_refs);
    assert!(
        !ecs.is_enabled::<RenderStale>(e),
        "(a) validate must observe fill's install_epoch bump and CLEAR RenderStale"
    );
    ecs.run_system(gather_mesh_draws);
    assert_eq!(
        instance_count(&ecs),
        1,
        "(a) a carrier bound while its mesh was Loading must be drawn once the mesh is filled \
         (validate must re-run on the install epoch and CLEAR the staleness)"
    );
    assert!(
        ecs.is_enabled::<RenderEnabled>(e),
        "(a)/(b) validate must never touch the user's RenderEnabled bit"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// G2 (b) — staleness is decoupled from user visibility.
// ════════════════════════════════════════════════════════════════════════════

/// A stale carrier stays undrawn even when `visibility_sync` (the sole owner of
/// `RenderEnabled`) sets the bit: stale wins for the gather, and the visibility bit
/// itself is left exactly as the user's `Visibility` dictates.
#[test]
fn stale_wins_for_the_gather_and_visibility_bit_is_untouched() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let h = mesh_assets.add(dummy_mesh_gpu());
    let slot = h.index();

    let mut ecs = world_with(materials_with_default(), mesh_assets);
    let e = spawn_drawable(&mut ecs, slot, 0.0, 0);
    ecs.run_system(apply_refcount_deltas);
    let stale_gen = ecs.get_component::<MeshRefGen>(e).copied().expect("lane present").0;
    assert_ne!(stale_gen, GEN_UNSYNCED, "test precondition: the apply stamped the lane");

    // Force-reuse (the f5 idiom): the slot is freed under the carrier and re-minted.
    ecs.non_send_resource_mut::<Assets<MeshGpu>>().remove(h);
    let new_h = ecs.non_send_resource_mut::<Assets<MeshGpu>>().add(dummy_mesh_gpu());
    assert_eq!(new_h.index(), slot, "LIFO reuse hands the freed row back at the same index");
    assert_ne!(new_h.generation(), stale_gen, "the reused row's generation differs");

    ecs.run_system(validate_asset_refs);
    assert!(ecs.is_enabled::<RenderStale>(e), "validate marks the gen-mismatched row stale");
    assert!(
        ecs.is_enabled::<RenderEnabled>(e),
        "(b) validate must not clear RenderEnabled — staleness is not visibility"
    );

    // The user toggles visibility; `visibility_sync` drives the bit.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(Visibility::Visible);
    });
    ecs.run_system(visibility_sync);
    assert!(
        ecs.is_enabled::<RenderEnabled>(e),
        "test precondition: visibility_sync set the bit for a Visible row"
    );

    ecs.run_system(gather_mesh_draws);
    assert_eq!(
        instance_count(&ecs),
        0,
        "(b) a stale row must not be drawn even while its RenderEnabled bit is set — the \
         gather must filter on the staleness tag, not rely on validate clearing visibility"
    );
    assert!(
        ecs.is_enabled::<RenderEnabled>(e),
        "(b) the visibility bit stays exactly as visibility_sync left it"
    );
    assert!(
        ecs.is_enabled::<RenderStale>(e),
        "(b) visibility_sync never touches the stale bit either — two bits, two owners"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// G4 (d) — visible MaterialStale substitution + the Loading-slot guard.
// ════════════════════════════════════════════════════════════════════════════

/// A carrier whose material slot was retired and reused under it (gen mismatch) is
/// drawn with the pinned default material (slot 0, the default colour), and the
/// per-frame "any non-default material" gate is NOT flipped by the substituted row.
#[test]
fn stale_material_substitutes_default_slot_0() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh = mesh_assets.add(dummy_mesh_gpu()).index();

    let mut material_assets = materials_with_default();
    let m_handle = material_assets.add(red_material());
    let m = m_handle.index();
    assert_eq!(m, 1, "test precondition: the red material is slot 1");

    let mut ecs = world_with(material_assets, mesh_assets);
    let e = spawn_drawable(&mut ecs, mesh, 0.0, m as u16);
    ecs.run_system(apply_refcount_deltas);
    let stale_gen = ecs.get_component::<MaterialRefGen>(e).copied().expect("lane present").0;
    assert_ne!(stale_gen, GEN_UNSYNCED, "test precondition: the apply stamped the material lane");

    // Force-reuse the material slot under the carrier.
    ecs.resource_mut::<Assets<Material>>().remove(m_handle);
    let new_h = ecs.resource_mut::<Assets<Material>>().add(red_material());
    assert_eq!(new_h.index(), m, "LIFO reuse hands the freed row back at the same index");
    assert_ne!(new_h.generation(), stale_gen, "the reused row's generation differs");

    ecs.run_system(validate_asset_refs);
    assert!(ecs.is_enabled::<MaterialStale>(e), "(d) validate's material arm marks the row");
    assert!(!ecs.is_enabled::<RenderStale>(e), "the MESH lane is untouched by material staleness");
    ecs.run_system(gather_mesh_draws);

    let scratch = ecs.resource::<MeshRenderScratch>();
    assert_eq!(scratch.instance_count(), 1, "a stale MATERIAL does not drop the mesh instance");
    let slot = slot_of(scratch, 0.0);
    let ids = scratch.material_ids.as_read_slice();
    assert_eq!(
        ids[slot].id, 0,
        "(d) a gen-mismatched material carrier must be drawn with the pinned default slot 0"
    );
    assert_eq!(
        ids[slot].base_color, DEFAULT_BASE_COLOR,
        "(d) the substituted instance carries the DEFAULT material's colour, not the reused slot's"
    );
    // The TEXTURED payload lane is a SECOND, index-aligned pass with its OWN closure —
    // the two lanes are read by two different shaders (`vb_shade.comp` reads
    // `PerInstanceMaterialTex::material_id`, the base VB path reads
    // `PerInstanceMaterial::id`), so a guard applied to one only is a per-instance
    // disagreement. Pinned on BOTH lanes here and in the Loading twin below.
    let tex = scratch.material_tex.as_read_slice();
    assert_eq!(
        tex[slot].material_id, 0,
        "(d) the TEXTURED payload lane must carry the SAME substituted id as the primary lane"
    );
    assert_eq!(
        tex[slot].base_color, DEFAULT_BASE_COLOR,
        "(d) and the same substituted colour"
    );
    assert!(
        !scratch.any_non_default_material(),
        "(d) a substituted row must not flip the per-frame non-default-material gate"
    );
    assert!(ecs.is_enabled::<RenderEnabled>(e), "validate never touches RenderEnabled");
}

/// A carrier bound to a `reserve()`d (Loading) material slot on the spawn frame —
/// no epoch bump, so validate does not run — must still never ship the raw slot id
/// (a hole row of the material SSBO) to the shader: the gather's own construction
/// guard maps a non-Loaded slot to id 0.
#[test]
fn loading_material_ships_id_0_without_validate() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh = mesh_assets.add(dummy_mesh_gpu()).index();

    let mut material_assets = materials_with_default();
    let loading = material_assets.reserve();
    let r = loading.index();
    assert_eq!(
        material_assets.state_of_index(r),
        Some(AssetLoadState::Loading),
        "test precondition: the reserved material slot is Loading"
    );

    let mut ecs = world_with(material_assets, mesh_assets);
    let e = spawn_drawable(&mut ecs, mesh, 0.0, r as u16);
    // Deliberately NO apply / NO validate: the guard is the gather's own.
    ecs.run_system(gather_mesh_draws);
    assert!(!ecs.is_enabled::<MaterialStale>(e), "test precondition: validate never ran");

    let scratch = ecs.resource::<MeshRenderScratch>();
    assert_eq!(scratch.instance_count(), 1);
    let slot = slot_of(scratch, 0.0);
    let ids = scratch.material_ids.as_read_slice();
    assert_eq!(
        ids[slot].id, 0,
        "(d) a Loading material slot must never reach the shader as a raw id — the gather \
         maps a non-Loaded slot to the pinned default 0 by construction"
    );
    assert_eq!(ids[slot].base_color, DEFAULT_BASE_COLOR);
    assert!(!scratch.any_non_default_material());
    // THE SECOND LANE. `PerInstanceMaterialTex::material_id` indexes the SAME
    // `Materials` SSBO from the TEXTURED shaders (`vb_shade.comp.hlsl` /
    // `gbuffer_mrt.vs.hlsl` → `gbuffer_mrt.fs.hlsl`'s `pack_material_id_ba`), so the
    // construction guard is only real if BOTH lanes carry it — otherwise the two
    // index-aligned lanes disagree per instance and the hole id reaches the shader on
    // every textured path.
    let tex = scratch.material_tex.as_read_slice();
    assert_eq!(
        tex[slot].material_id, 0,
        "(d) the TEXTURED payload lane must apply the SAME Loading-slot guard as the \
         primary lane — one truth, two lanes"
    );
    assert_eq!(tex[slot].base_color, DEFAULT_BASE_COLOR);
    assert!(!scratch.any_textured_material());
}

// ════════════════════════════════════════════════════════════════════════════
// G7 (a, material twin) — the MATERIAL clear arm un-strands a filled carrier.
// ════════════════════════════════════════════════════════════════════════════

/// The material-lane mirror of G1. A carrier bound to a `reserve()`d material is marked
/// `MaterialStale` and drawn with the substituted default; when the async `fill` lands,
/// validate's `q_mat_stale` CLEAR arm must lift the bit so the row is drawn with its REAL
/// material. Without that arm the substitution is permanent — the (a) bug, material side.
///
/// MEASURED red-first by mutation: replacing the `q_mat_stale` loop in
/// `validate_asset_refs` with `let _ = &q_mat_stale;` turns exactly this test red
/// (`(a/material) validate must observe fill's install_epoch bump and CLEAR MaterialStale`);
/// before this test existed that mutation left every lane suite green.
#[test]
fn material_fill_re_enables_a_carrier_bound_while_loading() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh = mesh_assets.add(dummy_mesh_gpu()).index();

    let mut material_assets = materials_with_default();
    let loading = material_assets.reserve();
    let r = loading.index();
    assert_eq!(
        material_assets.state_of_index(r),
        Some(AssetLoadState::Loading),
        "test precondition: the reserved material slot is Loading"
    );

    let mut ecs = world_with(material_assets, mesh_assets);
    let e = spawn_drawable(&mut ecs, mesh, 0.0, r as u16);
    ecs.run_system(apply_refcount_deltas);
    let stamped = ecs.get_component::<MaterialRefGen>(e).copied().expect("lane present").0;
    assert_ne!(stamped, GEN_UNSYNCED, "test precondition: the apply stamped the material lane");

    // First validate: the slot is Loading (≠ Loaded) ⇒ material-stale, mesh untouched.
    ecs.run_system(validate_asset_refs);
    assert!(
        ecs.is_enabled::<MaterialStale>(e),
        "test precondition: a Loading-bound material carrier is material-stale"
    );
    assert!(!ecs.is_enabled::<RenderStale>(e), "the MESH lane is untouched");
    ecs.run_system(gather_mesh_draws);
    {
        let scratch = ecs.resource::<MeshRenderScratch>();
        assert_eq!(scratch.instance_count(), 1, "a stale material never drops the instance");
        let slot = slot_of(scratch, 0.0);
        assert_eq!(
            scratch.material_ids.as_read_slice()[slot].id,
            0,
            "test precondition: the substituted default ships while the material is Loading"
        );
    }

    // The async material load completes.
    ecs.resource_mut::<Assets<Material>>()
        .fill(loading, red_material())
        .unwrap_or_else(|(err, _)| panic!("fill must succeed on a Loading row: {err:?}"));

    // THE GATE: the CLEAR arm must lift MaterialStale, and the gather must then ship the
    // REAL slot — not the default, forever.
    ecs.run_system(validate_asset_refs);
    assert!(
        !ecs.is_enabled::<MaterialStale>(e),
        "(a/material) validate must observe fill's install_epoch bump and CLEAR MaterialStale"
    );
    ecs.run_system(gather_mesh_draws);
    let scratch = ecs.resource::<MeshRenderScratch>();
    let slot = slot_of(scratch, 0.0);
    assert_eq!(
        scratch.material_ids.as_read_slice()[slot].id,
        r,
        "(a/material) once filled, the carrier must be drawn with its REAL material slot"
    );
    assert_eq!(
        scratch.material_ids.as_read_slice()[slot].base_color,
        red_material().gpu.base_color,
        "(a/material) and with that slot's REAL colour, not the substituted default"
    );
    assert_eq!(
        scratch.material_tex.as_read_slice()[slot].material_id,
        r,
        "(a/material) the textured payload lane agrees — one truth, two lanes"
    );
    assert!(ecs.is_enabled::<RenderEnabled>(e), "validate never touches RenderEnabled");
}

// ════════════════════════════════════════════════════════════════════════════
// G9 (b, caster twin) — the caster gather honours the stale bit too.
// ════════════════════════════════════════════════════════════════════════════

/// The caster mirror of G2. `gather_shadow_casters` reads a raw `MeshHandle.0` exactly as
/// the main gather does, so it carries the same `Disabled<RenderStale>` term; without it a
/// caster whose mesh slot was retired and reused under it passes `try_get` with the NEW
/// tenant and casts the WRONG mesh's shadow while the main gather correctly drops it.
///
/// MEASURED red-first by mutation: dropping `Disabled<RenderStale>` from
/// `gather_shadow_casters`' filter turns exactly this test red (the caster instance count
/// stays 1); before this test existed that mutation left every lane suite green,
/// 540-test lib target included.
#[test]
fn stale_caster_is_dropped_from_the_shadow_gather() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let h = mesh_assets.add(dummy_mesh_gpu());
    let slot = h.index();

    let mut ecs = world_with(materials_with_default(), mesh_assets);
    let e = spawn_drawable(&mut ecs, slot, 0.0, 0);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(ShadowCaster);
    });
    ecs.run_system(apply_refcount_deltas);
    let stale_gen = ecs.get_component::<MeshRefGen>(e).copied().expect("lane present").0;
    assert_ne!(stale_gen, GEN_UNSYNCED, "test precondition: the apply stamped the lane");

    // The healthy frame: the caster IS gathered, so the assertion below cannot pass
    // vacuously (an empty caster scratch would satisfy a bare `== 0`).
    ecs.run_system(gather_shadow_casters);
    assert_eq!(
        ecs.resource::<CsmCasterScratch>().instance_count(),
        1,
        "test precondition: a healthy ShadowCaster is gathered — the gate is not vacuous"
    );

    // Force-reuse (the f5 idiom): the slot is freed under the caster and re-minted, so
    // `try_get` resolves to the NEW tenant and only the generation lane can tell.
    ecs.non_send_resource_mut::<Assets<MeshGpu>>().remove(h);
    let new_h = ecs.non_send_resource_mut::<Assets<MeshGpu>>().add(dummy_mesh_gpu());
    assert_eq!(new_h.index(), slot, "LIFO reuse hands the freed row back at the same index");
    assert_ne!(new_h.generation(), stale_gen, "the reused row's generation differs");

    ecs.run_system(validate_asset_refs);
    assert!(ecs.is_enabled::<RenderStale>(e), "validate marks the gen-mismatched caster stale");
    assert!(ecs.is_enabled::<RenderEnabled>(e), "(b) validate never clears RenderEnabled");

    ecs.run_system(gather_mesh_draws);
    assert_eq!(instance_count(&ecs), 0, "the MAIN gather drops the stale row");

    // THE GATE: the caster gather must drop it too, or the shadow of the NEW tenant is
    // cast at the OLD carrier's transform while nothing visible is drawn there.
    ecs.run_system(gather_shadow_casters);
    assert_eq!(
        ecs.resource::<CsmCasterScratch>().instance_count(),
        0,
        "(b/caster) a stale caster must not reach the cascade depth pass — the caster \
         gather reads a raw MeshHandle.0 and needs the SAME Disabled<RenderStale> term"
    );
    assert_eq!(
        ecs.resource::<CsmCasterScratch>().batch_count(),
        0,
        "(b/caster) and emits no caster batch for the reused slot"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// G5 (e) — a schema-older save's lane-less carrier is flagged, counted, not drawn.
// ════════════════════════════════════════════════════════════════════════════

/// Saves one drawable row, optionally with its `MeshRefGen` and/or `MaterialRefGen` lane
/// removed (the schema-older shape `load_archetype` cannot repair — no `#[require]`
/// expansion runs on load), and returns the bytes.
///
/// The two flags are independent because the two orphan passes are: G5 strips the mesh
/// lane, G8 strips the material lane, and each must turn its OWN pass red.
fn save_one_drawable(
    mesh_id: u32,
    material_raw: u16,
    strip_mesh_ref_gen: bool,
    strip_material_ref_gen: bool,
) -> Vec<u8> {
    let mut src = EcsMaster::new();
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();
    let e: Entity = src.run_system(move |mut cmds: Commands| {
        // The material rides the BUNDLE: inserting a `MaterialHandle` over the one
        // `MeshBundle` already carries would retire the pinned default slot (see
        // `spawn_drawable`'s doc).
        cmds.spawn(MeshBundle {
            material: MaterialHandle(material_raw),
            ..MeshBundle::new(
                MeshHandle(mesh_id),
                Transform::from_translation(Vec3::new(7.0, 0.0, 0.0)),
            )
        })
        .id()
    });
    if strip_mesh_ref_gen {
        src.run_system(move |mut cmds: Commands| {
            cmds.entity(e).remove::<MeshRefGen>();
        });
        assert!(
            src.get_component::<MeshRefGen>(e).is_none(),
            "test precondition: the saved row lacks its MeshRefGen lane"
        );
    }
    if strip_material_ref_gen {
        src.run_system(move |mut cmds: Commands| {
            cmds.entity(e).remove::<MaterialRefGen>();
        });
        assert!(
            src.get_component::<MaterialRefGen>(e).is_none(),
            "test precondition: the saved row lacks its MaterialRefGen lane"
        );
    }
    let mut out = Vec::new();
    save_world(&src, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// The loaded row's live entity (fresh ids under `Remap`, so it is located by query).
fn only_mesh_carrier(ecs: &mut EcsMaster) -> Entity {
    let ids: Vec<_> = ecs.run_system(|q: Query<&MeshHandle>| q.iter_entities().map(|(id, _)| id).collect());
    assert_eq!(ids.len(), 1, "exactly one carrier row was loaded");
    ecs.get_entity(ids[0]).expect("the loaded row is live")
}

#[test]
fn schema_older_save_lacking_ref_gen_lane_is_flagged_not_drawn() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh = mesh_assets.add(dummy_mesh_gpu()).index();
    let bytes = save_one_drawable(mesh, 0, true, false);

    let mut ecs = world_with(materials_with_default(), mesh_assets);
    // Sync the cursor BEFORE the load: the load bumps no store epoch, so the
    // validate below runs with NO epoch change — the orphan check must not hide
    // under the early-out.
    ecs.run_system(validate_asset_refs);

    let report = load_world(&mut ecs, &bytes, LoadEntityPolicy::Remap).expect("load");
    assert_eq!(report.entities_loaded, 1);
    let e = only_mesh_carrier(&mut ecs);
    assert!(
        ecs.get_component::<MeshRefGen>(e).is_none(),
        "test precondition: the loaded row still lacks MeshRefGen (no require expansion on load)"
    );
    ecs.enable::<RenderEnabled>(e);

    ecs.run_system(validate_asset_refs);
    ecs.run_system(gather_mesh_draws);
    assert_eq!(
        instance_count(&ecs),
        0,
        "(e) a lane-less (un-refcounted, unvalidatable) carrier must never be drawn — it \
         must be flagged stale by the orphan check even with no epoch change"
    );
    assert!(ecs.is_enabled::<RenderEnabled>(e), "validate never touches RenderEnabled");
    assert!(ecs.is_enabled::<RenderStale>(e), "(e) the lane-less row carries RenderStale");
    assert_eq!(
        ecs.resource::<ValidateCursor>().orphan_rows_flagged,
        1,
        "(e) the orphan pass counted exactly the one lane-less carrier"
    );

    // The `Disabled<RenderStale>` term makes each orphan cost one command, once.
    ecs.run_system(validate_asset_refs);
    assert_eq!(
        ecs.resource::<ValidateCursor>().orphan_rows_flagged,
        1,
        "(e) an already-flagged orphan is not re-counted on the next frame"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// G8 (e, material twin) — the MATERIAL orphan pass flags the lane-less carrier.
// ════════════════════════════════════════════════════════════════════════════

/// The material-lane mirror of G5. A schema-older save whose `MaterialHandle` row lacks
/// `MaterialRefGen` is AND-filtered out of BOTH material arms of `validate_asset_refs`, so
/// without the `q_mat_orphan` pass it is drawn with its RAW, un-refcounted, unvalidatable
/// material id — the (e) silent-skip hole, material side.
///
/// The saved row binds material slot 1 (not the default 0) precisely so the two outcomes
/// are DISTINGUISHABLE: unflagged ⇒ the red slot's id and colour reach the lanes; flagged
/// ⇒ the `MaterialStale` tail substitutes the pinned default. The MESH lane is left intact
/// (a `GEN_UNSYNCED` lane is trusted, not flagged), so the instance is still drawn — this
/// gate is about WHAT it is drawn with, not whether it is drawn.
///
/// MEASURED red-first by mutation: replacing the `q_mat_orphan` loop with
/// `let _ = &q_mat_orphan;` turns exactly this test red; before this test existed that
/// mutation left every lane suite green.
#[test]
fn schema_older_save_lacking_material_ref_gen_lane_is_flagged() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh = mesh_assets.add(dummy_mesh_gpu()).index();
    let bytes = save_one_drawable(mesh, 1, false, true);

    let mut material_assets = materials_with_default();
    let red = material_assets.add(red_material());
    assert_eq!(red.index(), 1, "test precondition: the red material is the slot the save binds");

    let mut ecs = world_with(material_assets, mesh_assets);
    // Sync the cursor BEFORE the load (see G5): the load bumps no store epoch, so the
    // orphan check must not hide under the four-epoch early-out.
    ecs.run_system(validate_asset_refs);

    let report = load_world(&mut ecs, &bytes, LoadEntityPolicy::Remap).expect("load");
    assert_eq!(report.entities_loaded, 1);
    let e = only_mesh_carrier(&mut ecs);
    assert!(
        ecs.get_component::<MaterialRefGen>(e).is_none(),
        "test precondition: the loaded row still lacks MaterialRefGen (no require expansion)"
    );
    assert!(
        ecs.get_component::<MeshRefGen>(e).is_some(),
        "test precondition: the MESH lane survived — this is the material hole alone"
    );
    ecs.enable::<RenderEnabled>(e);

    ecs.run_system(validate_asset_refs);
    assert!(
        ecs.is_enabled::<MaterialStale>(e),
        "(e/material) a lane-less MaterialHandle carrier must be flagged by the orphan pass \
         even with no epoch change — both material arms AND-filter it out"
    );
    assert!(!ecs.is_enabled::<RenderStale>(e), "the MESH lane is trusted, not flagged");
    assert_eq!(
        ecs.resource::<ValidateCursor>().orphan_rows_flagged,
        1,
        "(e/material) the orphan pass counted exactly the one lane-less carrier"
    );

    ecs.run_system(gather_mesh_draws);
    let scratch = ecs.resource::<MeshRenderScratch>();
    assert_eq!(scratch.instance_count(), 1, "the mesh lane is healthy, so the row IS drawn");
    let slot = slot_of(scratch, 7.0);
    assert_eq!(
        scratch.material_ids.as_read_slice()[slot].id,
        0,
        "(e/material) the flagged row is drawn with the pinned default, NOT its raw id"
    );
    assert_eq!(
        scratch.material_ids.as_read_slice()[slot].base_color,
        DEFAULT_BASE_COLOR,
        "(e/material) and with the default colour, not the un-refcounted slot's red"
    );
    assert_eq!(
        scratch.material_tex.as_read_slice()[slot].material_id,
        0,
        "(e/material) the textured payload lane agrees — one truth, two lanes"
    );

    // The `Disabled<MaterialStale>` term makes each orphan cost one command, once.
    ecs.run_system(validate_asset_refs);
    assert_eq!(
        ecs.resource::<ValidateCursor>().orphan_rows_flagged,
        1,
        "(e/material) an already-flagged orphan is not re-counted on the next frame"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// G6 (e-confirm PIN, green today) — load_world replays no carrier hook.
// ════════════════════════════════════════════════════════════════════════════

/// Pins the kernel's current behaviour: `load_archetype` fires no `on_insert` for a
/// loaded `MeshHandle`/`MaterialHandle`, so a loaded carrier pushes no `+1` and the
/// lane keeps its saved value. The day this goes red (hooks replayed on load), the
/// orphan check (D7) and the B1 "loaded carriers never `+1`" blocker must be
/// revisited together.
#[test]
fn load_world_fires_no_carrier_hooks() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh = mesh_assets.add(dummy_mesh_gpu()).index();
    let bytes = save_one_drawable(mesh, 0, false, false);

    let mut ecs = world_with(materials_with_default(), mesh_assets);
    load_world(&mut ecs, &bytes, LoadEntityPolicy::Remap).expect("load");
    assert!(
        ecs.resource::<RefcountDeltas>().is_empty(),
        "PIN: load_world must not fire the carrier on_insert hooks (no +1 pushed)"
    );
    let e = only_mesh_carrier(&mut ecs);
    let lane_before = ecs.get_component::<MeshRefGen>(e).copied().expect("the full row keeps its lane");
    assert_eq!(lane_before, MeshRefGen(GEN_UNSYNCED), "the saved lane value round-trips verbatim");

    ecs.run_system(apply_refcount_deltas);
    assert!(ecs.resource::<DeferredFree>().is_empty(), "nothing to apply ⇒ nothing retired");
    assert_eq!(
        ecs.get_component::<MeshRefGen>(e).copied(),
        Some(lane_before),
        "no delta ⇒ no lane re-stamp"
    );

    // A full-lane loaded row is NOT an orphan: the (e) check is about the LANE's
    // presence, not about the (B1, kernel-level) missing `+1`.
    ecs.run_system(validate_asset_refs);
    assert_eq!(ecs.resource::<ValidateCursor>().orphan_rows_flagged, 0);
    assert!(!ecs.is_enabled::<RenderStale>(e), "GEN_UNSYNCED is trusted, not flagged");
}
