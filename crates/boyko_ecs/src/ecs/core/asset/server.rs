//! [`AssetServer`] — the path→[`Handle`] intern (rung A3a: reserve → decode →
//! stage; the GPU-upload half of loading is rung A3b). Loader dispatch itself
//! is `HasLoaders::LOADERS`, a compile-time-static const table (asset-
//! streaming plan F3) — not part of this server.

use std::any::TypeId;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use crate::ecs::core::asset::asset::Asset;
use crate::ecs::core::asset::assets::Assets;
use crate::ecs::core::asset::backing::AssetBacking;
use crate::ecs::core::asset::error::AssetError;
use crate::ecs::core::asset::handle::Handle;
use crate::ecs::core::asset::loader::HasLoaders;
use crate::ecs::core::asset::staging::{AssetStaging, Staged};
use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;

/// World-global path→[`Handle`] intern, registered as a [`Resource`].
///
/// # Rung A3a scope — reserve, decode, stage; no GPU upload
///
/// [`load`](Self::load) interns `(TypeId::of::<A>(), path)` and, the first
/// time a path is requested for asset type `A`, reads the file, decodes it
/// through the [`HasLoaders::LOADERS`] entry matching its extension,
/// [`reserve`](Assets::reserve)s a row in `assets`, and queues the decoded
/// value on `staging`. GPU upload (turning the queued value into a resident
/// asset and calling [`Assets::fill`](crate::ecs::core::asset::assets::Assets::fill))
/// is a separate, render-side pass — rung A3b.
///
/// A read or decode failure does NOT panic and does NOT leave the row
/// unminted: `load` still reserves a row (so the returned `Handle` is
/// well-formed and dedupes future calls the same way) and marks it
/// [`Failed`](crate::ecs::core::asset::asset::AssetLoadState::Failed). The
/// caller polls [`Assets::state`](crate::ecs::core::asset::assets::Assets::state)
/// to observe the outcome.
///
/// # Cold path
///
/// `interned` is a `HashMap` — acceptable ONLY because path interning is a
/// setup-time operation (asset declarations), never touched on the
/// per-frame hot path. Mirrors the identical cold-path `HashMap` exception
/// `resource_type_registry` documents for its `TypeId → ResourceId` map.
/// Loader dispatch itself is no longer a runtime registry at all
/// (asset-streaming plan F3): it is [`HasLoaders::LOADERS`], a compile-time
/// `&'static` const table baked into the binary per asset type.
#[derive(Default)]
pub struct AssetServer {
    /// `(asset TypeId, path)` → the `(index, generation)` pair of the
    /// [`Handle`] minted for that pair. Cold path only — see the struct doc.
    interned: HashMap<(TypeId, String), (u32, u32)>,
}

impl AssetServer {
    /// Creates an empty server with no interned paths.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes `bytes` as asset type `A`, linearly scanning `A::LOADERS`
    /// (asset-streaming plan F3) for the entry whose `extensions` contains
    /// `ext` (lowercase, no leading dot) and calling its TYPED `decode`
    /// directly — no erasure, no downcast.
    ///
    /// This is the fs-free, host-testable half of [`load`](Self::load)'s
    /// pipeline: no path parsing, no disk read — just a linear scan over
    /// `A::LOADERS` (a handful of entries per asset type) and the matched
    /// entry's `decode`.
    ///
    /// # Errors
    /// [`AssetError::UnsupportedExtension`] if no entry in `A::LOADERS`
    /// claims `ext`. [`AssetError::Decode`] if the matched entry's `decode`
    /// itself rejects `bytes`.
    #[inline]
    pub fn decode_bytes<A: HasLoaders>(
        &self,
        ext: &str,
        bytes: &[u8],
    ) -> Result<<A as Asset>::Cpu, AssetError> {
        let entry = A::LOADERS
            .iter()
            .find(|entry| entry.extensions.contains(&ext))
            .ok_or_else(|| unsupported_extension(ext))?;
        (entry.decode)(bytes)
    }

    /// Interns `path` for asset type `A`, returning the SAME [`Handle<A>`]
    /// for every repeated call with an equal path (and the same `A`).
    ///
    /// See the struct doc for the reserve→decode→stage pipeline and the
    /// failure-path contract (a read/decode error reserves + fails the row
    /// rather than panicking).
    ///
    /// `A: AssetBacking` (asset-streaming plan F1): `assets: &mut Assets<A>`
    /// requires it — `Assets<T>`'s own generic bound. `A: HasLoaders`
    /// (asset-streaming plan F3): `load` dispatches decode through `A`'s own
    /// static loader table.
    pub fn load<A: HasLoaders + AssetBacking>(
        &mut self,
        path: &str,
        assets: &mut Assets<A>,
        staging: &mut AssetStaging<A>,
    ) -> Handle<A> {
        let key = (TypeId::of::<A>(), path.to_owned());
        if let Some(&(index, generation)) = self.interned.get(&key) {
            return Handle::new(index, generation);
        }

        let handle = match std::fs::read(path) {
            Ok(bytes) => {
                let ext = extension_of(path);
                match self.decode_bytes::<A>(&ext, &bytes) {
                    Ok(cpu) => {
                        let handle = assets.reserve();
                        staging.push(Staged { handle, cpu });
                        handle
                    }
                    Err(err) => reserve_failed(assets, path, err),
                }
            }
            Err(io_err) => reserve_failed(assets, path, AssetError::Io(io_err.to_string())),
        };

        self.interned.insert(key, (handle.index(), handle.generation()));
        handle
    }
}

/// Lowercases and strips the leading dot from `path`'s extension, or returns
/// an empty string if `path` has none. An empty extension never matches an
/// entry in `A::LOADERS`, so it resolves to `AssetError::UnsupportedExtension`
/// the same way any other unclaimed extension would.
#[inline]
fn extension_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Reserves a row in `assets` and immediately marks it `Failed`, logging
/// `err` for `path` — the shared cold path both of [`AssetServer::load`]'s
/// failure branches (I/O and decode) fall through.
#[cold]
#[inline(never)]
fn reserve_failed<A: Asset + AssetBacking>(assets: &mut Assets<A>, path: &str, err: AssetError) -> Handle<A> {
    eprintln!("boyko_ecs: asset load failed for '{path}': {err}");
    let handle = assets.reserve();
    assets.fail(handle);
    handle
}

#[cold]
#[inline(never)]
fn unsupported_extension(ext: &str) -> AssetError {
    AssetError::UnsupportedExtension { extension: ext.to_owned() }
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
    use crate::ecs::core::asset::asset::AssetLoadState;
    use crate::ecs::core::asset::assets::Assets;
    use crate::ecs::core::asset::loader::{AssetLoader, LoaderEntry};
    use crate::ecs::core::asset::staging::AssetStaging;

    struct Dummy;
    impl Asset for Dummy {
        type Cpu = ();
    }

    struct Other;
    impl Asset for Other {
        type Cpu = ();
    }

    /// A tiny non-trivial `Asset`/`AssetLoader` pair for the `decode_bytes`/
    /// `load` pipeline tests below: `decode` parses the first byte of the
    /// payload as a tag, or errors on an empty payload — enough to prove
    /// decoded content actually flows through the dispatch, not just that it
    /// type-checks.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TestCpu {
        tag: u8,
    }

    struct TestAsset;
    impl Asset for TestAsset {
        type Cpu = TestCpu;
    }

    // Asset-streaming plan F1: `Assets<A>` (and therefore `AssetServer::load`)
    // requires `A: AssetBacking` in addition to `A: Asset` — a POD backing
    // with no device teardown is the correct fit for every test type here.
    crate::impl_asset_pod_backing!(Dummy, Other, TestAsset);

    // Asset-streaming plan F3: `AssetServer::load` requires `A: HasLoaders`.
    // `Dummy`/`Other` never decode in these tests (only their I/O-failure /
    // dedup paths run), so an empty table is enough for them to type-check.
    impl HasLoaders for Dummy {
        const LOADERS: &'static [LoaderEntry<Self>] = &[];
    }
    impl HasLoaders for Other {
        const LOADERS: &'static [LoaderEntry<Self>] = &[];
    }

    struct TestLoader;
    impl AssetLoader for TestLoader {
        type Out = TestAsset;
        const EXTENSIONS: &'static [&'static str] = &["test"];
        fn decode(bytes: &[u8]) -> Result<TestCpu, AssetError> {
            bytes
                .first()
                .copied()
                .map(|tag| TestCpu { tag })
                .ok_or_else(|| AssetError::Decode("empty test-asset payload".to_owned()))
        }
    }

    impl HasLoaders for TestAsset {
        const LOADERS: &'static [LoaderEntry<Self>] = &[LoaderEntry::of::<TestLoader>()];
    }

    /// `decode_bytes::<A>` linearly scans `A::LOADERS` for the extension and
    /// calls the matched entry's typed `decode` directly (plan §F3 unit:
    /// decode_bytes happy path, static dispatch).
    #[test]
    fn decode_bytes_dispatches_via_matching_extension() {
        let server = AssetServer::new();

        let result = server.decode_bytes::<TestAsset>("test", &[0x42, 0xFF]);

        assert_eq!(result, Ok(TestCpu { tag: 0x42 }));
    }

    /// An extension no entry in `A::LOADERS` claims errors
    /// `UnsupportedExtension` (plan §F3 unit: decode_bytes rejects an
    /// unclaimed extension).
    #[test]
    fn decode_bytes_missing_extension_is_unsupported() {
        let server = AssetServer::new();

        let result = server.decode_bytes::<TestAsset>("nope", &[1]);

        assert_eq!(result, Err(AssetError::UnsupportedExtension { extension: "nope".to_owned() }));
    }

    /// Malformed bytes (an empty payload, per `TestLoader::decode`) surface
    /// the loader's own `Decode` error unchanged (plan §F3 unit:
    /// decode_bytes propagates a loader decode failure).
    #[test]
    fn decode_bytes_malformed_payload_is_decode_error() {
        let server = AssetServer::new();

        let result = server.decode_bytes::<TestAsset>("test", &[]);

        assert!(matches!(result, Err(AssetError::Decode(_))), "empty payload must surface Decode, got {result:?}");
    }

    /// A read/decode failure still returns a resolvable `Handle` in the
    /// `Failed` state — `load` never panics on a missing file (plan §A3a
    /// unit: load's I/O failure path reserves+fails rather than panicking).
    #[test]
    fn load_missing_file_reserves_and_fails_without_panicking() {
        let mut server = AssetServer::new();
        let mut assets = Assets::<Dummy>::with_reserved(4);
        let mut staging = AssetStaging::<Dummy>::default();

        let handle = server.load::<Dummy>("definitely/does/not/exist.bin", &mut assets, &mut staging);

        assert_eq!(assets.state(handle), Some(AssetLoadState::Failed));
        assert!(assets.get(handle).is_none());
        assert!(staging.is_empty(), "a failed load must not queue a staged entry");
    }

    /// A successful decode reserves the row (still `Loading` — GPU upload,
    /// rung A3b, is what fills it) and queues exactly one staged entry whose
    /// handle matches the reserved row (plan §A3a unit: load's success path
    /// stages a Staged entry bound to the reserved handle).
    #[test]
    fn load_success_stages_entry_bound_to_reserved_handle() {
        let mut server = AssetServer::new();
        let mut assets = Assets::<TestAsset>::with_reserved(4);
        let mut staging = AssetStaging::<TestAsset>::default();

        let path =
            std::env::temp_dir().join(format!("boyko_ecs_asset_test_{}_{}.test", std::process::id(), line!()));
        std::fs::write(&path, [0x7A]).expect("test setup: write temp asset file");

        let handle = server.load::<TestAsset>(
            path.to_str().expect("test setup: temp path must be valid UTF-8"),
            &mut assets,
            &mut staging,
        );

        std::fs::remove_file(&path).ok();

        assert_eq!(
            assets.state(handle),
            Some(AssetLoadState::Loading),
            "the row awaits GPU upload (A3b), not yet Loaded"
        );
        let staged: Vec<_> = staging.drain().collect();
        assert_eq!(staged.len(), 1, "exactly one entry must be staged for a successful decode");
        assert_eq!(staged[0].handle, handle, "the staged entry's handle must be the exact row load() reserved");
        assert_eq!(staged[0].cpu, TestCpu { tag: 0x7A });
    }

    /// Repeated `load` calls for the same path + asset type dedupe to the
    /// same handle (plan §A0 unit: `AssetServer::load` dedupes a repeated
    /// path to the same handle) — still holds at A3a even though neither
    /// path exists on disk (the first call reserves+fails the row, but the
    /// SAME handle is cached and returned on every repeat).
    #[test]
    fn load_dedupes_repeated_path_to_same_handle() {
        let mut server = AssetServer::new();
        let mut assets = Assets::<Dummy>::with_reserved(4);
        let mut staging = AssetStaging::<Dummy>::default();
        let a = server.load::<Dummy>("meshes/cube.gltf", &mut assets, &mut staging);
        let b = server.load::<Dummy>("meshes/cube.gltf", &mut assets, &mut staging);
        assert_eq!(a, b, "the same path must intern to the same handle");
    }

    /// Distinct paths mint distinct handles.
    #[test]
    fn load_distinct_paths_mint_distinct_handles() {
        let mut server = AssetServer::new();
        let mut assets = Assets::<Dummy>::with_reserved(4);
        let mut staging = AssetStaging::<Dummy>::default();
        let a = server.load::<Dummy>("meshes/cube.gltf", &mut assets, &mut staging);
        let b = server.load::<Dummy>("meshes/sphere.gltf", &mut assets, &mut staging);
        assert_ne!(a, b, "distinct paths must not collide");
    }

    /// The SAME path string, requested for two DIFFERENT asset types, does
    /// NOT collapse onto one `interned` entry — the key is `(TypeId, path)`,
    /// so each type occupies its own entry, and each dedupes independently
    /// within its own table/staging on a repeat call.
    ///
    /// (NOT tested via `a.index() != b.index()`: `Dummy` and `Other` mint
    /// from two INDEPENDENT `Assets<T>` tables, so both handles legitimately
    /// start at index 0 — comparing indices across tables proves nothing
    /// about the intern keying. `interned`'s entry count, read directly since
    /// this test module is a descendant of `AssetServer`'s own module, is
    /// the actual `(TypeId, path)` keying property.)
    #[test]
    fn load_same_path_different_types_do_not_alias() {
        let mut server = AssetServer::new();
        let mut dummy_assets = Assets::<Dummy>::with_reserved(4);
        let mut dummy_staging = AssetStaging::<Dummy>::default();
        let mut other_assets = Assets::<Other>::with_reserved(4);
        let mut other_staging = AssetStaging::<Other>::default();

        let a = server.load::<Dummy>("shared/name.bin", &mut dummy_assets, &mut dummy_staging);
        let b = server.load::<Other>("shared/name.bin", &mut other_assets, &mut other_staging);
        assert_eq!(
            server.interned.len(),
            2,
            "the same path for two distinct asset types must occupy two DISTINCT \
             (TypeId, path) intern entries, not collapse onto one"
        );

        // Each type dedupes independently within its own table/staging.
        let a_again = server.load::<Dummy>("shared/name.bin", &mut dummy_assets, &mut dummy_staging);
        let b_again = server.load::<Other>("shared/name.bin", &mut other_assets, &mut other_staging);
        assert_eq!(a, a_again, "Dummy's load of the shared path must dedupe to the same handle");
        assert_eq!(b, b_again, "Other's load of the shared path must dedupe to the same handle");
        assert_eq!(
            server.interned.len(),
            2,
            "repeat loads of an already-interned path must not grow the intern map further"
        );
    }
}
