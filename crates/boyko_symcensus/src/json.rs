//! A minimal JSON reader for cargo's message stream and `llvm-symbolizer --output-style=JSON`.
//!
//! Hand-rolled for the reason `tests/reflect_manifest_census.rs` gives for its own copy: a gate
//! whose subject is the build must not bring a third-party graph into it. Objects are key-ordered
//! pair vectors (no `HashMap`; the workspace's type ban applies here as everywhere).

use crate::red::{Red, RedKind, Result};

/// One JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `{…}`, in document order.
    Object(Vec<(String, Json)>),
    /// `[…]`.
    Array(Vec<Json>),
    /// A string, unescaped.
    Str(String),
    /// A number, kept as `f64` (cargo and the symbolizer emit only small integers).
    Num(f64),
    /// `true` / `false`.
    Bool(bool),
    /// `null`.
    Null,
}

impl Json {
    /// The member `key` of an object, or `None`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string, or `None` for any other variant.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The bool, or `None`.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The number, or `None`.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// The array's items; empty for any other variant.
    #[must_use]
    pub fn items(&self) -> &[Json] {
        match self {
            Self::Array(items) => items,
            _ => &[],
        }
    }
}

/// Parses one complete JSON document.
pub fn parse(text: &str) -> Result<Json> {
    let mut p = Parser { bytes: text.as_bytes(), pos: 0 };
    let v = p.value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(p.fail("trailing bytes after the document"));
    }
    Ok(v)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    #[cold]
    fn fail(&self, what: &str) -> Red {
        Red::new(RedKind::Malformed, format!("JSON at byte {}: {what}", self.pos))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.bytes.get(self.pos).copied().unwrap_or(0)
    }

    fn expect(&mut self, b: u8) -> Result<()> {
        if self.peek() != b {
            return Err(self.fail(&format!("expected `{}`", b as char)));
        }
        self.pos += 1;
        Ok(())
    }

    fn value(&mut self) -> Result<Json> {
        self.skip_ws();
        match self.peek() {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.fail("unexpected byte at the start of a value")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.fail("bad literal"))
        }
    }

    fn number(&mut self) -> Result<Json> {
        let start = self.pos;
        while matches!(self.peek(), b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("");
        text.parse::<f64>().map(Json::Num).map_err(|_| self.fail("bad number"))
    }

    fn hex4(&mut self) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = (self.peek() as char).to_digit(16).ok_or_else(|| self.fail("bad \\u escape"))?;
            v = v * 16 + d;
            self.pos += 1;
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let esc = self.peek();
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi)
                                && self.bytes[self.pos..].starts_with(b"\\u")
                            {
                                self.pos += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.fail("bad escape")),
                    }
                }
                0 if self.pos >= self.bytes.len() => return Err(self.fail("unterminated string")),
                _ => {
                    let start = self.pos;
                    while self.pos < self.bytes.len() && !matches!(self.bytes[self.pos], b'"' | b'\\') {
                        self.pos += 1;
                    }
                    let run = std::str::from_utf8(&self.bytes[start..self.pos])
                        .map_err(|_| self.fail("non-UTF-8 inside a string"))?;
                    out.push_str(run);
                }
            }
        }
    }

    fn object(&mut self) -> Result<Json> {
        self.expect(b'{')?;
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == b'}' {
            self.pos += 1;
            return Ok(Json::Object(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value()?;
            pairs.push((key, value));
            self.skip_ws();
            match self.peek() {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    return Ok(Json::Object(pairs));
                }
                _ => return Err(self.fail("expected `,` or `}` in an object")),
            }
        }
    }

    fn array(&mut self) -> Result<Json> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == b']' {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.fail("expected `,` or `]` in an array")),
            }
        }
    }
}
