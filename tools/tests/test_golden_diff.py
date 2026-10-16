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
            self.assertEqual(gd.main(["bound", "--bound", "hdr-transition", "--rounding", cls, a, b]), 0, cls)

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
        self.assertEqual(gd.main(["bound", "--bound", "hdr-transition", "--rounding", "N", a, b]), 1)

    def test_an_unknown_class_is_a_usage_error(self):
        a, b = self.pair(self.b10_store("ne"))
        self.assertEqual(gd.main(["bound", "--bound", "hdr-transition", "--rounding", "Q", a, b]), 2)


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
