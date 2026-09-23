//! Normalisation (03 §6 leg (2) "Normalisation", plus PC-2): names, anonymous data and bodies.
//!
//! **Names.** The gate host mangles with v0 (the stable default since Rust 1.97, PC-2), and
//! `llvm-nm --demangle` already prints v0 paths without crate disambiguators (measured at B3).
//! [`norm_name`] still strips an `ident[<hex>]::` disambiguator, a legacy `::h<16 hex>` hash and a
//! `.llvm.<N>` suffix, so a demangler that prints them cannot turn a rebuild into a move.
//!
//! **Anonymous data.** A data reference with no stable name prints as `<anon-data>`. Panic
//! `Location`s embed a path and a line, so without the placeholder a line shift above a release
//! `assert!` would change a pinned body's text without changing its code (green control (ix)).
//! [`ANON_PREFIXES`] holds exactly the forms the plan lists plus the forms B3's probe (i) saw.
//!
//! **Owned compiler data** (SEH tables, funclets, switch tables) is named after its owning
//! function; the owner is demangled and normalised, and unstable counters are dropped.

use std::collections::BTreeMap;

use crate::llvm::Disasm;
use crate::red::{Red, RedKind, Result};

/// Raw-name prefixes of anonymous data: the 03 §6 list (`anon.*`, `.L*`, `__unnamed_*`,
/// `alloc_*`, `str.*`) and section symbols (`.rdata`, `.rdata$…`, `.data`, …), which also begin
/// with `.`. Probe (i) records every form seen in a candidate body; a form outside this list and
/// outside the owned-data forms of [`demangle`] is reported UNKNOWN there.
pub const ANON_PREFIXES: [&str; 5] = ["anon.", ".", "__unnamed_", "alloc_", "str."];

/// Raw-name prefixes of content-named constants (`__real@3f800000`): their name IS their value,
/// so they are kept as names, never folded into `<anon-data>`.
pub const CONTENT_NAMED_PREFIXES: [&str; 4] = ["__real@", "__xmm@", "__ymm@", "__zmm@"];

/// SEH table prefixes MSVC-style codegen names after the owning function.
pub const SEH_PREFIXES: [&str; 5] = ["$cppxdata$", "$ip2state$", "$stateUnwindMap$", "$tryMap$", "$handlerMap$"];

/// The placeholder for an anonymous data reference.
pub const ANON: &str = "<anon-data>";

/// The normalised spelling of every `$ehgcr_<ordinal>_<n>` label.
pub const EH_LABEL: &str = "$ehgcr";

/// `true` if the raw name is anonymous data.
#[must_use]
pub fn is_anon(raw: &str) -> bool {
    ANON_PREFIXES.iter().any(|p| raw.starts_with(p))
}

/// Strips v0 disambiguators, a legacy hash, `.llvm.<N>`, and LLVM's local-name uniquifier from a
/// demangled name.
///
/// The uniquifier is MEASURED at B3: when fat LTO internalises two locals of one name, LLVM renames
/// the later `<name>.<N>` with a module-wide counter (`$cppxdata$…SystemBox3new.4715`), and the
/// demangler prints the suffix as ` (.4715)`. The counter moves with unrelated code, so it is
/// dropped; the copies then count as one name with multiplicity in leg (7)(b).
#[must_use]
pub fn norm_name(demangled: &str) -> String {
    let mut s = demangled;
    while let Some(head) = s.strip_suffix(')').and_then(|h| h.rsplit_once(" (.")).and_then(|(h, n)| n.bytes().all(|b| b.is_ascii_digit()).then_some(h)) {
        s = head;
    }
    while let Some((head, n)) = s.rsplit_once('.')
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
        && (head.starts_with("_R") || head.starts_with('$') || head.starts_with('?'))
    {
        s = head;
    }
    if let Some(i) = s.find(".llvm.")
        && s[i + 6..].bytes().all(|b| b.is_ascii_digit())
    {
        s = &s[..i];
    }
    if s.len() > 19 {
        let tail = &s[s.len() - 19..];
        if tail.starts_with("::h") && tail[3..].bytes().all(|b| b.is_ascii_hexdigit()) {
            s = &s[..s.len() - 19];
        }
    }
    strip_disambiguators(s)
}

/// `ident[0123abcd]::` → `ident::`. Only after an identifier character and only before `]::`, so
/// `<[f64]>::len` (a slice type) is never touched.
fn strip_disambiguators(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'[' && i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j > i + 1 && s[j..].starts_with("]::") {
                i = j + 1;
                continue;
            }
        }
        let ch = s[i..].chars().next().expect("invariant: i is on a char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// The demangled spelling of `raw`: from `map` (nm's own demangling), or, for owned compiler data
/// (`$cppxdata$<fn>`, `?dtor$N@?0?<fn>@4HA`, `switch.table.<fn>[.N][.rel]`), the owner demangled
/// inside the wrapper with its unstable counter dropped.
#[must_use]
pub fn demangle(map: &BTreeMap<String, String>, raw: &str) -> String {
    if let Some(d) = map.get(raw)
        && d != raw
    {
        return d.clone();
    }
    for p in SEH_PREFIXES {
        if let Some(inner) = raw.strip_prefix(p) {
            // `$handlerMap$<n>$<owner>`: the handler index is part of the owner's own codegen.
            if let Some((n, owner_raw)) = inner.split_once('$')
                && !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
            {
                return format!("{p}{n}${}", owner(map, owner_raw));
            }
            return format!("{p}{}", owner(map, inner));
        }
    }
    // `$ehgcr_<function ordinal>_<n>`: MSVC-style EH continuation labels inside a function, numbered
    // by the function's ordinal in the module. Measured at B3 (probe (v)): adding one function to
    // `boyko_ecs` renumbered 66 of them in `boyko_demo`. The ordinal says nothing about the code.
    if let Some(rest) = raw.strip_prefix("$ehgcr_")
        && rest.bytes().all(|b| b.is_ascii_digit() || b == b'_')
    {
        return EH_LABEL.to_owned();
    }
    if let Some(rest) = raw.strip_prefix('?') {
        // `?dtor$14@?0?<fn>@4HA`, `?catch$3@?0?<fn>@4HA`
        if let (Some(at), true) = (rest.find("@?0?"), rest.ends_with("@4HA")) {
            let inner = &rest[at + 4..rest.len() - 4];
            return format!("?{}@?0?{}@4HA", &rest[..at], owner(map, inner));
        }
    }
    if let Some(inner) = raw.strip_prefix("switch.table.") {
        let mut inner = inner.strip_suffix(".rel").unwrap_or(inner);
        if let Some((head, tail)) = inner.rsplit_once('.')
            && tail.bytes().all(|b| b.is_ascii_digit())
        {
            inner = head;
        }
        return format!("switch.table({})", owner(map, inner));
    }
    raw.to_owned()
}

fn owner(map: &BTreeMap<String, String>, inner: &str) -> String {
    if let Some(d) = map.get(inner) {
        return norm_name(d);
    }
    // `<owner>.<N>`: the uniquified copy's owner may be absent under that spelling.
    match inner.rsplit_once('.') {
        Some((head, n)) if n.bytes().all(|b| b.is_ascii_digit()) => map.get(head).map_or_else(|| inner.to_owned(), |d| norm_name(d)),
        _ => inner.to_owned(),
    }
}

/// A declared rename list: `old -> new` per line (`→` accepted), `#` comments. Applied to
/// normalised names as a bounded substring replacement, so a renamed path is also mapped where it
/// appears inside another name's generic arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameList(pub Vec<(String, String)>);

impl RenameList {
    /// Parses a rename file.
    pub fn parse(text: &str) -> Result<Self> {
        let mut v = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let (old, new) = l
                .split_once("->")
                .or_else(|| l.split_once('→'))
                .ok_or_else(|| Red::new(RedKind::Malformed, format!("rename list line {}: `{l}` has no `->`", n + 1)))?;
            let (old, new) = (old.trim(), new.trim());
            if old.is_empty() || new.is_empty() {
                return Err(Red::new(RedKind::Malformed, format!("rename list line {}: empty side", n + 1)));
            }
            v.push((old.to_owned(), new.to_owned()));
        }
        Ok(Self(v))
    }

    /// `name` with every bounded occurrence of an `old` replaced by its `new`.
    #[must_use]
    pub fn apply(&self, name: &str) -> String {
        let mut s = name.to_owned();
        for (old, new) in &self.0 {
            s = replace_bounded(&s, old, new);
        }
        s
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Replaces each occurrence of `old` in `s` that is not glued to an identifier character on either
/// side (judged on the ORIGINAL text), so `a::f` does not rewrite `a::foo` or `xa::f`.
fn replace_bounded(s: &str, old: &str, new: &str) -> String {
    let first_len = old.chars().next().map_or(1, char::len_utf8);
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut prev: Option<u8> = None;
    while let Some(i) = rest.find(old) {
        let before = if i == 0 { prev } else { Some(rest.as_bytes()[i - 1]) };
        let after = rest.as_bytes().get(i + old.len()).copied();
        let bounded = !before.is_some_and(is_ident_byte) && !after.is_some_and(is_ident_byte);
        let take = if bounded { i + old.len() } else { i + first_len };
        if bounded {
            out.push_str(&rest[..i]);
            out.push_str(new);
        } else {
            out.push_str(&rest[..take]);
        }
        prev = rest.as_bytes()[..take].last().copied();
        rest = &rest[take..];
    }
    out.push_str(rest);
    out
}

/// The normalised spelling of a relocation target or symbol.
#[must_use]
pub fn norm_target(map: &BTreeMap<String, String>, raw: &str, rename: &RenameList) -> String {
    if is_anon(raw) {
        return ANON.to_owned();
    }
    rename.apply(&norm_name(&demangle(map, raw)))
}

/// The normalised body of one pinned function: every symbol of its section (the function and its
/// SEH funclets), in offset order, one instruction per line, with relocation targets named through
/// [`norm_target`] and objdump's self-referential `0x… <sym+0x…>` operands and `# …` comments
/// removed. Offsets are kept: they are section offsets, so they move only when code moves.
#[must_use]
pub fn norm_body(parts: &[Disasm], map: &BTreeMap<String, String>, rename: &RenameList) -> String {
    let mut ordered: Vec<&Disasm> = parts.iter().collect();
    ordered.sort_by_key(|d| d.start);
    let mut out = String::new();
    for d in ordered {
        out.push_str(&format!("<{}> @+0x{:x}:\n", norm_target(map, &d.symbol, rename), d.start));
        for insn in &d.insns {
            let text = strip_symbolic_operand(&insn.text);
            out.push_str(&format!("{:>6x}: {}", insn.offset, text.replace('\t', " ")));
            for r in &insn.relocs {
                let kind = r.kind.strip_prefix("IMAGE_REL_AMD64_").unwrap_or(&r.kind);
                out.push_str(&format!(" ; {kind} {}", norm_target(map, &r.target, rename)));
            }
            out.push('\n');
        }
    }
    out
}

/// Removes objdump's ` <name+0x…>` annotation after a branch or call operand. The numeric target is
/// kept for an intra-section branch; for a relocated call the relocation names the target.
fn strip_symbolic_operand(text: &str) -> String {
    match (text.rfind(" <"), text.ends_with('>')) {
        (Some(i), true) => text[..i].to_owned(),
        _ => text.to_owned(),
    }
}
