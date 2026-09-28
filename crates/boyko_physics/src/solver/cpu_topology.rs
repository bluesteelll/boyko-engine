//! S7 (the scaling plan's partial W16 lever, `docs/physics/perf-campaign/levers/scaling/01-DESIGN.md`
//! §6.7 and §13): the machine's physical-core count, read once when the colored solver is built.
//!
//! The colour waves take their participant cap from it: past the physical cores a wave's extra
//! threads are SMT siblings, which window 8 measured slower, not faster (W16 9–11 % above W8 on
//! J-T). The read belongs to the solver resource that uses it (Principle 0: a fact held by an ECS
//! resource, never a process global), and it runs once per resource, never per step.
//!
//! # Where the count comes from
//!
//! * **Windows:** `GetLogicalProcessorInformationEx(RelationProcessorCore)` into a stack buffer,
//!   one record per physical core in every processor group, in the engine's hand-declared FFI
//!   style (`boyko_memory/src/vm.rs`). The call is the one `unsafe` block; the parse is safe code
//!   over the filled bytes.
//! * **Linux:** the online CPUs (`/sys/devices/system/cpu/online`) and each one's
//!   `topology/core_id` and `topology/physical_package_id`; the count is the number of distinct
//!   `(package, core)` pairs. The files are read with the C library's `open` / `read` / `close`
//!   (std links it on this target) into stack buffers, in the same FFI style, so neither arm
//!   touches the heap.
//! * **Anything else, or a failed read:** the logical count, `std::thread::available_parallelism`
//!   ([`TopologySource::Fallback`]).
//! * **Under Miri:** no OS call runs ([`TopologySource::Miri`]); the solver resolves no cap, so a
//!   pool-backed Miri test takes the parent's path, and the capped arm is driven by its own test.

/// Where a solver's physical-core count came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologySource {
    /// The OS's own topology report.
    Os,
    /// The OS read failed or this target has none: the physical count is the logical count.
    Fallback,
    /// A Miri build: no OS call ran, and the colour waves resolve no cap from it.
    Miri,
}

/// The processor topology one colored solver was built on (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CpuTopology {
    /// Physical cores, at least 1.
    pub(crate) physical: u32,
    /// Logical processors this process may run on (`available_parallelism`), at least 1.
    pub(crate) logical: u32,
    /// Where `physical` came from.
    pub(crate) source: TopologySource,
}

impl CpuTopology {
    /// Reads the topology (module docs). Out of line and cold: it runs once, when a solver
    /// resource is built.
    #[cold]
    #[inline(never)]
    pub(crate) fn read() -> Self {
        #[cfg(miri)]
        {
            Self { physical: 1, logical: 1, source: TopologySource::Miri }
        }
        #[cfg(not(miri))]
        {
            let logical =
                std::thread::available_parallelism().map_or(1, |n| u32::try_from(n.get()).unwrap_or(u32::MAX));
            match os_physical_cores() {
                Some(physical) => Self { physical, logical, source: TopologySource::Os },
                None => Self { physical: logical, logical, source: TopologySource::Fallback },
            }
        }
    }
}

/// The OS's physical-core count, `None` when the read fails or this target has no reader.
#[cfg(all(windows, not(miri)))]
fn os_physical_cores() -> Option<u32> {
    win::physical_cores()
}

/// The OS's physical-core count, `None` when the read fails or this target has no reader.
#[cfg(all(target_os = "linux", not(miri)))]
fn os_physical_cores() -> Option<u32> {
    linux::physical_cores()
}

/// The OS's physical-core count: this target has no reader.
#[cfg(not(any(miri, windows, target_os = "linux")))]
fn os_physical_cores() -> Option<u32> {
    None
}

/// `RelationProcessorCore`, the `LOGICAL_PROCESSOR_RELATIONSHIP` of one physical core's record.
#[cfg(any(test, windows))]
const RELATION_PROCESSOR_CORE: u32 = 0;

/// Counts the physical-core records in a filled `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX` buffer:
/// each record starts with its relationship (`u32`) and its own size in bytes (`u32`), and the
/// records are packed back to back. `None` for a record smaller than its header, one that runs
/// past the filled bytes, or no core at all.
#[cfg(any(test, windows))]
fn count_cores(bytes: &[u8]) -> Option<u32> {
    let (mut at, mut cores) = (0usize, 0u32);
    while at < bytes.len() {
        let head = bytes.get(at..at + 8)?;
        let relationship = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if size < 8 || size > bytes.len() - at {
            return None;
        }
        cores += u32::from(relationship == RELATION_PROCESSOR_CORE);
        at += size;
    }
    (cores > 0).then_some(cores)
}

/// Windows kernel32's topology report (module docs).
#[cfg(all(windows, not(miri)))]
mod win {
    /// Bytes of the stack buffer the report is written into: 341 one-group core records of 48 B.
    /// A machine with more cores than fit fails the call and falls back to the logical count.
    const INFO_BYTES: usize = 16 * 1024;

    /// The buffer, 8-aligned as the records' pointer-sized group masks require.
    #[repr(C, align(8))]
    struct InfoBuffer([u8; INFO_BYTES]);

    // SAFETY: the signature matches the Win64 kernel32 ABI exactly
    //   (LOGICAL_PROCESSOR_RELATIONSHIP, a C enum -> i32; PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX
    //   -> *mut u8, the records being read as bytes; PDWORD -> *mut u32; BOOL -> i32). kernel32 is
    //   linked by std.
    unsafe extern "system" {
        fn GetLogicalProcessorInformationEx(relationship: i32, buffer: *mut u8, returned_length: *mut u32) -> i32;
    }

    /// The physical cores of every processor group; `None` when the call fails (the buffer is too
    /// small, or the OS refuses) or the report holds no core record.
    pub(super) fn physical_cores() -> Option<u32> {
        let mut buf = InfoBuffer([0; INFO_BYTES]);
        let mut len = INFO_BYTES as u32;
        // SAFETY: `buf` is a live, writable, 8-aligned buffer of exactly `len` bytes, and `len` is
        //   a live `u32` holding that size, which the call reads and overwrites with the bytes it
        //   wrote. Neither pointer outlives the call, and no reference to `buf` exists during it.
        let ok = unsafe {
            GetLogicalProcessorInformationEx(super::RELATION_PROCESSOR_CORE as i32, buf.0.as_mut_ptr(), &mut len)
        };
        if ok == 0 {
            return None;
        }
        super::count_cores(buf.0.get(..len as usize)?)
    }
}

/// Parses `/sys/devices/system/cpu/online` — comma-separated CPU indices and inclusive ranges,
/// `"0-7,16-23\n"` — calling `each` on every listed CPU in order and returning how many there
/// were. `None` for a malformed list, an empty one, or when `each` returns `None`.
#[cfg(any(test, target_os = "linux"))]
fn parse_online(bytes: &[u8], mut each: impl FnMut(u32) -> Option<()>) -> Option<u32> {
    let text = bytes.trim_ascii();
    if text.is_empty() {
        return None;
    }
    let mut n = 0u32;
    for part in text.split(|&b| b == b',') {
        let (lo, hi) = match part.iter().position(|&b| b == b'-') {
            Some(dash) => (parse_u32(&part[..dash])?, parse_u32(&part[dash + 1..])?),
            None => {
                let cpu = parse_u32(part)?;
                (cpu, cpu)
            }
        };
        if hi < lo {
            return None;
        }
        for cpu in lo..=hi {
            each(cpu)?;
            n += 1;
        }
    }
    Some(n)
}

/// A decimal `u32`, ASCII digits only.
#[cfg(any(test, target_os = "linux"))]
fn parse_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0u32, |v, &b| {
        let d = b.checked_sub(b'0').filter(|&d| d <= 9)?;
        v.checked_mul(10)?.checked_add(u32::from(d))
    })
}

/// A sysfs id: a decimal `u32` or `-1` (a package id the kernel could not determine), each its
/// own value.
#[cfg(any(test, target_os = "linux"))]
fn parse_id(bytes: &[u8]) -> Option<u32> {
    let text = bytes.trim_ascii();
    if text == b"-1" { Some(u32::MAX) } else { parse_u32(text) }
}

/// The number of distinct `(package, core)` pairs: two packages may reuse one `core_id`, and the
/// SMT siblings of one core share both ids.
#[cfg(any(test, target_os = "linux"))]
fn count_distinct(pairs: &mut [(u32, u32)]) -> u32 {
    pairs.sort_unstable();
    let mut n = 0u32;
    for (i, p) in pairs.iter().enumerate() {
        n += u32::from(i == 0 || pairs[i - 1] != *p);
    }
    n
}

/// Linux's sysfs topology (module docs).
#[cfg(all(target_os = "linux", not(miri)))]
mod linux {
    use core::ffi::{c_char, c_int};

    /// The most online CPUs the fixed pair buffer holds; a larger machine falls back to the
    /// logical count.
    const MAX_CPUS: usize = 1024;
    /// `O_RDONLY`, 0 on every Linux architecture.
    const O_RDONLY: c_int = 0;

    // SAFETY: the POSIX signatures of the C library std links on Linux: `open` is variadic in the
    //   C headers and declared so here; `ssize_t` -> isize and `size_t` -> usize on every Linux
    //   target Rust supports; `int` -> c_int.
    unsafe extern "C" {
        fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn read(fd: c_int, buf: *mut u8, count: usize) -> isize;
        fn close(fd: c_int) -> c_int;
    }

    /// Reads the whole file at `path` (NUL-terminated) into `buf`; the filled length, or `None`
    /// when it cannot be opened or read, or does not fit.
    fn read_file(path: &[u8], buf: &mut [u8]) -> Option<usize> {
        debug_assert_eq!(path.last(), Some(&0), "invariant: a NUL-terminated path");
        // SAFETY: `path` is a NUL-terminated byte string, live for the call; `open` reads it up to
        //   the NUL and keeps no pointer to it.
        let fd = unsafe { open(path.as_ptr().cast::<c_char>(), O_RDONLY) };
        if fd < 0 {
            return None;
        }
        let mut len = 0usize;
        let complete = loop {
            let rest = &mut buf[len..];
            if rest.is_empty() {
                break false;
            }
            // SAFETY: `fd` is the descriptor opened above and not yet closed; `rest` is a live,
            //   writable slice of exactly `rest.len()` bytes, and no other reference to it exists
            //   during the call.
            let n = unsafe { read(fd, rest.as_mut_ptr(), rest.len()) };
            match n {
                0 => break true,
                n if n > 0 => len += n as usize,
                _ => break false,
            }
        };
        // SAFETY: `fd` was opened above and is closed exactly once, here.
        unsafe { close(fd) };
        complete.then_some(len)
    }

    /// `/sys/devices/system/cpu/cpu<cpu>/topology/<leaf>\0` into `out`; the path's bytes.
    fn cpu_path<'a>(cpu: u32, leaf: &[u8], out: &'a mut [u8; 96]) -> &'a [u8] {
        const HEAD: &[u8] = b"/sys/devices/system/cpu/cpu";
        const MID: &[u8] = b"/topology/";
        let mut digits = [0u8; 10];
        let mut i = digits.len();
        let mut v = cpu;
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let mut n = 0;
        for part in [HEAD, &digits[i..], MID, leaf, &b"\0"[..]] {
            out[n..n + part.len()].copy_from_slice(part);
            n += part.len();
        }
        &out[..n]
    }

    /// One sysfs id of `cpu`.
    fn cpu_id(cpu: u32, leaf: &[u8]) -> Option<u32> {
        let mut path = [0u8; 96];
        let mut buf = [0u8; 32];
        let len = read_file(cpu_path(cpu, leaf, &mut path), &mut buf)?;
        super::parse_id(&buf[..len])
    }

    /// The distinct `(package, core)` pairs of the online CPUs.
    pub(super) fn physical_cores() -> Option<u32> {
        let mut online = [0u8; 4096];
        let len = read_file(b"/sys/devices/system/cpu/online\0", &mut online)?;
        let mut pairs = [(0u32, 0u32); MAX_CPUS];
        let mut filled = 0usize;
        super::parse_online(&online[..len], |cpu| {
            let slot = pairs.get_mut(filled)?;
            *slot = (cpu_id(cpu, b"physical_package_id")?, cpu_id(cpu, b"core_id")?);
            filled += 1;
            Some(())
        })?;
        let n = super::count_distinct(&mut pairs[..filled]);
        (n > 0).then_some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One synthetic `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX` record: its relationship, its size,
    /// and zero bytes to that size.
    fn record(relationship: u32, size: u32, out: &mut [u8; 1024], at: &mut usize) {
        out[*at..*at + 4].copy_from_slice(&relationship.to_le_bytes());
        out[*at + 4..*at + 8].copy_from_slice(&size.to_le_bytes());
        *at += size as usize;
    }

    /// G2-8 (M2-5's target): eight 48-byte core records count as eight cores, and the core count is
    /// the core records' alone when cache (2), package (3) and group (4) records sit between them.
    #[test]
    fn count_cores_counts_only_core_records() {
        let (mut buf, mut at) = ([0u8; 1024], 0);
        for _ in 0..8 {
            record(RELATION_PROCESSOR_CORE, 48, &mut buf, &mut at);
        }
        assert_eq!(count_cores(&buf[..at]), Some(8));

        let (mut buf, mut at) = ([0u8; 1024], 0);
        for rel in [0, 2, 0, 3, 2, 0, 4, 0] {
            record(rel, if rel == 4 { 80 } else { 48 }, &mut buf, &mut at);
        }
        assert_eq!(count_cores(&buf[..at]), Some(4), "cache, package and group records are not cores");
    }

    /// A record smaller than its header, one past the filled bytes, and a report with no core are
    /// refused.
    #[test]
    fn count_cores_refuses_malformed_reports() {
        let (mut buf, mut at) = ([0u8; 1024], 0);
        record(RELATION_PROCESSOR_CORE, 48, &mut buf, &mut at);
        let mut zero = at;
        record(RELATION_PROCESSOR_CORE, 0, &mut buf, &mut zero);
        assert_eq!(count_cores(&buf[..at + 8]), None, "a record of size 0 would never advance");
        assert_eq!(count_cores(&buf[..40]), None, "a record past the filled bytes");
        assert_eq!(count_cores(&buf[..5]), None, "a truncated header");
        let (mut buf, mut at) = ([0u8; 1024], 0);
        record(2, 48, &mut buf, &mut at);
        assert_eq!(count_cores(&buf[..at]), None, "no core record at all");
        assert_eq!(count_cores(&[]), None, "an empty report");
    }

    /// The online list's shapes: ranges and single CPUs, a trailing newline, malformed lists.
    #[test]
    fn parse_online_counts_ranges_and_single_cpus() {
        let mut seen = Vec::new();
        assert_eq!(
            parse_online(b"0-7,16-23\n", |c| {
                seen.push(c);
                Some(())
            }),
            Some(16)
        );
        assert_eq!(seen, (0..8).chain(16..24).collect::<Vec<_>>());
        assert_eq!(parse_online(b"0", |_| Some(())), Some(1));
        assert_eq!(parse_online(b"0,2-3,5\n", |_| Some(())), Some(4));
        for bad in [&b""[..], b"\n", b"x", b"3-1", b"0-", b"-2", b"0,,1", b"1.5"] {
            assert_eq!(parse_online(bad, |_| Some(())), None, "{:?}", String::from_utf8_lossy(bad));
        }
        assert_eq!(parse_online(b"0-3", |c| (c < 2).then_some(())), None, "a failed CPU read fails the list");
    }

    /// G2-8 (M2-6's target): SMT siblings share a pair; two packages reusing one `core_id` are two
    /// cores; `-1` is an id of its own.
    #[test]
    fn count_distinct_keys_on_package_and_core() {
        let mut siblings = [(0, 0), (0, 1), (0, 0), (0, 1)];
        assert_eq!(count_distinct(&mut siblings), 2);
        let mut two_packages = [(0, 0), (0, 1), (1, 0), (1, 1), (1, 0)];
        assert_eq!(count_distinct(&mut two_packages), 4, "equal core ids in two packages are two cores");
        assert_eq!(count_distinct(&mut []), 0);
        assert_eq!(parse_id(b"-1\n"), Some(u32::MAX));
        assert_eq!(parse_id(b"12\n"), Some(12));
        assert_eq!(parse_id(b"x"), None);
    }

    /// The read itself: a positive count, and under Miri no OS call.
    #[test]
    fn the_read_reports_a_positive_count() {
        let t = CpuTopology::read();
        assert!(t.physical >= 1 && t.logical >= 1, "{t:?}");
        if cfg!(miri) {
            assert_eq!(t.source, TopologySource::Miri);
        }
    }
}
