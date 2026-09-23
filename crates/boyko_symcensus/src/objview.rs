//! One post-LTO object, read once: nm's table (raw ↔ demangled), readobj's sections and symbols,
//! and the derived symbol sizes. Bodies and the leg-(7)(b) multiset are computed from it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::host::Host;
use crate::llvm::{self, CoffTable, Disasm, SymTab};
use crate::normalize::{self, RenameList};
use crate::red::{Red, RedKind, Result};
use crate::sha256;
use crate::tools::{self, Tool};

/// The LLVM binutils, resolved once per run.
#[derive(Clone, Debug)]
pub struct Llvm {
    /// `llvm-nm`.
    pub nm: Tool,
    /// `llvm-objdump`.
    pub objdump: Tool,
    /// `llvm-readobj`.
    pub readobj: Tool,
    /// `llvm-size`.
    pub size: Tool,
    /// `llvm-ar`.
    pub ar: Tool,
}

impl Llvm {
    /// Resolves all five; the first absence is the RED.
    pub fn resolve(host: &Host) -> Result<Self> {
        Ok(Self {
            nm: tools::llvm("llvm-nm", host)?,
            objdump: tools::llvm("llvm-objdump", host)?,
            readobj: tools::llvm("llvm-readobj", host)?,
            size: tools::llvm("llvm-size", host)?,
            ar: tools::llvm("llvm-ar", host)?,
        })
    }

    /// One receipt line per tool.
    #[must_use]
    pub fn receipt(&self) -> String {
        [&self.nm, &self.objdump, &self.readobj, &self.size, &self.ar].iter().map(|t| t.receipt_line() + "\n").collect()
    }
}

/// A loaded object.
#[derive(Clone, Debug)]
pub struct ObjView {
    /// The object file.
    pub path: PathBuf,
    /// nm's table.
    pub syms: SymTab,
    /// readobj's sections and symbols.
    pub coff: CoffTable,
    /// raw → demangled.
    pub map: BTreeMap<String, String>,
    sizes: BTreeMap<String, Vec<u64>>,
}

/// A candidate body: the section that holds the function, every symbol in it, their disassembly
/// and the normalised text.
#[derive(Clone, Debug)]
pub struct Body {
    /// The function's raw name.
    pub raw: String,
    /// Its normalised name.
    pub name: String,
    /// Its section number.
    pub section: u32,
    /// Its section name.
    pub section_name: String,
    /// The section's COMDAT selection (`NoDuplicates`, `Any`, … or empty).
    pub selection: String,
    /// The section's `RawDataSize`.
    pub section_len: u64,
    /// The aux record's `Length` for the section, if present.
    pub aux_len: Option<u64>,
    /// `(raw name, value)` of every symbol in the section, by value.
    pub members: Vec<(String, u64)>,
    /// The disassembly of each member.
    pub parts: Vec<Disasm>,
    /// The normalised body.
    pub text: String,
    /// sha256 of [`Body::text`].
    pub sha256: String,
}

impl ObjView {
    /// Reads `obj` with nm (four runs) and readobj; RED on an empty census from either.
    pub fn load(tools: &Llvm, obj: &Path) -> Result<Self> {
        let syms = llvm::nm(&tools.nm, obj)?;
        let coff = llvm::readobj(&tools.readobj, obj)?;
        let map = syms.name_map();
        let sizes = coff.sizes();
        Ok(Self { path: obj.to_path_buf(), syms, coff, map, sizes })
    }

    /// The section a raw-named (non-section) symbol is defined in.
    #[must_use]
    pub fn section_of(&self, raw: &str) -> Option<u32> {
        self.coff.symbols.iter().find(|s| s.name == raw && s.aux.is_none()).and_then(|s| s.section)
    }

    /// The whole-section body of the function `raw`.
    pub fn body(&self, objdump: &Tool, raw: &str, rename: &RenameList) -> Result<Body> {
        let n = self
            .section_of(raw)
            .ok_or_else(|| Red::new(RedKind::SymbolAbsent, format!("`{raw}` has no section in {}", self.path.display())))?;
        let sec = self.coff.section(n).ok_or_else(|| Red::new(RedKind::Malformed, format!("section {n} missing from readobj output")))?;
        let aux = self.coff.symbols.iter().find(|s| s.section == Some(n) && s.aux.is_some()).and_then(|s| s.aux.clone());
        let members: Vec<(String, u64)> = self.coff.symbols_in(n).iter().map(|s| (s.name.clone(), s.value)).collect();
        let names: Vec<&str> = members.iter().map(|(m, _)| m.as_str()).collect();
        let parts = llvm::disasm(objdump, &self.path, &names)?;
        let text = normalize::norm_body(&parts, &self.map, rename);
        let sha256 = sha256::bytes_hex(text.as_bytes());
        Ok(Body {
            raw: raw.to_owned(),
            name: normalize::norm_target(&self.map, raw, rename),
            section: n,
            section_name: sec.name.clone(),
            selection: aux.as_ref().map(|a| a.selection.clone()).unwrap_or_default(),
            section_len: sec.raw_size,
            aux_len: aux.map(|a| a.length),
            members,
            parts,
            text,
            sha256,
        })
    }

    /// Leg (7)(b): the defined-symbol multiset `(normalised name, class, size) → count`.
    ///
    /// Every binding is included (a fat-LTO survivor is local); anonymous data enters as
    /// (`<anon-data>`, class, size). Sizes are the distance to the next symbol in the section or to
    /// its end. RED if a defined, non-absolute nm symbol has no readobj record to size it.
    pub fn multiset(&self, rename: &RenameList) -> Result<BTreeMap<(String, char, u64), usize>> {
        let mut cursor: BTreeMap<&str, usize> = BTreeMap::new();
        let mut out: BTreeMap<(String, char, u64), usize> = BTreeMap::new();
        for s in &self.syms.defined {
            let size = if s.class == 'a' {
                0
            } else {
                let list = self.sizes.get(&s.raw).ok_or_else(|| {
                    Red::new(RedKind::Malformed, format!("nm defines `{}` but readobj has no sized record for it", s.raw))
                })?;
                let i = cursor.entry(s.raw.as_str()).or_insert(0);
                let v = list.get(*i).or_else(|| list.last()).copied().unwrap_or(0);
                *i += 1;
                v
            };
            *out.entry((normalize::norm_target(&self.map, &s.raw, rename), s.class, size)).or_insert(0) += 1;
        }
        Ok(out)
    }
}
