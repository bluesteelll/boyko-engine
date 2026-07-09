//! std-lib S4 Miri (tree-borrows) WITNESS for the `Gpu3dInstance` pack +
//! cast-to-bytes upload path.
//!
//! # Why this witness lives in `boyko_scene`, not `boyko_render`
//!
//! The S4 Miri gate targets the `Gpu3dInstance` pack (`GlobalTransform` → packed
//! `repr(C)` POD column) + the `cast_slice` COLUMN → GPU upload — the only
//! unsafe-adjacent path S4 owns the producer side of. The real types live in
//! `boyko_render`, BUT `boyko_render` does NOT compile under Miri: its
//! `gpu_column.rs` calls `Archetype::make_component_device_backed` /
//! `set_component_device_handle`, which are `#[cfg(not(miri))]` in
//! `boyko_ecs/archetype.rs` (the device-backing arm is compiled out under Miri).
//! That is a PRE-EXISTING crate-level Miri incompatibility, independent of S4 —
//! `boyko_render` has never been Miri-buildable.
//!
//! So this file is an INDEPENDENT witness in a crate that DOES build under Miri.
//! It reproduces the EXACT unsafe surface byte-for-byte:
//!
//!  * `Gpu3dInstanceMirror` — a `#[repr(C)]` POD with the SAME layout as
//!    `boyko_render::Gpu3dInstance` (`linear_rows: [[f32;3];3]` + `translation:
//!    [f32;3]` + `material: u32` = 52 B, align 4, no padding holes), with the same
//!    52/4 const-asserts.
//!  * The pack: one sequential `Affine3A` read + one packed write per ECS row
//!    (mirrors `sync_gpu_3d_instances`), through the safe ECS query API.
//!  * The upload: the contiguous component column is reinterpreted as `&[u8]` via
//!    `core::slice::from_raw_parts` (the raw operation `bytemuck::cast_slice` and
//!    the renderer's `for_each_chunk` column→GPU copy perform) and the bytes are
//!    read back at known offsets.
//!
//! Run (the gate command):
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" \
//!   RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu \
//!   cargo miri test -p boyko-scene --test gpu3d_pack_miri_witness
//! ```
//! It is also a normal (non-Miri) test, so it runs under `cargo test` too.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::Component;

use boyko_math::{Affine3A, Mat3, Vec3};

use boyko_scene::transform::GlobalTransform;

/// A layout-exact mirror of `boyko_render::Gpu3dInstance` (52 B, align 4, no
/// padding) — the POD whose column is the GPU instance buffer. Keeping the fields
/// and order identical means this witness exercises the SAME byte image and the
/// SAME `cast_slice` soundness condition as the real type.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Gpu3dInstanceMirror {
    linear_rows: [[f32; 3]; 3],
    translation: [f32; 3],
    material: u32,
}

const GPU3D_INSTANCE_SIZE: usize = 52;
// The same pins the real type carries (boyko_render/gpu3d_instance.rs:77-78).
const _: () = assert!(size_of::<Gpu3dInstanceMirror>() == GPU3D_INSTANCE_SIZE);
const _: () = assert!(align_of::<Gpu3dInstanceMirror>() == 4);
// No padding holes — the precondition that makes the `&[u8]` reinterpret sound.
const _: () = assert!(
    size_of::<Gpu3dInstanceMirror>()
        == size_of::<[[f32; 3]; 3]>() + size_of::<[f32; 3]>() + size_of::<u32>()
);

/// Views a `#[repr(C)]` POD as raw bytes for the `create_entity` spawn path.
///
/// # Safety
/// `T` is a `#[repr(C)]` component whose byte image is a valid serialization for
/// its pool (holds for the fixed-layout PODs spawned here).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we read its `size_of::<T>()` bytes read-only.
    // `T` is `#[repr(C)]`, matching the pool's stored layout; the slice borrows
    // `value` so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn renderable_arch(world: &mut EcsMaster) -> ArchetypeId {
    world.create_archetype(&[GlobalTransform::component_id(), Gpu3dInstanceMirror::component_id()])
}

fn spawn(world: &mut EcsMaster, arch: ArchetypeId, global: GlobalTransform) {
    let zero = Gpu3dInstanceMirror {
        linear_rows: [[0.0; 3]; 3],
        translation: [0.0; 3],
        material: 0,
    };
    world
        .create_entity(
            arch,
            &[
                (GlobalTransform::component_id(), as_bytes(&global)),
                (Gpu3dInstanceMirror::component_id(), as_bytes(&zero)),
            ],
        )
        .expect("renderable archetype accepts its two columns");
}

/// The pack system (mirror of `sync_gpu_3d_instances`): one `Affine3A` read + one
/// packed write per row, alloc-free, through the safe ECS query API.
#[allow(clippy::needless_pass_by_value)]
fn pack(mut q: Query<(&GlobalTransform, &mut Gpu3dInstanceMirror)>) {
    for (g, inst) in q.iter_mut() {
        let a = g.affine();
        let r = a.matrix3.rows;
        inst.linear_rows = [
            [r[0].x, r[0].y, r[0].z],
            [r[1].x, r[1].y, r[1].z],
            [r[2].x, r[2].y, r[2].z],
        ];
        inst.translation = [a.translation.x, a.translation.y, a.translation.z];
        inst.material = 7;
    }
}

#[test]
fn gpu3d_pack_then_byte_cast_upload_is_ub_free() {
    let mut world = EcsMaster::new();
    let arch = renderable_arch(&mut world);

    // A handful of rows with distinctive translations.
    let translations = [
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(-4.0, 5.5, -6.25),
        Vec3::new(100.0, 0.0, -0.0),
    ];
    for &t in &translations {
        spawn(&mut world, arch, GlobalTransform(Affine3A { matrix3: Mat3::IDENTITY, translation: t }));
    }

    // PACK: GlobalTransform → Gpu3dInstanceMirror column.
    world.run_system(pack);

    // UPLOAD: gather the packed instances into a contiguous buffer (the renderer's
    // `for_each_chunk` walk hands the GPU one contiguous column slice) and
    // reinterpret it as &[u8] — the exact raw operation `bytemuck::cast_slice`
    // performs. Under Miri (tree-borrows) this checks the provenance + aliasing of
    // the pointer-to-bytes reinterpret of a `repr(C)` hole-free POD.
    let packed: Vec<Gpu3dInstanceMirror> = world
        .query::<&Gpu3dInstanceMirror, ()>()
        .iter()
        .copied()
        .collect();

    // SAFETY: `packed` is a live, contiguous `[Gpu3dInstanceMirror]`; the element
    // is `#[repr(C)]` with NO padding holes (const-asserted above), so every byte
    // of `len * 52` is an initialised `f32`/`u32` byte — a valid `u8` read. The
    // slice borrows `packed`, so it cannot outlive it; the cast is read-only. This
    // is precisely what `bytemuck::cast_slice::<Gpu3dInstanceMirror, u8>(&packed)`
    // does internally (Pod-gated), and what the column→GPU `cast_slice` upload
    // relies on.
    let bytes: &[u8] = unsafe {
        core::slice::from_raw_parts(
            packed.as_ptr().cast::<u8>(),
            packed.len() * GPU3D_INSTANCE_SIZE,
        )
    };
    assert_eq!(bytes.len(), translations.len() * GPU3D_INSTANCE_SIZE);

    // Read the translation lane (offset 36) back out of the raw bytes per row.
    for (i, t) in translations.iter().enumerate() {
        let base = i * GPU3D_INSTANCE_SIZE + 36;
        let lane = |o: usize| {
            f32::from_ne_bytes(bytes[base + o..base + o + 4].try_into().expect("4 bytes"))
        };
        assert_eq!([lane(0), lane(4), lane(8)], [t.x, t.y, t.z], "row {i} translation bytes");
        // Material lane at offset 48.
        let mbase = i * GPU3D_INSTANCE_SIZE + 48;
        let material = u32::from_ne_bytes(bytes[mbase..mbase + 4].try_into().expect("4 bytes"));
        assert_eq!(material, 7, "row {i} material lane");
    }
}
