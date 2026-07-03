//! Common `boyko_app` imports, collapsed into one glob:
//! `use boyko_app::prelude::*;` — everything a windowed scene (the R3
//! `examples/room.rs` shape) needs: the App surface, the host types, the
//! spatial + camera components, the mesh spawn surface, and the math
//! vocabulary.

pub use boyko_ecs::{App, AppExit};

// System params a startup/scene system takes (`Commands` spawns; the NonSend
// pair reaches the device + mesh registry).
pub use boyko_ecs::ecs::core::system::{Commands, NonSendRes, NonSendResMut};

// Math vocabulary for authoring transforms.
pub use boyko_math::{Affine3A, Quat, Vec3};

// The mesh spawn surface: the drawable bundle + the registry it indexes.
pub use boyko_render::{MeshBundle, MeshRegistry};

// The lighting spawn surface (host plan R4): the light components + their
// placed-object bundles, the structural CSM caster capability, and the
// owner-set CSM config knob.
pub use boyko_render::{
    CsmConfig, DirectionalLight, DirectionalLightObject, PointLight, PointLightObject,
    ShadowCaster, SkyLight, SpotLight, SpotLightObject,
};

// The interpolation surface (host plan R5): the per-entity [`GpuTransform3D`] pair
// (its PRESENCE opts a body into interpolation — the pack shuffles prev/curr, the
// runner lerps at the frame overstep), the [`SnapInterpolation`] marker tag
// (present ⇒ snap `prev = curr` for one frame), and the
// [`TeleportCommandsExt::teleport_to`] command sugar (write `Transform` + attach the
// snap tag in one deferred command).
pub use boyko_render::{GpuTransform3D, SnapInterpolation, TeleportCommandsExt};

// The SDF instance surface (host plan R7): the per-entity [`SdfPrimitive`] component
// (an `SdfEdit` carrier — its PRESENCE direct-marches the primitive into the shared
// G-buffer), the [`SdfEdit`] std430 element + its `sphere`/`box_shape`/`capsule`
// constructors, and the [`sdf_op`] / [`sdf_kind`] discriminants for authoring edits.
pub use boyko_render::{SdfEdit, SdfPrimitive, sdf_kind, sdf_op};

// Spatial + camera components and the D4 Fixed ordering seam. The R6
// interactive-camera surface: the `FlyCamera` controller component, its
// `FlyCameraBundle` spawn preset, and the `CameraSet` ordering seam.
pub use boyko_scene::{
    Camera, CameraRig, CameraSet, FixedSet, FlyCamera, FlyCameraBundle, GlobalTransform,
    MaterialHandle, MeshHandle, Projection, Transform, Visibility,
};

// The R6 interactive-input surface: the fly-camera host plugin (input ingest +
// controller + ECS-native quit), its action enum, and the `KeyCode` vocabulary
// for authoring bindings / reading the input snapshot.
pub use boyko_input::KeyCode;

pub use crate::device::GpuDevice;
pub use crate::fly::{FlyAction, FlyCameraPlugin};
pub use crate::plugins::EnginePlugins;
pub use crate::window_info::{HostFrameStats, WindowInfo};
