"""Samples the OS thread count of a child process while it runs (structural; no timing kept)."""
import ctypes, os, subprocess, sys, time, tempfile
from ctypes import wintypes
class PE(ctypes.Structure):
    _fields_ = [('dwSize', wintypes.DWORD), ('cntUsage', wintypes.DWORD), ('th32ProcessID', wintypes.DWORD),
                ('th32DefaultHeapID', ctypes.c_size_t), ('th32ModuleID', wintypes.DWORD), ('cntThreads', wintypes.DWORD),
                ('th32ParentProcessID', wintypes.DWORD), ('pcPriClassBase', ctypes.c_long), ('dwFlags', wintypes.DWORD),
                ('szExeFile', ctypes.c_wchar * 260)]
k = ctypes.WinDLL('kernel32', use_last_error=True)
k.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
k.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
k.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PE)]
k.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PE)]
k.CloseHandle.argtypes = [wintypes.HANDLE]
def threads(pid):
    s = k.CreateToolhelp32Snapshot(2, 0); e = PE(); e.dwSize = ctypes.sizeof(PE)
    ok = k.Process32FirstW(s, ctypes.byref(e)); n = None
    while ok:
        if e.th32ProcessID == pid: n = e.cntThreads; break
        ok = k.Process32NextW(s, ctypes.byref(e))
    k.CloseHandle(s); return n
cmd = sys.argv[1:]
cwd = tempfile.mkdtemp(prefix='thr_', dir=os.environ.get('PROBE_DIR'))
p = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
seen = []
while p.poll() is None:
    n = threads(p.pid)
    if n is not None: seen.append(n)
    time.sleep(0.05)
out, err = p.communicate()
hist = {}
for n in seen: hist[n] = hist.get(n, 0) + 1
print(f"exit {p.returncode}; samples {len(seen)}; OS thread count histogram {dict(sorted(hist.items()))}")
