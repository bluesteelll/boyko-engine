//! BOOT-1 F-A: the persisted `VkPipelineCache` — the file format, its validation, the cache policy,
//! the crash-loop guard and the atomic save. Everything here is host code with no device in it, so
//! every rejection is testable without a GPU; `VulkanContext` (`device.rs`) owns the one
//! `VkPipelineCache` handle and calls into this module at boot and at save.
//!
//! # The file
//!
//! `<dir>/{vendorID:04x}-{deviceID:04x}.vkpc` = a 64-byte header, then the payload exactly as
//! `vkGetPipelineCacheData` returned it. All fields little-endian:
//!
//! | off | size | field | rejected as |
//! |---|---|---|---|
//! | 0 | 8 | magic `b"BOYKOVPC"` | [`Reject::BadMagic`] |
//! | 8 | 4 | format version = 1 | [`Reject::BadFormatVersion`] |
//! | 12 | 4 | header size = 64 | [`Reject::BadHeaderSize`] |
//! | 16 | 8 | payload size = file length − 64 | [`Reject::SizeMismatch`] |
//! | 24 | 8 | payload hash ([`payload_hash`]) | [`Reject::HashMismatch`], checked last (O(n)) |
//! | 32 | 4 | vendorID | [`Reject::ForeignDevice`] |
//! | 36 | 4 | deviceID | [`Reject::ForeignDevice`] |
//! | 40 | 4 | driverVersion | [`Reject::ForeignDriver`] |
//! | 44 | 4 | driver ABI = `size_of::<usize>()` | [`Reject::ForeignAbi`] |
//! | 48 | 16 | pipelineCacheUUID | [`Reject::ForeignDriver`] |
//! | 64 | n | payload; its first 32 B are `VkPipelineCacheHeaderVersionOne` | [`Reject::BadVulkanHeader`] |
//!
//! Every header byte is checked against a known value, so any single corrupted byte anywhere in the
//! file is rejected. Integrity is a spec obligation, not a nicety: VUID-VkPipelineCacheCreateInfo-
//! initialDataSize-00769 requires the initial data to be what a previous `vkGetPipelineCacheData`
//! returned, and some drivers crash on data they should have refused (zeux, "Robust pipeline cache
//! serialization"; Roblox, "Creating a Robust Pipeline Cache with Vulkan"). The header set matches
//! Godot's `PipelineCacheHeader` (magic, data size, data hash, vendor, device, driver version, UUID,
//! driver ABI) and Flutter Impeller's.
//!
//! # The key (check I2)
//!
//! The file name is vendor + device only; the driver version and the UUID live INSIDE the file and
//! reject a foreign blob there. The exe, the build and the profile are never part of the key: the
//! SPIR-V is `include_bytes!`-embedded, so a Rust-only rebuild leaves every create-info unchanged and
//! a `cargo run` after a Rust edit stays warm.

use core::panic::Location;
use std::ffi::OsString;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// The file magic.
pub const MAGIC: [u8; 8] = *b"BOYKOVPC";
/// The file format version this build writes and accepts.
pub const FORMAT_VERSION: u32 = 1;
/// The size of the file header that precedes the driver's payload.
pub const HEADER_SIZE: usize = 64;
/// The size of `VkPipelineCacheHeaderVersionOne`, the payload's own leading header.
pub const VK_HEADER_SIZE: usize = 32;
/// The largest payload accepted on load and written on save (32 MiB). Past it the cache restarts
/// empty: stale entries accumulate across shader edits and the driver cannot filter them.
pub const PAYLOAD_CAP: u64 = 32 * 1024 * 1024;
/// A `*.tmp` sibling older than this is a crashed save's leftover and is swept after a save.
pub const STALE_TMP_AGE: Duration = Duration::from_secs(10 * 60);
/// The driver ABI word the file carries: the pointer width the payload was produced under.
pub const DRIVER_ABI: u32 = core::mem::size_of::<usize>() as u32;

/// `BOYKO_PIPELINE_CACHE`: `on` (or unset) keeps the boot's policy; `off` creates no
/// `VkPipelineCache` at all (the pre-BOOT-1 driver path). Any other value panics at boot.
pub const ENV_SWITCH: &str = "BOYKO_PIPELINE_CACHE";
/// `BOYKO_PIPELINE_CACHE_DIR`: overrides the per-user cache directory (tests, portable installs).
pub const ENV_DIR: &str = "BOYKO_PIPELINE_CACHE_DIR";

/// The device and driver identity a cache blob is valid for — the fields the Vulkan cache header
/// carries (vendor, device, `pipelineCacheUUID`) plus the driver version.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceIdentity {
    /// `VkPhysicalDeviceProperties::vendorID`.
    pub vendor_id: u32,
    /// `VkPhysicalDeviceProperties::deviceID`.
    pub device_id: u32,
    /// `VkPhysicalDeviceProperties::driverVersion`.
    pub driver_version: u32,
    /// `VkPhysicalDeviceProperties::pipelineCacheUUID`.
    pub pipeline_cache_uuid: [u8; 16],
}

/// Why a cache file was not handed to the driver. Every rejection yields an empty cache; the next
/// save rewrites the file.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reject {
    /// Shorter than the two headers.
    TooSmall,
    /// Longer than [`HEADER_SIZE`] + [`PAYLOAD_CAP`].
    OverCap,
    /// Not [`MAGIC`].
    BadMagic,
    /// Not [`FORMAT_VERSION`].
    BadFormatVersion,
    /// Not [`HEADER_SIZE`].
    BadHeaderSize,
    /// The recorded payload size is not the file length minus the header (a torn file).
    SizeMismatch,
    /// Another vendor or device.
    ForeignDevice,
    /// Another driver version or `pipelineCacheUUID`.
    ForeignDriver,
    /// Written under another pointer width.
    ForeignAbi,
    /// The payload's `VkPipelineCacheHeaderVersionOne` is malformed or names another device.
    BadVulkanHeader,
    /// The payload hash does not match (a corrupt file).
    HashMismatch,
    /// The file could not be read.
    Io,
}

/// What a context's boot did with the pipeline cache.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CacheLoad {
    /// `BOYKO_PIPELINE_CACHE=off`: no `VkPipelineCache` exists.
    #[default]
    Off,
    /// An empty, in-memory, never-persisted cache (every non-runner context).
    Ephemeral,
    /// Persisted, but no file existed: the cache starts empty.
    Absent,
    /// Persisted, and the file validated: its payload seeded the cache.
    Loaded {
        /// The payload bytes handed to `vkCreatePipelineCache`.
        bytes: u64,
    },
    /// Persisted, and the file was rejected: the cache starts empty.
    Rejected(Reject),
    /// Persisted, and the previous boot died with a blob handed to the driver (the crash-loop
    /// sentinel was present): the file was discarded and the cache starts empty.
    CrashGuard,
    /// Persisted, the file validated, but the driver refused it: the cache was re-created empty
    /// and the next save rewrites the file (critique W3; zeux / Roblox recover the same way).
    DriverRefused,
    /// `vkCreatePipelineCache` failed even with no initial data: every create passes no cache.
    CreateFailed,
}

impl CacheLoad {
    /// A short human label for the boot telemetry line.
    pub fn label(&self) -> &'static str {
        match self {
            CacheLoad::Off => "off: BOYKO_PIPELINE_CACHE=off",
            CacheLoad::Ephemeral => "ephemeral: not persisted",
            CacheLoad::Absent => "cold: file absent",
            CacheLoad::Loaded { .. } => "warm: file loaded",
            CacheLoad::Rejected(_) => "cold: file rejected",
            CacheLoad::CrashGuard => "cold: crash sentinel, file discarded",
            CacheLoad::DriverRefused => "cold: driver refused the file",
            CacheLoad::CreateFailed => "none: vkCreatePipelineCache failed",
        }
    }
}

/// Why a save did not complete.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveError {
    /// The cache directory could not be created.
    CreateDir,
    /// The temp file could not be written.
    Write,
    /// The temp file could not be renamed over the cache file.
    Rename,
    /// `vkGetPipelineCacheData` failed, or changed size twice.
    GetData,
    /// The payload exceeds [`PAYLOAD_CAP`]: the file was removed, so the next boot starts empty.
    OverCap,
}

/// What one save attempt did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CacheSave {
    /// No persisted cache (off, ephemeral, or no handle).
    #[default]
    NotPersisted,
    /// Nothing missed since the last save: no write.
    Clean,
    /// The file was rewritten.
    Saved {
        /// The file's size in bytes (header included).
        bytes: u64,
        /// The wall time of the read-back + seal + write + rename, in nanoseconds.
        nanos: u64,
    },
    /// The save did not complete; the previous file (if any) is untouched unless `OverCap`.
    Failed(SaveError),
}

/// Where a context's pipeline cache comes from — handed to
/// [`VulkanContext::boot_singleton`](crate::device::VulkanContext::boot_singleton) by value, once.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PipelineCachePolicy {
    /// No `VkPipelineCache`: every create passes `VK_NULL_HANDLE`.
    Off,
    /// An empty in-memory cache, never read from or written to disk.
    Ephemeral,
    /// A cache loaded from and saved to `dir`.
    Persisted {
        /// The directory holding the cache file.
        dir: PathBuf,
    },
}

impl PipelineCachePolicy {
    /// The per-user policy: `BOYKO_PIPELINE_CACHE_DIR` if set, else
    /// `%LOCALAPPDATA%\boyko-engine\pipeline-cache` on Windows, else [`Self::Ephemeral`].
    pub fn per_user_from_env() -> Self {
        Self::per_user_from(|key| std::env::var_os(key))
    }

    /// [`Self::per_user_from_env`] over an injected variable getter (testable without `set_var`).
    pub fn per_user_from(get: impl Fn(&str) -> Option<OsString>) -> Self {
        if let Some(dir) = get(ENV_DIR).filter(|d| !d.is_empty()) {
            return Self::Persisted { dir: PathBuf::from(dir) };
        }
        if cfg!(windows)
            && let Some(base) = get("LOCALAPPDATA").filter(|d| !d.is_empty())
        {
            return Self::Persisted {
                dir: PathBuf::from(base).join("boyko-engine").join("pipeline-cache"),
            };
        }
        Self::Ephemeral
    }

    /// Applies the `BOYKO_PIPELINE_CACHE` switch: `off` overrides every policy with [`Self::Off`].
    pub fn with_switch(self, switch: CacheSwitch) -> Self {
        match switch {
            CacheSwitch::On => self,
            CacheSwitch::Off => Self::Off,
        }
    }
}

/// The `BOYKO_PIPELINE_CACHE` switch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CacheSwitch {
    /// Unset or `on`: the boot's own policy.
    On,
    /// `off`: no cache at all.
    Off,
}

/// Reads `BOYKO_PIPELINE_CACHE` once. Panics on any value but `on` / `off`: a typo must not
/// silently turn a cold measurement warm (the `parse_hier_cull_env` precedent).
pub fn switch_from_env() -> CacheSwitch {
    switch_from(|key| std::env::var_os(key))
}

/// [`switch_from_env`] over an injected getter.
pub fn switch_from(get: impl Fn(&str) -> Option<OsString>) -> CacheSwitch {
    match get(ENV_SWITCH) {
        None => CacheSwitch::On,
        Some(v) if v == "on" => CacheSwitch::On,
        Some(v) if v == "off" => CacheSwitch::Off,
        Some(v) => bad_switch(&v),
    }
}

#[cold]
#[inline(never)]
fn bad_switch(v: &OsString) -> ! {
    panic!("invariant: {ENV_SWITCH} must be `on` or `off`, got {v:?}")
}

/// The cache file name for `id`: vendor + device only (check I2).
pub fn file_name(id: &DeviceIdentity) -> String {
    format!("{:04x}-{:04x}.vkpc", id.vendor_id, id.device_id)
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The payload hash.
// ─────────────────────────────────────────────────────────────────────────────────────────────

const H_P1: u64 = 0x9E37_79B1_85EB_CA87;
const H_P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const H_P3: u64 = 0x1656_67B1_9E37_79F9;
const H_P4: u64 = 0x85EB_CA77_C2B2_AE63;

/// The payload hash: one XXH64-style lane over little-endian `u64` words (the tail zero-padded into
/// one final word), seeded with the length, finished with XXH64's avalanche.
///
/// Per word: `h ^= rotl(w * P2, 31) * P1; h = rotl(h, 27) * P1 + P4`. The multiply-rotate-multiply
/// round makes each word's contribution non-linear in every bit, which plain word-wise FNV-1a is
/// not: there, flipping bit 63 of any two words cancels deterministically (`(x ^ 2^63)·P =
/// (x·P) ^ 2^63` for odd `P`; critique O3), and a collision is exactly the VUID-00769 hazard this
/// hash exists to exclude. The cost class is FNV's (a few multiplies per 8 bytes).
pub fn payload_hash(payload: &[u8]) -> u64 {
    let mut h = H_P3 ^ (payload.len() as u64).wrapping_mul(H_P1);
    let (words, tail) = payload.as_chunks::<8>();
    for w in words {
        h = hash_round(h, u64::from_le_bytes(*w));
    }
    if !tail.is_empty() {
        let mut last = [0u8; 8];
        last[..tail.len()].copy_from_slice(tail);
        h = hash_round(h, u64::from_le_bytes(last));
    }
    h ^= h >> 33;
    h = h.wrapping_mul(H_P2);
    h ^= h >> 29;
    h = h.wrapping_mul(H_P3);
    h ^ (h >> 32)
}

#[inline]
fn hash_round(h: u64, w: u64) -> u64 {
    let k = w.wrapping_mul(H_P2).rotate_left(31).wrapping_mul(H_P1);
    (h ^ k).rotate_left(27).wrapping_mul(H_P1).wrapping_add(H_P4)
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Validate / seal.
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[inline]
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().expect("invariant: 4-byte field"))
}

#[inline]
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("invariant: 8-byte field"))
}

/// Validates a whole cache file against the running device and returns its payload. Every check
/// is cheap and ordered so the O(n) hash runs last.
pub fn validate<'a>(file: &'a [u8], current: &DeviceIdentity) -> Result<&'a [u8], Reject> {
    if file.len() < HEADER_SIZE + VK_HEADER_SIZE {
        return Err(Reject::TooSmall);
    }
    if file.len() as u64 > HEADER_SIZE as u64 + PAYLOAD_CAP {
        return Err(Reject::OverCap);
    }
    if file[0..8] != MAGIC {
        return Err(Reject::BadMagic);
    }
    if u32_at(file, 8) != FORMAT_VERSION {
        return Err(Reject::BadFormatVersion);
    }
    if u32_at(file, 12) as usize != HEADER_SIZE {
        return Err(Reject::BadHeaderSize);
    }
    if u64_at(file, 16) != (file.len() - HEADER_SIZE) as u64 {
        return Err(Reject::SizeMismatch);
    }
    if u32_at(file, 32) != current.vendor_id || u32_at(file, 36) != current.device_id {
        return Err(Reject::ForeignDevice);
    }
    if u32_at(file, 40) != current.driver_version {
        return Err(Reject::ForeignDriver);
    }
    if u32_at(file, 44) != DRIVER_ABI {
        return Err(Reject::ForeignAbi);
    }
    if file[48..64] != current.pipeline_cache_uuid {
        return Err(Reject::ForeignDriver);
    }
    let payload = &file[HEADER_SIZE..];
    // `VkPipelineCacheHeaderVersionOne`, written least-significant byte first by the spec.
    if u32_at(payload, 0) as usize != VK_HEADER_SIZE
        || u32_at(payload, 4) != 1
        || u32_at(payload, 8) != current.vendor_id
        || u32_at(payload, 12) != current.device_id
        || payload[16..32] != current.pipeline_cache_uuid
    {
        return Err(Reject::BadVulkanHeader);
    }
    if u64_at(file, 24) != payload_hash(payload) {
        return Err(Reject::HashMismatch);
    }
    Ok(payload)
}

/// Writes the 64-byte file header for `payload` into `header`.
pub fn write_header(header: &mut [u8], payload: &[u8], id: &DeviceIdentity) {
    assert_eq!(header.len(), HEADER_SIZE, "invariant: the file header is {HEADER_SIZE} bytes");
    header[0..8].copy_from_slice(&MAGIC);
    header[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    header[16..24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[24..32].copy_from_slice(&payload_hash(payload).to_le_bytes());
    header[32..36].copy_from_slice(&id.vendor_id.to_le_bytes());
    header[36..40].copy_from_slice(&id.device_id.to_le_bytes());
    header[40..44].copy_from_slice(&id.driver_version.to_le_bytes());
    header[44..48].copy_from_slice(&DRIVER_ABI.to_le_bytes());
    header[48..64].copy_from_slice(&id.pipeline_cache_uuid);
}

/// Seals `payload` into a whole cache file in `out` (cleared first).
pub fn seal(payload: &[u8], id: &DeviceIdentity, out: &mut Vec<u8>) {
    out.clear();
    out.resize(HEADER_SIZE, 0);
    out.extend_from_slice(payload);
    let (header, body) = out.split_at_mut(HEADER_SIZE);
    write_header(header, body, id);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The store: one file per GPU, its crash sentinel, the atomic save.
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The persisted side of one context's cache: where its file lives and which device it is for.
/// Immutable after boot.
#[derive(Clone, Debug)]
pub struct PipelineCacheStore {
    dir: PathBuf,
    path: PathBuf,
    sentinel: PathBuf,
    identity: DeviceIdentity,
}

/// A save's monotonic suffix, so two saves of one process never share a temp name.
static SAVE_SEQ: AtomicU32 = AtomicU32::new(0);

impl PipelineCacheStore {
    /// The store for `identity` under `dir`.
    pub fn new(dir: &Path, identity: DeviceIdentity) -> Self {
        let name = file_name(&identity);
        Self {
            dir: dir.to_path_buf(),
            path: dir.join(&name),
            sentinel: dir.join(format!("{name}.loading")),
            identity,
        }
    }

    /// The cache file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The crash-loop sentinel's path.
    pub fn sentinel_path(&self) -> &Path {
        &self.sentinel
    }

    /// The device identity the file is keyed by.
    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    /// Reads and validates the file once. Returns the outcome and, for [`CacheLoad::Loaded`], the
    /// whole file (its payload is `[HEADER_SIZE..]`); an empty `Vec` otherwise.
    ///
    /// A crash sentinel left by a previous boot (ruling 91 Q3) wins over everything: that boot died
    /// after handing this file to the driver, so the file is deleted, the sentinel with it, and the
    /// cache starts empty — one bad blob cannot brick every boot.
    pub fn load(&self) -> (CacheLoad, Vec<u8>) {
        if self.sentinel.exists() {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_file(&self.sentinel);
            report_crash_guard(&self.path);
            return (CacheLoad::CrashGuard, Vec::new());
        }
        let mut file = match fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (CacheLoad::Absent, Vec::new()),
            Err(_) => return (CacheLoad::Rejected(Reject::Io), Vec::new()),
        };
        let len = match file.metadata() {
            Ok(m) => m.len(),
            Err(_) => return (CacheLoad::Rejected(Reject::Io), Vec::new()),
        };
        // The cap applies from the length, before any read.
        if len < (HEADER_SIZE + VK_HEADER_SIZE) as u64 {
            return (CacheLoad::Rejected(Reject::TooSmall), Vec::new());
        }
        if len > HEADER_SIZE as u64 + PAYLOAD_CAP {
            return (CacheLoad::Rejected(Reject::OverCap), Vec::new());
        }
        // Read once, bounded: a file that grew after the length check is cut at the cap plus one,
        // which `validate` then rejects; everything below runs on this one in-memory copy.
        let mut bytes = Vec::with_capacity(len as usize);
        let limit = HEADER_SIZE as u64 + PAYLOAD_CAP + 1;
        if (&mut file).take(limit).read_to_end(&mut bytes).is_err() {
            return (CacheLoad::Rejected(Reject::Io), Vec::new());
        }
        match validate(&bytes, &self.identity) {
            Ok(payload) => {
                let n = payload.len() as u64;
                (CacheLoad::Loaded { bytes: n }, bytes)
            }
            Err(r) => (CacheLoad::Rejected(r), Vec::new()),
        }
    }

    /// Writes the crash sentinel, right before a loaded blob is handed to the driver. Best effort:
    /// a failure only disarms the guard.
    pub fn arm_sentinel(&self) {
        let _ = fs::write(&self.sentinel, std::process::id().to_le_bytes());
    }

    /// Removes the crash sentinel once boot got past every use of the loaded blob.
    pub fn clear_sentinel(&self) {
        let _ = fs::remove_file(&self.sentinel);
    }

    /// Atomically replaces the cache file with `file` (a whole sealed file): write
    /// `<name>.<pid>.<seq>.tmp`, then rename it over the file on the same volume. No fsync: a torn
    /// file after a power loss fails the size or hash check and degrades to empty. After a
    /// successful rename, `*.tmp` siblings older than [`STALE_TMP_AGE`] (a crash mid-save) are
    /// swept, best effort.
    pub fn write_atomic(&self, file: &[u8]) -> Result<(), SaveError> {
        fs::create_dir_all(&self.dir).map_err(|_| SaveError::CreateDir)?;
        let name = file_name(&self.identity);
        let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = self.dir.join(format!("{name}.{}.{seq}.tmp", std::process::id()));
        if fs::write(&tmp, file).is_err() {
            let _ = fs::remove_file(&tmp);
            return Err(SaveError::Write);
        }
        if fs::rename(&tmp, &self.path).is_err() {
            let _ = fs::remove_file(&tmp);
            return Err(SaveError::Rename);
        }
        self.sweep_stale_tmp(&name);
        Ok(())
    }

    /// Removes the cache file (the over-cap reset).
    pub fn remove(&self) {
        let _ = fs::remove_file(&self.path);
    }

    fn sweep_stale_tmp(&self, name: &str) {
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(s) = file_name.to_str() else { continue };
            if !(s.starts_with(name) && s.ends_with(".tmp")) {
                continue;
            }
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age >= STALE_TMP_AGE);
            if old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Seals `file` in place (its payload is `[HEADER_SIZE..]`; the header bytes are overwritten) and
/// replaces the store's file. `start` is when the save began, for the reported time. A payload over
/// [`PAYLOAD_CAP`] removes the file instead, so the next boot starts empty.
pub fn save_file(store: &PipelineCacheStore, file: &mut [u8], start: Instant) -> CacheSave {
    assert!(file.len() >= HEADER_SIZE, "invariant: a sealed file starts with its header");
    if (file.len() - HEADER_SIZE) as u64 > PAYLOAD_CAP {
        store.remove();
        return CacheSave::Failed(SaveError::OverCap);
    }
    let (header, payload) = file.split_at_mut(HEADER_SIZE);
    write_header(header, payload, store.identity());
    match store.write_atomic(file) {
        Ok(()) => CacheSave::Saved {
            bytes: file.len() as u64,
            nanos: start.elapsed().as_nanos() as u64,
        },
        Err(e) => CacheSave::Failed(e),
    }
}

/// [`save_file`] for a payload held apart: copies it behind a header slot in `scratch`.
pub fn save_payload(store: &PipelineCacheStore, payload: &[u8], scratch: &mut Vec<u8>) -> CacheSave {
    let start = Instant::now();
    scratch.clear();
    scratch.resize(HEADER_SIZE, 0);
    scratch.extend_from_slice(payload);
    save_file(store, scratch, start)
}

#[cold]
#[inline(never)]
fn report_crash_guard(path: &Path) {
    boyko_log::info!(
        boyko_log::RhiVulkan,
        "pipeline cache: the previous boot died after loading {}; the file was discarded and this boot starts empty",
        path.to_str().unwrap_or("<a non-UTF-8 path>")
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Creation with the driver-refusal fallback (critique W3).
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Creates the cache from `initial` (a validated payload, if any) through `create`, falling back to
/// an empty cache when the driver refuses the data — the recovery zeux and Roblox describe. Returns
/// the handle (`None` when even the empty create failed) and the load outcome to record.
///
/// `create` is the `vkCreatePipelineCache` call: `Some(data)` with initial data, `None` without.
/// Generic so the fallback order is testable without a device.
pub fn create_with_fallback<H>(
    load: CacheLoad,
    initial: Option<&[u8]>,
    mut create: impl FnMut(Option<&[u8]>) -> Option<H>,
) -> (Option<H>, CacheLoad) {
    if let Some(data) = initial {
        if let Some(h) = create(Some(data)) {
            return (Some(h), load);
        }
        return match create(None) {
            Some(h) => (Some(h), CacheLoad::DriverRefused),
            None => (None, CacheLoad::CreateFailed),
        };
    }
    match create(None) {
        Some(h) => (Some(h), load),
        None => (None, CacheLoad::CreateFailed),
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Creation feedback statistics.
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// `VK_PIPELINE_CREATION_FEEDBACK_VALID_BIT`.
pub const FEEDBACK_VALID: u32 = 0x1;
/// `VK_PIPELINE_CREATION_FEEDBACK_APPLICATION_PIPELINE_CACHE_HIT_BIT`.
pub const FEEDBACK_APP_CACHE_HIT: u32 = 0x2;
/// How many of the slowest creations the stats keep.
pub const TOP_N: usize = 5;

/// Which kind of pipeline a creation built.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PipelineKind {
    /// A compute pipeline.
    #[default]
    Compute,
    /// A graphics pipeline.
    Graphics,
}

/// One of the slowest creations.
#[derive(Clone, Copy, Debug, Default)]
pub struct TopEntry {
    /// The driver-reported creation time, nanoseconds.
    pub ns: u64,
    /// The caller of the creation entry point (`#[track_caller]`).
    pub site: Option<&'static Location<'static>>,
    /// Compute or graphics.
    pub kind: PipelineKind,
}

/// Per-context pipeline-creation counters, filled from `VkPipelineCreationFeedback` after every
/// create. A `Copy` POD kept in a `Cell` on the `!Sync` context; boot only.
#[derive(Clone, Copy, Debug, Default)]
pub struct PipelineCreationStats {
    /// Compute pipelines created.
    pub compute: u32,
    /// Graphics pipelines created.
    pub graphics: u32,
    /// Creations the driver reported as application-cache hits (VALID and HIT both set).
    pub hits: u32,
    /// Creations whose feedback came back without the VALID bit.
    pub feedback_invalid: u32,
    /// The driver-reported creation time summed over every creation, nanoseconds.
    pub driver_ns: u64,
    /// Creations that missed the cache since the last save — the save's dirty count.
    pub misses_since_save: u32,
    /// The [`TOP_N`] slowest creations, descending by `ns`.
    pub top: [TopEntry; TOP_N],
}

impl PipelineCreationStats {
    /// Every pipeline created.
    #[inline]
    pub fn total(&self) -> u32 {
        self.compute + self.graphics
    }

    /// Records one creation's feedback.
    pub fn record(
        &mut self,
        kind: PipelineKind,
        flags: u32,
        duration_ns: u64,
        site: &'static Location<'static>,
    ) {
        match kind {
            PipelineKind::Compute => self.compute += 1,
            PipelineKind::Graphics => self.graphics += 1,
        }
        let valid = flags & FEEDBACK_VALID != 0;
        let hit = valid && flags & FEEDBACK_APP_CACHE_HIT != 0;
        if !valid {
            self.feedback_invalid += 1;
        }
        if hit {
            self.hits += 1;
        } else {
            self.misses_since_save += 1;
        }
        let ns = if valid { duration_ns } else { 0 };
        self.driver_ns = self.driver_ns.saturating_add(ns);
        if let Some(i) = self.top.iter().position(|e| ns > e.ns) {
            self.top.copy_within(i..TOP_N - 1, i + 1);
            self.top[i] = TopEntry { ns, site: Some(site), kind };
        }
    }
}

#[cfg(test)]
mod tests {
    //! BOOT-1 C3 host tests: the file format, its rejections, the key, the policy grammar, the
    //! crash-loop guard, the driver-refusal fallback and the atomic save. No device.

    use super::*;

    use proptest::prelude::*;

    fn id() -> DeviceIdentity {
        DeviceIdentity {
            vendor_id: 0x10de,
            device_id: 0x2504,
            driver_version: 0x8_1234,
            pipeline_cache_uuid: *b"0123456789abcdef",
        }
    }

    /// A payload shaped like a driver's: a valid `VkPipelineCacheHeaderVersionOne` for `id()`, then
    /// `body` bytes of a deterministic pattern.
    fn payload(body: usize) -> Vec<u8> {
        let id = id();
        let mut p = Vec::with_capacity(VK_HEADER_SIZE + body);
        p.extend_from_slice(&(VK_HEADER_SIZE as u32).to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes());
        p.extend_from_slice(&id.vendor_id.to_le_bytes());
        p.extend_from_slice(&id.device_id.to_le_bytes());
        p.extend_from_slice(&id.pipeline_cache_uuid);
        p.extend((0..body).map(|i| (i.wrapping_mul(31) ^ (i >> 3)) as u8));
        p
    }

    fn sealed(body: usize) -> Vec<u8> {
        let mut out = Vec::new();
        seal(&payload(body), &id(), &mut out);
        out
    }

    fn get<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(v))
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("boyko_pc_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("invariant: the temp dir accepts a subdirectory");
        d
    }

    #[test]
    fn roundtrip_accepts_its_own_blob() {
        for body in [0usize, 1, 7, 8, 9, 4096, 70_001] {
            let file = sealed(body);
            assert_eq!(file.len(), HEADER_SIZE + VK_HEADER_SIZE + body);
            let got = validate(&file, &id()).expect("its own blob validates");
            assert_eq!(got, &payload(body)[..], "the payload comes back byte for byte");
        }
    }

    #[test]
    fn rejects_every_truncation() {
        let file = sealed(300);
        for len in 0..file.len() {
            let r = validate(&file[..len], &id());
            assert!(r.is_err(), "a {len}-byte prefix of a {}-byte file validated", file.len());
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 2048, ..ProptestConfig::default() })]
        #[test]
        fn rejects_any_single_byte_flip(offset in 0usize..(HEADER_SIZE + VK_HEADER_SIZE + 513), mask in 1u8..=255) {
            let mut file = sealed(513);
            prop_assume!(offset < file.len());
            file[offset] ^= mask;
            prop_assert!(validate(&file, &id()).is_err(), "a flip of {mask:#04x} at byte {offset} validated");
        }
    }

    /// Critique O3: plain word-wise FNV-1a collides on two bit-63 flips in different words; the
    /// shipped hash must not.
    #[test]
    fn rejects_paired_top_bit_flips() {
        let base = sealed(4096);
        let words = (base.len() - HEADER_SIZE - VK_HEADER_SIZE) / 8;
        for (a, b) in [(0usize, 1usize), (0, words - 1), (5, 77), (100, 101), (200, 400)] {
            let mut file = base.clone();
            for w in [a, b] {
                let byte = HEADER_SIZE + VK_HEADER_SIZE + w * 8 + 7;
                file[byte] ^= 0x80;
            }
            assert_eq!(
                validate(&file, &id()),
                Err(Reject::HashMismatch),
                "bit 63 flipped in payload words {a} and {b} validated"
            );
        }
    }

    #[test]
    fn rejects_foreign_vendor_device_driver_uuid_abi() {
        let file = sealed(64);
        let mut other = id();
        other.vendor_id ^= 1;
        assert_eq!(validate(&file, &other), Err(Reject::ForeignDevice));
        let mut other = id();
        other.device_id ^= 1;
        assert_eq!(validate(&file, &other), Err(Reject::ForeignDevice));
        let mut other = id();
        other.driver_version += 1;
        assert_eq!(validate(&file, &other), Err(Reject::ForeignDriver));
        let mut other = id();
        other.pipeline_cache_uuid[15] ^= 1;
        assert_eq!(validate(&file, &other), Err(Reject::ForeignDriver));
        let mut abi = file.clone();
        abi[44..48].copy_from_slice(&(DRIVER_ABI / 2).to_le_bytes());
        assert_eq!(validate(&abi, &id()), Err(Reject::ForeignAbi));
    }

    /// Resealed after the edit, so the outer identity matches and the hash is valid: only the inner
    /// `VkPipelineCacheHeaderVersionOne` check can catch it.
    #[test]
    fn rejects_bad_inner_vulkan_header() {
        let edits: [(usize, u8); 6] = [(0, 0x01), (4, 0x02), (8, 0x01), (12, 0x01), (16, 0x01), (31, 0x80)];
        for (off, mask) in edits {
            let mut p = payload(128);
            p[off] ^= mask;
            let mut file = Vec::new();
            seal(&p, &id(), &mut file);
            assert_eq!(
                validate(&file, &id()),
                Err(Reject::BadVulkanHeader),
                "an inner-header edit at payload byte {off} validated"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "miri-unsupported: filesystem under isolation")]
    fn rejects_zero_payload_and_over_cap() {
        let mut empty = Vec::new();
        seal(&[], &id(), &mut empty);
        assert_eq!(validate(&empty, &id()), Err(Reject::TooSmall));
        // Over the cap, from the length alone: the store refuses before reading a byte.
        let dir = scratch_dir("overcap");
        let store = PipelineCacheStore::new(&dir, id());
        let f = fs::File::create(store.path()).expect("invariant: scratch is writable");
        f.set_len(HEADER_SIZE as u64 + PAYLOAD_CAP + 1).expect("invariant: a sparse length");
        drop(f);
        assert_eq!(store.load().0, CacheLoad::Rejected(Reject::OverCap));
        let f = fs::File::create(store.path()).expect("invariant: scratch is writable");
        f.set_len(HEADER_SIZE as u64).expect("invariant: a short length");
        drop(f);
        assert_eq!(store.load().0, CacheLoad::Rejected(Reject::TooSmall));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Check I2: the name is vendor + device only; driver, UUID, profile and exe change nothing.
    #[test]
    fn file_name_is_vendor_device_only() {
        let base = id();
        assert_eq!(file_name(&base), "10de-2504.vkpc");
        let mut other = base;
        other.driver_version += 7;
        other.pipeline_cache_uuid = [0xee; 16];
        assert_eq!(file_name(&other), file_name(&base), "a driver update keeps the file name");
        let mut other = base;
        other.device_id = 0x2503;
        assert_ne!(file_name(&other), file_name(&base), "another GPU gets its own file");
        let name = file_name(&base);
        assert!(
            !name.contains(env!("CARGO_PKG_VERSION")) && !name.contains("debug") && !name.contains("release"),
            "the key never carries the build"
        );
    }

    #[test]
    fn policy_env_grammar() {
        assert_eq!(switch_from(get(&[])), CacheSwitch::On);
        assert_eq!(switch_from(get(&[(ENV_SWITCH, "on")])), CacheSwitch::On);
        assert_eq!(switch_from(get(&[(ENV_SWITCH, "off")])), CacheSwitch::Off);
        let p = PipelineCachePolicy::Persisted { dir: PathBuf::from("x") };
        assert_eq!(p.clone().with_switch(CacheSwitch::Off), PipelineCachePolicy::Off);
        assert_eq!(p.clone().with_switch(CacheSwitch::On), p);
        assert_eq!(PipelineCachePolicy::per_user_from(get(&[])), PipelineCachePolicy::Ephemeral);
    }

    #[test]
    #[should_panic(expected = "BOYKO_PIPELINE_CACHE must be `on` or `off`")]
    fn policy_env_grammar_rejects_a_typo() {
        let _ = switch_from(get(&[(ENV_SWITCH, "of")]));
    }

    #[test]
    fn dir_override_beats_localappdata() {
        let both = [(ENV_DIR, "D:/override"), ("LOCALAPPDATA", "C:/Users/u/AppData/Local")];
        assert_eq!(
            PipelineCachePolicy::per_user_from(get(&both)),
            PipelineCachePolicy::Persisted { dir: PathBuf::from("D:/override") }
        );
        let local = [("LOCALAPPDATA", "C:/Users/u/AppData/Local")];
        let want = if cfg!(windows) {
            PipelineCachePolicy::Persisted {
                dir: PathBuf::from("C:/Users/u/AppData/Local").join("boyko-engine").join("pipeline-cache"),
            }
        } else {
            PipelineCachePolicy::Ephemeral
        };
        assert_eq!(PipelineCachePolicy::per_user_from(get(&local)), want);
        let empty = [(ENV_DIR, ""), ("LOCALAPPDATA", "")];
        assert_eq!(PipelineCachePolicy::per_user_from(get(&empty)), PipelineCachePolicy::Ephemeral);
    }

    #[test]
    #[cfg_attr(miri, ignore = "miri-unsupported: filesystem under isolation")]
    fn save_is_atomic_and_leaves_no_tmp() {
        let dir = scratch_dir("atomic");
        let store = PipelineCacheStore::new(&dir, id());
        let mut scratch = Vec::new();
        // A pre-existing file is replaced.
        fs::write(store.path(), b"stale bytes").expect("invariant: scratch is writable");
        let first = save_payload(&store, &payload(1000), &mut scratch);
        assert!(matches!(first, CacheSave::Saved { .. }), "{first:?}");
        let (load, bytes) = store.load();
        assert_eq!(load, CacheLoad::Loaded { bytes: (VK_HEADER_SIZE + 1000) as u64 });
        assert_eq!(&bytes[HEADER_SIZE..], &payload(1000)[..]);
        // A stale tmp is swept; a fresh one (another process mid-save) is left alone.
        let stale = dir.join(format!("{}.999.0.tmp", file_name(&id())));
        let fresh = dir.join(format!("{}.998.0.tmp", file_name(&id())));
        fs::write(&stale, b"torn").expect("invariant: scratch is writable");
        fs::write(&fresh, b"in flight").expect("invariant: scratch is writable");
        let old = SystemTime::now() - STALE_TMP_AGE - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&stale)
            .and_then(|f| f.set_modified(old))
            .expect("invariant: scratch mtimes are settable");
        let second = save_payload(&store, &payload(2000), &mut scratch);
        assert!(matches!(second, CacheSave::Saved { .. }), "{second:?}");
        assert!(!stale.exists(), "a tmp older than {STALE_TMP_AGE:?} is swept");
        assert!(fresh.exists(), "a fresh tmp may be another process's save in flight");
        let _ = fs::remove_file(&fresh);
        let tmps: Vec<_> = fs::read_dir(&dir)
            .expect("invariant: scratch is listable")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmps.is_empty(), "the save leaves no tmp: {tmps:?}");
        // A torn tmp next to the file never replaces it: the old file stays valid.
        fs::write(dir.join(format!("{}.1.1.tmp", file_name(&id()))), &scratch[..100])
            .expect("invariant: scratch is writable");
        assert_eq!(store.load().0, CacheLoad::Loaded { bytes: (VK_HEADER_SIZE + 2000) as u64 });
        let _ = fs::remove_dir_all(&dir);
    }

    /// Ruling 91 Q3: a sentinel left by a boot that died with the blob in the driver discards the
    /// file, and the boot starts empty; the next boot is a plain cold one.
    #[test]
    #[cfg_attr(miri, ignore = "miri-unsupported: filesystem under isolation")]
    fn crash_sentinel_discards_the_file_and_starts_empty() {
        let dir = scratch_dir("sentinel");
        let store = PipelineCacheStore::new(&dir, id());
        let mut scratch = Vec::new();
        assert!(matches!(save_payload(&store, &payload(512), &mut scratch), CacheSave::Saved { .. }));
        assert!(matches!(store.load().0, CacheLoad::Loaded { .. }), "the saved file loads");
        // A boot arms the sentinel before handing the blob over, then dies.
        store.arm_sentinel();
        let (load, bytes) = store.load();
        assert_eq!(load, CacheLoad::CrashGuard, "the sentinel wins over a valid file");
        assert!(bytes.is_empty(), "nothing is handed to the driver after a crash");
        assert!(!store.path().exists(), "the file is discarded");
        assert!(!store.sentinel_path().exists(), "the sentinel is consumed");
        assert_eq!(store.load().0, CacheLoad::Absent, "the boot after is a plain cold one");
        // A boot that finishes clears its sentinel, and its file loads again.
        assert!(matches!(save_payload(&store, &payload(512), &mut scratch), CacheSave::Saved { .. }));
        store.arm_sentinel();
        store.clear_sentinel();
        assert!(matches!(store.load().0, CacheLoad::Loaded { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Critique W3: a validated blob the driver refuses falls back to an empty cache that stays
    /// persisted (so the boot-end save rewrites the file); only a refused EMPTY create yields none.
    #[test]
    fn driver_refusal_retries_empty_and_stays_persisted() {
        let loaded = CacheLoad::Loaded { bytes: 100 };
        let data = [0u8; 100];
        let mut calls = Vec::new();
        let (h, load) = create_with_fallback(loaded, Some(&data), |d| {
            calls.push(d.is_some());
            if d.is_some() { None } else { Some(7u64) }
        });
        assert_eq!((h, load), (Some(7), CacheLoad::DriverRefused));
        assert_eq!(calls, [true, false], "the data create, then one empty retry");
        let (h, load) = create_with_fallback(loaded, Some(&data), |_| None::<u64>);
        assert_eq!((h, load), (None, CacheLoad::CreateFailed));
        let (h, load) = create_with_fallback(loaded, Some(&data), |_| Some(1u64));
        assert_eq!((h, load), (Some(1), loaded), "an accepted blob keeps its outcome");
        let (h, load) = create_with_fallback(CacheLoad::Absent, None, |d| d.is_none().then_some(2u64));
        assert_eq!((h, load), (Some(2), CacheLoad::Absent));
    }

    #[test]
    fn stats_keep_the_top_creations_sorted() {
        let site = Location::caller();
        let mut s = PipelineCreationStats::default();
        for (i, ns) in [5u64, 50, 1, 40, 30, 20, 10, 60].into_iter().enumerate() {
            let kind = if i % 2 == 0 { PipelineKind::Compute } else { PipelineKind::Graphics };
            s.record(kind, FEEDBACK_VALID, ns, site);
        }
        s.record(PipelineKind::Compute, FEEDBACK_VALID | FEEDBACK_APP_CACHE_HIT, 2, site);
        s.record(PipelineKind::Graphics, 0, 99, site);
        assert_eq!((s.compute, s.graphics, s.total()), (5, 5, 10));
        assert_eq!((s.hits, s.feedback_invalid, s.misses_since_save), (1, 1, 9));
        assert_eq!(s.driver_ns, 5 + 50 + 1 + 40 + 30 + 20 + 10 + 60 + 2);
        let top: Vec<u64> = s.top.iter().map(|e| e.ns).collect();
        assert_eq!(top, [60, 50, 40, 30, 20]);
    }
}
