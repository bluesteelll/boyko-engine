//! The [`AppExit`] resource — the cooperative exit signal read by
//! [`App::run`](crate::ecs::core::app::app::App::run).

use std::sync::OnceLock;

use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;

/// Cooperative exit flag for [`App::run`](crate::ecs::core::app::app::App::run).
///
/// Set `AppExit(true)` from a system (via `ResMut<AppExit>`) to make
/// [`App::run`](crate::ecs::core::app::app::App::run) exit after the current
/// frame completes. [`App::run`] inserts an `AppExit(false)` before the loop, so
/// the per-frame read never panics on a missing resource. The bounded runner
/// [`App::run_n`](crate::ecs::core::app::app::App::run_n) and the single-frame
/// [`App::update`](crate::ecs::core::app::app::App::update) do NOT read it (no
/// exit branch on those hot paths).
#[derive(Default)]
pub struct AppExit(pub bool);

// Hand-implemented rather than `#[derive(Resource)]`: `boyko-macros` is a
// dev-dependency of `boyko-ecs`, so its derives are unavailable in normal
// builds. This mirrors EXACTLY what `#[derive(Resource)]` expands to — a
// per-type `OnceLock<ResourceId>` minted once via the global registry
// (`register_new`). No atomic on the hot path after the first call.
impl Resource for AppExit {
    #[inline]
    fn resource_id() -> ResourceId {
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId(register_new::<Self>()))
    }
}
