"""Self-tests of tools/golden_diff.py (render wave PX0, H3 Scope 5 Tests; px0/cut.md PX0-7).

Run: ``python -m unittest discover -s tools/tests -p "test_*.py"`` from the repository root, or
through ``cargo test -p boyko-engine --test golden_diff_selftest`` (tests/golden_diff_selftest.rs),
which is how ``cargo test --workspace`` runs it.
"""

import os
import random
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

import golden_diff as gd  # noqa: E402

VECTORS = os.path.join(HERE, "vectors")


def tmp(name):
    d = tempfile.mkdtemp(prefix="golden_diff_")
    return os.path.join(d, name)


def write_dump(path, fmt, w, h, rgb):
    """A synthetic OBSD dump of RGB values encoded in ``fmt`` (122 or 97), nearest-even."""
    out = bytearray(struct.pack("<8I", struct.unpack("<I", b"OBSD")[0], fmt, w, h, 0, 1, 0, 4 if fmt == 122 else 8))
    for r, g, b in rgb:
        if fmt == 122:
            word = gd.encode("uf11", r, "ne") | (gd.encode("uf11", g, "ne") << 11) | (gd.encode("uf10", b, "ne") << 22)
            out += struct.pack("<I", word)
        else:
            out += struct.pack("<4H", gd.encode("f16", r, "ne"), gd.encode("f16", g, "ne"), gd.encode("f16", b, "ne"), 0x3C00)
    with open(path, "wb") as f:
        f.write(out)


def q8(v):
    return int(min(max(v, 0.0), 1.0) * 255.0 + 0.5)


class Compare(unittest.TestCase):
    def setUp(self):
        rnd = random.Random(7)
        self.w, self.h = 17, 9  # odd width: 24-bpp rows carry padding
        self.px = [(rnd.randrange(256), rnd.randrange(256), rnd.randrange(256)) for _ in range(self.w * self.h)]

    def bmp(self, px, bpp=24):
        p = tmp(f"x{bpp}.bmp")
        gd.write_bmp(p, self.w, self.h, px, bpp)
        return p

    def test_identical_is_zero(self):
        a = self.bmp(self.px)
        self.assertEqual(gd.main(["compare", a, a, "--max-delta", "0"]), 0)

    def test_one_channel_plus_two(self):
        px = list(self.px)
        r, g, b = px[5]
        px[5] = (r, min(g + 2, 255) if g < 254 else g - 2, b)
        md, frac, _ = gd.compare_bmps(gd.read_bmp(self.bmp(self.px)), gd.read_bmp(self.bmp(px)))
        self.assertEqual(max(md), 2)
        self.assertAlmostEqual(frac, 1 / (3 * self.w * self.h))
        self.assertEqual(gd.main(["compare", self.bmp(self.px), self.bmp(px), "--max-delta", "1"]), 1)

    def test_channel_order_mutation_is_caught(self):
        swapped = [(b, g, r) for r, g, b in self.px]
        md, _, _ = gd.compare_bmps(gd.read_bmp(self.bmp(self.px)), gd.read_bmp(self.bmp(swapped)))
        self.assertGreater(md[0], 0)
        self.assertGreater(md[2], 0)
        self.assertEqual(md[1], 0)

    def test_odd_width_rows_round_trip_in_both_depths(self):
        for bpp in (24, 32):
            self.assertEqual(gd.read_bmp(self.bmp(self.px, bpp)).rgb, self.px, f"{bpp} bpp")


class Bound(unittest.TestCase):
    """AD-19's hdr-transition bound on a synthetic pre/post pair."""

    def setUp(self):
        rnd = random.Random(11)
        self.w, self.h = 24, 16
        self.r = [tuple(rnd.uniform(0.0, 3.0) for _ in range(3)) for _ in range(self.w * self.h)]

    def pair(self, post_fn):
        pre = [tuple(q8(v) for v in gd.tail(c)) for c in self.r]
        post = [tuple(q8(v) for v in post_fn(c)) for c in self.r]
        a, b = tmp("pre.bmp"), tmp("post.bmp")
        gd.write_bmp(a, self.w, self.h, pre)
        gd.write_bmp(b, self.w, self.h, post)
        return a, b

    def b10_store(self, mode):
        def store(c):
            return gd.tail(
                (
                    gd.decode("uf11", gd.encode("uf11", c[0], mode)),
                    gd.decode("uf11", gd.encode("uf11", c[1], mode)),
                    gd.decode("uf10", gd.encode("uf10", c[2], mode)),
                )
            )

        return store

    def test_one_store_passes_under_n_and_z(self):
        for cls, mode in (("N", "ne"), ("Z", "tz")):
            a, b = self.pair(self.b10_store(mode))
            self.assertEqual(gd.main(["bound", "--bound", "hdr-transition", "--format", "b10g11r11", "--rounding", cls, a, b]), 0, cls)

    def test_the_tail_twice_fails_on_most_pixels(self):
        a, b = self.pair(lambda c: gd.tail(gd.tail(c)))
        pre, post = gd.read_bmp(a), gd.read_bmp(b)
        fails = sum(
            1
            for p, q in zip(pre.rgb, post.rgb)
            if any(abs(q[c] - p[c]) > gd.pixel_bound(p, "b10g11r11", "N")[c] for c in range(3))
        )
        self.assertGreater(fails / len(pre.rgb), 0.9)

    def test_background_base_uninverted_fails_on_the_background(self):
        base = (0.26, 0.32, 0.42)  # a display-referred constant
        pre = [tuple(q8(v) for v in base)] * (self.w * self.h)
        post = [tuple(q8(v) for v in gd.tail(base))] * (self.w * self.h)
        a, b = tmp("pre.bmp"), tmp("post.bmp")
        gd.write_bmp(a, self.w, self.h, pre)
        gd.write_bmp(b, self.w, self.h, post)
        self.assertEqual(gd.main(["bound", "--bound", "hdr-transition", "--format", "b10g11r11", "--rounding", "N", a, b]), 1)

    def test_an_unknown_class_is_a_usage_error(self):
        a, b = self.pair(self.b10_store("ne"))
        self.assertEqual(gd.main(["bound", "--bound", "hdr-transition", "--format", "b10g11r11", "--rounding", "Q", a, b]), 2)


class Flags(unittest.TestCase):
    """Render wave PX1a D-T1 (ruling 67 W3): ``bound --format`` is required (its old default
    ``b10g11r11`` grades RGBA16F with a 16x wider epsilon - a forgotten flag was a wrong green),
    ``--flat-tiles REF.bmp`` restricts ``compare`` / ``bound`` to the tiles of zero variance in REF
    (GL-H1 classes 1 and 3), and ``bound --min-in-bound-frac F`` passes when at least F of the
    channel values are in bound (class 4). Red-first on the unmodified tool."""

    def setUp(self):
        # Two 16x16 tiles side by side: the left one flat in REF, the right one varied.
        self.w, self.h = 32, 16
        rnd = random.Random(23)
        self.ref = [
            (90, 120, 150) if (i % self.w) < 16 else (rnd.randrange(256), rnd.randrange(256), rnd.randrange(256))
            for i in range(self.w * self.h)
        ]
        self.pre = [(100, 110, 120)] * (self.w * self.h)

    def bmp(self, px, name):
        p = tmp(name)
        gd.write_bmp(p, self.w, self.h, px)
        return p

    def right_tile_moved(self):
        return [p if (i % self.w) < 16 else (min(p[0] + 100, 255), p[1], p[2]) for i, p in enumerate(self.pre)]

    def test_bound_requires_format(self):
        a = self.bmp(self.pre, "pre.bmp")
        with self.assertRaises(SystemExit) as cm:
            gd.main(["bound", "--bound", "hdr-transition", "--rounding", "N", a, a])
        self.assertEqual(cm.exception.code, 2)

    def test_flat_tiles_restrict_the_bound(self):
        ref, a, b = self.bmp(self.ref, "ref.bmp"), self.bmp(self.pre, "pre.bmp"), self.bmp(self.right_tile_moved(), "post.bmp")
        base = ["bound", "--bound", "hdr-transition", "--format", "rgba16f", "--rounding", "Z"]
        self.assertEqual(gd.main(base + [a, b]), 1, "the moved right tile fails everywhere")
        self.assertEqual(gd.main(base + ["--flat-tiles", ref, a, b]), 0, "only REF's flat left tile is graded")

    def test_flat_tiles_restrict_compare(self):
        ref, a, b = self.bmp(self.ref, "ref.bmp"), self.bmp(self.pre, "pre.bmp"), self.bmp(self.right_tile_moved(), "post.bmp")
        self.assertEqual(gd.main(["compare", "--max-delta", "0", a, b]), 1)
        self.assertEqual(gd.main(["compare", "--max-delta", "0", "--flat-tiles", ref, a, b]), 0)

    def test_min_in_bound_frac(self):
        a = self.bmp(self.pre, "pre.bmp")
        n = 3 * self.w * self.h

        def moved(k):
            px = list(self.pre)
            for i in range(k):
                r, g, b = px[i * 7]
                px[i * 7] = (255, g, b)
            return self.bmp(px, f"post{k}.bmp")

        base = ["bound", "--bound", "hdr-transition", "--format", "rgba16f", "--rounding", "Z"]
        few, many = moved(5), moved(20)
        self.assertLess(5 / n, 0.005)
        self.assertGreater(20 / n, 0.005)
        self.assertEqual(gd.main(base + [a, few]), 1, "without the flag every channel must be in bound")
        self.assertEqual(gd.main(base + ["--min-in-bound-frac", "0.995", a, few]), 0)
        self.assertEqual(gd.main(base + ["--min-in-bound-frac", "0.995", a, many]), 1)


class ClassOneForm(unittest.TestCase):
    """Ruling 84 (1): GL-H1 class 1 (``compare --flat-tiles AAOFF --max-delta 2``) graded the flat
    pixels on a tile border that TAA's silhouette dilation legitimately moves (red on the pre-move
    tree too), and passed a leg whose AA-off frame has no flat tile at all (graded 0 px). The form
    gains ``--edge-margin K`` (flat pixels within K px of an edge of AAOFF are not graded) and a
    minimum graded area (``--min-graded N``; an empty mask is always vacuous): exit 3, never 0.
    Red-first on the unmodified tool."""

    def setUp(self):
        # Three 16x16 tile columns x two tile rows. Column 0 is an object (non-flat: a texture);
        # columns 1-2 are background, flat, with two silhouette seams that end exactly on a tile
        # border, both sides flat:
        # - a HORIZONTAL seam at y = 15|16: the top background tiles are (90, 120, 150), the bottom
        #   ones darker;
        # - a VERTICAL seam at x = 31|32 in the bottom tile row: (40, 60, 80) left, (70, 90, 110)
        #   right.
        # The margin's column pass is the only one that sees the horizontal seam's value change and
        # its row pass the only one that sees the vertical seam's (ruling 85 (1), review W1: with
        # the horizontal seam alone, a row pass that compared the wrong pixels passed every test).
        self.w, self.h = 48, 32
        rnd = random.Random(31)
        self.ref = []
        for i in range(self.w * self.h):
            x, y = i % self.w, i // self.w
            if x < 16:
                self.ref.append((rnd.randrange(256), rnd.randrange(256), rnd.randrange(256)))
            elif y < 16:
                self.ref.append((90, 120, 150))
            else:
                self.ref.append((40, 60, 80) if x < 32 else (70, 90, 110))

    def bmp(self, px, name):
        p = tmp(name)
        gd.write_bmp(p, self.w, self.h, px)
        return p

    def moved(self, pixels, by=30):
        px = list(self.ref)
        for x, y in pixels:
            r, g, b = px[y * self.w + x]
            px[y * self.w + x] = (min(r + by, 255), g, b)
        return px

    def run_compare(self, taa, *flags):
        ref = self.bmp(self.ref, "aaoff.bmp")
        return gd.main(["compare", "--flat-tiles", ref, "--max-delta", "2", *flags, ref, self.bmp(taa, "taa.bmp")])

    def test_a_margin_of_zero_reds_a_dilated_edge(self):
        # TAA dilates the object's silhouette one pixel into the flat background tile next to it.
        dilated = self.moved([(16, y) for y in range(self.h)])
        self.assertEqual(self.run_compare(dilated), 1, "no margin: the dilated column is graded")
        self.assertEqual(self.run_compare(dilated, "--edge-margin", "0"), 1, "a margin of 0 is the flat mask itself")
        self.assertEqual(self.run_compare(dilated, "--edge-margin", "1"), 0, "the column lies 1 px from the object tile")

    def test_a_seam_between_two_flat_tiles_is_an_edge(self):
        # The silhouette that ends exactly on a tile border: both tiles flat, the edge between them
        # (the horizontal seam, seen only by the margin's column pass).
        dilated = self.moved([(x, 15) for x in range(16, self.w)] + [(x, 16) for x in range(16, self.w)])
        self.assertEqual(self.run_compare(dilated, "--edge-margin", "0"), 1)
        self.assertEqual(self.run_compare(dilated, "--edge-margin", "1"), 0, "rows 15 and 16 lie 1 px from the other flat side")

    def test_a_vertical_seam_between_two_flat_tiles_is_an_edge(self):
        # The same silhouette turned 90 degrees: the vertical seam at x = 31|32 in the bottom tile
        # row, seen only by the margin's row pass (review W1: a row pass reading row 0's pixels, or
        # ignoring their values, graded these columns).
        dilated = self.moved([(31, y) for y in range(16, self.h)] + [(32, y) for y in range(16, self.h)])
        self.assertEqual(self.run_compare(dilated, "--edge-margin", "0"), 1)
        self.assertEqual(self.run_compare(dilated, "--edge-margin", "1"), 0, "columns 31 and 32 lie 1 px from the other flat side")

    def test_the_margin_still_grades_the_interior(self):
        # A move 3 px or more from every edge (an exposure / history error in a flat region) is red
        # whatever the margin that excuses the silhouettes.
        interior = self.moved([(32, 7)])
        self.assertEqual(self.run_compare(interior, "--edge-margin", "2"), 1)
        # (32, 7)'s nearest differing pixel is (32, 16), 9 px away: graded up to a margin of 8.
        self.assertEqual(self.run_compare(interior, "--edge-margin", "8"), 1)
        self.assertEqual(self.run_compare(interior, "--edge-margin", "9"), 0)

    def test_the_margin_is_the_chebyshev_window(self):
        # edge_margin_mask against the brute-force definition: graded iff every pixel within
        # Chebyshev k is flat and equal to it.
        ref = gd.read_bmp(self.bmp(self.ref, "aaoff.bmp"))
        flat = gd.flat_tile_mask(ref, 16)
        for k in range(0, 5):
            got = gd.edge_margin_mask(ref, flat, k)
            for i in range(self.w * self.h):
                x, y = i % self.w, i // self.w
                want = flat[i] and all(
                    flat[yy * self.w + xx] and ref.rgb[yy * self.w + xx] == ref.rgb[i]
                    for yy in range(max(y - k, 0), min(y + k, self.h - 1) + 1)
                    for xx in range(max(x - k, 0), min(x + k, self.w - 1) + 1)
                )
                self.assertEqual(got[i], want, f"k={k} pixel ({x}, {y})")

    def test_an_all_edge_frame_is_red_vacuous(self):
        # Every tile textured: no flat tile, nothing graded - identical images must still red.
        rnd = random.Random(37)
        noise = [(rnd.randrange(256), rnd.randrange(256), rnd.randrange(256)) for _ in range(self.w * self.h)]
        ref = self.bmp(noise, "noise.bmp")
        self.assertEqual(gd.main(["compare", "--flat-tiles", ref, "--max-delta", "2", ref, ref]), 3)
        self.assertEqual(gd.main(["compare", "--flat-tiles", ref, "--max-delta", "2", "--edge-margin", "2", ref, ref]), 3)

    def test_min_graded_is_the_vacuity_floor(self):
        # By hand at margin 2 (the object tile ends at x = 15, the horizontal seam is at y = 15|16,
        # the vertical one at x = 31|32 below it, the frame border excludes nothing):
        # - the top band keeps x 18..47 (30 columns) x rows 0..13 (14) = 420 px;
        # - the bottom band keeps x 18..29 and 34..47 (12 + 14 columns) x rows 18..31 (14) = 364 px;
        # 784 px in all.
        same = list(self.ref)
        self.assertEqual(self.run_compare(same, "--edge-margin", "2", "--min-graded", "784"), 0)
        self.assertEqual(self.run_compare(same, "--edge-margin", "2", "--min-graded", "785"), 3)

    def test_usage_errors(self):
        ref = self.bmp(self.ref, "aaoff.bmp")
        self.assertEqual(gd.main(["compare", "--edge-margin", "1", ref, ref]), 2, "a margin needs --flat-tiles")
        self.assertEqual(gd.main(["compare", "--flat-tiles", ref, "--edge-margin", "-1", ref, ref]), 2)
        self.assertEqual(gd.main(["compare", "--flat-tiles", ref, "--min-graded", "0", ref, ref]), 2)



def scene(w, h):
    """A grey scene with no symmetry: brightness grows with the distance to an off-centre point."""
    cx, cy = 0.3 * w, 0.65 * h
    return [(v, v, v) for y in range(h) for x in range(w) for v in [min(255, int(400.0 * ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5 / w))]]


class SceneAnchor(unittest.TestCase):
    """Ruling 85 (3): GL-H1 class 1 cannot tell a scene from no scene - two identical blank frames
    pass the flat-tile form (review OQ1). ``compare --scene-anchor ANCHOR`` runs the device receipts'
    whole-scene check (crates/boyko_app/tests/px1a_graph_receipt.rs) on B: B's Rec. 709 luma and
    ANCHOR's, box-reduced to ``SCENE_EDGE``^2, rank alike at rho >= 0.9, or exit 1. The small
    fixtures shrink ``SCENE_EDGE`` to 8 (16^2 frames, the device check's 2:1 box). Red-first on the
    tool without the flag."""

    EDGE = 8

    def bmp(self, px, name, w=16):
        p = tmp(name)
        gd.write_bmp(p, w, len(px) // w, px)
        return p

    def anchored(self, frame, anchor, *flags):
        with mock.patch.object(gd, "SCENE_EDGE", self.EDGE):
            return gd.main(["compare", *flags, "--scene-anchor", anchor, frame, frame])

    def test_spearman_mirrors_the_device_check(self):
        self.assertEqual(gd.ranks([3.0, 1.0, 3.0, 2.0]), [2.5, 0.0, 2.5, 1.0], "0-based, ties at their average rank")
        # Ranks a = [0, 1.5, 1.5, 3], b = [0, 2, 1, 3]: sab 4.5, saa 4.5, sbb 5 -> 3 / sqrt(10).
        self.assertAlmostEqual(gd.spearman([1.0, 2.0, 2.0, 3.0], [1.0, 3.0, 2.0, 4.0]), 3.0 / 10.0**0.5, places=12)
        v = [0.3, 0.1, 0.7, 0.2, 0.9]
        self.assertEqual(gd.spearman(v, v), 1.0)
        self.assertEqual(gd.spearman(v, [-x for x in v]), -1.0)
        # The device check's MG guard: a constant side carries no ranking and reads 0 - read as 1,
        # a blank frame would pass as the scene.
        self.assertEqual(gd.spearman(v, [0.5] * 5), 0.0)
        self.assertEqual(gd.spearman([0.5] * 5, v), 0.0)
        self.assertEqual(gd.spearman([0.5] * 5, [0.5] * 5), 0.0)

    def test_reduced_luma_is_the_box_average(self):
        # Rec. 709 weights on pixfmt's f32 UNORM8 decode (51 / 255 is the f32 0.2, not the f64 one).
        self.assertEqual([gd.display_luma(p) for p in ((255, 0, 0), (0, 255, 0), (0, 0, 255))], [0.2126, 0.7152, 0.0722])
        self.assertEqual(gd.display_luma((51, 0, 0)), 0.2126 * 0.20000000298023224)
        self.assertEqual(gd.display_luma((0, 0, 0)), 0.0)
        px = [(0, 0, 0), (255, 255, 255), (51, 51, 51), (51, 51, 51)] * 2 + [(255, 0, 0)] * 8
        img = gd.Bmp(4, 4, px)
        got = gd.reduced_luma(img, 2)
        lum = gd.display_luma
        self.assertEqual(got[0], (lum(px[0]) + lum(px[1]) + lum(px[4]) + lum(px[5])) / 4.0)
        self.assertEqual(got[1], (lum(px[2]) + lum(px[3]) + lum(px[6]) + lum(px[7])) / 4.0)
        self.assertEqual(got[2:], [lum((255, 0, 0))] * 2)
        self.assertEqual(gd.reduced_luma(img, 4), [lum(p) for p in px], "1:1 is the per-pixel luma")
        for bad in (gd.Bmp(4, 2, px[:8]), gd.Bmp(4, 4, px)):
            with self.assertRaises(ValueError):
                gd.reduced_luma(bad, 3 if bad.h == 4 else 2)

    def test_a_frame_of_the_anchors_scene_passes(self):
        anchor = scene(16, 16)
        a = self.bmp(anchor, "anchor.bmp")
        self.assertEqual(gd.scene_anchor(gd.read_bmp(a), gd.read_bmp(a), self.EDGE), 1.0)
        self.assertEqual(self.anchored(a, a), 0, "the anchor itself (the pre-move run)")
        # A render of the same scene a step or two off on a quarter of its pixels (h1's rounding).
        rnd = random.Random(41)
        near = [(min(v + rnd.randrange(3), 255),) * 3 if rnd.random() < 0.25 else p for p in anchor for v in [p[0]]]
        self.assertNotEqual(near, anchor)
        self.assertEqual(self.anchored(self.bmp(near, "near.bmp"), a), 0)

    def test_a_blank_or_never_written_frame_reds(self):
        a = self.bmp(scene(16, 16), "anchor.bmp")
        for name, px in (("black", [(0, 0, 0)] * 256), ("grey", [(128, 128, 128)] * 256)):
            self.assertEqual(self.anchored(self.bmp(px, f"{name}.bmp"), a), 1, f"a blank {name} frame")
        rnd = random.Random(43)
        garbage = [(rnd.randrange(256), rnd.randrange(256), rnd.randrange(256)) for _ in range(256)]
        self.assertEqual(self.anchored(self.bmp(garbage, "garbage.bmp"), a), 1, "a never-written frame of garbage")
        # The device check's own red case: the scene's top-left quarter, magnified to the frame.
        anchor = scene(16, 16)
        quarter = [anchor[(y // 2) * 16 + x // 2] for y in range(16) for x in range(16)]
        self.assertEqual(self.anchored(self.bmp(quarter, "quarter.bmp"), a), 1, "the top-left quarter")
        # A blank anchor (a never-written pre-move frame) passes nothing, not even itself.
        blank = self.bmp([(0, 0, 0)] * 256, "blank.bmp")
        self.assertEqual(self.anchored(a, blank), 1)
        self.assertEqual(self.anchored(blank, blank), 1)
        # The anchor judges B (class 1's TAA frame), not A (its AA-off reference).
        with mock.patch.object(gd, "SCENE_EDGE", self.EDGE):
            self.assertEqual(gd.main(["compare", "--scene-anchor", a, a, blank]), 1)
            self.assertEqual(gd.main(["compare", "--scene-anchor", a, blank, a]), 0)

    def test_two_identical_blank_frames_red_under_the_class_one_form(self):
        # Review OQ1's construction at full size: two uniform 512^2 frames of one colour under the
        # class-1 form pass it (the whole frame is one flat area); the scene anchor reds them,
        # whether the anchor is a scene or blank as well.
        w = 2 * gd.SCENE_EDGE
        blank = self.bmp([(96, 96, 96)] * (w * w), "blank512.bmp", w)
        form = ["compare", "--flat-tiles", blank, "--edge-margin", "2", "--min-graded", "57196", "--max-delta", "2"]
        self.assertEqual(gd.main(form + [blank, blank]), 0, "the gap ruling 85 (3) closes")
        self.assertEqual(gd.main(form + ["--scene-anchor", blank, blank, blank]), 1)
        anchor = self.bmp(scene(w, w), "scene512.bmp", w)
        self.assertEqual(gd.main(form + ["--scene-anchor", anchor, blank, blank]), 1)
        self.assertEqual(gd.main(["compare", "--scene-anchor", anchor, anchor, anchor]), 0, "the scene itself at full size")

    def test_a_misfit_frame_is_an_input_error(self):
        a = self.bmp(scene(16, 16), "anchor.bmp")
        self.assertEqual(self.anchored(self.bmp(scene(12, 12), "odd.bmp", 12), a), 2, "12^2 is no whole multiple of 8^2")
        self.assertEqual(self.anchored(self.bmp(scene(16, 8), "wide.bmp"), a), 2, "not square")
        self.assertEqual(self.anchored(a, self.bmp(scene(12, 12), "odd_anchor.bmp", 12)), 2)

class LitStep(unittest.TestCase):
    def setUp(self):
        self.w, self.h = 8, 4
        self.base = [(1.0 + i / 64.0, 0.5, 0.25) for i in range(self.w * self.h)]

    def dumps(self, steps, n_changed):
        a, b = tmp("a.bin"), tmp("b.bin")
        moved = [
            (gd.step_by("uf11", r, steps) if i < n_changed else r, g, bl) for i, (r, g, bl) in enumerate(self.base)
        ]
        write_dump(a, 122, self.w, self.h, self.base)
        write_dump(b, 122, self.w, self.h, moved)
        return a, b

    def test_one_step_passes_at_k1_three_fail(self):
        a, b = self.dumps(1, 5)
        self.assertEqual(gd.main(["lit-step", "--k", "1", a, b]), 0)
        a, b = self.dumps(3, 5)
        self.assertEqual(gd.main(["lit-step", "--k", "1", a, b]), 1)

    def test_reported_fraction_is_the_hand_count(self):
        a, b = self.dumps(1, 5)
        ok, frac, worst, _ = gd.lit_step(gd.read_dump(a), gd.read_dump(b), 1)
        self.assertTrue(ok)
        self.assertEqual(worst, 1)
        self.assertAlmostEqual(frac, 5 / (3 * self.w * self.h))

    def test_signed_zero_is_no_step_on_rgba16f(self):
        a, b = tmp("a.bin"), tmp("b.bin")
        write_dump(a, 97, 2, 1, [(0.0, 1.0, -0.0), (2.0, 1.0, 1.0)])
        write_dump(b, 97, 2, 1, [(-0.0, 1.0, 0.0), (2.0, 1.0, 1.0)])
        ok, frac, worst, _ = gd.lit_step(gd.read_dump(a), gd.read_dump(b), 0)
        self.assertTrue(ok)
        self.assertEqual((frac, worst), (0.0, 0))


class OrderNoise(unittest.TestCase):
    def test_spread_equals_the_hand_spread(self):
        base = [(1.0, 1.0, 1.0)] * 4
        paths = []
        for k in (0, 2, -1):
            p = tmp(f"n{k}.bin")
            write_dump(p, 122, 2, 2, [(gd.step_by("uf11", 1.0, k) if i == 0 else 1.0, 1.0, 1.0) for i in range(4)])
            paths.append(p)
        steps, frac = gd.order_noise([gd.read_dump(p) for p in paths])
        self.assertEqual(steps, 3)
        self.assertAlmostEqual(frac, 1 / 12)
        del base


class EvalPost(unittest.TestCase):
    def leg(self, with_scene=True, reads=1):
        d = tempfile.mkdtemp(prefix="golden_diff_leg_")
        accesses = "".join(f"\n[[access]]\npass = 0\nres = {i}\naccess = 32\n" for i in range(reads))
        receipt = (
            "[config]\n" + "".join(f"post_word{i} = {i}\n" for i in range(8))
            + '\n[[resource]]\nindex = 0\nname = "lit"\n\n[[resource]]\nindex = 1\nname = "bloom"\n'
            + '\n[[pass]]\nindex = 0\nname = "post_final"\n'
            + accesses
        )
        with open(os.path.join(d, "receipt.toml"), "w", encoding="utf-8") as f:
            f.write(receipt)
        if with_scene:
            write_dump(os.path.join(d, "lit.bin"), 122, 1, 1, [(1.0, 1.0, 1.0)])
        fake = os.path.join(d, "fake_post_eval.py")
        with open(fake, "w", encoding="utf-8") as f:
            f.write(
                "import sys\n"
                "if sys.argv[1] == 'bindings':\n"
                "    print('gScene tex2_load pass:post_final:0')\n"
                "    print('gDisplay store none')\n"
                "    sys.exit(0)\n"
                "args = sys.argv[2:]\n"
                "assert '--words' in args and '0,1,2,3,4,5,6,7' in args, args\n"
                "assert any(a.startswith('gScene=') and a.endswith('lit.bin') for a in args), args\n"
                "sys.exit(0)\n"
            )
        wrapper = os.path.join(d, "fake_post_eval.cmd" if os.name == "nt" else "fake_post_eval.sh")
        with open(wrapper, "w", encoding="utf-8") as f:
            if os.name == "nt":
                f.write(f'@"{sys.executable}" "{fake}" %*\n')
            else:
                f.write(f'#!/bin/sh\nexec "{sys.executable}" "{fake}" "$@"\n')
        os.chmod(wrapper, 0o755)
        return d, wrapper

    def test_resolution_against_a_synthetic_receipt(self):
        d, exe = self.leg()
        self.assertEqual(gd.main(["eval-post", d, "--post-eval", exe]), 0)

    def test_a_missing_dump_is_a_named_error(self):
        d, exe = self.leg(with_scene=False)
        self.assertEqual(gd.main(["eval-post", d, "--post-eval", exe]), 2)

    def test_a_count_mismatch_is_a_named_error(self):
        d, exe = self.leg(reads=0)
        with self.assertRaises(LookupError):
            gd.resolve_bindings(gd.load_receipt(os.path.join(d, "receipt.toml")), gd.post_eval_bindings(exe), d)


class Pin(unittest.TestCase):
    def setUp(self):
        self.ref = tmp("ref.bmp")
        gd.write_bmp(self.ref, 4, 4, [(10, 20, 30)] * 16, 32)

    def bmp(self, delta):
        p = tmp("leg.bmp")
        px = [(10, 20, 30)] * 16
        px[3] = (10 + delta, 20, 30)
        gd.write_bmp(p, 4, 4, px, 32)
        return p

    def run_pin(self, bmp, md="1", frac="0.5", bound="3", bless=False):
        args = ["pin", "--pin", "p", "--leg", "software", "--bmp", bmp, "--reference", self.ref,
                "--max-delta", md, "--max-changed-frac", frac, "--max-delta-bound", bound]
        return gd.main(args + (["--bless"] if bless else []))

    def test_exit_codes(self):
        self.assertEqual(self.run_pin(self.bmp(0)), 0)
        self.assertEqual(self.run_pin(self.bmp(1)), 0)
        self.assertEqual(self.run_pin(self.bmp(2)), 1)
        self.assertEqual(self.run_pin(self.bmp(5)), 4)

    def test_bless_refuses_pending_and_writes_the_reference(self):
        self.assertEqual(self.run_pin(self.bmp(2), md="PENDING", bless=True), 2)
        self.assertEqual(self.run_pin(self.bmp(2), bless=True), 0)
        self.assertEqual(gd.read_bmp(self.ref).rgb[3], (12, 20, 30))


class Vectors(unittest.TestCase):
    def test_the_mirrors_reproduce_every_rust_vector(self):
        self.assertTrue(os.path.exists(os.path.join(VECTORS, "pixfmt_steps.txt")), "the Rust vectors are missing")
        bad = gd.check_vectors(VECTORS)
        self.assertEqual(bad, [], "\n".join(bad[:10]))


if __name__ == "__main__":
    unittest.main()
