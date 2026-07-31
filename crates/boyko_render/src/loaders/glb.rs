//! In-house glTF 2.0 **binary** (`.glb`) mesh decoder — VG-R0 rung R0b, §3.3.
//!
//! # Why in-house, and why `.glb`
//!
//! Licence-clean high-poly corpora ship as `.glb`. OBJ carries no tangents, no
//! index buffer (the sibling loader sort-dedups every corner) and is a text parse
//! over hundreds of megabytes. A `.glb` is a 12-byte header plus a JSON chunk plus
//! a BIN chunk; only the JSON chunk needs a new reader. That is loader code, not
//! hot-path code — the same class of work `boyko_image`'s in-house PNG/zlib
//! decoder already carries, and it keeps the zero-third-party-dependency posture.
//!
//! # The subset, stated as a scope cut rather than discovered as a bug
//!
//! **Supported:** `mode == TRIANGLES`, `POSITION`, `NORMAL`, `TEXCOORD_0`,
//! `TANGENT`, `COLOR_0`, and indexed primitives with `u16`/`u32` indices.
//!
//! **Unsupported, and a hard [`AssetError`] rather than a silent fallback:**
//! sparse accessors, Draco/meshopt compression (any non-empty
//! `extensionsRequired`), animation, skins, morph targets, non-triangle modes,
//! non-indexed primitives, `u8` indices, and a scene graph that is not exactly
//! **one mesh with one primitive under an identity (or absent) node transform**.
//!
//! That last refusal is the one worth stating twice, because it is invisible to
//! every gate downstream. Flattening a node hierarchy is *scene assembly*, not
//! decoding, and a decoder that silently ignored a node's TRS would pass R0b's
//! triangle-count equality, its `gMeshMeta` row and its allocation check — all
//! three are affine-invariant — while the census rendered a different scene than
//! the manifest describes. §4.3's manifest author selects, or re-exports, assets
//! that satisfy this.
//!
//! A missing `TANGENT` runs the existing [`generate_tangents`] post-pass; a
//! missing `COLOR_0` takes the neutral default. A missing `POSITION` or `NORMAL`
//! is refused: normals are universally present in the scanned/sculpted content
//! this corpus is about, and inventing them would be exactly the silent
//! substitution this subset exists to forbid.

use boyko_ecs::ecs::core::asset::{Asset, AssetError, AssetLoader};

use crate::mesh::{MeshGpu, Vertex};
use crate::mesh_data::MeshData;
use crate::tangent::generate_tangents;

/// The neutral vertex color used when a primitive carries no `COLOR_0` — the
/// same default the `.obj` loader applies.
const DEFAULT_VERTEX_COLOR: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

/// `glTF` in ASCII, little-endian — the `.glb` magic.
const GLB_MAGIC: u32 = 0x4674_6C67;
/// `JSON` in ASCII, little-endian — the structural chunk's type tag.
const CHUNK_JSON: u32 = 0x4E4F_534A;
/// `BIN\0` in ASCII, little-endian — the binary chunk's type tag.
const CHUNK_BIN: u32 = 0x004E_4942;

/// glTF `primitive.mode` for triangles; the only mode this subset accepts.
const MODE_TRIANGLES: u64 = 4;

fn err(msg: impl Into<String>) -> AssetError {
    AssetError::Decode(msg.into())
}

// ---------------------------------------------------------------------------------------------
// A minimal JSON reader.
//
// Objects are `Vec<(String, Json)>`, not a map: a glTF structural chunk has a
// handful of keys per object, lookup is load-time, and `HashMap` is disallowed in
// this workspace. Linear scan over <20 keys beats hashing them anyway.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn usize_at(&self, key: &str) -> Option<usize> {
        self.get(key)?.num().map(|n| n as usize)
    }

    fn u64_at(&self, key: &str) -> Option<u64> {
        self.get(key)?.num().map(|n| n as u64)
    }

    fn str_at(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Json::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// An array member, or an empty slice when the key is absent — the shape
    /// glTF's optional top-level arrays have.
    fn arr_at(&self, key: &str) -> &[Json] {
        self.get(key).and_then(Json::arr).unwrap_or(&[])
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }

    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.peek().ok_or("unexpected end of JSON")? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::Str),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            _ => self.number(),
        }
    }

    fn literal(&mut self, word: &str, out: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(out)
        } else {
            Err(format!("bad literal at byte {}", self.i))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        while self.i < self.b.len()
            && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        {
            self.i += 1;
        }
        if start == self.i {
            return Err(format!("expected a value at byte {start}"));
        }
        std::str::from_utf8(&self.b[start..self.i])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(Json::Num)
            .ok_or_else(|| format!("bad number at byte {start}"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek().ok_or("unterminated escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self
                                .b
                                .get(self.i..self.i + 4)
                                .ok_or("truncated \\u escape")?;
                            let cp = u32::from_str_radix(
                                std::str::from_utf8(hex).map_err(|_| "bad \\u escape")?,
                                16,
                            )
                            .map_err(|_| "bad \\u escape")?;
                            self.i += 4;
                            // Lone surrogates are replaced rather than rejected:
                            // glTF names are not load-bearing here.
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(format!("unknown escape at byte {}", self.i)),
                    }
                }
                _ => {
                    // Multi-byte UTF-8 passes through verbatim.
                    let start = self.i - 1;
                    let len = utf8_len(c);
                    let end = start + len;
                    let s = self.b.get(start..end).ok_or("truncated UTF-8")?;
                    out.push_str(std::str::from_utf8(s).map_err(|_| "invalid UTF-8")?);
                    self.i = end;
                }
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.eat(b'[')?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Arr(out));
        }
        loop {
            out.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(out));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.i)),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.eat(b'{')?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Obj(out));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            self.eat(b':')?;
            let v = self.value()?;
            out.push((k, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(out));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.i)),
            }
        }
    }
}

fn utf8_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn parse_json(bytes: &[u8]) -> Result<Json, String> {
    let mut p = Parser::new(bytes);
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!("trailing bytes at {}", p.i));
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// Accessor reading.
// ---------------------------------------------------------------------------------------------

/// glTF `componentType` codes this subset understands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Comp {
    U8,
    U16,
    U32,
    F32,
}

impl Comp {
    fn from_code(code: u64) -> Option<Self> {
        match code {
            5121 => Some(Comp::U8),
            5123 => Some(Comp::U16),
            5125 => Some(Comp::U32),
            5126 => Some(Comp::F32),
            _ => None,
        }
    }

    fn size(self) -> usize {
        match self {
            Comp::U8 => 1,
            Comp::U16 => 2,
            Comp::U32 | Comp::F32 => 4,
        }
    }
}

fn type_components(ty: &str) -> Option<usize> {
    match ty {
        "SCALAR" => Some(1),
        "VEC2" => Some(2),
        "VEC3" => Some(3),
        "VEC4" => Some(4),
        _ => None,
    }
}

/// One accessor resolved against its buffer view, ready to be read element-wise.
struct Accessor<'a> {
    bin: &'a [u8],
    base: usize,
    stride: usize,
    count: usize,
    comp: Comp,
    components: usize,
    normalized: bool,
}

impl<'a> Accessor<'a> {
    fn resolve(root: &Json, bin: &'a [u8], index: usize, what: &str) -> Result<Self, AssetError> {
        let acc = root
            .arr_at("accessors")
            .get(index)
            .ok_or_else(|| err(format!("{what}: accessor {index} is out of range")))?;
        if acc.get("sparse").is_some() {
            return Err(err(format!(
                "{what}: sparse accessors are outside this decoder's subset (§3.3) — refusing \
                 rather than decoding a partial mesh"
            )));
        }
        let comp = Comp::from_code(
            acc.u64_at("componentType")
                .ok_or_else(|| err(format!("{what}: accessor has no componentType")))?,
        )
        .ok_or_else(|| err(format!("{what}: unsupported componentType")))?;
        let ty = acc
            .str_at("type")
            .ok_or_else(|| err(format!("{what}: accessor has no type")))?;
        let components = type_components(ty)
            .ok_or_else(|| err(format!("{what}: unsupported accessor type {ty}")))?;
        let count = acc
            .usize_at("count")
            .ok_or_else(|| err(format!("{what}: accessor has no count")))?;
        let normalized = matches!(acc.get("normalized"), Some(Json::Bool(true)));

        let view_index = acc
            .usize_at("bufferView")
            .ok_or_else(|| err(format!("{what}: accessor has no bufferView (sparse-only?)")))?;
        let view = root
            .arr_at("bufferViews")
            .get(view_index)
            .ok_or_else(|| err(format!("{what}: bufferView {view_index} is out of range")))?;
        if view.usize_at("buffer").unwrap_or(0) != 0 {
            return Err(err(format!(
                "{what}: bufferView names buffer {} — a .glb's only buffer is the BIN chunk",
                view.usize_at("buffer").unwrap_or(0)
            )));
        }
        let view_offset = view.usize_at("byteOffset").unwrap_or(0);
        let elem = comp.size() * components;
        let stride = view.usize_at("byteStride").unwrap_or(elem);
        if stride < elem {
            return Err(err(format!("{what}: byteStride {stride} is smaller than one element")));
        }
        let base = view_offset + acc.usize_at("byteOffset").unwrap_or(0);

        // Bounds-check once, here, so element reads need no per-access branch.
        let last = base
            .checked_add(stride.saturating_mul(count.saturating_sub(1)))
            .and_then(|o| o.checked_add(elem))
            .ok_or_else(|| err(format!("{what}: accessor range overflows")))?;
        if count > 0 && last > bin.len() {
            return Err(err(format!(
                "{what}: accessor reads {last} bytes of a {} byte BIN chunk",
                bin.len()
            )));
        }

        Ok(Accessor { bin, base, stride, count, comp, components, normalized })
    }

    /// Reads element `i` as up to four `f32` lanes, applying glTF's normalized-
    /// integer convention when the accessor asks for it.
    fn read(&self, i: usize) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        let at = self.base + i * self.stride;
        for (c, lane) in out.iter_mut().enumerate().take(self.components.min(4)) {
            let o = at + c * self.comp.size();
            *lane = match self.comp {
                Comp::F32 => f32::from_le_bytes([
                    self.bin[o],
                    self.bin[o + 1],
                    self.bin[o + 2],
                    self.bin[o + 3],
                ]),
                Comp::U8 => {
                    let v = self.bin[o] as f32;
                    if self.normalized { v / 255.0 } else { v }
                }
                Comp::U16 => {
                    let v = u16::from_le_bytes([self.bin[o], self.bin[o + 1]]) as f32;
                    if self.normalized { v / 65535.0 } else { v }
                }
                Comp::U32 => u32::from_le_bytes([
                    self.bin[o],
                    self.bin[o + 1],
                    self.bin[o + 2],
                    self.bin[o + 3],
                ]) as f32,
            };
        }
        out
    }

    /// Reads element `i` as an index. `u8` is deliberately absent: §3.3's subset
    /// is `u16`/`u32`.
    fn read_index(&self, i: usize) -> Result<u32, AssetError> {
        let o = self.base + i * self.stride;
        match self.comp {
            Comp::U16 => Ok(u16::from_le_bytes([self.bin[o], self.bin[o + 1]]) as u32),
            Comp::U32 => Ok(u32::from_le_bytes([
                self.bin[o],
                self.bin[o + 1],
                self.bin[o + 2],
                self.bin[o + 3],
            ])),
            Comp::U8 => Err(err(
                "u8 indices are outside this decoder's subset (§3.3 names u16/u32)".to_string(),
            )),
            Comp::F32 => Err(err("index accessor has a float componentType".to_string())),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The loader.
// ---------------------------------------------------------------------------------------------

/// Splits a `.glb` container into its JSON and BIN chunks.
fn split_chunks(bytes: &[u8]) -> Result<(&[u8], &[u8]), AssetError> {
    if bytes.len() < 12 {
        return Err(err("glb is shorter than its 12-byte header"));
    }
    let u32_at = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    if u32_at(0) != GLB_MAGIC {
        return Err(err("not a .glb (bad magic)"));
    }
    let version = u32_at(4);
    if version != 2 {
        return Err(err(format!("glb container version {version}, expected 2")));
    }
    let total = u32_at(8) as usize;
    if total > bytes.len() {
        return Err(err(format!(
            "glb header declares {total} bytes but the file is {}",
            bytes.len()
        )));
    }

    let mut json: Option<&[u8]> = None;
    let mut bin: Option<&[u8]> = None;
    let mut o = 12;
    while o + 8 <= total {
        let len = u32_at(o) as usize;
        let kind = u32_at(o + 4);
        let start = o + 8;
        let end = start
            .checked_add(len)
            .filter(|e| *e <= total)
            .ok_or_else(|| err("glb chunk overruns the container"))?;
        match kind {
            CHUNK_JSON if json.is_none() => json = Some(&bytes[start..end]),
            CHUNK_BIN if bin.is_none() => bin = Some(&bytes[start..end]),
            _ => {}
        }
        // Chunks are 4-byte aligned.
        o = end + ((4 - (len % 4)) % 4);
    }

    let json = json.ok_or_else(|| err("glb has no JSON chunk"))?;
    let bin = bin.unwrap_or(&[]);
    Ok((json, bin))
}

/// Refuses any scene graph that is not one mesh under an identity transform.
///
/// Silent acceptance here is invisible downstream: triangle count, the
/// `gMeshMeta` row and the allocation check are all affine-invariant, so a
/// dropped node transform would pass every R0b gate part while the census
/// rendered a different scene.
fn assert_flat_identity_scene(root: &Json) -> Result<(), AssetError> {
    const IDENTITY: [f64; 16] =
        [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

    let nodes = root.arr_at("nodes");
    let mut mesh_nodes = 0usize;
    for (i, node) in nodes.iter().enumerate() {
        if node.get("mesh").is_some() {
            mesh_nodes += 1;
        }
        if !node.arr_at("children").is_empty() {
            return Err(err(format!(
                "node {i} has children — flattening a hierarchy is scene assembly, not decoding \
                 (§3.3)"
            )));
        }
        if let Some(Json::Arr(m)) = node.get("matrix") {
            let vals: Vec<f64> = m.iter().filter_map(Json::num).collect();
            if vals.len() != 16 || vals != IDENTITY {
                return Err(err(format!("node {i} carries a non-identity matrix (§3.3)")));
            }
        }
        for (key, identity) in [("translation", 0.0), ("rotation", f64::NAN), ("scale", 1.0)] {
            let Some(Json::Arr(v)) = node.get(key) else { continue };
            let vals: Vec<f64> = v.iter().filter_map(Json::num).collect();
            let is_default = if key == "rotation" {
                vals == [0.0, 0.0, 0.0, 1.0]
            } else {
                vals.iter().all(|x| *x == identity)
            };
            if !is_default {
                return Err(err(format!("node {i} carries a non-identity {key} (§3.3)")));
            }
        }
    }
    if mesh_nodes > 1 {
        return Err(err(format!(
            "{mesh_nodes} nodes reference a mesh — this decoder accepts exactly one instance (§3.3)"
        )));
    }
    Ok(())
}

/// Decodes a `.glb` into the engine's [`MeshData`] intermediate.
pub struct GlbMeshLoader;

impl AssetLoader for GlbMeshLoader {
    type Out = MeshGpu;

    const EXTENSIONS: &'static [&'static str] = &["glb"];

    fn decode(bytes: &[u8]) -> Result<<Self::Out as Asset>::Cpu, AssetError> {
        let (json_bytes, bin) = split_chunks(bytes)?;
        let root = parse_json(json_bytes).map_err(|e| err(format!("glb JSON: {e}")))?;

        if let Some(v) = root.get("asset").and_then(|a| a.str_at("version"))
            && !v.starts_with('2')
        {
            return Err(err(format!("glTF asset version {v}, expected 2.x")));
        }

        // Every refusal below is a SCOPE CUT stated in §3.3, not a bug discovered
        // later: an extension this decoder does not implement (Draco, meshopt)
        // makes the buffer contents something else entirely.
        for (key, why) in [
            ("extensionsRequired", "a required extension (Draco/meshopt?)"),
            ("animations", "animation"),
            ("skins", "skins"),
        ] {
            if !root.arr_at(key).is_empty() {
                return Err(err(format!(
                    "{why} is outside this decoder's subset (§3.3): `{key}` is non-empty"
                )));
            }
        }

        assert_flat_identity_scene(&root)?;

        let meshes = root.arr_at("meshes");
        if meshes.len() != 1 {
            return Err(err(format!(
                "{} meshes — this decoder accepts exactly one (§3.3)",
                meshes.len()
            )));
        }
        let prims = meshes[0].arr_at("primitives");
        if prims.len() != 1 {
            return Err(err(format!(
                "{} primitives — this decoder accepts exactly one (§3.3)",
                prims.len()
            )));
        }
        let prim = &prims[0];

        let mode = prim.u64_at("mode").unwrap_or(MODE_TRIANGLES);
        if mode != MODE_TRIANGLES {
            return Err(err(format!("primitive mode {mode} is not TRIANGLES (§3.3)")));
        }
        if !prim.arr_at("targets").is_empty() {
            return Err(err("morph targets are outside this decoder's subset (§3.3)"));
        }
        let index_accessor = prim
            .usize_at("indices")
            .ok_or_else(|| err("non-indexed primitives are outside this decoder's subset (§3.3)"))?;

        let attrs = prim
            .get("attributes")
            .ok_or_else(|| err("primitive has no attributes"))?;
        let attr = |name: &str| attrs.usize_at(name);

        let pos = Accessor::resolve(
            &root,
            bin,
            attr("POSITION").ok_or_else(|| err("primitive has no POSITION"))?,
            "POSITION",
        )?;
        let nrm = Accessor::resolve(
            &root,
            bin,
            attr("NORMAL").ok_or_else(|| {
                err("primitive has no NORMAL — refused rather than invented (§3.3)")
            })?,
            "NORMAL",
        )?;
        if nrm.count != pos.count {
            return Err(err(format!(
                "NORMAL count {} != POSITION count {}",
                nrm.count, pos.count
            )));
        }

        let uv = match attr("TEXCOORD_0") {
            Some(i) => Some(Accessor::resolve(&root, bin, i, "TEXCOORD_0")?),
            None => None,
        };
        let tan = match attr("TANGENT") {
            Some(i) => Some(Accessor::resolve(&root, bin, i, "TANGENT")?),
            None => None,
        };
        let col = match attr("COLOR_0") {
            Some(i) => Some(Accessor::resolve(&root, bin, i, "COLOR_0")?),
            None => None,
        };

        let mut vertices = Vec::with_capacity(pos.count);
        for i in 0..pos.count {
            let p = pos.read(i);
            let n = nrm.read(i);
            let t = uv.as_ref().map_or([0.0; 4], |a| a.read(i));
            let c = col.as_ref().map_or(DEFAULT_VERTEX_COLOR, |a| {
                let v = a.read(i);
                // COLOR_0 may be VEC3; alpha then defaults to opaque.
                [v[0], v[1], v[2], if a.components == 4 { v[3] } else { 1.0 }]
            });
            let g = tan.as_ref().map_or([0.0; 4], |a| a.read(i));
            vertices.push(Vertex {
                position: [p[0], p[1], p[2]],
                normal: [n[0], n[1], n[2]],
                color: c,
                uv: [t[0], t[1]],
                tangent: g,
            });
        }

        let idx = Accessor::resolve(&root, bin, index_accessor, "indices")?;
        if idx.components != 1 {
            return Err(err("index accessor is not SCALAR"));
        }
        if !idx.count.is_multiple_of(3) {
            return Err(err(format!(
                "index count {} is not a multiple of 3 (TRIANGLES)",
                idx.count
            )));
        }
        let mut indices = Vec::with_capacity(idx.count);
        for i in 0..idx.count {
            let v = idx.read_index(i)?;
            if v as usize >= vertices.len() {
                return Err(err(format!(
                    "index {v} is out of range for {} vertices",
                    vertices.len()
                )));
            }
            indices.push(v);
        }

        // A missing TANGENT takes the engine's existing post-pass rather than a
        // zero basis, which would flatten normal mapping silently.
        if tan.is_none() {
            generate_tangents(&mut vertices, &indices);
        }

        Ok(MeshData { vertices, indices })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_reads_the_shapes_gltf_uses() {
        let v = parse_json(br#"{"a":[1,2.5,-3e2],"b":{"c":"x\ny"},"d":true,"e":null}"#).unwrap();
        assert_eq!(v.arr_at("a").len(), 3);
        assert_eq!(v.arr_at("a")[2].num(), Some(-300.0));
        assert_eq!(v.get("b").unwrap().str_at("c"), Some("x\ny"));
        assert_eq!(v.get("d"), Some(&Json::Bool(true)));
        assert_eq!(v.get("e"), Some(&Json::Null));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        assert!(parse_json(b"{} junk").is_err());
    }
}
