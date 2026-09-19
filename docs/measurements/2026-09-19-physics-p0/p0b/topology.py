"""SMT sibling map of this machine, from Windows itself: GetLogicalProcessorInformationEx.

RelationProcessorCore (0): one record per PHYSICAL core; its GROUP_AFFINITY mask names that core's
logical CPUs (two on an SMT core). RelationCache (2): which logical CPUs share each L3.
Also reports EfficiencyClass per core (Windows' own ranking field; 0 on a homogeneous part) and the
process's current affinity masks. Writes topology.json next to this file.
"""
import ctypes
import json
import os
import struct
from ctypes import wintypes

k = ctypes.WinDLL('kernel32', use_last_error=True)
k.GetLogicalProcessorInformationEx.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.POINTER(wintypes.DWORD)]
k.GetLogicalProcessorInformationEx.restype = wintypes.BOOL
k.GetCurrentProcess.restype = wintypes.HANDLE
k.GetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)]


def query(rel):
    n = wintypes.DWORD(0)
    k.GetLogicalProcessorInformationEx(rel, None, ctypes.byref(n))
    buf = ctypes.create_string_buffer(n.value)
    if not k.GetLogicalProcessorInformationEx(rel, buf, ctypes.byref(n)):
        raise OSError(ctypes.get_last_error())
    raw = buf.raw[:n.value]
    out = []
    off = 0
    while off < len(raw):
        relationship, size = struct.unpack_from('<II', raw, off)
        out.append((relationship, raw[off:off + size]))
        off += size
    return out


def bits(mask):
    return [i for i in range(64) if mask >> i & 1]


def main():
    cores = []
    for relationship, rec in query(0):  # RelationProcessorCore
        # PROCESSOR_RELATIONSHIP at +8: Flags(1) EfficiencyClass(1) Reserved[20] GroupCount(2) then
        # GROUP_AFFINITY[] at +32 (8-aligned): Mask(8) Group(2) Reserved[3](6).
        flags, eff = struct.unpack_from('<BB', rec, 8)
        group_count, = struct.unpack_from('<H', rec, 30)
        masks = []
        for g in range(group_count):
            mask, group = struct.unpack_from('<QH', rec, 32 + 16 * g)
            masks.append({'group': group, 'mask': hex(mask), 'cpus': bits(mask)})
        cores.append({'smt': bool(flags & 1), 'efficiency_class': eff, 'masks': masks})
    caches = []
    for relationship, rec in query(2):  # RelationCache
        # CACHE_RELATIONSHIP at +8: Level(1) Associativity(1) LineSize(2) CacheSize(4) Type(4)
        # Reserved[18] GroupCount(2) at +38, GROUP_AFFINITY[] at +40.
        level, assoc, line, size, ctype = struct.unpack_from('<BBHII', rec, 8)
        gc, = struct.unpack_from('<H', rec, 38)
        gc = max(gc, 1)
        mask, group = struct.unpack_from('<QH', rec, 40)
        caches.append({'level': level, 'type': ctype, 'size_kib': size // 1024, 'cpus': bits(mask)})
    pm, sm = ctypes.c_size_t(), ctypes.c_size_t()
    k.GetProcessAffinityMask(k.GetCurrentProcess(), ctypes.byref(pm), ctypes.byref(sm))
    res = {'physical_cores': cores, 'caches': caches, 'process_mask': hex(pm.value), 'system_mask': hex(sm.value),
           'os_cpu_count': os.cpu_count()}
    with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'topology.json'), 'w') as f:
        json.dump(res, f, indent=1)
    for i, c in enumerate(cores):
        print(f'core {i}: smt {c["smt"]} efficiency_class {c["efficiency_class"]} cpus {c["masks"][0]["cpus"]} mask {c["masks"][0]["mask"]}')
    for c in caches:
        if c['level'] >= 2:
            print(f'L{c["level"]} type {c["type"]} {c["size_kib"]} KiB cpus {c["cpus"]}')
    print('process mask', res['process_mask'], 'system mask', res['system_mask'])


if __name__ == '__main__':
    main()
