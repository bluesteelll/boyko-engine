"""Per-logical-CPU '% Processor Performance' sampler (PDH via ctypes), a WITNESS of each CPU's
effective clock relative to nominal, sampled at ~1 Hz in a parent-side thread while a timed process
runs. It launches nothing and touches no measured process."""
import ctypes
import threading
import time
from ctypes import wintypes

pdh = ctypes.WinDLL('pdh')
PDH_FMT_DOUBLE = 0x00000200
PDH_FMT_NOCAP100 = 0x00008000
PDH_MORE_DATA = 0x800007D2


class FMTVAL(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [('longValue', ctypes.c_long), ('doubleValue', ctypes.c_double), ('largeValue', ctypes.c_longlong),
                    ('AnsiStringValue', ctypes.c_char_p), ('WideStringValue', ctypes.c_wchar_p)]
    _fields_ = [('CStatus', wintypes.DWORD), ('u', _U)]


class ITEM(ctypes.Structure):
    _fields_ = [('szName', ctypes.c_wchar_p), ('FmtValue', FMTVAL)]


pdh.PdhOpenQueryW.argtypes = [ctypes.c_wchar_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_void_p)]
pdh.PdhAddEnglishCounterW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_void_p)]
pdh.PdhCollectQueryData.argtypes = [ctypes.c_void_p]
pdh.PdhGetFormattedCounterArrayW.argtypes = [ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
                                             ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
pdh.PdhCloseQuery.argtypes = [ctypes.c_void_p]
for f in (pdh.PdhOpenQueryW, pdh.PdhAddEnglishCounterW, pdh.PdhCollectQueryData, pdh.PdhGetFormattedCounterArrayW,
          pdh.PdhCloseQuery):
    f.restype = ctypes.c_long


class PerfSampler:
    PATH = r'\Processor Information(*)\% Processor Performance'

    def __init__(self, period=1.0):
        self.period = period
        self.q = ctypes.c_void_p()
        self.c = ctypes.c_void_p()
        st = pdh.PdhOpenQueryW(None, 0, ctypes.byref(self.q))
        if st != 0:
            raise OSError(f'PdhOpenQueryW {st:#x}')
        st = pdh.PdhAddEnglishCounterW(self.q, self.PATH, 0, ctypes.byref(self.c))
        if st != 0:
            raise OSError(f'PdhAddEnglishCounterW {st & 0xffffffff:#x}')
        pdh.PdhCollectQueryData(self.q)
        self.samples = []
        self._stop = threading.Event()
        self._t = None

    def read(self):
        if pdh.PdhCollectQueryData(self.q) != 0:
            return None
        size, count = wintypes.DWORD(0), wintypes.DWORD(0)
        st = pdh.PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, ctypes.byref(size),
                                              ctypes.byref(count), None)
        if (st & 0xffffffff) != PDH_MORE_DATA:
            return None
        buf = ctypes.create_string_buffer(size.value)
        st = pdh.PdhGetFormattedCounterArrayW(self.c, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, ctypes.byref(size),
                                              ctypes.byref(count), buf)
        if st != 0:
            return None
        items = ctypes.cast(buf, ctypes.POINTER(ITEM))
        out = {}
        for i in range(count.value):
            it = items[i]
            if it.FmtValue.CStatus in (0, 1):  # PDH_CSTATUS_VALID_DATA / NEW_DATA
                out[it.szName] = round(it.FmtValue.u.doubleValue, 1)
        return out

    def _loop(self):
        while not self._stop.wait(self.period):
            v = self.read()
            if v is not None:
                self.samples.append((round(time.perf_counter(), 3), v))

    def start(self):
        self.read()  # re-baseline the rate counter at the start of the timed region
        self.samples = []
        self._stop.clear()
        self._t = threading.Thread(target=self._loop, daemon=True)
        self._t.start()

    def stop(self):
        self._stop.set()
        if self._t:
            self._t.join()
        v = self.read()  # the tail since the last tick
        if v is not None:
            self.samples.append((round(time.perf_counter(), 3), v))
        return self.samples

    def close(self):
        pdh.PdhCloseQuery(self.q)


if __name__ == '__main__':
    s = PerfSampler(1.0)
    s.start()
    x = 0
    t0 = time.time()
    while time.time() - t0 < 3.2:
        x += 1
    for t, v in s.stop():
        print(t, {k: v[k] for k in sorted(v)[:4]}, '...', len(v), 'total', v.get('_Total'), v.get('0,_Total'))
