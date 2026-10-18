#!/usr/bin/env python3
"""golden_diff.py - render wave PX0's golden / dump comparator (H3 Scope 5; px0/cut.md D-T1).

Stdlib only, Python >= 3.11 (``tomllib``). Subcommands:

  compare A.bmp B.bmp [--max-delta N] [--max-changed-frac F] [--tiles K] [--flat-tiles REF.bmp]
      Two BMPs (24/32-bpp BI_RGB, either row order, 24-bpp rows padded to 4 bytes): the max |Δ|
      per channel, the fraction of changed channel values, the K hottest 16x16 tiles. Exit 1 past
      a given limit. ``--flat-tiles`` grades only the 16x16 tiles (``--flat-tile-size``) of zero
      variance in REF (render wave PX1a D-T1: GL-H1 classes 1 and 3).
  bound --bound hdr-transition --rounding CLASS --format b10g11r11|rgba16f [--flat-tiles REF.bmp]
        [--min-in-bound-frac F] PRE.bmp POST.bmp
      ``--format`` is REQUIRED (PX1a D-T1: a default graded RGBA16F with B10G11R11's 16x wider ε).
      ``--min-in-bound-frac`` passes when at least F of the graded channel values are in bound
      (GL-H1 class 4); without it every one must be.
      AD-19's per-pixel bound for "this pixel went through one lit-format store before the same
      ACES-Hill + OETF tail" (ACES-Hill-specific: PX1a / PX1b). From the pre-move 8-bit value it
      inverts the OETF and the Hill fit in closed form, takes the Jacobian of OETF∘ACES at the
      8-bit cell's corners, and bounds 255·Σ_j |J_ij|·ε_j·|c_j| + 1 LSB, ε the class's step.
  lit-step --k K A.bin B.bin
      Two OBSD scene-input dumps compared per channel within K format steps (pixfmt's step:
      binary16 on a signed line where ±0 are one point); prints the lit-domain changed fraction.
  order-noise D1.bin D2.bin [...]
      The per-pixel spread of >= 2 dumps in format steps: order_noise_steps (rounded up, >= 1) and
      the lit-domain fraction at one step (AD-20's inputs; critique_arch_r5 OQ2).
  eval-post LEG_DIR --post-eval EXE [--receipt NAME] [--display NAME] [--subtexel-bits N]
      Reads the leg's graph receipt (TOML), resolves post_final's binding rows to the leg's dumps
      (a PassInput row = the k-th declared read access of its pass), runs ``post_eval eval``.
  pin --pin P --leg L --bmp B --reference R --max-delta N --max-changed-frac F
      --max-delta-bound X [--bless]
      golden.ps1's tolerance-mode check (exit 0 / 1 over max_delta or max_changed_frac / 4 over
      max_delta_bound = a defect, never re-bless) and bless (refused while a field is PENDING).
  vectors DIR
      Checks this file's mirrors against the Rust-generated vectors (tools/tests/vectors).

Exit codes: 0 pass, 1 over a limit, 2 usage / input error, 4 over max_delta_bound.
"""

from __future__ import annotations

import argparse
import math
import os
import shutil
import struct
import subprocess
import sys

# ---- pixfmt mirror (crates/boyko_shaderdsl/src/pixfmt.rs) ------------------------------------

#: channel -> (mantissa bits, signed)
CHANNELS = {"uf11": (6, False), "uf10": (5, False), "f16": (10, True)}
#: dump VkFormat -> the three channel encodings of its RGB
FORMAT_CHANNELS = {122: ("uf11", "uf11", "uf10"), 97: ("f16", "f16", "f16")}


class StepError(Exception):
    """A value with no place on a channel's step line (NonFinite | Negative)."""


def f32_bits(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", x))[0]


def f32_from_bits(b: int) -> float:
    return struct.unpack("<f", struct.pack("<I", b & 0xFFFFFFFF))[0]


def to_f32(x: float) -> float:
    return f32_from_bits(f32_bits(x))


def inf_code(ch: str) -> int:
    m, _ = CHANNELS[ch]
    return 31 << m


def max_finite_code(ch: str) -> int:
    return inf_code(ch) - 1


def encode(ch: str, x: float, mode: str) -> int:
    """pixfmt::encode for the float channels; ``mode`` is 'ne' or 'tz'. ``x`` is taken as f32."""
    m, signed = CHANNELS[ch]
    bits = f32_bits(x)
    neg = bits >> 31 != 0
    exp = (bits >> 23) & 0xFF
    man = bits & 0x7FFFFF
    inf = 31 << m
    sign_bit = (1 << (m + 5)) if signed else 0
    if exp == 0xFF and man != 0:
        return (sign_bit if neg else 0) | inf | (1 << (m - 1))
    if neg and not signed:
        return 0
    s = sign_bit if neg else 0
    if exp == 0xFF:
        return s | inf
    if exp == 0 and man == 0:
        return s
    sig, e = (man, -126) if exp == 0 else (man | 0x800000, exp - 127)
    if e > 15:
        return s | (inf if mode == "ne" else inf - 1)
    sh = 23 - m if e >= -14 else 23 - m + (-14 - e)
    if sh >= 25:
        q = 0
    else:
        t = sig >> sh
        if mode == "tz":
            q = t
        else:
            rem = sig & ((1 << sh) - 1)
            half = 1 << (sh - 1)
            q = t + 1 if (rem > half or (rem == half and t & 1)) else t
    code = (((e + 15) << m) + q - (1 << m)) if e >= -14 else q
    return s | code


def decode(ch: str, code: int) -> float:
    m, signed = CHANNELS[ch]
    neg = signed and (code >> (m + 5)) & 1 == 1
    e = (code >> m) & 0x1F
    mb = code & ((1 << m) - 1)
    if e == 0:
        mag = mb * 2.0 ** (-14 - m)
    elif e == 31:
        mag = math.inf if mb == 0 else math.nan
    else:
        mag = (1 + mb / (1 << m)) * 2.0 ** (e - 15)
    v = -mag if neg else mag
    return to_f32(v) if not math.isnan(v) else f32_from_bits(0x7FC00000)


def ordinal(ch: str, x: float) -> int:
    """pixfmt::ordinal - the monotone integer line (binary16: ±magnitude, ±0 one point)."""
    m, signed = CHANNELS[ch]
    if math.isnan(x) or math.isinf(x):
        raise StepError("NonFinite")
    if not signed and x < 0.0:
        raise StepError("Negative")
    code = encode(ch, x, "ne")
    mag = code & ((1 << (m + 5)) - 1)
    if mag >= inf_code(ch):
        raise StepError("NonFinite")
    neg = signed and (code >> (m + 5)) & 1 == 1
    return -mag if neg else mag


def steps_between(ch: str, a: float, b: float) -> int:
    return abs(ordinal(ch, a) - ordinal(ch, b))


def step_by(ch: str, x: float, k: int) -> float:
    m, signed = CHANNELS[ch]
    o = ordinal(ch, x)
    hi = max_finite_code(ch)
    lo = -hi if signed else 0
    t = min(max(o + k, lo), hi)
    code = ((1 << (m + 5)) | -t) if t < 0 else t
    return decode(ch, code)


def convert(member: str, ch: str, x: float) -> int:
    """pixfmt::Member::convert for N, Z, N16, Z16."""
    mode = "tz" if member.startswith("Z") else "ne"
    if member.endswith("16"):
        x = decode("f16", encode("f16", x, mode))
    return encode(ch, x, mode)


def pixel_is_nonfinite(rgb) -> bool:
    return any(math.isnan(v) or math.isinf(v) for v in rgb)


# ---- the ACES Hill + OETF tail (crates/boyko_shaderdsl/src/post.rs), f64 ---------------------

ACES_IN = ((0.59719, 0.35458, 0.04823), (0.07600, 0.90834, 0.01566), (0.02840, 0.13383, 0.83777))
ACES_OUT = ((1.60475, -0.53108, -0.07367), (-0.10208, 1.10813, -0.00605), (-0.00327, -0.07276, 1.07602))
ACES_A, ACES_B, ACES_C, ACES_D, ACES_E = 0.0245786, 0.000090537, 0.983729, 0.432951, 0.238081
OETF_EXP = to_f32(1.0 / 2.2)


def mat3(m, v):
    return tuple(m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2] for i in range(3))


def inv3(m):
    (a, b, c), (d, e, f), (g, h, i) = m
    det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    return (
        ((e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det),
        ((f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det),
        ((d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det),
    )


ACES_IN_INV = inv3(ACES_IN)
ACES_OUT_INV = inv3(ACES_OUT)


def aces_hill(c):
    v = mat3(ACES_IN, c)
    q = tuple((x * (x + ACES_A) - ACES_B) / (x * (ACES_C * x + ACES_D) + ACES_E) for x in v)
    return tuple(min(max(y, 0.0), 1.0) for y in mat3(ACES_OUT, q))


def oetf(c):
    return tuple(max(v, 0.0) ** OETF_EXP for v in c)


def tail(c):
    return oetf(aces_hill(c))


def aces_curve_inverse(y: float) -> float:
    """The rational fit's inverse: (1 − yC)v² + (A − yD)v − (B + yE) = 0, the non-negative root."""
    a2 = 1.0 - y * ACES_C
    b1 = ACES_A - y * ACES_D
    c0 = -(ACES_B + y * ACES_E)
    if abs(a2) < 1e-12:
        return -c0 / b1
    disc = max(b1 * b1 - 4.0 * a2 * c0, 0.0)
    return (-b1 + math.sqrt(disc)) / (2.0 * a2)


def tail_inverse(d):
    """Display (0..1, unquantised) -> linear radiance through the OETF and the Hill fit."""
    lin = tuple(max(v, 0.0) ** (1.0 / OETF_EXP) for v in d)
    y = mat3(ACES_OUT_INV, lin)
    v = tuple(aces_curve_inverse(min(max(t, 0.0), 0.999999)) for t in y)
    return mat3(ACES_IN_INV, v)


#: class -> (epsilon per channel of B10 [r, g, b], one-sided)
def class_epsilon(cls: str, fmt: str):
    if cls.startswith("Unmodelled:"):
        ulp = float(cls.split(":", 1)[1])
        base = (2.0 ** -6, 2.0 ** -6, 2.0 ** -5) if fmt == "b10g11r11" else (2.0 ** -10,) * 3
        return tuple(ulp * b for b in base), False
    if fmt == "rgba16f":
        return ((2.0 ** -11,) * 3, False) if cls in ("N", "N16", "Npre") else ((2.0 ** -10,) * 3, True)
    if cls in ("N", "N16", "Npre"):
        return (2.0 ** -7, 2.0 ** -7, 2.0 ** -6), False
    if cls in ("Z", "Zpre"):
        return (2.0 ** -6, 2.0 ** -6, 2.0 ** -5), True
    raise ValueError(f"unknown rounding class {cls!r} (N, N16, Npre, Z, Zpre, Unmodelled:<ulp>)")


def jacobian(c, h=1e-4):
    """d tail / d c by central differences (relative step), 3x3."""
    base = tail(c)
    j = [[0.0] * 3 for _ in range(3)]
    for k in range(3):
        step = max(abs(c[k]) * h, 1e-7)
        lo = list(c)
        hi = list(c)
        lo[k] = max(c[k] - step, 0.0)
        hi[k] = c[k] + step
        tl, th = tail(lo), tail(hi)
        for i in range(3):
            j[i][k] = (th[i] - tl[i]) / (hi[k] - lo[k])
    del base
    return j


def pixel_bound(pre_rgb8, fmt: str, cls: str):
    """AD-19's per-channel bound in display LSB for one pre-move pixel."""
    eps, _one_sided = class_epsilon(cls, fmt)
    best = [0.0, 0.0, 0.0]
    for corner in (-0.5, 0.5):
        d = tuple(min(max((p + corner) / 255.0, 0.0), 1.0) for p in pre_rgb8)
        c = tail_inverse(d)
        j = jacobian(c)
        for i in range(3):
            b = 255.0 * sum(abs(j[i][k]) * eps[k] * abs(c[k]) for k in range(3))
            best[i] = max(best[i], b)
    return [b + 1.0 for b in best]


# ---- BMP ------------------------------------------------------------------------------------


class Bmp:
    def __init__(self, w: int, h: int, rgb: list):
        self.w, self.h, self.rgb = w, h, rgb  # rgb: list of (r, g, b), row-major top-down


def read_bmp(path: str) -> Bmp:
    with open(path, "rb") as f:
        data = f.read()
    if data[:2] != b"BM":
        raise ValueError(f"{path}: not a BMP")
    off = struct.unpack_from("<I", data, 10)[0]
    w, h = struct.unpack_from("<ii", data, 18)
    bpp = struct.unpack_from("<H", data, 28)[0]
    comp = struct.unpack_from("<I", data, 30)[0]
    if comp not in (0, 3) or bpp not in (24, 32):
        raise ValueError(f"{path}: only 24/32-bpp uncompressed BMPs are read (bpp {bpp}, compression {comp})")
    top_down = h < 0
    h = abs(h)
    bpp_bytes = bpp // 8
    row = (w * bpp_bytes + 3) & ~3
    rgb = [None] * (w * h)
    for y in range(h):
        src_y = y if top_down else h - 1 - y
        base = off + src_y * row
        for x in range(w):
            p = base + x * bpp_bytes
            b, g, r = data[p], data[p + 1], data[p + 2]
            rgb[y * w + x] = (r, g, b)
    return Bmp(w, h, rgb)


def write_bmp(path: str, w: int, h: int, rgb: list, bpp: int = 24) -> None:
    """A synthetic BMP for the self-tests (bottom-up, 24 or 32 bpp)."""
    bpp_bytes = bpp // 8
    row = (w * bpp_bytes + 3) & ~3
    data = bytearray()
    for y in range(h - 1, -1, -1):
        line = bytearray()
        for x in range(w):
            r, g, b = rgb[y * w + x]
            line += bytes((b, g, r)) + (b"\xff" if bpp == 32 else b"")
        line += b"\x00" * (row - len(line))
        data += line
    head = b"BM" + struct.pack("<IHHI", 54 + len(data), 0, 0, 54)
    info = struct.pack("<IiiHHIIiiII", 40, w, h, 1, bpp, 0, len(data), 0, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(head + info + data)


def flat_tile_mask(ref: Bmp, tile: int = 16):
    """Per pixel, True iff its ``tile`` x ``tile`` tile of ``ref`` has zero variance (every pixel
    equal) - GL-H1's flat tiles (render wave PX1a D-T1, classes 1 and 3)."""
    flat = {}
    for i, p in enumerate(ref.rgb):
        key = ((i % ref.w) // tile, (i // ref.w) // tile)
        first = flat.get(key)
        if first is None:
            flat[key] = p
        elif first is not False and first != p:
            flat[key] = False
    return [flat[((i % ref.w) // tile, (i // ref.w) // tile)] is not False for i in range(ref.w * ref.h)]


def load_mask(path, tile: int, w: int, h: int):
    """The flat-tile mask of the REF BMP at ``path`` (None = every pixel), sized like the compared pair."""
    if path is None:
        return None
    ref = read_bmp(path)
    if (ref.w, ref.h) != (w, h):
        raise ValueError(f"--flat-tiles reference is {ref.w}x{ref.h}, the compared images {w}x{h}")
    return flat_tile_mask(ref, tile)


def compare_bmps(a: Bmp, b: Bmp, tile: int = 16, mask=None):
    if (a.w, a.h) != (b.w, b.h):
        raise ValueError(f"extents differ: {a.w}x{a.h} vs {b.w}x{b.h}")
    max_d = [0, 0, 0]
    changed = 0
    tiles = {}
    graded = 0
    for i, (pa, pb) in enumerate(zip(a.rgb, b.rgb)):
        if mask is not None and not mask[i]:
            continue
        graded += 1
        for c in range(3):
            d = abs(pa[c] - pb[c])
            if d:
                changed += 1
                max_d[c] = max(max_d[c], d)
                key = ((i % a.w) // tile, (i // a.w) // tile)
                tiles[key] = tiles.get(key, 0) + 1
    frac = changed / (3 * graded) if graded else 0.0
    return max_d, frac, sorted(tiles.items(), key=lambda kv: -kv[1])


# ---- OBSD dumps (crates/boyko_shaderdsl/src/dump.rs mirror) ---------------------------------


class Dump:
    def __init__(self, fmt, w, h, layers, texel_bytes, texels):
        self.format, self.w, self.h, self.layers, self.texel_bytes, self.texels = fmt, w, h, layers, texel_bytes, texels

    def rgb(self, i: int):
        t = self.texels[i * self.texel_bytes : (i + 1) * self.texel_bytes]
        if self.format == 122:
            word = struct.unpack("<I", t)[0]
            return (decode("uf11", word & 0x7FF), decode("uf11", (word >> 11) & 0x7FF), decode("uf10", (word >> 22) & 0x3FF))
        if self.format == 97:
            return tuple(decode("f16", struct.unpack_from("<H", t, 2 * c)[0]) for c in range(3))
        raise ValueError(f"VkFormat {self.format} is not a lit-domain format (122, 97)")


def read_dump(path: str) -> Dump:
    with open(path, "rb") as f:
        data = f.read()
    if len(data) < 32:
        raise ValueError(f"{path}: shorter than the 32-byte header")
    magic, fmt, w, h, _mip, layers, _frame, tb = struct.unpack_from("<8I", data, 0)
    if magic != struct.unpack("<I", b"OBSD")[0]:
        raise ValueError(f"{path}: bad magic {magic:#x}")
    if len(data) - 32 != w * h * layers * tb:
        raise ValueError(f"{path}: texel bytes {len(data) - 32} != {w}x{h}x{layers}x{tb}")
    return Dump(fmt, w, h, layers, tb, data[32:])


def lit_step(a: Dump, b: Dump, k: int):
    """(passes, changed fraction at >= 1 step, worst steps, non-finite pixels skipped)."""
    if (a.format, a.w, a.h) != (b.format, b.w, b.h):
        raise ValueError("the two dumps differ in format or extent")
    chans = FORMAT_CHANNELS[a.format]
    n = a.w * a.h
    changed = worst = skipped = 0
    for i in range(n):
        pa, pb = a.rgb(i), b.rgb(i)
        if pixel_is_nonfinite(pa) or pixel_is_nonfinite(pb):
            skipped += 1
            continue
        for c in range(3):
            s = steps_between(chans[c], pa[c], pb[c])
            if s:
                changed += 1
                worst = max(worst, s)
    return worst <= k, changed / (3 * n), worst, skipped


def order_noise(dumps: list):
    """(order_noise_steps, lit-domain fraction at one step)."""
    chans = FORMAT_CHANNELS[dumps[0].format]
    n = dumps[0].w * dumps[0].h
    worst = 0
    changed = 0
    for i in range(n):
        px = [d.rgb(i) for d in dumps]
        if any(pixel_is_nonfinite(p) for p in px):
            continue
        for c in range(3):
            o = [ordinal(chans[c], p[c]) for p in px]
            spread = max(o) - min(o)
            worst = max(worst, spread)
            if spread:
                changed += 1
    return max(1, worst), changed / (3 * n)


# ---- eval-post ----------------------------------------------------------------------------------


def load_receipt(path: str):
    import tomllib  # Python >= 3.11

    with open(path, "rb") as f:
        return tomllib.load(f)


def post_eval_bindings(exe: str):
    out = subprocess.run([exe, "bindings"], capture_output=True, text=True, check=True).stdout
    rows = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 3 and not line.startswith("#"):
            rows.append({"name": parts[0], "kind": parts[1], "source": parts[2]})
    return rows


#: VkAccessFlags write bits (shader, colour, depth, transfer, host, memory): an access with none of
#: them is a READ access for the resolution rule below.
WRITE_ACCESS_BITS = 0x40 | 0x100 | 0x400 | 0x1000 | 0x4000 | 0x10000


def resolve_bindings(receipt: dict, rows: list, leg_dir: str):
    """Maps each fetching row to a dump file. ``resource:<name>`` names the dump directly; a
    PassInput row ``pass:<P>:<k>`` resolves to the k-th declared READ access of pass P in the
    receipt (the rule PX1a pins in its receipt test). A missing row, pass, access or dump is a named
    error, never a zero."""
    passes = {p.get("name"): p.get("index") for p in receipt.get("pass", [])}
    accesses = receipt.get("access", [])
    resources = {r.get("index"): r.get("name") for r in receipt.get("resource", [])}
    out = {}
    for row in rows:
        src = row["source"]
        if src == "none":
            continue
        if src.startswith("resource:"):
            name = src.split(":", 1)[1]
        elif src.startswith("pass:"):
            _, pname, k = src.split(":")
            if pname not in passes:
                raise LookupError(f"binding {row['name']}: the receipt declares no pass `{pname}`")
            reads = [
                a for a in accesses
                if a.get("pass") == passes[pname] and int(a.get("access", 0)) & WRITE_ACCESS_BITS == 0
            ]
            if int(k) >= len(reads):
                raise LookupError(
                    f"binding {row['name']}: pass `{pname}` declares {len(reads)} read access(es), "
                    f"the row is fetching row #{k} (count mismatch)"
                )
            res = reads[int(k)].get("res")
            if res not in resources:
                raise LookupError(f"binding {row['name']}: access names resource index {res}, absent from the receipt")
            name = resources[res]
        else:
            raise LookupError(f"binding {row['name']}: unknown dump source `{src}`")
        path = os.path.join(leg_dir, f"{name}.bin")
        if not os.path.exists(path):
            raise LookupError(f"binding {row['name']}: no dump `{path}` for resource `{name}`")
        out[row["name"]] = path
    return out


def receipt_words(receipt: dict):
    cfg = receipt.get("config", {})
    words = []
    for i in range(8):
        key = f"post_word{i}"
        if key not in cfg:
            raise LookupError(f"the receipt's [config] has no `{key}` (PX1a writes post::POST_FINAL_RECEIPT_KEYS)")
        words.append(int(cfg[key]))
    return words


# ---- the CLI ---------------------------------------------------------------------------------


def cmd_compare(a):
    try:
        x, y = read_bmp(a.a), read_bmp(a.b)
        mask = load_mask(a.flat_tiles, a.flat_tile_size, x.w, x.h)
        max_d, frac, tiles = compare_bmps(x, y, mask=mask)
    except (OSError, ValueError) as e:
        print(f"golden_diff compare: {e}", file=sys.stderr)
        return 2
    scope = "all" if mask is None else f"flat_tiles={sum(mask)}px"
    print(f"max_delta r={max_d[0]} g={max_d[1]} b={max_d[2]} changed_frac={frac:.6f} scope={scope}")
    for (tx, ty), n in tiles[: a.tiles]:
        print(f"tile ({tx},{ty}) changed={n}")
    if a.max_delta is not None and max(max_d) > a.max_delta:
        return 1
    if a.max_changed_frac is not None and frac > a.max_changed_frac:
        return 1
    return 0


def cmd_bound(a):
    if a.bound != "hdr-transition":
        print(f"golden_diff bound: unknown bound `{a.bound}`", file=sys.stderr)
        return 2
    try:
        pre, post = read_bmp(a.pre), read_bmp(a.post)
        class_epsilon(a.rounding, a.format)
        if (pre.w, pre.h) != (post.w, post.h):
            raise ValueError("extents differ")
        mask = load_mask(a.flat_tiles, a.flat_tile_size, pre.w, pre.h)
    except (OSError, ValueError) as e:
        print(f"golden_diff bound: {e}", file=sys.stderr)
        return 2
    if a.min_in_bound_frac is not None and not 0.0 <= a.min_in_bound_frac <= 1.0:
        print("golden_diff bound: --min-in-bound-frac must lie in [0, 1]", file=sys.stderr)
        return 2
    fails = 0
    graded = 0
    worst = 0.0
    for i, (p, q) in enumerate(zip(pre.rgb, post.rgb)):
        if mask is not None and not mask[i]:
            continue
        graded += 3
        b = pixel_bound(p, a.format, a.rounding)
        for c in range(3):
            d = abs(q[c] - p[c])
            if d > b[c]:
                fails += 1
                worst = max(worst, d - b[c])
    if graded == 0:
        print("golden_diff bound: no channel value graded (no flat tile in the reference)", file=sys.stderr)
        return 2
    in_frac = 1.0 - fails / graded
    scope = "all" if mask is None else f"flat_tiles={graded // 3}px"
    print(
        f"bound hdr-transition rounding={a.rounding} format={a.format} failing_frac={fails / graded:.6f} "
        f"in_bound_frac={in_frac:.6f} worst_excess={worst:.3f} scope={scope}"
    )
    if a.min_in_bound_frac is not None:
        return 0 if in_frac >= a.min_in_bound_frac else 1
    return 1 if fails else 0


def cmd_lit_step(a):
    try:
        ok, frac, worst, skipped = lit_step(read_dump(a.a), read_dump(a.b), a.k)
    except (OSError, ValueError, StepError) as e:
        print(f"golden_diff lit-step: {e}", file=sys.stderr)
        return 2
    print(f"lit-step k={a.k} worst_steps={worst} changed_frac_lit={frac:.6f} nonfinite_skipped={skipped}")
    return 0 if ok else 1


def cmd_order_noise(a):
    if len(a.dumps) < 2:
        print("golden_diff order-noise: needs >= 2 dumps", file=sys.stderr)
        return 2
    try:
        steps, frac = order_noise([read_dump(p) for p in a.dumps])
    except (OSError, ValueError, StepError) as e:
        print(f"golden_diff order-noise: {e}", file=sys.stderr)
        return 2
    flag = " FLAG: > 4, report to the orchestrator before bless" if steps > 4 else ""
    print(f"order_noise_steps={steps} changed_frac_lit={frac:.6f}{flag}")
    return 0


def cmd_eval_post(a):
    try:
        receipt = load_receipt(os.path.join(a.leg_dir, a.receipt))
        words = receipt_words(receipt)
        rows = post_eval_bindings(a.post_eval)
        binds = resolve_bindings(receipt, rows, a.leg_dir)
    except (OSError, ValueError, LookupError, subprocess.CalledProcessError) as e:
        print(f"golden_diff eval-post: {e}", file=sys.stderr)
        return 2
    cmd = [a.post_eval, "eval", "--words", ",".join(str(w) for w in words)]
    for name, path in binds.items():
        cmd += ["--bind", f"{name}={path}"]
    cmd += ["--display", os.path.join(a.leg_dir, a.display)]
    if a.subtexel_bits is not None:
        cmd += ["--subtexel-bits", str(a.subtexel_bits)]
    return subprocess.run(cmd).returncode


def cmd_pin(a):
    fields = {
        "max_delta": a.max_delta,
        "max_changed_frac": a.max_changed_frac,
        "max_delta_bound": a.max_delta_bound,
    }
    if a.bless:
        pending = [k for k, v in fields.items() if v == "PENDING"]
        if pending:
            print(f"golden_diff pin {a.pin}/{a.leg}: bless refused, measured field(s) PENDING: {', '.join(pending)}", file=sys.stderr)
            return 2
        os.makedirs(os.path.dirname(os.path.abspath(a.reference)), exist_ok=True)
        shutil.copyfile(a.bmp, a.reference)
        print(f"golden_diff pin {a.pin}/{a.leg}: blessed {a.reference}")
        return 0
    try:
        max_delta = int(a.max_delta)
        max_frac = float(a.max_changed_frac)
        bound = float(a.max_delta_bound)
        max_d, frac, _ = compare_bmps(read_bmp(a.bmp), read_bmp(a.reference))
    except (OSError, ValueError) as e:
        print(f"golden_diff pin {a.pin}/{a.leg}: {e}", file=sys.stderr)
        return 2
    observed = max(max_d)
    print(f"pin {a.pin}/{a.leg}: observed max_delta={observed} changed_frac={frac:.6f} (max_delta={max_delta}, max_changed_frac={max_frac}, max_delta_bound={bound})")
    if observed > bound:
        print(f"pin {a.pin}/{a.leg}: observed {observed} > max_delta_bound {bound} - a DEFECT (a race, a lost barrier), never re-bless", file=sys.stderr)
        return 4
    if observed > max_delta or frac > max_frac:
        return 1
    return 0


def check_vectors(directory: str) -> list:
    """Every mismatch between this file's mirrors and the Rust vectors (empty = in sync)."""
    bad = []
    with open(os.path.join(directory, "pixfmt_steps.txt"), encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            lhs, want = line.split(" -> ")
            p = lhs.split()
            try:
                if p[0] == "encode":
                    got = f"{encode(p[1], f32_from_bits(int(p[2], 16)), p[3]):x}"
                elif p[0] == "decode":
                    got = f"{f32_bits(decode(p[1], int(p[2], 16))):08x}"
                elif p[0] == "convert":
                    got = f"{convert(p[1], p[2], f32_from_bits(int(p[3], 16))):x}"
                elif p[0] == "steps":
                    got = str(steps_between(p[1], f32_from_bits(int(p[2], 16)), f32_from_bits(int(p[3], 16))))
                elif p[0] == "step_by":
                    got = f"{f32_bits(step_by(p[1], f32_from_bits(int(p[2], 16)), int(p[3]))):08x}"
                else:
                    got = f"<unknown op {p[0]}>"
            except StepError as e:
                got = f"err:{e}"
            if got != want:
                bad.append(f"pixfmt_steps.txt:{n}: {lhs} -> {want}, mirror gives {got}")
    with open(os.path.join(directory, "aces_hill.txt"), encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            lhs, want = line.split(" -> ")
            p = lhs.split()
            c = tuple(f32_from_bits(int(v, 16)) for v in p[1:4])
            w = tuple(f32_from_bits(int(v, 16)) for v in want.split())
            got = aces_hill(c) if p[0] == "aces" else oetf(c)
            for gv, wv in zip(got, w):
                if abs(gv - wv) > 4e-6 + 2e-5 * abs(wv):
                    bad.append(f"aces_hill.txt:{n}: {lhs} -> {w}, mirror gives {got}")
                    break
    return bad


def cmd_vectors(a):
    bad = check_vectors(a.dir)
    for b in bad[:20]:
        print(b, file=sys.stderr)
    print(f"vectors: {len(bad)} mismatch(es)")
    return 1 if bad else 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="golden_diff.py", description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("compare")
    p.add_argument("a")
    p.add_argument("b")
    p.add_argument("--max-delta", type=int)
    p.add_argument("--max-changed-frac", type=float)
    p.add_argument("--tiles", type=int, default=8)
    p.add_argument("--flat-tiles", help="REF.bmp: grade only the tiles of zero variance in REF")
    p.add_argument("--flat-tile-size", type=int, default=16)
    p.set_defaults(fn=cmd_compare)
    p = sub.add_parser("bound")
    p.add_argument("--bound", required=True)
    p.add_argument("--rounding", required=True)
    # Required (render wave PX1a D-T1): a default grades one format's output with the other's epsilon.
    p.add_argument("--format", required=True, choices=("b10g11r11", "rgba16f"))
    p.add_argument("--flat-tiles", help="REF.bmp: grade only the tiles of zero variance in REF")
    p.add_argument("--flat-tile-size", type=int, default=16)
    p.add_argument("--min-in-bound-frac", type=float, help="pass when at least this fraction of channel values is in bound")
    p.add_argument("pre")
    p.add_argument("post")
    p.set_defaults(fn=cmd_bound)
    p = sub.add_parser("lit-step")
    p.add_argument("--k", type=int, required=True)
    p.add_argument("a")
    p.add_argument("b")
    p.set_defaults(fn=cmd_lit_step)
    p = sub.add_parser("order-noise")
    p.add_argument("dumps", nargs="+")
    p.set_defaults(fn=cmd_order_noise)
    p = sub.add_parser("eval-post")
    p.add_argument("leg_dir")
    p.add_argument("--post-eval", required=True)
    p.add_argument("--receipt", default="receipt.toml")
    p.add_argument("--display", default="display.bin")
    p.add_argument("--subtexel-bits", type=int)
    p.set_defaults(fn=cmd_eval_post)
    p = sub.add_parser("pin")
    for k in ("--pin", "--leg", "--bmp", "--reference", "--max-delta", "--max-changed-frac", "--max-delta-bound"):
        p.add_argument(k, required=True)
    p.add_argument("--bless", action="store_true")
    p.set_defaults(fn=cmd_pin)
    p = sub.add_parser("vectors")
    p.add_argument("dir")
    p.set_defaults(fn=cmd_vectors)
    a = ap.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
