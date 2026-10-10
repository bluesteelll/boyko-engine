#!/usr/bin/env python3
"""Convert OpenGL-convention tangent-space normal maps to this engine's DIRECTX canonical.

This engine's canonical normal-map convention is **DirectX** (green-down: +G = a slope
facing image-BOTTOM) -- the Unreal Engine convention. The engine NEVER re-signs a normal
map at runtime: neither the texture loader nor the TEXTURED shaders negate the green
channel. Every `normal.png` an engine material folder loads is REQUIRED to already be
DirectX-convention.

Third-party PBR packs almost universally ship the OPENGL convention (green-up; vendors
label these `*-ogl`). Those are converted ONCE, offline, by this script -- the sign lives
in the asset pipeline, not in the renderer.

The conversion is a green-channel inversion (`g -> 255 - g`), which is the exact algebraic
negation of the decoded tangent-space Y axis, with no precision loss:

    decode(255 - g) = 2*(255 - g)/255 - 1 = -(2*g/255 - 1) = -decode(g)

Only the GREEN channel is touched: red (X), blue (Z) and alpha carry data that must not
change. The conversion is its own inverse -- running this twice restores the original, so
NEVER run it on an already-DirectX map.

NEVER TRUST THE FILENAME. Vendors mislabel: `assets/materials/alley-brick-wall` ships its
normal map as `alley-brick-wall_normal-ogl.png` and it is measurably **DirectX**, while the
same vendor's `industrial-walls_normal-ogl.png` really is OpenGL. Packs from ONE vendor came
in MIXED conventions under identical `-ogl` naming. Converting a DirectX map "because the
name says ogl" inverts its relief -- exactly the bug this script exists to avoid.

MEASURE instead, with `--detect`: correlate the normal map's green channel against the pack's
HEIGHT map (the vendor's own ground truth for the relief). In image coordinates (y DOWN) the
normal implied by a height field is `(-dH/dx, -dH/dy, 1)`, so:

    corr(green, -dH/dy) > 0  =>  DirectX  (green encodes the y-DOWN axis)
    corr(green, -dH/dy) < 0  =>  OpenGL   (green encodes the y-UP axis)

`--detect` also prints a CONTROL correlation, `corr(red, -dH/dx)`, which is convention-
INDEPENDENT and must come out strongly POSITIVE; if it does not, the height map and the
normal map are not aligned (different bake/scale/crop) and the verdict is not trustworthy.

Usage:
    python scripts/normal_ogl_to_dx.py --detect <normal.png> <height.png>  # measure, write nothing
    python scripts/normal_ogl_to_dx.py --check  <normal.png>               # stats, write nothing
    python scripts/normal_ogl_to_dx.py <normal.png> [<normal.png> ...]     # convert OGL -> DX

Each converted file is rewritten in place after a `<name>.ogl.bak` backup is made (unless
one already exists, so a re-run never clobbers the pristine source).
"""

import shutil
import sys
from pathlib import Path

from PIL import Image


def green_stats(img: Image.Image) -> tuple[float, int, int]:
    """Returns (mean, min, max) of the green channel."""
    g = img.getchannel("G")
    lo, hi = g.getextrema()
    hist = g.histogram()
    total = sum(hist)
    mean = sum(v * c for v, c in enumerate(hist)) / total if total else 0.0
    return mean, lo, hi


def convert(path: Path, check_only: bool) -> int:
    if not path.is_file():
        print(f"  SKIP  {path} (not found)")
        return 1

    img = Image.open(path)
    fmt_mode = img.mode
    # Normal maps are RGB/RGBA 8-bit; normalize to RGBA so channel ops are uniform, then
    # write back in the original mode to avoid gratuitously changing the file's shape.
    rgba = img.convert("RGBA")
    mean, lo, hi = green_stats(rgba)
    print(f"  {path.name}: {img.size[0]}x{img.size[1]} mode={fmt_mode} "
          f"green mean={mean:.1f} min={lo} max={hi}")

    if check_only:
        return 0

    r, g, b, a = rgba.split()
    g = g.point(lambda v: 255 - v)
    out = Image.merge("RGBA", (r, g, b, a))
    if fmt_mode == "RGB":
        out = out.convert("RGB")

    backup = path.with_suffix(path.suffix + ".ogl.bak")
    if not backup.exists():
        shutil.copy2(path, backup)
        print(f"    backup -> {backup.name}")
    else:
        print(f"    backup exists ({backup.name}), not overwritten")

    out.save(path)
    new_mean, new_lo, new_hi = green_stats(Image.open(path).convert("RGBA"))
    print(f"    CONVERTED OpenGL -> DirectX: green mean {mean:.1f} -> {new_mean:.1f} "
          f"(min {new_lo}, max {new_hi})")
    return 0


def detect(normal_path: Path, height_path: Path) -> int:
    """Measures a normal map's convention against the pack's HEIGHT map (ground truth).

    Prints the DirectX/OpenGL verdict plus a convention-independent control correlation
    that validates the two maps are actually aligned. See this module's doc.
    """
    try:
        import numpy as np
    except ImportError:
        print("  --detect needs numpy (pip install numpy)")
        return 2
    for p in (normal_path, height_path):
        if not p.is_file():
            print(f"  SKIP  {p} (not found)")
            return 1

    h_img = Image.open(height_path).convert("L")
    n_img = Image.open(normal_path).convert("RGB")
    if h_img.size != n_img.size:
        h_img = h_img.resize(n_img.size)
    H = np.asarray(h_img, dtype=np.float64)
    N = np.asarray(n_img, dtype=np.float64)

    # Central differences; y increases DOWNWARD (image row order).
    dHdy = np.zeros_like(H)
    dHdy[1:-1, :] = (H[2:, :] - H[:-2, :]) / 2.0
    dHdx = np.zeros_like(H)
    dHdx[:, 1:-1] = (H[:, 2:] - H[:, :-2]) / 2.0

    ny = N[:, :, 1] / 255.0 * 2.0 - 1.0
    nx = N[:, :, 0] / 255.0 * 2.0 - 1.0
    # Only where the surface actually slopes: flat areas carry no signal, only noise.
    mask = np.abs(dHdy) > 2.0
    if int(mask.sum()) < 1000:
        print("  too few sloped samples -- height map is flat or misaligned; no verdict")
        return 1

    def corr(a, b):
        a = a[mask] - a[mask].mean()
        b = b[mask] - b[mask].mean()
        return float((a * b).sum() / np.sqrt((a * a).sum() * (b * b).sum()))

    c_dx = corr(ny, -dHdy)
    c_ctl = corr(nx, -dHdx)
    verdict = "DirectX" if c_dx > 0 else "OpenGL"
    print(f"  {normal_path.name} vs {height_path.name}: sloped={int(mask.sum())}")
    print(f"    corr(green, -dH/dy) = {c_dx:+.3f}  -> {verdict}")
    print(f"    corr(red,   -dH/dx) = {c_ctl:+.3f}  (control: must be strongly POSITIVE)")
    if c_ctl < 0.2:
        print("    WARNING: weak control -- maps may be misaligned; verdict NOT trustworthy")
    if verdict == "DirectX":
        print("    => already canonical; do NOT convert.")
    else:
        print("    => convert: python scripts/normal_ogl_to_dx.py "
              f"{normal_path}")
    return 0


def main(argv: list[str]) -> int:
    flags = {a for a in argv[1:] if a.startswith("--")}
    args = [a for a in argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        return 2

    print("normal_ogl_to_dx: engine canonical = DirectX (green-down, Unreal convention)")

    if "--detect" in flags:
        if len(args) != 2:
            print("  --detect takes exactly: <normal.png> <height.png>")
            return 2
        return detect(Path(args[0]), Path(args[1]))

    check_only = "--check" in flags
    rc = 0
    for raw in args:
        rc |= convert(Path(raw), check_only)
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv))
