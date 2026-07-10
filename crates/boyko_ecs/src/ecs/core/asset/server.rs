//! [`AssetServer`] — the path→[`Handle`] intern + loader registry (rung A3a:
//! reserve → decode → stage; the GPU-upload half of loading is rung A3b).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use crate::ecs::core::asset::asset::Asset;
use crate::ecs::core::asset::assets::Assets;
use crate::ecs::core::asset::error::AssetError;
use crate::ecs::core::asset::handle::Handle;
use crate::ecs::core::asset::loader::AssetLoader;
use crate::ecs::core::asset::staging::{AssetStaging, Staged};
use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;

/// The type-erased decode entry point [`AssetServer`]'s loader registry
/// stores per extension — a factored-out alias (clippy `type_complexity`),
/// not a new abstraction. See [`decode_thunk`]'s doc for what it does and why
/// it is safe.
type DecodeFn = fn(&[u8]) -> Result<Box<dyn Any + Send>, AssetError>;

/// Monomorphized per `L: AssetLoader` by [`AssetServer::register_loader`];
/// boxes `L`'s decoded [`Asset::Cpu`] behind `dyn Any + Send` so a single
/// non-generic [`DecodeFn`] pointer can dispatch to it, stored directly as
/// the registry's `HashMap` value (no wrapper struct — there is no other
/// per-loader state to carry: the ONE runtime type check the registry needs,
/// "does this extension's loader produce the asset type the caller
/// requested", is the `Any::downcast` in [`AssetServer::decode_bytes`]
/// itself, not a separately-stored `TypeId`).
///
/// This is the ENTIRE type-erasure mechanism: safe because `Box::new` +
/// unsizing to `Box<dyn Any + Send>` costs one allocation and involves no
/// `unsafe` at all, and the box is later reopened by
/// [`AssetServer::decode_bytes`] via std's own `TypeId`-checked
/// `Any::downcast` — never a hand-rolled transmute thunk.
fn decode_thunk<L: AssetLoader>(bytes: &[u8]) -> Result<Box<dyn Any + Send>, AssetError> {
    Ok(Box::new(L::decode(bytes)?))
}

/// World-global path→[`Handle`] intern + extension→loader registry,
/// registered as a [`Resource`].
///
/// # Rung A3a scope — reserve, decode, stage; no GPU upload
///
/// [`load`](Self::load) interns `(TypeId::of::<A>(), path)` and, the first
/// time a path is requested for asset type `A`, reads the file, decodes it
/// through the loader [`register_loader`](Self::register_loader) registered
/// for its extension, [`reserve`](Assets::reserve)s a row in `assets`, and
/// queues the decoded value on `staging`. GPU upload (turning the queued
/// value into a resident asset and calling
/// [`Assets::fill`](crate::ecs::core::asset::assets::Assets::fill)) is a
/// separate, render-side pass — rung A3b.
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
/// Both `interned` and `loaders` are `HashMap`s — acceptable ONLY because
/// path interning and loader registration are setup-time operations (asset
/// declarations / plugin registration), never touched on the per-frame hot
/// path. Mirrors the identical cold-path `HashMap` exception
/// `resource_type_registry` documents for its `TypeId → ResourceId` map.
#[derive(Default)]
pub struct AssetServer {
    /// `(asset TypeId, path)` → the `(index, generation)` pair of the
    /// [`Handle`] minted for that pair. Cold path only — see the struct doc.
    interned: HashMap<(TypeId, String), (u32, u32)>,
    /// Extension (lowercase, no leading dot) → its registered loader's
    /// type-erased decode entry point. Cold path only — see the struct doc.
    loaders: HashMap<String, DecodeFn>,
}

impl AssetServer {
    /// Creates an empty server with no interned paths and no registered
    /// loaders.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `L` for every extension in [`AssetLoader::EXTENSIONS`],
    /// routing that extension's future [`decode_bytes`](Self::decode_bytes) /
    /// [`load`](Self::load) calls through `L::decode`.
    ///
    /// Registering a second loader for an already-registered extension
    /// replaces the first (last-registered wins) — a setup-time convenience,
    /// not a runtime hot path.
    pub fn register_loader<L: AssetLoader>(&mut self) {
        for &ext in L::EXTENSIONS {
            self.loaders.insert(ext.to_owned(), decode_thunk::<L>);
        }
    }

    /// Decodes `bytes` as asset type `A`, dispatching through the loader
    /// registered for `ext` (lowercase, no leading dot).
    ///
    /// This is the fs-free, host-testable half of [`load`](Self::load)'s
    /// pipeline: no path parsing, no disk read — just a registry lookup, the
    /// loader's `decode`, and the safe erasure downcast.
    ///
    /// # Errors
    /// [`AssetError::UnsupportedExtension`] if no loader is registered for
    /// `ext`. [`AssetError::Decode`] if the registered loader's `decode`
    /// itself rejects `bytes`. [`AssetError::LoaderTypeMismatch`] if `ext`'s
    /// registered loader produces a DIFFERENT asset type than `A` — a
    /// RECOVERABLE, every-build error (two asset types' loaders can
    /// legitimately share an extension by caller mistake), detected by the
    /// `Any::downcast` below via its own `TypeId` comparison. There is
    /// deliberately no separate `debug_assert_eq!` pre-check here: that would
    /// turn this same, ordinary-API-misuse case into an unconditional debug
    /// panic, masking the graceful `Err` path in every non-release build.
    pub fn decode_bytes<A: Asset>(
        &self,
        ext: &str,
        bytes: &[u8],
    ) -> Result<<A as Asset>::Cpu, AssetError> {
        let decode = self.loaders.get(ext).ok_or_else(|| unsupported_extension(ext))?;
        let boxed = decode(bytes)?;
        boxed.downcast::<A::Cpu>().map(|b| *b).map_err(|_| loader_type_mismatch(ext))
    }

    /// Interns `path` for asset type `A`, returning the SAME [`Handle<A>`]
    /// for every repeated call with an equal path (and the same `A`).
    ///
    /// See the struct doc for the reserve→decode→stage pipeline and the
    /// failure-path contract (a read/decode error reserves + fails the row
    /// rather than panicking).
    pub fn load<A: Asset>(
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
/// an empty string if `path` has none. An empty extension never matches a
/// registered loader, so it resolves to `AssetError::UnsupportedExtension`
/// the same way any other unregistered extension would.
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
fn reserve_failed<A: Asset>(assets: &mut Assets<A>, path: &str, err: AssetError) -> Handle<A> {
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

#[cold]
#[inline(never)]
fn loader_type_mismatch(ext: &str) -> AssetError {
    AssetError::LoaderTypeMismatch { extension: ext.to_owned() }
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

    /// `decode_bytes::<A>` dispatches through the registered loader and
    /// returns the decoded value (plan §A3a unit: decode_bytes happy path).
    #[test]
    fn decode_bytes_dispatches_registered_loader() {
        let mut server = AssetServer::new();
        server.register_loader::<TestLoader>();

        let result = server.decode_bytes::<TestAsset>("test", &[0x42, 0xFF]);

        assert_eq!(result, Ok(TestCpu { tag: 0x42 }));
    }

    /// An extension with no registered loader errors `UnsupportedExtension`
    /// (plan §A3a unit: decode_bytes rejects an unregistered extension).
    #[test]
    fn decode_bytes_missing_extension_is_unsupported() {
        let server = AssetServer::new();

        let result = server.decode_bytes::<TestAsset>("nope", &[1]);

        assert_eq!(result, Err(AssetError::UnsupportedExtension { extension: "nope".to_owned() }));
    }

    /// Malformed bytes (an empty payload, per `TestLoader::decode`) surface
    /// the loader's own `Decode` error unchanged (plan §A3a unit:
    /// decode_bytes propagates a loader decode failure).
    #[test]
    fn decode_bytes_malformed_payload_is_decode_error() {
        let mut server = AssetServer::new();
        server.register_loader::<TestLoader>();

        let result = server.decode_bytes::<TestAsset>("test", &[]);

        assert!(matches!(result, Err(AssetError::Decode(_))), "empty payload must surface Decode, got {result:?}");
    }

    /// The W1 fix: an extension whose registered loader produces a
    /// DIFFERENT asset type than requested must be reported as
    /// `LoaderTypeMismatch`, NOT `UnsupportedExtension` (plan §A3a unit:
    /// decode_bytes distinguishes a type mismatch from a missing loader).
    #[test]
    fn decode_bytes_type_mismatch_is_loader_type_mismatch() {
        let mut server = AssetServer::new();
        server.register_loader::<TestLoader>(); // registers "test" -> TestAsset

        let result = server.decode_bytes::<Other>("test", &[1]);

        assert_eq!(
            result,
            Err(AssetError::LoaderTypeMismatch { extension: "test".to_owned() }),
            "a loader registered for TestAsset called as Other must report LoaderTypeMismatch, not \
             UnsupportedExtension"
        );
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
        server.register_loader::<TestLoader>();
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
