//! Just enough PE parsing to say where a differing byte of an image lives (probe (iv)).

/// One PE section header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeSection {
    /// The 8-byte name, NUL-trimmed.
    pub name: String,
    /// File offset of the raw data.
    pub raw_ptr: u32,
    /// Size of the raw data.
    pub raw_size: u32,
}

/// The parsed headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeHeaders {
    /// File offset of the `PE\0\0` signature (`e_lfanew`).
    pub pe_offset: usize,
    /// File offset of the COFF header's `TimeDateStamp`.
    pub timestamp_offset: usize,
    /// Section headers.
    pub sections: Vec<PeSection>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(o)?, *b.get(o + 1)?]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(o)?, *b.get(o + 1)?, *b.get(o + 2)?, *b.get(o + 3)?]))
}

/// Parses the DOS stub pointer, the COFF header and the section table; `None` if not a PE.
#[must_use]
pub fn headers(b: &[u8]) -> Option<PeHeaders> {
    if b.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(b, 0x3c)? as usize;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let nsec = usize::from(u16_at(b, coff + 2)?);
    let opt = usize::from(u16_at(b, coff + 16)?);
    let table = coff + 20 + opt;
    let mut sections = Vec::with_capacity(nsec);
    for i in 0..nsec {
        let s = table + i * 40;
        let raw_name = b.get(s..s + 8)?;
        let name = String::from_utf8_lossy(raw_name).trim_end_matches('\0').to_owned();
        sections.push(PeSection { name, raw_size: u32_at(b, s + 16)?, raw_ptr: u32_at(b, s + 20)? });
    }
    Some(PeHeaders { pe_offset: pe, timestamp_offset: coff + 4, sections })
}

/// Where file offset `o` lies: `headers`, a section name, or `overlay`.
#[must_use]
pub fn locate(h: &PeHeaders, o: usize) -> String {
    if o >= h.timestamp_offset && o < h.timestamp_offset + 4 {
        return "COFF header TimeDateStamp".to_owned();
    }
    for s in &h.sections {
        let (a, n) = (s.raw_ptr as usize, s.raw_size as usize);
        if o >= a && o < a + n {
            return format!("{} +0x{:x}", s.name, o - a);
        }
    }
    if h.sections.iter().all(|s| o < s.raw_ptr as usize) { "headers".to_owned() } else { "overlay".to_owned() }
}

/// Differing byte ranges of two equal-length buffers, merged when closer than 16 bytes; at most
/// `cap` ranges. `None` when the lengths differ.
#[must_use]
pub fn diff_ranges(a: &[u8], b: &[u8], cap: usize) -> Option<Vec<(usize, usize)>> {
    if a.len() != b.len() {
        return None;
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        if x == y {
            continue;
        }
        match out.last_mut() {
            Some((_, end)) if i <= *end + 16 => *end = i + 1,
            _ => {
                if out.len() == cap {
                    break;
                }
                out.push((i, i + 1));
            }
        }
    }
    Some(out)
}
