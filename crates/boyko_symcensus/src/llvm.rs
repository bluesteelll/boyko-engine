//! Thin wrappers over the LLVM binutils. Each one checks status, then stderr, then emptiness —
//! in that order and BEFORE any filtering — because "the tool read nothing" and "the tool read
//! everything and found no match" are different answers that the old instrument once conflated.

use std::collections::BTreeMap;
use std::path::Path;

use crate::red::{Red, RedKind, Result};
use crate::tools::Tool;

/// One `llvm-nm` symbol line, raw and demangled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sym {
    /// The name as it is in the symbol table (v0 `_R…`, SEH `$…$`, anonymous `anon.…`).
    pub raw: String,
    /// `llvm-nm --demangle`'s spelling (equal to `raw` when there is nothing to demangle).
    pub name: String,
    /// The nm class character (`t`, `T`, `r`, `R`, `d`, `b`, `a`, `U`, …).
    pub class: char,
    /// The value column; `None` for an undefined symbol.
    pub value: Option<u64>,
}

/// An object's symbol table as `llvm-nm` prints it, defined and undefined.
#[derive(Clone, Debug, Default)]
pub struct SymTab {
    /// `--defined-only`, in symbol-table order.
    pub defined: Vec<Sym>,
    /// `--undefined-only`, in symbol-table order.
    pub undefined: Vec<Sym>,
}

impl SymTab {
    /// raw → demangled for every symbol, defined or not; relocation targets are raw names.
    #[must_use]
    pub fn name_map(&self) -> BTreeMap<String, String> {
        self.defined.iter().chain(&self.undefined).map(|s| (s.raw.clone(), s.name.clone())).collect()
    }
}

/// Runs `llvm-nm` four times over `obj` (defined/undefined × raw/demangled, all `--no-sort`, so the
/// raw and demangled runs pair line by line) and returns the table.
///
/// RED [`RedKind::EmptyCensus`] if the defined run says `no symbols` or prints no symbol line: on a
/// `link.exe` image `llvm-nm` exits 0 with an empty stdout, and a zero read there is not a count.
pub fn nm(tool: &Tool, obj: &Path) -> Result<SymTab> {
    let defined = nm_pair(tool, obj, "--defined-only", true)?;
    let undefined = nm_pair(tool, obj, "--undefined-only", false)?;
    Ok(SymTab { defined, undefined })
}

fn nm_pair(tool: &Tool, obj: &Path, which: &str, must_be_nonempty: bool) -> Result<Vec<Sym>> {
    let raw = nm_run(tool, obj, &[which, "--no-sort"], must_be_nonempty)?;
    let dem = nm_run(tool, obj, &[which, "--no-sort", "--demangle"], must_be_nonempty)?;
    if raw.len() != dem.len() {
        return Err(Red::new(
            RedKind::Malformed,
            format!("llvm-nm {which} printed {} raw and {} demangled lines for {}", raw.len(), dem.len(), obj.display()),
        ));
    }
    let mut out = Vec::with_capacity(raw.len());
    for (r, d) in raw.into_iter().zip(dem) {
        if r.1 != d.1 || r.0 != d.0 {
            return Err(Red::new(
                RedKind::Malformed,
                format!("llvm-nm raw/demangled runs disagree on order in {}: {} vs {}", obj.display(), r.2, d.2),
            ));
        }
        out.push(Sym { raw: r.2, name: d.2, class: r.1, value: r.0 });
    }
    Ok(out)
}

type NmLine = (Option<u64>, char, String);

fn nm_run(tool: &Tool, obj: &Path, flags: &[&str], must_be_nonempty: bool) -> Result<Vec<NmLine>> {
    let out = tool.output(flags.iter().map(std::ffi::OsStr::new).chain([obj.as_os_str()]))?;
    if !out.status.success() {
        return Err(Red::new(
            RedKind::ToolFailed,
            format!("{} {} exited {:?} on {}: {}", tool.name, flags.join(" "), out.status.code(), obj.display(), String::from_utf8_lossy(&out.stderr).trim()),
        ));
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if must_be_nonempty && stderr.contains("no symbols") {
        return Err(Red::new(
            RedKind::EmptyCensus,
            format!(
                "{} reported `no symbols` for {}: the tool CENSUSED NOTHING, which is not the answer \
                 `no symbol matches` and must never be recorded as that number. stderr: {}",
                tool.name,
                obj.display(),
                stderr.trim()
            ),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        lines.push(parse_nm_line(line).ok_or_else(|| Red::new(RedKind::Malformed, format!("llvm-nm line not understood: {line}")))?);
    }
    if must_be_nonempty && lines.is_empty() {
        return Err(Red::new(
            RedKind::EmptyCensus,
            format!("{} printed no symbol line for {}; a census with no subject has no zero to report", tool.name, obj.display()),
        ));
    }
    Ok(lines)
}

/// `00000000 t name` (defined) or `         U name` (undefined). Names may contain spaces once
/// demangled (`<A as B>::f`), so the name is everything after the class and one space.
fn parse_nm_line(line: &str) -> Option<NmLine> {
    if line.starts_with(' ') {
        let t = line.trim_start();
        let class = t.chars().next()?;
        let name = t.get(2..)?.to_owned();
        return Some((None, class, name));
    }
    let sp = line.find(' ')?;
    let value = u64::from_str_radix(&line[..sp], 16).ok()?;
    let rest = &line[sp + 1..];
    let class = rest.chars().next()?;
    let name = rest.get(2..)?.to_owned();
    Some((Some(value), class, name))
}

/// One relocation `llvm-objdump -r` printed under an instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reloc {
    /// Offset of the fixup within the section.
    pub offset: u64,
    /// `IMAGE_REL_AMD64_REL32`, …
    pub kind: String,
    /// The raw target symbol name.
    pub target: String,
}

/// One disassembled instruction and the relocations that apply inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insn {
    /// Offset within the section.
    pub offset: u64,
    /// `mnemonic\toperands`, with objdump's `# …` comment removed.
    pub text: String,
    /// The comment objdump appended (`# 0xb <sym+0xb>`), kept for probes, never for bodies.
    pub comment: String,
    /// Relocations whose fixup lies inside this instruction.
    pub relocs: Vec<Reloc>,
}

/// The disassembly of one symbol, up to the next symbol in its section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disasm {
    /// The raw symbol name.
    pub symbol: String,
    /// The section name objdump printed.
    pub section: String,
    /// The symbol's value (offset in its section).
    pub start: u64,
    /// The instructions.
    pub insns: Vec<Insn>,
}

/// Disassembles each raw-named symbol in `raws` from `obj` (`-d -r -z --no-show-raw-insn`, Intel
/// syntax, NO `--demangle`: with it, `--disassemble-symbols` stops matching mangled names — measured
/// at B3). RED if any requested symbol is not disassembled.
pub fn disasm(tool: &Tool, obj: &Path, raws: &[&str]) -> Result<Vec<Disasm>> {
    let mut out: Vec<Disasm> = Vec::with_capacity(raws.len());
    // Windows caps a command line at 32767 UTF-16 units and v0 names run to kilobytes.
    let mut batch: Vec<&str> = Vec::new();
    let mut len = 0usize;
    for (i, r) in raws.iter().enumerate() {
        batch.push(r);
        len += r.len() + 1;
        if len > 20_000 || i + 1 == raws.len() {
            out.extend(disasm_batch(tool, obj, &batch)?);
            batch.clear();
            len = 0;
        }
    }
    for r in raws {
        if !out.iter().any(|d| d.symbol == *r) {
            return Err(Red::new(RedKind::SymbolAbsent, format!("llvm-objdump did not disassemble `{r}` from {}", obj.display())));
        }
    }
    Ok(out)
}

fn disasm_batch(tool: &Tool, obj: &Path, raws: &[&str]) -> Result<Vec<Disasm>> {
    let sel = format!("--disassemble-symbols={}", raws.join(","));
    let args: [&std::ffi::OsStr; 7] = [
        "-d".as_ref(),
        "-r".as_ref(),
        "-z".as_ref(),
        "--no-show-raw-insn".as_ref(),
        "--x86-asm-syntax=intel".as_ref(),
        sel.as_ref(),
        obj.as_os_str(),
    ];
    let out = tool.output(args)?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-objdump exited {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr).trim())));
    }
    parse_objdump(&String::from_utf8_lossy(&out.stdout))
}

/// Parses `llvm-objdump -d -r` output: `Disassembly of section S:`, `<hex> <name>:` headers,
/// `  <hex>:\t<insn>` lines, and `\t\t<hex>:  <IMAGE_REL_…>\t<target>` relocation lines, which
/// attach to the instruction whose offset precedes the fixup.
fn parse_objdump(text: &str) -> Result<Vec<Disasm>> {
    let mut out: Vec<Disasm> = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Disassembly of section ") {
            section = rest.trim_end_matches(':').to_owned();
            continue;
        }
        if line.is_empty() {
            continue;
        }
        // `0000000000000000 <name>:`
        if line.ends_with(">:")
            && !line.starts_with(char::is_whitespace)
            && let Some(lt) = line.find(" <")
        {
            let start = u64::from_str_radix(line[..lt].trim(), 16).unwrap_or(0);
            let name = line[lt + 2..line.len() - 2].to_owned();
            out.push(Disasm { symbol: name, section: section.clone(), start, insns: Vec::new() });
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let trimmed = line.trim_start();
        // Relocation line: `\t\t0000000000000007:  IMAGE_REL_AMD64_REL32\tanon.…`
        if line.starts_with("\t\t")
            && trimmed.contains("IMAGE_REL_")
            && let Some((off, rest)) = trimmed.split_once(':')
            && let Ok(offset) = u64::from_str_radix(off.trim(), 16)
        {
            let mut parts = rest.split_whitespace();
            let kind = parts.next().unwrap_or("").to_owned();
            let target = parts.collect::<Vec<_>>().join(" ");
            if let Some(insn) = cur.insns.iter_mut().rev().find(|i| i.offset <= offset) {
                insn.relocs.push(Reloc { offset, kind, target });
            }
            continue;
        }
        // Instruction line: `      4:      \tlea\trax, [rip]   # 0xb <…>`
        if let Some((off, rest)) = trimmed.split_once(':')
            && let Ok(offset) = u64::from_str_radix(off.trim(), 16)
        {
            let body = rest.trim_start_matches([' ', '\t']);
            let (text, comment) = match body.find(" # ").or_else(|| body.find("\t# ")) {
                Some(i) => (body[..i].trim_end().to_owned(), body[i..].trim().to_owned()),
                None => (body.trim_end().to_owned(), String::new()),
            };
            cur.insns.push(Insn { offset, text, comment, relocs: Vec::new() });
        }
    }
    Ok(out)
}

/// A COFF section header, from `llvm-readobj --sections`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoffSection {
    /// 1-based section number.
    pub number: u32,
    /// The resolved name (`.text`, `.text$unlikely`, `.rdata`, …).
    pub name: String,
    /// `RawDataSize`.
    pub raw_size: u64,
    /// `IMAGE_SCN_LNK_COMDAT` set.
    pub comdat: bool,
    /// `IMAGE_SCN_CNT_CODE` set.
    pub code: bool,
}

/// A COFF symbol record, from `llvm-readobj --symbols`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoffSymbol {
    /// The raw name.
    pub name: String,
    /// `Value` (offset in its section).
    pub value: u64,
    /// The section number, or `None` for undefined / absolute / debug.
    pub section: Option<u32>,
    /// `ComplexType: Function`.
    pub function: bool,
    /// `StorageClass` label (`Static`, `External`, …).
    pub storage: String,
    /// The `AuxSectionDef` record, present on a section symbol.
    pub aux: Option<AuxSection>,
}

/// A section-definition auxiliary record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuxSection {
    /// `Length`.
    pub length: u64,
    /// `Selection` label (`NoDuplicates`, `Any`, `Associative`, …), empty if not COMDAT.
    pub selection: String,
}

/// Sections and symbols of a COFF object.
#[derive(Clone, Debug, Default)]
pub struct CoffTable {
    /// Section headers, in order.
    pub sections: Vec<CoffSection>,
    /// Symbol records, in order.
    pub symbols: Vec<CoffSymbol>,
}

impl CoffTable {
    /// The section numbered `n`.
    #[must_use]
    pub fn section(&self, n: u32) -> Option<&CoffSection> {
        self.sections.get((n as usize).wrapping_sub(1)).filter(|s| s.number == n)
    }

    /// Non-section symbols defined in section `n`, sorted by value.
    #[must_use]
    pub fn symbols_in(&self, n: u32) -> Vec<&CoffSymbol> {
        let mut v: Vec<&CoffSymbol> = self.symbols.iter().filter(|s| s.section == Some(n) && s.aux.is_none()).collect();
        v.sort_by_key(|s| s.value);
        v
    }

    /// Every defined non-section symbol's size: the distance to the next symbol in its section,
    /// or to the section's end. Keyed by raw name; a name defined twice gets both sizes in order.
    #[must_use]
    pub fn sizes(&self) -> BTreeMap<String, Vec<u64>> {
        let mut by_section: BTreeMap<u32, Vec<&CoffSymbol>> = BTreeMap::new();
        for s in &self.symbols {
            if let (Some(n), None) = (s.section, &s.aux) {
                by_section.entry(n).or_default().push(s);
            }
        }
        let mut out: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for (n, mut syms) in by_section {
            syms.sort_by_key(|s| s.value);
            let end = self.section(n).map_or(0, |s| s.raw_size);
            for (i, s) in syms.iter().enumerate() {
                let next = syms[i + 1..].iter().map(|t| t.value).find(|&v| v > s.value).unwrap_or(end);
                out.entry(s.name.clone()).or_default().push(next.saturating_sub(s.value));
            }
        }
        out
    }
}

/// `llvm-readobj --sections --symbols obj`, parsed. RED if no symbol record is found.
pub fn readobj(tool: &Tool, obj: &Path) -> Result<CoffTable> {
    let out = tool.output([std::ffi::OsStr::new("--sections"), "--symbols".as_ref(), obj.as_os_str()])?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-readobj exited {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr).trim())));
    }
    let table = parse_readobj(&String::from_utf8_lossy(&out.stdout));
    if table.symbols.is_empty() || table.sections.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, format!("llvm-readobj printed no section or no symbol record for {}", obj.display())));
    }
    Ok(table)
}

fn trailing_number(v: &str) -> Option<i64> {
    let open = v.rfind('(')?;
    v[open + 1..].trim_end_matches(')').trim().parse().ok()
}

fn parse_readobj(text: &str) -> CoffTable {
    #[derive(PartialEq)]
    enum Block {
        None,
        Section,
        Symbol,
        Aux,
    }
    let mut t = CoffTable::default();
    let mut block = Block::None;
    let mut sec: Option<CoffSection> = None;
    let mut sym: Option<CoffSymbol> = None;
    for line in text.lines() {
        let l = line.trim();
        match l {
            "Section {" => {
                block = Block::Section;
                sec = Some(CoffSection { number: 0, name: String::new(), raw_size: 0, comdat: false, code: false });
                continue;
            }
            "Symbol {" => {
                block = Block::Symbol;
                sym = Some(CoffSymbol { name: String::new(), value: 0, section: None, function: false, storage: String::new(), aux: None });
                continue;
            }
            "AuxSectionDef {" => {
                block = Block::Aux;
                if let Some(s) = sym.as_mut() {
                    s.aux = Some(AuxSection { length: 0, selection: String::new() });
                }
                continue;
            }
            "}" => {
                match block {
                    Block::Aux => block = Block::Symbol,
                    Block::Symbol => {
                        if let Some(s) = sym.take() {
                            t.symbols.push(s);
                        }
                        block = Block::None;
                    }
                    Block::Section => {
                        if let Some(s) = sec.take() {
                            t.sections.push(s);
                        }
                        block = Block::None;
                    }
                    Block::None => {}
                }
                continue;
            }
            _ => {}
        }
        let Some((k, v)) = l.split_once(": ") else {
            if block == Block::Section
                && let Some(s) = sec.as_mut()
            {
                if l.starts_with("IMAGE_SCN_LNK_COMDAT") {
                    s.comdat = true;
                } else if l.starts_with("IMAGE_SCN_CNT_CODE") {
                    s.code = true;
                }
            }
            continue;
        };
        match block {
            Block::Section => {
                if let Some(s) = sec.as_mut() {
                    match k {
                        "Number" => s.number = v.parse().unwrap_or(0),
                        "Name" => s.name = v.rsplit_once(" (").map_or(v, |(n, _)| n).to_owned(),
                        "RawDataSize" => s.raw_size = v.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            Block::Symbol => {
                if let Some(s) = sym.as_mut() {
                    match k {
                        "Name" => s.name = v.to_owned(),
                        "Value" => s.value = v.parse().unwrap_or(0),
                        "Section" => s.section = trailing_number(v).and_then(|n| u32::try_from(n).ok()).filter(|&n| n > 0),
                        "ComplexType" => s.function = v.starts_with("Function"),
                        "StorageClass" => s.storage = v.split(" (").next().unwrap_or(v).to_owned(),
                        _ => {}
                    }
                }
            }
            Block::Aux => {
                if let Some(a) = sym.as_mut().and_then(|s| s.aux.as_mut()) {
                    match k {
                        "Length" => a.length = v.parse().unwrap_or(0),
                        "Selection" => a.selection = v.split(" (").next().unwrap_or(v).to_owned(),
                        _ => {}
                    }
                }
            }
            Block::None => {}
        }
    }
    t
}

/// The raw bytes of section `n` of `obj` (`llvm-readobj --hex-dump=<n>`).
pub fn section_bytes(tool: &Tool, obj: &Path, n: u32) -> Result<Vec<u8>> {
    let out = tool.output([std::ffi::OsString::from(format!("--hex-dump={n}")), obj.as_os_str().to_owned()])?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-readobj --hex-dump={n} exited {:?}", out.status.code())));
    }
    let mut bytes = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `0x00000000 696e7661 7269616e 743a2061 72636865 invariant: arche` — fixed columns: the
        // address is 10 wide, then four 8-digit groups in 35 columns, then the ASCII rendering.
        if !line.starts_with("0x") || line.len() < 12 {
            continue;
        }
        let groups = &line[11..line.len().min(46)];
        let hex: String = groups.chars().filter(char::is_ascii_hexdigit).collect();
        for pair in hex.as_bytes().chunks(2) {
            let s = std::str::from_utf8(pair).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(s, 16) {
                bytes.push(b);
            }
        }
    }
    Ok(bytes)
}

/// Relocations by section number: `(offset, kind, raw target)`.
pub type Relocations = BTreeMap<u32, Vec<(u64, String, String)>>;

/// Every relocation of `obj` by section number.
pub fn relocations(tool: &Tool, obj: &Path) -> Result<Relocations> {
    let out = tool.output([std::ffi::OsStr::new("--relocations"), obj.as_os_str()])?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-readobj --relocations exited {:?}", out.status.code())));
    }
    let mut map: Relocations = BTreeMap::new();
    let mut cur: Option<u32> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Section (") {
            cur = rest.split(')').next().and_then(|n| n.parse().ok());
            continue;
        }
        if l == "}" {
            cur = None;
            continue;
        }
        let (Some(n), Some(off)) = (cur, l.strip_prefix("0x")) else { continue };
        let mut it = off.split_whitespace();
        let (Some(o), Some(kind), Some(target)) = (it.next(), it.next(), it.next()) else { continue };
        if let Ok(o) = u64::from_str_radix(o, 16) {
            map.entry(n).or_default().push((o, kind.to_owned(), target.to_owned()));
        }
    }
    Ok(map)
}

/// `llvm-size -A image`: `(section, size)` rows plus `("Total", n)`. RED if no row.
pub fn size_a(tool: &Tool, image: &Path) -> Result<Vec<(String, u64)>> {
    let out = tool.output([std::ffi::OsStr::new("-A"), image.as_os_str()])?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-size exited {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr).trim())));
    }
    let mut rows = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(size)) = (it.next(), it.next()) else { continue };
        if let Ok(n) = size.parse::<u64>() {
            rows.push((name.to_owned(), n));
        }
    }
    if rows.is_empty() {
        return Err(Red::new(RedKind::EmptyCensus, format!("llvm-size -A printed no section row for {}", image.display())));
    }
    Ok(rows)
}

/// `llvm-readobj --coff-exports image`: one `ordinal name rva` line per export (possibly none).
pub fn coff_exports(tool: &Tool, image: &Path) -> Result<Vec<String>> {
    let out = tool.output([std::ffi::OsStr::new("--coff-exports"), image.as_os_str()])?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-readobj --coff-exports exited {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr).trim())));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if !text.contains("Format: COFF") {
        return Err(Red::new(RedKind::Malformed, format!("{} is not read as COFF by llvm-readobj", image.display())));
    }
    let mut rows = Vec::new();
    let (mut ord, mut name, mut rva) = (String::new(), String::new(), String::new());
    for line in text.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("Ordinal: ") {
            ord = v.to_owned();
        } else if let Some(v) = l.strip_prefix("Name: ") {
            name = v.to_owned();
        } else if let Some(v) = l.strip_prefix("RVA: ") {
            rva = v.to_owned();
        } else if l == "}" && !name.is_empty() {
            rows.push(format!("{ord} {name} {rva}"));
            ord.clear();
            name.clear();
            rva.clear();
        }
    }
    Ok(rows)
}

/// The format of an archive member, from its leading bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberFormat {
    /// LLVM bitcode (`BC C0 DE`, or the `0B17C0DE` wrapper).
    Bitcode,
    /// A COFF object (x86-64 machine, or big-obj).
    Coff,
    /// An ELF object.
    Elf,
    /// rustc's `lib.rmeta` (skipped by the censuses).
    Metadata,
}

/// One rlib member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The member name.
    pub name: String,
    /// Its format.
    pub format: MemberFormat,
    /// Its size in bytes.
    pub size: usize,
}

/// Lists `rlib`'s members (`llvm-ar t`) and classifies each by content (`llvm-ar p`).
///
/// RED [`RedKind::MemberFormat`] on a member that is neither metadata, COFF, ELF nor bitcode, and
/// [`RedKind::MemberCount`] if no object member remains once metadata is skipped.
pub fn rlib_members(tool: &Tool, rlib: &Path) -> Result<Vec<Member>> {
    let out = tool.output([std::ffi::OsStr::new("t"), rlib.as_os_str()])?;
    if !out.status.success() {
        return Err(Red::new(RedKind::ToolFailed, format!("llvm-ar t exited {:?} on {}", out.status.code(), rlib.display())));
    }
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
    let mut members = Vec::with_capacity(names.len());
    for name in names {
        let body = tool.output([std::ffi::OsStr::new("p"), rlib.as_os_str(), name.as_ref()])?;
        if !body.status.success() {
            return Err(Red::new(RedKind::ToolFailed, format!("llvm-ar p {} {name} failed", rlib.display())));
        }
        let b = &body.stdout;
        let format = if name == "lib.rmeta" {
            MemberFormat::Metadata
        } else if b.starts_with(b"BC\xC0\xDE") || b.starts_with(&[0xDE, 0xC0, 0x17, 0x0B]) {
            MemberFormat::Bitcode
        } else if b.starts_with(&[0x64, 0x86]) || b.starts_with(&[0x00, 0x00, 0xFF, 0xFF]) {
            MemberFormat::Coff
        } else if b.starts_with(b"\x7FELF") {
            MemberFormat::Elf
        } else {
            let head: Vec<String> = b.iter().take(8).map(|x| format!("{x:02x}")).collect();
            return Err(Red::new(
                RedKind::MemberFormat,
                format!("member `{name}` of {} is neither metadata, COFF, ELF nor bitcode (head {})", rlib.display(), head.join(" ")),
            ));
        };
        members.push(Member { name, format, size: b.len() });
    }
    if !members.iter().any(|m| m.format != MemberFormat::Metadata) {
        return Err(Red::new(RedKind::MemberCount, format!("{} has zero object members once `lib.rmeta` is skipped", rlib.display())));
    }
    Ok(members)
}

/// One `llvm-symbolizer` frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The function name the tool printed.
    pub function: String,
    /// The source file.
    pub file: String,
    /// The line.
    pub line: u64,
}

/// Symbolizes each RVA in `rvas` against `image` (`--relative-address`, JSON output, inlined frames
/// included — the tool's default). Returns one frame list per address, innermost first.
///
/// There is no `--pdb` argument: the MSVC Build Tools copy (LLVM 21.0.0git) does not accept one
/// (measured at B3: `unknown argument '--pdb=…'`), and finds the PDB through the image's CodeView
/// record, which rustc writes as the bare file name (`/PDBALTPATH:%_PDB%`), beside the image.
pub fn symbolize(tool: &Tool, image: &Path, rvas: &[u64]) -> Result<Vec<Vec<Frame>>> {
    let args: Vec<std::ffi::OsString> = vec![
        "--output-style=JSON".into(),
        "--relative-address".into(),
        "--inlining".into(),
        format!("--obj={}", image.display()).into(),
    ];
    let mut all = Vec::with_capacity(rvas.len());
    for chunk in rvas.chunks(256) {
        let mut a = args.clone();
        a.extend(chunk.iter().map(|r| std::ffi::OsString::from(format!("0x{r:x}"))));
        let out = tool.output(&a)?;
        if !out.status.success() {
            return Err(Red::new(RedKind::ToolFailed, format!("llvm-symbolizer exited {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr).trim())));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines().filter(|l| l.trim_start().starts_with('{') || l.trim_start().starts_with('[')) {
            let v = crate::json::parse(line.trim())?;
            let entries: Vec<&crate::json::Json> = match &v {
                crate::json::Json::Array(items) => items.iter().collect(),
                other => vec![other],
            };
            for e in entries {
                let frames = e
                    .get("Symbol")
                    .map(|s| {
                        s.items()
                            .iter()
                            .map(|f| Frame {
                                function: f.get("FunctionName").and_then(crate::json::Json::as_str).unwrap_or("").to_owned(),
                                file: f.get("FileName").and_then(crate::json::Json::as_str).unwrap_or("").to_owned(),
                                line: f.get("Line").and_then(crate::json::Json::as_f64).map_or(0, |n| n as u64),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                all.push(frames);
            }
        }
    }
    if all.len() != rvas.len() {
        return Err(Red::new(RedKind::Malformed, format!("llvm-symbolizer answered {} of {} addresses", all.len(), rvas.len())));
    }
    Ok(all)
}
