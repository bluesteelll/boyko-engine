//! Integration tests for asset-system rung A3a: the reserve → decode →
//! stage → fill/fail → remove pipeline across [`AssetServer`], [`Assets<T>`],
//! and [`AssetStaging<A>`] working together (three modules, one scenario per
//! test) — the tester's cross-module complement to each module's own unit
//! tests.
//!
//! The C1-hardened contract under test: a `Reserved` (in-flight) row that is
//! removed before it ever resolves (`fill`/`fail`) must not underflow
//! `Assets::len()`, and its row index must recycle correctly for a later
//! reservation — exercised here end-to-end through the real `AssetServer`
//! path (a missing file on disk), not just the unit-level `Assets` API.

use boyko_ecs::ecs::core::asset::{
    Asset, AssetError, AssetLoadState, AssetLoader, AssetServer, AssetStaging, Assets, HasLoaders, LoaderEntry,
};

/// A minimal `Asset`/`AssetLoader` pair whose `decode` parses the first byte
/// of the payload as a tag — enough to prove decoded content survives the
/// reserve→stage→fill round trip without needing a real asset format.
///
/// `TagAsset` (the type `Assets<TagAsset>` stores directly — the resident
/// form) and `TagCpu` (`TagAsset::Cpu`, the decode intermediate
/// `AssetStaging` queues) are deliberately DISTINCT types, mirroring the
/// real shape a later GPU-upload pass (rung A3b, not implemented yet) would
/// consume: `TagCpu` is what `AssetLoader::decode` produces, `TagAsset` is
/// what `Assets::fill` ultimately stores. The "upload" from one to the other
/// is a trivial passthrough here since this rung is host-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TagCpu {
    tag: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TagAsset {
    tag: u8,
}
impl Asset for TagAsset {
    type Cpu = TagCpu;
}

// Asset-streaming plan F1: `Assets<A>` (and therefore `AssetServer::load`)
// requires `A: AssetBacking` in addition to `A: Asset` — a POD backing with
// no device teardown is the correct fit for this host-only test type.
boyko_ecs::impl_asset_pod_backing!(TagAsset);

struct TagLoader;
impl AssetLoader for TagLoader {
    type Out = TagAsset;
    const EXTENSIONS: &'static [&'static str] = &["tag"];
    fn decode(bytes: &[u8]) -> Result<TagCpu, AssetError> {
        bytes
            .first()
            .copied()
            .map(|tag| TagCpu { tag })
            .ok_or_else(|| AssetError::Decode("empty tag-asset payload".to_owned()))
    }
}

// Asset-streaming plan F3: `AssetServer::load` dispatches decode through a
// compile-time-static `HasLoaders::LOADERS` table, not a runtime registry.
impl HasLoaders for TagAsset {
    const LOADERS: &'static [LoaderEntry<Self>] = &[LoaderEntry::of::<TagLoader>()];
}

/// Writes `bytes` to a fresh temp file with extension `.tag` and returns its
/// path — test setup only, cleaned up by the caller.
fn write_temp_tag_file(bytes: &[u8], unique: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("boyko_ecs_asset_pipeline_{unique}.tag"));
    std::fs::write(&path, bytes).expect("test setup: write temp asset file");
    path
}

/// The full happy path: `AssetServer::load` reads a real file from disk,
/// decodes it through the registered loader, reserves a row in `Assets`, and
/// stages exactly one entry bound to that reserved row. A render-side upload
/// pass would then drain staging and call `fill` — reproduced by hand here
/// since A3b (the upload consumer) does not exist yet.
#[test]
fn load_reserve_stage_fill_round_trips_the_decoded_value() {
    let path = write_temp_tag_file(&[0x11], "happy_path");

    let mut server = AssetServer::new();
    let mut assets = Assets::<TagAsset>::with_reserved(4);
    let mut staging = AssetStaging::<TagAsset>::default();

    let handle = server.load::<TagAsset>(path.to_str().expect("utf8 path"), &mut assets, &mut staging);
    std::fs::remove_file(&path).ok();

    assert_eq!(
        assets.state(handle),
        Some(AssetLoadState::Loading),
        "load() reserves + decodes but does not fill — upload (A3b) is a separate pass"
    );
    assert!(assets.get(handle).is_none(), "a Loading row must not resolve to a value yet");

    let mut staged: Vec<_> = staging.drain().collect();
    assert_eq!(staged.len(), 1, "exactly one entry must be staged for the successful decode");
    let entry = staged.pop().expect("checked len == 1 above");
    assert_eq!(entry.handle, handle, "the staged entry's handle must be the exact row load() reserved");

    // The render-side upload pass this rung hands off to (A3b): resolve the
    // staged entry's value into the reserved row (a trivial passthrough
    // "upload" here — see `TagAsset`'s doc).
    assets
        .fill(entry.handle, TagAsset { tag: entry.cpu.tag })
        .expect("filling a fresh reservation must succeed");

    assert_eq!(assets.get(handle), Some(&TagAsset { tag: 0x11 }), "fill must make the decoded value resolvable");
    assert_eq!(assets.state(handle), Some(AssetLoadState::Loaded));
    assert_eq!(assets.len(), 1);
}

/// A missing file on disk: `load` must not panic, must still return a
/// resolvable `Handle`, and that handle's row must be `Failed` — the caller
/// polls `Assets::state` to observe the outcome, exactly as the failure-path
/// contract on `AssetServer::load` documents.
#[test]
fn load_missing_file_yields_a_resolvable_failed_handle() {
    let mut server = AssetServer::new();
    let mut assets = Assets::<TagAsset>::with_reserved(4);
    let mut staging = AssetStaging::<TagAsset>::default();

    let handle = server.load::<TagAsset>("does/not/exist/on/disk.tag", &mut assets, &mut staging);

    assert_eq!(assets.state(handle), Some(AssetLoadState::Failed));
    assert!(assets.get(handle).is_none());
    assert!(staging.is_empty(), "a failed load must not queue a staged entry");
    assert_eq!(assets.len(), 0, "a Failed row is never counted as live");
}

/// The C1 case, exercised end-to-end through the real `AssetServer` path: a
/// `load` of a missing file reserves-then-fails a row; removing that row
/// must not underflow `Assets::len()`, and the freed index must recycle for
/// a LATER successful load — proving the reserve/fill/fail/remove machinery
/// is sound across the whole pipeline, not just the `Assets` unit API.
#[test]
fn removing_a_failed_load_recycles_its_row_for_a_later_successful_load() {
    let mut server = AssetServer::new();
    let mut assets = Assets::<TagAsset>::with_reserved(4);
    let mut staging = AssetStaging::<TagAsset>::default();

    let failed = server.load::<TagAsset>("missing/one.tag", &mut assets, &mut staging);
    assert_eq!(assets.state(failed), Some(AssetLoadState::Failed));
    assert_eq!(assets.len(), 0);

    let removed = assets.remove(failed);
    assert_eq!(removed, None, "a Failed (Reserved) row carries no value to return");
    assert_eq!(assets.len(), 0, "removing a Reserved row must not underflow live");

    // A fresh reservation (independent of AssetServer, which would dedupe by
    // path — this proves the row-level recycling machinery directly) must
    // reuse the freed index with the generation bumped by exactly one.
    let recycled = assets.reserve();
    assert_eq!(recycled.index(), failed.index(), "the freed row must be reused (same index)");
    assert_eq!(recycled.generation(), failed.generation() + 1, "reuse must bump the generation by exactly one");

    assets.fill(recycled, TagAsset { tag: 0xAB }).expect("filling the recycled row must succeed");
    assert_eq!(assets.get(recycled), Some(&TagAsset { tag: 0xAB }));
    assert_eq!(assets.len(), 1, "exactly the recycled row is live now");

    // The stale, pre-removal handle must still be rejected even though its
    // index has been physically reused underneath it.
    assert!(assets.get(failed).is_none(), "the OLD handle must stay rejected after its row is reused");
}

/// Two distinct asset types (each with its OWN `HasLoaders::LOADERS` table)
/// load independently through the SAME `AssetServer` without
/// cross-contaminating each other's `Assets<T>` table or intern cache, and
/// repeated `load` calls for the same path dedupe to the same handle (plan
/// §A0/§A3a/§F3: `load` dedupes a repeated path; distinct types do not alias).
///
/// # Note — static per-type dispatch has no shared-extension hazard
///
/// Under the F3 `HasLoaders` const-table dispatch, each asset type carries
/// its OWN `LOADERS` table (there is no process-wide extension→loader
/// registry to collide over): two types COULD legitimately reuse the same
/// extension string with no cross-contamination, since `decode_bytes::<A>`
/// only ever scans `A::LOADERS`. This test still uses two distinct
/// extensions (`.tag` / `.other`) — a realistic authoring choice for two
/// distinct formats — not because a shared extension would be unsound.
#[test]
fn distinct_asset_types_load_independently_through_one_server() {
    struct OtherAsset;
    impl Asset for OtherAsset {
        type Cpu = ();
    }
    boyko_ecs::impl_asset_pod_backing!(OtherAsset);
    struct OtherLoader;
    impl AssetLoader for OtherLoader {
        type Out = OtherAsset;
        const EXTENSIONS: &'static [&'static str] = &["other"];
        fn decode(_bytes: &[u8]) -> Result<(), AssetError> {
            Ok(())
        }
    }
    impl HasLoaders for OtherAsset {
        const LOADERS: &'static [LoaderEntry<Self>] = &[LoaderEntry::of::<OtherLoader>()];
    }

    let tag_path = write_temp_tag_file(&[0x99], "distinct_types_tag");
    let other_path = std::env::temp_dir().join("boyko_ecs_asset_pipeline_distinct_types.other");
    std::fs::write(&other_path, [0x00]).expect("test setup: write temp asset file");

    let mut server = AssetServer::new();

    let mut tag_assets = Assets::<TagAsset>::with_reserved(4);
    let mut tag_staging = AssetStaging::<TagAsset>::default();
    let mut other_assets = Assets::<OtherAsset>::with_reserved(4);
    let mut other_staging = AssetStaging::<OtherAsset>::default();

    let tag_handle =
        server.load::<TagAsset>(tag_path.to_str().expect("utf8 path"), &mut tag_assets, &mut tag_staging);
    let other_handle =
        server.load::<OtherAsset>(other_path.to_str().expect("utf8 path"), &mut other_assets, &mut other_staging);
    std::fs::remove_file(&tag_path).ok();
    std::fs::remove_file(&other_path).ok();

    assert_eq!(tag_assets.state(tag_handle), Some(AssetLoadState::Loading));
    assert_eq!(other_assets.state(other_handle), Some(AssetLoadState::Loading));

    let staged_tag: Vec<_> = tag_staging.drain().collect();
    let staged_other: Vec<_> = other_staging.drain().collect();
    assert_eq!(staged_tag.len(), 1, "the TagAsset table gets its own staged entry");
    assert_eq!(staged_other.len(), 1, "the OtherAsset table gets its own staged entry, independent of TagAsset's");
    assert_eq!(staged_tag[0].cpu, TagCpu { tag: 0x99 }, "TagAsset's own loader must have decoded its file's bytes");

    // Repeating the SAME path for the SAME type still dedupes within that
    // type's own intern entry, exercised through the SAME server instance
    // that also serves the other, unrelated asset type above.
    let missing = "missing-cache-would-refail-if-broken.tag";
    let tag_again = server.load::<TagAsset>(missing, &mut tag_assets, &mut tag_staging);
    let tag_again_repeat = server.load::<TagAsset>(missing, &mut tag_assets, &mut tag_staging);
    assert_eq!(tag_again, tag_again_repeat, "repeated load of the same path for the same type must dedupe");
}
