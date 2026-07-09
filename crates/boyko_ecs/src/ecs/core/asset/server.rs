//! [`AssetServer`] — the path→[`Handle`] intern (rung A0: dedupe only; the
//! decode/upload dispatch is TODO(A3)).

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::ecs::core::asset::asset::Asset;
use crate::ecs::core::asset::handle::Handle;
use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;

/// World-global path→[`Handle`] intern, registered as a [`Resource`].
///
/// # A0 scope — dedupe only, no decode dispatch
///
/// [`load`](Self::load) interns `(TypeId::of::<A>(), path)` and mints a
/// fresh, monotonically increasing synthetic index the FIRST time a path is
/// requested for asset type `A`; every repeat request of the same path (for
/// the same `A`) returns the SAME [`Handle<A>`]. It does NOT read the file,
/// decode bytes, or insert a row into any
/// [`Assets<A>`](crate::ecs::core::asset::assets::Assets) table — there is
/// no [`AssetLoader`](crate::ecs::core::asset::loader::AssetLoader) registry
/// yet to dispatch to (that lands at rung A3).
///
/// The returned handle is therefore a **reserved identity only**:
/// `Assets::<A>::get` on it returns `None` until a later rung backs it with
/// a real, index-matching row. TODO(A3): wire `load` to the loader registry
/// and `Assets::<A>::add`, preserving this reserved index/generation
/// pairing so handles already handed out by A0 resolve once decoding lands.
///
/// # Cold path
///
/// `interned` is a `HashMap` — acceptable ONLY because path interning is a
/// setup-time operation (asset declarations at scene load), never touched
/// on the per-frame hot path. Mirrors the identical cold-path `HashMap`
/// exception `resource_type_registry` documents for its `TypeId →
/// ResourceId` map.
#[derive(Default)]
pub struct AssetServer {
    /// `(asset TypeId, path)` → the synthetic index minted for that pair.
    /// Cold path only — see the struct doc.
    interned: HashMap<(TypeId, String), u32>,
    /// Next synthetic index to mint for an unseen `(TypeId, path)` pair.
    next_index: u32,
}

impl AssetServer {
    /// Creates an empty server with no interned paths.
    #[inline]
    pub fn new() -> Self {
        Self {
            interned: HashMap::new(),
            next_index: 0,
        }
    }

    /// Interns `path` for asset type `A`, returning the SAME [`Handle<A>`]
    /// for every repeated call with an equal path (and the same `A`).
    ///
    /// See the struct doc for the A0 scope limitation — the returned handle
    /// is a reserved identity, not yet backed by a live
    /// [`Assets<A>`](crate::ecs::core::asset::assets::Assets) row.
    pub fn load<A: Asset>(&mut self, path: &str) -> Handle<A> {
        let key = (TypeId::of::<A>(), path.to_owned());
        if let Some(&index) = self.interned.get(&key) {
            return Handle::new(index, 0);
        }
        let index = self.next_index;
        self.next_index += 1;
        self.interned.insert(key, index);
        Handle::new(index, 0)
    }
}

// Hand-implemented rather than `#[derive(Resource)]`: `boyko-macros` is a
// dev-dependency of `boyko-ecs`, so its derives are unavailable in normal
// builds. `AssetServer` is a concrete (non-generic) type, so — unlike
// `Assets<T>` — the conventional per-impl `static ID: OnceLock<ResourceId>`
// is sound here (mirrors `AppExit`'s hand-written impl exactly).
impl Resource for AssetServer {
    #[inline]
    fn resource_id() -> ResourceId {
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId(register_new::<Self>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy;
    impl Asset for Dummy {
        type Cpu = ();
    }

    struct Other;
    impl Asset for Other {
        type Cpu = ();
    }

    /// Repeated `load` calls for the same path + asset type dedupe to the
    /// same handle (plan §A0 unit: `AssetServer::load` dedupes a repeated
    /// path to the same handle).
    #[test]
    fn load_dedupes_repeated_path_to_same_handle() {
        let mut server = AssetServer::new();
        let a = server.load::<Dummy>("meshes/cube.gltf");
        let b = server.load::<Dummy>("meshes/cube.gltf");
        assert_eq!(a, b, "the same path must intern to the same handle");
    }

    /// Distinct paths mint distinct handles.
    #[test]
    fn load_distinct_paths_mint_distinct_handles() {
        let mut server = AssetServer::new();
        let a = server.load::<Dummy>("meshes/cube.gltf");
        let b = server.load::<Dummy>("meshes/sphere.gltf");
        assert_ne!(a, b, "distinct paths must not collide");
    }

    /// The SAME path string, requested for two DIFFERENT asset types, does
    /// not collapse onto one intern entry — the key is `(TypeId, path)`, so
    /// each type gets its own fresh mint off the shared `next_index` counter.
    #[test]
    fn load_same_path_different_types_do_not_alias() {
        let mut server = AssetServer::new();
        let a = server.load::<Dummy>("shared/name.bin");
        let b = server.load::<Other>("shared/name.bin");
        assert_ne!(
            a.index(),
            b.index(),
            "same path string for two distinct asset types must not share one intern slot"
        );

        // Repeating either call still dedupes within its own type.
        let a_again = server.load::<Dummy>("shared/name.bin");
        assert_eq!(a, a_again);
    }
}
