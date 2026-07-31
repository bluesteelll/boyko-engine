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
//! non-indexed primitives, `u8` indices, and a node HIERARCHY.
//!
//! # The node transform is BAKED, not refused — and that distinction is the point
//!
//! A decoder that silently *ignored* a node's TRS would pass R0b's triangle-count
//! equality, its `gMeshMeta` row and its allocation check — all three are
//! affine-invariant — while the census rendered a different scene than the
//! manifest describes. Rev 33 saw that correctly and then drew the wrong
//! conclusion: it refused *any* non-identity transform. Running this decoder
//! against real content settled it — **every** single-mesh sample asset probed
//! carries a node transform, so the restriction refused essentially all real
//! `.glb`. *Applying* a transform is not *ignoring* it. Positions take the matrix,
//! normals take its inverse transpose (so non-uniform scale does not shear them
//! off the surface), tangents take the matrix with their handedness `w` carried
//! through. Multiple primitives and multiple ROOT mesh nodes are CONCATENATED, each
//! with its own transform baked and its indices offset: neither places one mesh
//! relative to another, so both are decoding. Composing a parent transform with a
//! child's does place them relative to one another — that is scene assembly, and a
//! hierarchy stays refused. Measured justification: every genuinely high-poly sample
//! asset is multi-primitive, so "exactly one primitive" capped the corpus at ~18 k
//! triangles, three orders of magnitude below the regime K1 is about.
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
///
/// ⚠️ **This was hand-written as `0x4674_6C67`, which spells `g l t F` with a LOWERCASE
/// `t`, where the spec's tag is `glTF` — so the decoder rejected every real `.glb`.** Each
/// synthetic fixture passed anyway, because the test's own container builder repeated the
/// same constant: the fixture agreed with the bug. It took a REAL asset to expose it.
/// Deriving from a byte literal removes the class — there is no second place to mistype.
const GLB_MAGIC: u32 = u32::from_le_bytes(*b"glTF");
/// `JSON` in ASCII, little-endian — the structural chunk's type tag.
const CHUNK_JSON: u32 = u32::from_le_bytes(*b"JSON");
/// `BIN\0` in ASCII, little-endian — the binary chunk's type tag.
const CHUNK_BIN: u32 = u32::from_le_bytes(*b"BIN\0");

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

/// A column-major 4×4, glTF's own convention.
type Mat4 = [f32; 16];

const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0, //
];

fn f32s(v: &Json) -> Vec<f32> {
    v.arr().unwrap_or(&[]).iter().filter_map(Json::num).map(|n| n as f32).collect()
}

/// The node's local transform: an explicit `matrix`, or `T * R * S` composed from the
/// separate channels. glTF forbids mixing the two forms on one node.
fn node_matrix(node: &Json) -> Mat4 {
    if let Some(m) = node.get("matrix") {
        let v = f32s(m);
        if v.len() == 16 {
            let mut out = IDENTITY;
            out.copy_from_slice(&v);
            return out;
        }
    }

    let t = node.get("translation").map(f32s).unwrap_or_default();
    let r = node.get("rotation").map(f32s).unwrap_or_default();
    let s = node.get("scale").map(f32s).unwrap_or_default();
    let (tx, ty, tz) = (
        t.first().copied().unwrap_or(0.0),
        t.get(1).copied().unwrap_or(0.0),
        t.get(2).copied().unwrap_or(0.0),
    );
    // glTF stores the quaternion as (x, y, z, w).
    let (qx, qy, qz, qw) = (
        r.first().copied().unwrap_or(0.0),
        r.get(1).copied().unwrap_or(0.0),
        r.get(2).copied().unwrap_or(0.0),
        r.get(3).copied().unwrap_or(1.0),
    );
    let (sx, sy, sz) = (
        s.first().copied().unwrap_or(1.0),
        s.get(1).copied().unwrap_or(1.0),
        s.get(2).copied().unwrap_or(1.0),
    );

    // Rotation matrix from the unit quaternion, then scaled column-wise (R * S).
    let (x2, y2, z2) = (qx + qx, qy + qy, qz + qz);
    let (xx, xy, xz) = (qx * x2, qx * y2, qx * z2);
    let (yy, yz, zz) = (qy * y2, qy * z2, qz * z2);
    let (wx, wy, wz) = (qw * x2, qw * y2, qw * z2);

    [
        (1.0 - (yy + zz)) * sx,
        (xy + wz) * sx,
        (xz - wy) * sx,
        0.0,
        (xy - wz) * sy,
        (1.0 - (xx + zz)) * sy,
        (yz + wx) * sy,
        0.0,
        (xz + wy) * sz,
        (yz - wx) * sz,
        (1.0 - (xx + yy)) * sz,
        0.0,
        tx,
        ty,
        tz,
        1.0,
    ]
}

fn is_identity(m: &Mat4) -> bool {
    m.iter().zip(IDENTITY.iter()).all(|(a, b)| a == b)
}

fn transform_point(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

/// The cofactor matrix of `m`'s upper-left 3×3, divided by its determinant — i.e. the
/// **inverse transpose**, which is what a normal must be transformed by. Using `m` itself
/// would shear normals off the surface under non-uniform scale, and dividing by the
/// determinant keeps handedness right under a mirroring transform.
fn normal_matrix(m: &Mat4) -> [f32; 9] {
    let (a, b, c) = (m[0], m[4], m[8]);
    let (d, e, f) = (m[1], m[5], m[9]);
    let (g, h, i) = (m[2], m[6], m[10]);
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det == 0.0 {
        // A degenerate transform collapses the mesh; leave normals untouched rather than
        // producing NaNs, and let the caller's own checks speak.
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let inv = 1.0 / det;
    // COLUMN-major, matching `transform_dir`'s indexing and the `Mat4` convention. ⚠️ Laid
    // out row-major first, which silently applied the TRANSPOSE — for a pure rotation that
    // is the inverse rotation, so positions and normals came out of the same file rotated
    // opposite ways. The rotation test below is what caught it.
    [
        (e * i - f * h) * inv,
        -(b * i - c * h) * inv,
        (b * f - c * e) * inv,
        -(d * i - f * g) * inv,
        (a * i - c * g) * inv,
        -(a * f - c * d) * inv,
        (d * h - e * g) * inv,
        -(a * h - b * g) * inv,
        (a * e - b * d) * inv,
    ]
}

fn transform_dir(n: &[f32; 9], v: [f32; 3]) -> [f32; 3] {
    let out = [
        n[0] * v[0] + n[3] * v[1] + n[6] * v[2],
        n[1] * v[0] + n[4] * v[1] + n[7] * v[2],
        n[2] * v[0] + n[5] * v[1] + n[8] * v[2],
    ];
    let len = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt();
    if len > 0.0 { [out[0] / len, out[1] / len, out[2] / len] } else { v }
}

/// Resolves the transform of the single mesh-bearing node, refusing anything this decoder
/// cannot express as one instance.
///
/// ⚠️ **Rev 37 BAKES the transform where Rev 33 refused it, and the distinction is the
/// whole point.** Rev 33 was right that a decoder which silently *ignored* a node's TRS
/// would pass every R0b gate part — triangle count, the `gMeshMeta` row and the allocation
/// check are all affine-invariant — while the census rendered a different scene. It then
/// drew the wrong conclusion and refused *any* non-identity TRS. Running the decoder
/// against real content settled it: **every** single-mesh sample asset probed carries a
/// node transform, so the restriction refused essentially all real `.glb`, and §4.3's
/// "the manifest author re-exports assets that satisfy this" was an instruction nobody
/// could follow. *Applying* a transform is not *ignoring* it; ignoring is the defect, and
/// baking is the fix.
///
/// What is still refused, because it is scene assembly rather than decoding: a node
/// hierarchy, and more than one instance of the mesh.
fn resolve_instance_transform(root: &Json) -> Result<Vec<(usize, Mat4)>, AssetError> {
    let nodes = root.arr_at("nodes");
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if !node.arr_at("children").is_empty() {
            return Err(err(format!(
                "node {i} has children — composing a parent transform with a child's PLACES one \
                 mesh relative to another, which is scene assembly rather than decoding (§3.3)"
            )));
        }
        if let Some(m) = node.usize_at("mesh") {
            out.push((m, node_matrix(node)));
        }
    }
    // A file with no `nodes` at all still has its meshes; take them at identity.
    if out.is_empty() {
        out = (0..root.arr_at("meshes").len()).map(|m| (m, IDENTITY)).collect();
    }
    Ok(out)
}


/// Decodes ONE primitive into model-space vertices + indices, with no placement applied.
///
/// ⚠️ **Concatenating primitives and root-level mesh nodes is DECODING, not scene assembly, and
/// Rev 38 draws that line where it can be defended.** A mesh's primitives already share one
/// coordinate space (they are split by MATERIAL, which the density census does not read), and each
/// root node's transform is applied to its own mesh alone. Neither act places one mesh RELATIVE to
/// another. Composing a parent transform with a child's does, which is why a hierarchy is still
/// refused. Measured justification: every genuinely high-poly sample asset is multi-primitive, so
/// "exactly one primitive" capped this corpus at ~18 k triangles -- three orders of magnitude below
/// the regime K1 is about.
fn decode_primitive(
    root: &Json,
    bin: &[u8],
    prim: &Json,
    what: &str,
) -> Result<(Vec<Vertex>, Vec<u32>), AssetError> {
    let mode = prim.u64_at("mode").unwrap_or(MODE_TRIANGLES);
    if mode != MODE_TRIANGLES {
        return Err(err(format!("{what}: mode {mode} is not TRIANGLES (§3.3)")));
    }
    if !prim.arr_at("targets").is_empty() {
        return Err(err(format!("{what}: morph targets are outside this subset (§3.3)")));
    }
    let index_accessor = prim
        .usize_at("indices")
        .ok_or_else(|| err(format!("{what}: non-indexed primitives are outside this subset (§3.3)")))?;

    let attrs = prim.get("attributes").ok_or_else(|| err(format!("{what}: no attributes")))?;
    let attr = |name: &str| attrs.usize_at(name);

    let pos = Accessor::resolve(
        root,
        bin,
        attr("POSITION").ok_or_else(|| err(format!("{what}: no POSITION")))?,
        what,
    )?;
    let nrm = Accessor::resolve(
        root,
        bin,
        attr("NORMAL")
            .ok_or_else(|| err(format!("{what}: no NORMAL — refused rather than invented (§3.3)")))?,
        what,
    )?;
    if nrm.count != pos.count {
        return Err(err(format!("{what}: NORMAL count {} != POSITION count {}", nrm.count, pos.count)));
    }

    let uv = match attr("TEXCOORD_0") {
        Some(i) => Some(Accessor::resolve(root, bin, i, what)?),
        None => None,
    };
    let tan = match attr("TANGENT") {
        Some(i) => Some(Accessor::resolve(root, bin, i, what)?),
        None => None,
    };
    let col = match attr("COLOR_0") {
        Some(i) => Some(Accessor::resolve(root, bin, i, what)?),
        None => None,
    };

    let mut vertices = Vec::with_capacity(pos.count);
    for i in 0..pos.count {
        let p = pos.read(i);
        let n = nrm.read(i);
        let t = uv.as_ref().map_or([0.0; 4], |a| a.read(i));
        let c = col.as_ref().map_or(DEFAULT_VERTEX_COLOR, |a| {
            let v = a.read(i);
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

    let idx = Accessor::resolve(root, bin, index_accessor, what)?;
    if idx.components != 1 {
        return Err(err(format!("{what}: index accessor is not SCALAR")));
    }
    if !idx.count.is_multiple_of(3) {
        return Err(err(format!("{what}: index count {} is not a multiple of 3", idx.count)));
    }
    let mut indices = Vec::with_capacity(idx.count);
    for i in 0..idx.count {
        let v = idx.read_index(i)?;
        if v as usize >= vertices.len() {
            return Err(err(format!("{what}: index {v} out of range for {} vertices", vertices.len())));
        }
        indices.push(v);
    }

    // A missing TANGENT takes the engine's existing post-pass rather than a zero basis, which
    // would flatten normal mapping silently. Run per primitive, BEFORE placement, so the basis is
    // generated in the space the positions are in.
    if tan.is_none() {
        generate_tangents(&mut vertices, &indices);
    }

    Ok((vertices, indices))
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

        let placements = resolve_instance_transform(&root)?;
        let meshes = root.arr_at("meshes");
        if placements.is_empty() || meshes.is_empty() {
            return Err(err("the document contains no mesh"));
        }

        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        for (mesh_index, xform) in &placements {
            let mesh = meshes
                .get(*mesh_index)
                .ok_or_else(|| err(format!("node references mesh {mesh_index}, which does not exist")))?;
            for (pi, prim) in mesh.arr_at("primitives").iter().enumerate() {
                let what = format!("mesh {mesh_index} primitive {pi}");
                let base = vertices.len() as u32;
                let (mut vs, is) = decode_primitive(&root, bin, prim, &what)?;

                // BAKE this placement into model space. The engine places instances itself, so a
                // transform left in the file would be data nothing consumes -- and DROPPING it is
                // invisible to every R0b gate part (triangle count, the gMeshMeta row and the
                // allocation check are all affine-invariant) while the census renders a different
                // scene than the manifest describes.
                if !is_identity(xform) {
                    let nm = normal_matrix(xform);
                    let tm = [
                        xform[0], xform[1], xform[2], //
                        xform[4], xform[5], xform[6], //
                        xform[8], xform[9], xform[10],
                    ];
                    for v in &mut vs {
                        v.position = transform_point(xform, v.position);
                        v.normal = transform_dir(&nm, v.normal);
                        // A tangent runs ALONG the surface, so it takes the transform itself
                        // rather than the inverse transpose; `w` is the bitangent handedness sign
                        // and is carried through untouched.
                        let t = transform_dir(&tm, [v.tangent[0], v.tangent[1], v.tangent[2]]);
                        v.tangent = [t[0], t[1], t[2], v.tangent[3]];
                    }
                }

                vertices.extend(vs);
                indices.extend(is.into_iter().map(|k| k + base));
            }
        }

        if indices.is_empty() {
            return Err(err("the document decodes to zero triangles"));
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
