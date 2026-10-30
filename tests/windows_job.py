"""Windows-only, owned process-tree deadline for bootstrap qualification.

Documented Win32 APIs: CreateProcessW(CREATE_SUSPENDED), AssignProcessToJobObject,
ResumeThread, and JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE. Output goes to private files,
never pipes; only the three explicit standard handles are inherited.
https://learn.microsoft.com/windows/win32/procthread/job-objects
https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw
"""
import ctypes
from ctypes import wintypes
from contextlib import contextmanager
import locale
import os
import shutil
import subprocess
import tempfile
import time


class StartupInfo(ctypes.Structure):
    _fields_ = [('cb', wintypes.DWORD), ('lpReserved', wintypes.LPWSTR),
                ('lpDesktop', wintypes.LPWSTR), ('lpTitle', wintypes.LPWSTR),
                ('dwX', wintypes.DWORD), ('dwY', wintypes.DWORD),
                ('dwXSize', wintypes.DWORD), ('dwYSize', wintypes.DWORD),
                ('dwXCountChars', wintypes.DWORD), ('dwYCountChars', wintypes.DWORD),
                ('dwFillAttribute', wintypes.DWORD), ('dwFlags', wintypes.DWORD),
                ('wShowWindow', wintypes.WORD), ('cbReserved2', wintypes.WORD),
                ('lpReserved2', ctypes.c_void_p), ('hStdInput', wintypes.HANDLE),
                ('hStdOutput', wintypes.HANDLE), ('hStdError', wintypes.HANDLE)]


class StartupInfoEx(ctypes.Structure):
    _fields_ = [('StartupInfo', StartupInfo), ('lpAttributeList', ctypes.c_void_p)]


class ProcessInformation(ctypes.Structure):
    _fields_ = [('hProcess', wintypes.HANDLE), ('hThread', wintypes.HANDLE),
                ('dwProcessId', wintypes.DWORD), ('dwThreadId', wintypes.DWORD)]


class BasicLimits(ctypes.Structure):
    _fields_ = [('PerProcessUserTimeLimit', ctypes.c_longlong),
                ('PerJobUserTimeLimit', ctypes.c_longlong), ('LimitFlags', wintypes.DWORD),
                ('MinimumWorkingSetSize', ctypes.c_size_t), ('MaximumWorkingSetSize', ctypes.c_size_t),
                ('ActiveProcessLimit', wintypes.DWORD), ('Affinity', ctypes.c_size_t),
                ('PriorityClass', wintypes.DWORD), ('SchedulingClass', wintypes.DWORD)]


class IoCounters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_ulonglong) for name in (
        'ReadOperationCount', 'WriteOperationCount', 'OtherOperationCount',
        'ReadTransferCount', 'WriteTransferCount', 'OtherTransferCount')]


class ExtendedLimits(ctypes.Structure):
    _fields_ = [('BasicLimitInformation', BasicLimits), ('IoInfo', IoCounters),
                ('ProcessMemoryLimit', ctypes.c_size_t), ('JobMemoryLimit', ctypes.c_size_t),
                ('PeakProcessMemoryUsed', ctypes.c_size_t), ('PeakJobMemoryUsed', ctypes.c_size_t)]


class Accounting(ctypes.Structure):
    _fields_ = [('TotalUserTime', ctypes.c_longlong), ('TotalKernelTime', ctypes.c_longlong),
                ('ThisPeriodTotalUserTime', ctypes.c_longlong), ('ThisPeriodTotalKernelTime', ctypes.c_longlong),
                ('TotalPageFaultCount', wintypes.DWORD), ('TotalProcesses', wintypes.DWORD),
                ('ActiveProcesses', wintypes.DWORD), ('TotalTerminatedProcesses', wintypes.DWORD)]


class JobTimeout(subprocess.TimeoutExpired):
    """Deadline exceeded, with owned job accounting captured after termination."""


class JobCleanupError(AssertionError):
    """Ownership termination could not be verified; keep the private fixture."""


@contextmanager
def job_fixture(prefix):
    directory = tempfile.mkdtemp(prefix=prefix)
    preserve = False
    try:
        yield directory
    except JobCleanupError:
        preserve = True
        raise
    finally:
        if not preserve:
            shutil.rmtree(directory)


def kernel_api():
    api = ctypes.WinDLL('kernel32', use_last_error=True)
    signatures = {
        'CreateJobObjectW': ([ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
        'SetInformationJobObject': ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD], wintypes.BOOL),
        'QueryInformationJobObject': ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                      wintypes.DWORD, ctypes.c_void_p], wintypes.BOOL),
        'AssignProcessToJobObject': ([wintypes.HANDLE, wintypes.HANDLE], wintypes.BOOL),
        'TerminateJobObject': ([wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
        'InitializeProcThreadAttributeList': ([ctypes.c_void_p, wintypes.DWORD,
                                              wintypes.DWORD, ctypes.POINTER(ctypes.c_size_t)], wintypes.BOOL),
        'UpdateProcThreadAttribute': ([ctypes.c_void_p, wintypes.DWORD, ctypes.c_size_t,
                                      ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_void_p], wintypes.BOOL),
        'DeleteProcThreadAttributeList': ([ctypes.c_void_p], None),
        'CreateProcessW': ([wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p, ctypes.c_void_p,
                            wintypes.BOOL, wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR,
                            ctypes.POINTER(StartupInfoEx), ctypes.POINTER(ProcessInformation)], wintypes.BOOL),
        'ResumeThread': ([wintypes.HANDLE], wintypes.DWORD),
        'WaitForSingleObject': ([wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD),
        'GetExitCodeProcess': ([wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
        'TerminateProcess': ([wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
        'CloseHandle': ([wintypes.HANDLE], wintypes.BOOL),
    }
    for name, (arguments, result) in signatures.items():
        function = getattr(api, name)
        function.argtypes = arguments
        function.restype = result
    return api


def checked(result):
    if not result:
        raise ctypes.WinError(ctypes.get_last_error())
    return result


def run_windows_job(arguments, *, env, directory):
    """Run the whole owned tree for at most 30s, then allow at most 5s to prove termination.

    Assignment happens before the primary thread runs. Breakaway is not enabled,
    so native children remain in this job even if their immediate parent exits.
    A setup failure never resumes the suspended process. No PID lookup or reuse.
    """
    if os.name != 'nt':
        raise RuntimeError('Windows Job Object qualification requires Windows')
    import msvcrt

    started = time.monotonic()
    deadline = started + 30
    api = kernel_api()
    job = checked(api.CreateJobObjectW(None, None))
    process = ProcessInformation()
    attributes = None
    attributes_initialized = False
    assigned = False
    tree_empty = False
    cleanup_started = False

    def accounting():
        state = Accounting()
        checked(api.QueryInformationJobObject(job, 1, ctypes.byref(state), ctypes.sizeof(state), None))
        return state

    def terminate_job():
        nonlocal cleanup_started
        cleanup_started = True
        cleanup_deadline = time.monotonic() + 5
        try:
            checked(api.TerminateJobObject(job, 1))
            state = accounting()
            while state.ActiveProcesses and time.monotonic() < cleanup_deadline:
                time.sleep(0.01)
                state = accounting()
        except OSError as error:
            raise JobCleanupError(f'owned job termination unverified; fixture={directory}') from error
        if state.ActiveProcesses:
            raise JobCleanupError(f'owned job cleanup exceeded 5s: active={state.ActiveProcesses}; fixture={directory}')
        return state

    try:
        limits = ExtendedLimits()
        limits.BasicLimitInformation.LimitFlags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        checked(api.SetInformationJobObject(job, 9, ctypes.byref(limits), ctypes.sizeof(limits)))
        with (open(os.devnull, 'rb') as stdin,
              open(os.path.join(directory, 'entry-stdout.log'), 'w+b') as stdout,
              open(os.path.join(directory, 'entry-stderr.log'), 'w+b') as stderr):
            handles = [msvcrt.get_osfhandle(stream.fileno()) for stream in (stdin, stdout, stderr)]
            size = ctypes.c_size_t()
            api.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(size))
            if not size.value:
                raise ctypes.WinError(ctypes.get_last_error())
            attributes = ctypes.create_string_buffer(size.value)
            checked(api.InitializeProcThreadAttributeList(attributes, 1, 0, ctypes.byref(size)))
            attributes_initialized = True
            inherited = (wintypes.HANDLE * len(handles))(*handles)
            startup = StartupInfoEx()
            startup.StartupInfo.cb = ctypes.sizeof(startup)
            startup.StartupInfo.dwFlags = 0x100  # STARTF_USESTDHANDLES
            startup.StartupInfo.hStdInput, startup.StartupInfo.hStdOutput, startup.StartupInfo.hStdError = handles
            startup.lpAttributeList = ctypes.cast(attributes, ctypes.c_void_p)
            command = ctypes.create_unicode_buffer(subprocess.list2cmdline(arguments))
            environment = ctypes.create_unicode_buffer('\0'.join(
                f'{key}={value}' for key, value in sorted(env.items(), key=lambda item: item[0].upper())) + '\0')
            try:
                for handle in handles:
                    os.set_handle_inheritable(handle, True)
                checked(api.UpdateProcThreadAttribute(attributes, 0, 0x20002, inherited,
                                                      ctypes.sizeof(inherited), None, None))  # HANDLE_LIST
                checked(api.CreateProcessW(arguments[0], command, None, None, True,
                                            0x4 | 0x400 | 0x80000, environment, str(directory),
                                            ctypes.byref(startup), ctypes.byref(process)))
            finally:
                for handle in handles:
                    os.set_handle_inheritable(handle, False)
            checked(api.AssignProcessToJobObject(job, process.hProcess))
            assigned = True
            if time.monotonic() < deadline:
                if api.ResumeThread(process.hThread) == 0xFFFFFFFF:
                    raise ctypes.WinError(ctypes.get_last_error())

            state = accounting()
            while state.ActiveProcesses and time.monotonic() < deadline:
                time.sleep(min(0.05, max(0, deadline - time.monotonic())))
                state = accounting()
            timed_out = bool(state.ActiveProcesses)
            if timed_out:
                state = terminate_job()
            tree_empty = True
            host_exited = api.WaitForSingleObject(process.hProcess, 0) == 0
            if not host_exited:
                raise JobCleanupError(f'retained host handle is not signaled; fixture={directory}')
            code = wintypes.DWORD()
            checked(api.GetExitCodeProcess(process.hProcess, ctypes.byref(code)))
            output = []
            for stream in (stdout, stderr):
                stream.seek(0)
                output.append(stream.read().decode(locale.getpreferredencoding(False), 'replace').replace('\r\n', '\n'))
            result = subprocess.CompletedProcess(arguments, code.value, *output)
            if timed_out:
                result = JobTimeout(arguments, 30, output=output[0], stderr=output[1])
            result.job_active_processes = state.ActiveProcesses
            result.job_total_processes = state.TotalProcesses
            result.host_exited = host_exited
            result.elapsed = time.monotonic() - started
            if timed_out:
                raise result
            return result
    finally:
        # On every exceptional path kill only this job, or the still-suspended
        # process handle if assignment failed. Closing the job is a second guard.
        try:
            if assigned and not tree_empty and not cleanup_started:
                terminate_job()
            elif not assigned and process.hProcess:
                terminated = api.TerminateProcess(process.hProcess, 1)
                exited = api.WaitForSingleObject(process.hProcess, 5000) == 0
                if not terminated or not exited:
                    raise JobCleanupError(f'suspended host termination unverified; fixture={directory}')
        finally:
            close_failed = False
            for handle in (job, process.hThread, process.hProcess):
                if handle and not api.CloseHandle(handle):
                    close_failed = True
            if attributes_initialized:
                api.DeleteProcThreadAttributeList(attributes)
            if close_failed:
                raise JobCleanupError(f'owned handle closure unverified; fixture={directory}')
