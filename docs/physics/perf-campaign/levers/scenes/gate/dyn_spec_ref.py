#!/usr/bin/env python3
"""Dynamic parity scenes: an INDEPENDENT Python derivation of the canonical scene dump.

The Rust spec (crates/boyko_physics/benches/jolt_parity_pyramid/dyn_spec.rs) and Jolt's C++ hunk
derive the same text; this file is written from the record (01-DESIGN.md, "The programs"), not
from either, so the known-answer pins of tests/dyn_scenes_spec.rs are two derivations agreeing.

usage:
  dyn_spec_ref.py                      print `<program> <fnv1a64 of the dump> <bytes> <lines>` for all four
  dyn_spec_ref.py --write DIR          also write DIR/<program>.dump
  dyn_spec_ref.py --program P --print  the dump of P on stdout
  dyn_spec_ref.py --mutant NAME        the same, under a named red-first mutant of the spec
"""
import argparse
import math
import struct
import sys

M64 = (1 << 64) - 1

MUTANT = None


def f32_hex(x):
    return '%08x' % struct.unpack('<I', struct.pack('<f', x))[0]


def f32(x):
    """Round a Python float to f32 (exact for every value this file converts)."""
    return struct.unpack('<f', struct.pack('<f', x))[0]


class SplitMix64:
    def __init__(self, seed):
        self.s = seed & M64

    def next(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & M64
        z = self.s
        mul1 = 0xBF58476D1CE4E5B8 if MUTANT == 'mul1' else 0xBF58476D1CE4E5B9
        z = ((z ^ (z >> 30)) * mul1) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        return z ^ (z >> 31)

    def below(self, n):
        t = ((1 << 64) - n) % n
        while True:
            z = self.next()
            if z >= t:
                return z % n

    def range(self, lo, hi):
        return lo + self.below(hi - lo + 1)


JT_BODIES = 1240
DT = 1.0 / 60.0
G = -9.81
FRICTION = 0.2

FLOOR = ((50, 1, 50), (0, -1, 0))
ARENA = [((1, 31, 52), (51, 30, 0)), ((1, 31, 52), (-51, 30, 0)), ((52, 31, 1), (0, 30, 51)),
         ((52, 31, 1), (0, 30, -51)), ((52, 1, 52), (0, 62, 0))]
STATICS = {
    'none': [FLOOR],
    'kick': [FLOOR],
    'shoot': [FLOOR] + ARENA,
    'slide': [FLOOR, ((1, 31, 52), (31, 30, 0))] + ARENA[1:],
}
STEPS = {'none': 500, 'kick': 800, 'shoot': 800, 'slide': 500}
EVENTS = {'none': 0, 'kick': 24, 'shoot': 60, 'slide': 0}


def gravity_hex(program):
    if program == 'slide':
        # 9.81 (sin t, -cos t, 0), tan t = 1/2, rounded once to f32; y is exactly -2x in f32.
        gx = f32(9.81 / math.sqrt(5.0))
        return [f32_hex(gx), f32_hex(-2.0 * gx), f32_hex(0.0)]
    return [f32_hex(0.0), f32_hex(G), f32_hex(0.0)]


def jt_lattice():
    out = []
    for i in range(15):
        for j in range(i // 2, 15 - (i + 1) // 2):
            for k in range(i // 2, 15 - (i + 1) // 2):
                odd = 1.0 if i & 1 else 0.0
                out.append((-15.0 + 2.0 * j + odd, 1.0 + 2.5 * i, -15.0 + 2.0 * k + odd))
    assert len(out) == JT_BODIES
    return out


def kick_program():
    rng = SplitMix64(0x6b69636b00000001)
    events = []
    kmax = 319 if MUTANT == 'krange' else 320
    for e in range(24):
        idx = []
        while len(idx) < 62:
            c = rng.below(JT_BODIES)
            if c not in idx:
                idx.append(c)
        kicks = []
        for i in idx:
            while True:
                k = (rng.range(-320, kmax), rng.range(-320, kmax), rng.range(-320, kmax))
                n2 = k[0] * k[0] + k[1] * k[1] + k[2] * k[2]
                lo = 4095 if MUTANT == 'shell4095' else 4096
                if lo <= n2 <= 102400:
                    break
            kicks.append((i, k))
        kicks.sort()
        events.append((200 + 25 * e, kicks))
    return events


def trunc_div(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


def shoot_program():
    rng = SplitMix64(0x73686f6f00000001)
    out = []
    prev = None
    for k in range(60):
        while True:
            a = rng.range(-544, 544)
            b = rng.range(-544, 544)
            r2 = a * a + b * b
            if not (480 * 480 <= r2 <= 544 * 544):
                continue
            if prev is not None and MUTANT != 'nosep':
                dot = a * prev[0] + b * prev[1]
                pr2 = prev[0] ** 2 + prev[1] ** 2
                if not (dot <= 0 or 4 * dot * dot <= 3 * r2 * pr2):
                    continue
            break
        hl = rng.range(640, 768)
        ht = rng.range(64, 448)
        s = rng.range(15, 40)
        d = (-a, ht - hl, -b)
        n = math.isqrt(d[0] ** 2 + d[1] ** 2 + d[2] ** 2)
        if MUTANT == 'floordiv':
            v = tuple((s * 256 * di) // n for di in d)
        else:
            v = tuple(trunc_div(s * 256 * di, n) for di in d)
        pos = (a - 16, hl, b - 16)
        out.append((200 + 10 * k, JT_BODIES + k, pos, v))
        prev = (a, b)
    return out


def body_fields(i, p, v):
    h = [f32_hex(x) for x in p]
    h += [f32_hex(0.0)] * 3 + [f32_hex(1.0)]          # rotation (x, y, z, w) = identity
    h += [f32_hex(x) for x in v]
    h += [f32_hex(0.0)] * 3                            # angular velocity
    h += [f32_hex(1.0)] * 3                            # half-extents
    h += [f32_hex(FRICTION), f32_hex(0.0), f32_hex(0.0)]  # friction, restitution, convex radius
    h += [f32_hex(1.0)] * 4                            # inverse mass and inertia ratios
    h += [f32_hex(0.0)] * 2                            # damping
    return '%d %s 0 0' % (i, ' '.join(h))              # can_sleep 0, ccd 0


def dump(program):
    lines = ['boyko-dyn-scene v1', 'program ' + program, 'steps %d' % STEPS[program],
             'dt ' + f32_hex(DT), 'gravity ' + ' '.join(gravity_hex(program))]
    for half, c in STATICS[program]:
        h = [f32_hex(float(x)) for x in half] + [f32_hex(float(x)) for x in c]
        h += [f32_hex(0.0)] * 3 + [f32_hex(1.0)] + [f32_hex(FRICTION), f32_hex(0.0), f32_hex(0.0)]
        lines.append('static ' + ' '.join(h))
    for n, p in enumerate(jt_lattice()):
        lines.append('body ' + body_fields(n, p, (0.0, 0.0, 0.0)))
    bodies = JT_BODIES
    if program == 'kick':
        for step, kicks in kick_program():
            for i, k in kicks:
                lines.append('kick %d %d %s' % (step, i, ' '.join(f32_hex(x / 64.0) for x in k)))
    elif program == 'shoot':
        for step, i, pos, v in shoot_program():
            lines.append('launch %d %s' % (step, body_fields(i, tuple(x / 16.0 for x in pos),
                                                              tuple(x / 256.0 for x in v))))
        bodies += 60
    lines.append('end %d %d' % (EVENTS[program], bodies))
    return '\n'.join(lines) + '\n'


def fnv1a64(data):
    h = 0xcbf29ce484222325
    for b in data:
        h = ((h ^ b) * 0x100000001b3) & M64
    return h


def main():
    global MUTANT
    ap = argparse.ArgumentParser()
    ap.add_argument('--write')
    ap.add_argument('--program')
    ap.add_argument('--print', action='store_true')
    ap.add_argument('--mutant', choices=['mul1', 'shell4095', 'krange', 'nosep', 'floordiv'])
    a = ap.parse_args()
    MUTANT = a.mutant
    programs = [a.program] if a.program else ['none', 'kick', 'shoot', 'slide']
    for p in programs:
        text = dump(p).encode('ascii')
        if a.print:
            sys.stdout.write(text.decode('ascii'))
            continue
        if a.write:
            with open('%s/%s.dump' % (a.write, p), 'wb') as f:
                f.write(text)
        print('%s 0x%016x %d %d' % (p, fnv1a64(text), len(text), text.count(b'\n')))


if __name__ == '__main__':
    main()
