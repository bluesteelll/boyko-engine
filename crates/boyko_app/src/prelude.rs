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

// Spatial + camera components and the D4 Fixed ordering seam.
pub use boyko_scene::{
    Camera, CameraRig, FixedSet, GlobalTransform, MaterialHandle, MeshHandle, Projection,
    Transform, Visibility,
};

pub use crate::device::GpuDevice;
pub use crate::plugins::EnginePlugins;
pub use crate::window_info::WindowInfo;
