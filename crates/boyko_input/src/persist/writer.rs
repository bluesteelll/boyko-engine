//! The canonical `.keys` serializer (plan §9.3).
//!
//! Emits a byte-stable canonical form: `version` first, then one `[context …]`
//! header, then one `action = spec, ...` line per action in declaration order,
//! with chord parts **sorted** and composite params in a **fixed order**. The
//! canonical form is a fixed point of the parser: `parse ∘ serialize` is
//! byte-identical on canonical output (the round-trip property, plan §9.3 / §14
//! I5). User comments are dropped on rewrite (documented; matches every surveyed
//! engine) — only the engine header is re-emitted.
//!
//! This is a **cold** path (save-on-quit / save-on-rebind); it allocates the
//! output `String` freely. The per-frame input path never touches it.

use crate::action::actionlike::Actionlike;
use crate::action::map::{AxisMode, BindSpec, ClashStrategy, InputMap, InputRef};
use crate::persist::grammar::KEYS_FORMAT_VERSION;
use crate::persist::keyname::{name_of, push_u32, write_mouse_token};
use crate::raw::keycode::KeyCode;

/// The single context name v1 emits. The contexts/priority-stack model (plan §6
/// V3) lands in a later round; until then the whole map is one canonical
/// context, named `gameplay` to match the plan's example (§9.2).
const DEFAULT_CONTEXT: &str = "gameplay";

/// Serializes `map` into `out` in canonical form (plan §9.3).
///
/// `out` is cleared first, so the function is idempotent: calling it again with
/// the same map yields the same bytes. The emitted text re-parses to a map with
/// identical bindings (the round-trip property).
pub fn save_keys<A: Actionlike>(map: &InputMap<A>, out: &mut String) {
    out.clear();

    out.push_str("# boyko-engine keybinds — \"action = primary, secondary, ...\"\n");
    out.push_str("version = ");
    push_u32(KEYS_FORMAT_VERSION, out);
    out.push('\n');
    out.push('\n');

    out.push('[');
    out.push_str(DEFAULT_CONTEXT);
    out.push_str(" clash=");
    out.push_str(clash_token(map.clash()));
    out.push_str("]\n");

    for i in 0..map.action_count() {
        let Some(action) = A::from_index(i) else {
            continue;
        };
        out.push_str(action.name());
        out.push_str(" = ");
        write_action_specs(map.bindings_at(i), out);
        out.push('\n');
    }
}

/// Convenience wrapper: serialize into a freshly-allocated `String`.
pub fn keys_to_string<A: Actionlike>(map: &InputMap<A>) -> String {
    let mut out = String::new();
    save_keys(map, &mut out);
    out
}

/// The canonical token for a [`ClashStrategy`].
fn clash_token(c: ClashStrategy) -> &'static str {
    match c {
        ClashStrategy::PrioritizeLongest => "longest",
        ClashStrategy::AllowAll => "all",
    }
}

/// Writes an action's specs as a comma-joined list (`spec, spec, ...`). An action
/// with no bindings serializes as `none` (the explicit-unbind canonical form).
fn write_action_specs(specs: &[BindSpec], out: &mut String) {
    // Filter out `None` markers; an action that is *only* `None` (or empty)
    // serializes as the single `none` token so the override-delta round-trips.
    let mut wrote_any = false;
    for spec in specs {
        if matches!(spec, BindSpec::None) {
            continue;
        }
        if wrote_any {
            out.push_str(", ");
        }
        write_spec(spec, out);
        wrote_any = true;
    }
    if !wrote_any {
        out.push_str("none");
    }
}

/// Writes one [`BindSpec`] in canonical form.
fn write_spec(spec: &BindSpec, out: &mut String) {
    match spec {
        BindSpec::Key(code) => write_key(*code, out),
        BindSpec::Mouse(button) => write_mouse_token(*button, out),
        BindSpec::Chord { keys, len } => write_chord(&keys[..*len as usize], out),
        BindSpec::Axis1 { neg, pos, dz } => {
            out.push_str("axis1(neg=");
            write_input_ref(*neg, out);
            out.push_str(", pos=");
            write_input_ref(*pos, out);
            out.push_str(", dz=");
            write_f32(*dz, out);
            out.push(')');
        }
        BindSpec::Axis2 {
            up,
            down,
            left,
            right,
            dz,
            mode,
        } => {
            out.push_str("axis2(up=");
            write_input_ref(*up, out);
            out.push_str(", down=");
            write_input_ref(*down, out);
            out.push_str(", left=");
            write_input_ref(*left, out);
            out.push_str(", right=");
            write_input_ref(*right, out);
            out.push_str(", dz=");
            write_f32(*dz, out);
            out.push_str(", mode=");
            out.push_str(axis_mode_token(*mode));
            out.push(')');
        }
        // The reserved gamepad seam: v1 carries no stick fields, so it serializes
        // as a canonical empty `stick()` token, which the parser accepts and
        // round-trips (preserved, runtime-ignored — plan §13).
        BindSpec::Stick => out.push_str("stick()"),
        BindSpec::None => out.push_str("none"),
    }
}

/// Writes a single key as its canonical name, or `raw(0xNN)` for an
/// [`KeyCode::Unidentified`] (the lossless exotic-key form, plan §9.3).
fn write_key(code: KeyCode, out: &mut String) {
    match name_of(code) {
        Some(name) => out.push_str(name),
        None => match code {
            KeyCode::Unidentified(n) => {
                out.push_str("raw(0x");
                push_hex_u32(n, out);
                out.push(')');
            }
            // `name_of` returns `None` only for `Unidentified`; any other miss is
            // an unnamed canonical variant (a `KEY_NAMES` table gap). Such a key
            // would serialize to `raw(0x<dense_index>)` and re-parse to a
            // *different* value (`Unidentified`), silently corrupting it — so a
            // table gap is a build-time bug. It is caught loudly in debug here and
            // by `every_canonical_key_is_named` (test); release degrades to `raw`
            // rather than panicking or dropping the key.
            other => {
                debug_assert!(
                    false,
                    "invariant: every canonical KeyCode must have a KEY_NAMES entry (table gap)"
                );
                out.push_str("raw(0x");
                push_hex_u32(other.dense_index().unwrap_or(0) as u32, out);
                out.push(')');
            }
        },
    }
}

/// Writes a chord as `A+B+C` with parts **sorted** by canonical token so the
/// form is deterministic (plan §9.3 — chord parts sorted on serialize).
fn write_chord(keys: &[KeyCode], out: &mut String) {
    // Render each key to its token, sort the tokens, then join with `+`. Sorting
    // by the rendered token (not by discriminant) keeps the canonical order
    // textual and stable across a discriminant reorder.
    let mut parts: Vec<String> = keys
        .iter()
        .map(|&k| {
            let mut s = String::new();
            write_key(k, &mut s);
            s
        })
        .collect();
    parts.sort_unstable();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push('+');
        }
        out.push_str(p);
    }
}

/// Writes an [`InputRef`] axis leg (a key name or a mouse token).
fn write_input_ref(r: InputRef, out: &mut String) {
    match r {
        InputRef::Key(code) => write_key(code, out),
        InputRef::Mouse(button) => write_mouse_token(button, out),
    }
}

/// The canonical [`AxisMode`] token (`radial` for normalized, `raw` for
/// unnormalized — matches the parser's accepted forms).
fn axis_mode_token(mode: AxisMode) -> &'static str {
    match mode {
        AxisMode::DigitalNormalized => "radial",
        AxisMode::DigitalRaw => "raw",
    }
}

/// Writes an `f32` in a canonical, parser-round-trippable form.
///
/// Uses Rust's shortest round-trip `f32` formatting (`{}`), which the parser's
/// `str::parse::<f32>` consumes losslessly, and normalizes an integral value to
/// always carry a `.0` so `dz=0` re-emits identically (`0.0`) rather than
/// drifting between `0` and `0.0` across save cycles.
fn write_f32(v: f32, out: &mut String) {
    use core::fmt::Write as _;
    let start = out.len();
    // `{}` is the shortest representation that parses back to the same f32.
    let _ = write!(out, "{v}");
    // Ensure a decimal point so the canonical form is stable (e.g. `0` → `0.0`).
    if out[start..].bytes().all(|b| b != b'.' && b != b'e' && b != b'E') {
        out.push_str(".0");
    }
}

/// Appends `n` as lowercase hex digits (no `0x` prefix), no temporary alloc.
fn push_hex_u32(n: u32, out: &mut String) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 8];
    let mut i = buf.len();
    let mut v = n;
    while v > 0 {
        i -= 1;
        let d = (v & 0xF) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + (d - 10) };
        v >>= 4;
    }
    // SAFETY: `buf[i..]` holds only ASCII hex-digit bytes written above, so the
    // slice is valid UTF-8.
    out.push_str(unsafe { core::str::from_utf8_unchecked(&buf[i..]) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::keycode::MouseButton;

    #[test]
    fn write_key_named_and_raw() {
        let mut s = String::new();
        write_key(KeyCode::Space, &mut s);
        assert_eq!(s, "Space");
        s.clear();
        write_key(KeyCode::Unidentified(0x56), &mut s);
        assert_eq!(s, "raw(0x56)");
    }

    #[test]
    fn chord_parts_sorted() {
        let mut s = String::new();
        // Input order S then LCtrl; canonical output is sorted: "LCtrl+S".
        write_chord(&[KeyCode::KeyS, KeyCode::ControlLeft], &mut s);
        assert_eq!(s, "LCtrl+S");
    }

    #[test]
    fn mouse_back_forward_canonical() {
        let mut s = String::new();
        write_mouse_token(MouseButton::Back, &mut s);
        assert_eq!(s, "MouseBack");
        s.clear();
        write_mouse_token(MouseButton::Other(9), &mut s);
        assert_eq!(s, "MouseOther(9)");
    }

    #[test]
    fn f32_carries_decimal_point() {
        let mut s = String::new();
        write_f32(0.0, &mut s);
        assert_eq!(s, "0.0");
        s.clear();
        write_f32(0.15, &mut s);
        assert_eq!(s, "0.15");
    }
}
