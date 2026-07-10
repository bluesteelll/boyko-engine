//! [`RonMaterialLoader`] — the `.mat` text-format material loader
//! (asset-system rung A3b).

use boyko_ecs::ecs::core::asset::{Asset, AssetError, AssetLoader};

use crate::material::MaterialGpu;

/// Loads a `.mat` file: a plain, in-house key-value text format (no
/// `ron` / `serde` dependency) — one `key = value` (or `key: value`) pair per
/// line.
///
/// # Format
///
/// ```text
/// # a comment line (or a blank line) is skipped
/// base_color = 0.8, 0.2, 0.2, 1.0
/// metallic: 0.0
/// roughness = 0.4
/// reflectance = 0.5
/// emissive = 0.0 0.0 0.0
/// ```
///
/// Recognized keys: `base_color` (4 floats), `metallic` / `roughness` /
/// `reflectance` (1 float each), `emissive` (3 floats) — values are
/// whitespace- and/or comma-separated. An unrecognized key is IGNORED
/// (forward-compatible with a future field); a MISSING recognized key falls
/// back to [`MaterialGpu::default`]'s value for that field. `flags` is not
/// author-facing at this rung (always `0`).
pub struct RonMaterialLoader;

impl AssetLoader for RonMaterialLoader {
    type Out = MaterialGpu;

    const EXTENSIONS: &'static [&'static str] = &["mat"];

    fn decode(bytes: &[u8]) -> Result<<Self::Out as Asset>::Cpu, AssetError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| decode_error("material file is not valid UTF-8".to_owned()))?;

        let default = MaterialGpu::default();
        let mut base_color = default.base_color;
        let mut metallic = default.metallic();
        let mut roughness = default.roughness();
        let mut reflectance = default.reflectance();
        let mut emissive = [default.emissive[0], default.emissive[1], default.emissive[2]];

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = split_key_value(line) else {
                continue;
            };
            match key {
                "base_color" => base_color = parse_floats::<4>(value)?,
                "metallic" => metallic = parse_floats::<1>(value)?[0],
                "roughness" => roughness = parse_floats::<1>(value)?[0],
                "reflectance" => reflectance = parse_floats::<1>(value)?[0],
                "emissive" => emissive = parse_floats::<3>(value)?,
                // Forward-compatible: an unrecognized key (a future field, an
                // author typo, ...) is ignored, not an error.
                _ => {}
            }
        }

        Ok(MaterialGpu::new(base_color, metallic, roughness, reflectance, emissive, 0))
    }
}

/// Splits `line` on the FIRST `=` or `:`, whichever appears earlier, trimming
/// both sides. Returns `None` if neither separator is present — a malformed
/// line is skipped, not an error (forward-compatible with stray text).
#[inline]
fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let sep = line.find(['=', ':'])?;
    let (key, value) = line.split_at(sep);
    Some((key.trim(), value[1..].trim()))
}

/// Parses exactly `N` whitespace/comma-separated `f32`s from `value`.
///
/// # Errors
/// [`AssetError::Decode`] if `value` does not split into exactly `N` fields,
/// or any field fails to parse as `f32`.
fn parse_floats<const N: usize>(value: &str) -> Result<[f32; N], AssetError> {
    let mut out = [0.0f32; N];
    let mut count = 0usize;
    for field in value.split([',', ' ', '\t']).filter(|s| !s.is_empty()) {
        if count >= N {
            return Err(decode_error(format!(
                "expected {N} float(s), found extra field(s) in '{value}'"
            )));
        }
        out[count] = field
            .parse()
            .map_err(|_| decode_error(format!("invalid float '{field}' in '{value}'")))?;
        count += 1;
    }
    if count != N {
        return Err(decode_error(format!("expected {N} float(s), found {count} in '{value}'")));
    }
    Ok(out)
}

/// Builds an [`AssetError::Decode`] for a malformed `.mat` file — the parser's
/// sole error path.
#[cold]
#[inline(never)]
fn decode_error(msg: String) -> AssetError {
    AssetError::Decode(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 5 fields parse from a well-formed file with comments, blank lines,
    /// leading whitespace, and both `=` and `:` separators.
    #[test]
    fn decode_parses_all_five_fields_with_comments_and_whitespace() {
        let text = "\
# a material
base_color = 0.1, 0.2, 0.3, 1.0

metallic: 0.6
  roughness = 0.25
reflectance = 0.7
emissive = 0.05 0.1 0.2
";
        let mat = RonMaterialLoader::decode(text.as_bytes()).expect("well-formed .mat must decode");

        assert_eq!(mat.base_color, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(mat.metallic(), 0.6);
        assert_eq!(mat.roughness(), 0.25);
        assert_eq!(mat.reflectance(), 0.7);
        assert_eq!(mat.emissive, [0.05, 0.1, 0.2, 0.0]);
    }

    /// A missing recognized key falls back to `MaterialGpu::default`'s value.
    #[test]
    fn decode_missing_field_falls_back_to_default() {
        let text = "base_color = 0.9, 0.9, 0.9, 1.0\n";
        let mat = RonMaterialLoader::decode(text.as_bytes()).expect("a partial .mat must still decode");
        let default = MaterialGpu::default();

        assert_eq!(mat.base_color, [0.9, 0.9, 0.9, 1.0]);
        assert_eq!(mat.metallic(), default.metallic());
        assert_eq!(mat.roughness(), default.roughness());
        assert_eq!(mat.reflectance(), default.reflectance());
        assert_eq!(mat.emissive, default.emissive);
    }

    /// An unparseable float surfaces `AssetError::Decode`.
    #[test]
    fn decode_bad_float_is_decode_error() {
        let text = "metallic = not_a_number\n";
        let result = RonMaterialLoader::decode(text.as_bytes());
        assert!(matches!(result, Err(AssetError::Decode(_))), "got {result:?}");
    }

    /// An unrecognized key is ignored, not an error.
    #[test]
    fn decode_unknown_key_is_ignored() {
        let text = "sheen = 1.0\nmetallic = 0.3\n";
        let mat = RonMaterialLoader::decode(text.as_bytes()).expect("an unknown key must not fail decode");
        assert_eq!(mat.metallic(), 0.3);
    }

    /// A field with the wrong arity (too few / too many floats) errors.
    #[test]
    fn decode_wrong_arity_is_decode_error() {
        let too_few = RonMaterialLoader::decode(b"base_color = 0.1, 0.2\n");
        assert!(matches!(too_few, Err(AssetError::Decode(_))), "got {too_few:?}");

        let too_many = RonMaterialLoader::decode(b"metallic = 0.1, 0.2\n");
        assert!(matches!(too_many, Err(AssetError::Decode(_))), "got {too_many:?}");
    }
}
