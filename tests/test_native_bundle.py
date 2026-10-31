"""Native bundle contract. Helper scenarios need FRAYD_NATIVE_HELPER; the real-server scenario also needs FRAYD_NATIVE_ARCHIVE and FRAYD_NATIVE_BUILD_SOURCE."""
import base64
import errno
import fcntl
import hashlib
import importlib.util
import json
import math
import os
import pty
import re
import secrets
import select
import shlex
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import termios
import threading
import time
import unittest
import urllib.request
import zipfile
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
HELPER = Path(os.environ.get('FRAYD_NATIVE_HELPER', ROOT / 'target/native-qualification/debug/frayd-native'))
SERVER = HELPER.with_name('frayd-server')
PACKAGE = ROOT / 'scripts/native/package.py'
TEMP_ROOT = Path('/var/tmp').resolve()
ARCHITECTURE = 'aarch64' if os.uname().machine in ['arm64', 'aarch64'] else 'x86_64'
TARGET = ARCHITECTURE + ('-apple-darwin' if os.uname().sysname == 'Darwin' else '-unknown-linux-gnu')


class RunBoundedCleanupUncertain(RuntimeError):
    """The caller must retain its fixture when owned cleanup cannot be confirmed."""


def run_bounded(arguments, *, timeout=60, check=False, input=None, capture_output=False, terminal_master=None, **options):
    """Bound communication and cleanup of one spawn-owned session.

    Popen/preexec admission precedes the communication budget and is not bounded
    by it. terminal_master is borrowed exclusively during the call. Its owned
    duplicate temporarily shares nonblocking flags; the reader restores the old
    mode and closes its copy on exit, including after an uncertain join. The
    borrowed mode can remain nonblocking until that outstanding reader exits.
    """
    if not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('run_bounded timeout must be positive and finite before spawning')
    # This single-owner fixture permits no custom SIGCHLD handler or other
    # waiter. Fresh WNOWAIT below must still confirm custody before signalling.
    if signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL:
        raise RuntimeError('run_bounded requires default SIGCHLD custody before spawning')
    if not callable(getattr(os, 'waitid', None)) or not all(hasattr(os, name) for name in ['WNOWAIT', 'WEXITED', 'WNOHANG', 'P_PID']):
        raise RuntimeError('run_bounded requires WNOWAIT child custody before spawning')
    if capture_output:
        options['stdout'] = options['stderr'] = subprocess.PIPE
    if input is not None:
        options['stdin'] = subprocess.PIPE
    process = subprocess.Popen(arguments, start_new_session=True, **options)
    operation_deadline = time.monotonic() + timeout
    terminal_output = bytearray()
    terminal_errors = []
    terminal_stop = threading.Event()
    terminal_deadline = [operation_deadline + 5]
    terminal_reader = None
    terminal_ready = threading.Event()
    terminal_admission = threading.Lock()
    terminal_cancelled = False
    terminal_copy = None
    terminal_blocking = None
    terminal_cleanup_errors = []

    def close_terminal_copy():
        if terminal_copy is None:
            return
        if terminal_blocking is not None:
            try:
                os.set_blocking(terminal_copy, terminal_blocking)
            except OSError as error:
                terminal_cleanup_errors.append(error)
        try:
            os.close(terminal_copy)
        except OSError as error:
            terminal_cleanup_errors.append(error)

    def drain_terminal():
        # Only the reader and admission resolver use this nonblocking lock.
        if not terminal_admission.acquire(blocking=False):
            terminal_errors.append(RuntimeError('terminal reader admission was being resolved'))
            return
        try:
            if terminal_cancelled:
                return
            terminal_ready.set()
        finally:
            terminal_admission.release()
        # A cancelled late target returns before this descriptor-owning finally.
        try:
            while time.monotonic() < terminal_deadline[0]:
                try:
                    chunk = os.read(terminal_copy, 65536)
                except BlockingIOError:
                    # Wait for readiness, not a fixed delay per small PTY turn.
                    remaining = max(0, terminal_deadline[0] - time.monotonic())
                    delay = 0 if terminal_stop.is_set() else min(0.01, remaining)
                    try:
                        ready, _, _ = select.select([terminal_copy], [], [], delay)
                    except (OSError, ValueError) as error:
                        # FD >= FD_SETSIZE fails closed; there is no selector fallback.
                        terminal_errors.append(error)
                        return
                    if not ready and terminal_stop.is_set():
                        return
                    continue
                except OSError as error:
                    # PTY master hangup can report EIO. Reaping is still required.
                    if error.errno != errno.EIO:
                        terminal_errors.append(error)
                    return
                if not chunk:
                    return
                remaining = 1024 * 1024 - len(terminal_output)
                terminal_output.extend(chunk[:remaining])
                if len(chunk) > remaining and not terminal_errors:
                    terminal_errors.append(RuntimeError('terminal output exceeded the 1 MiB capture limit'))
                # On overflow keep draining through cleanup, but never accept the run.
            terminal_errors.append(RuntimeError('terminal reader exceeded its operation/cleanup deadline'))
        finally:
            # Only this reader touches its descriptor once ownership transfers.
            close_terminal_copy()

    operation_error = None
    stdout = stderr = None
    try:
        if terminal_master is not None:
            terminal_copy = os.dup(terminal_master)
            terminal_blocking = os.get_blocking(terminal_copy)
            os.set_blocking(terminal_copy, False)
            terminal_reader = threading.Thread(target=drain_terminal, name="frayd-pty-reader", daemon=True)
            terminal_reader.start()
        stdout, stderr = process.communicate(input, timeout=max(0, operation_deadline - time.monotonic()))
    except BaseException as error:  # noqa: BLE001 - Cancellation needs cleanup; preserve the original error.
        operation_error = error

    cleanup_deadline = time.monotonic() + 5
    terminal_deadline[0] = min(terminal_deadline[0], cleanup_deadline)
    cleanup_errors = []
    if operation_error is not None and process.returncode is None:
        # No poll/try_wait here: keep the spawn-owned leader unreaped while
        # signalling its original session group. Never signal after a known reap.
        try:
            info = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG)
            if info is not None and info.si_pid != process.pid:
                raise RuntimeError('owned child wait identity changed; no group signal sent')
        except (OSError, RuntimeError) as error:
            cleanup_errors.append(error)
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except OSError as error:
                cleanup_errors.append(error)
    if process.returncode is None:
        try:
            process.wait(timeout=max(0, cleanup_deadline - time.monotonic()))
        except BaseException as error:  # noqa: BLE001 - Preserve cleanup cancellation beside the original error.
            cleanup_errors.append(error)
    # Keep consuming the PTY until the owned process has reaped or cleanup held.
    terminal_stop.set()
    if terminal_reader is None:
        close_terminal_copy()
    else:
        acknowledged = terminal_ready.wait(timeout=max(0, cleanup_deadline - time.monotonic()))
        if not acknowledged:
            reclaimed = False
            if terminal_admission.acquire(blocking=False):
                try:
                    if not terminal_ready.is_set():
                        terminal_cancelled = True
                        reclaimed = True
                finally:
                    terminal_admission.release()
                if reclaimed:
                    close_terminal_copy()
                    if operation_error is None:
                        terminal_errors.append(RuntimeError('terminal reader was cancelled before admission'))
            else:
                cleanup_errors.append(RuntimeError('terminal reader ownership could not be resolved within the cleanup budget'))
        if terminal_ready.is_set():
            terminal_reader.join(timeout=max(0, cleanup_deadline - time.monotonic()))
            if terminal_reader.is_alive():
                cleanup_errors.append(RuntimeError('terminal reader did not stop within the cleanup budget'))
    # An uncertain join leaves the owned copy with the reader, never the caller.
    cleanup_errors.extend(terminal_cleanup_errors)
    for stream in (process.stdin, process.stdout, process.stderr):
        if stream is not None:
            try:
                stream.close()
            except OSError as error:
                cleanup_errors.append(error)
    if cleanup_errors:
        kinds = ', '.join(type(error).__name__ for error in cleanup_errors)
        error = RunBoundedCleanupUncertain(f'owned process cleanup uncertain ({kinds}); retain the fixture and receipts')
        error.process = process
        error.terminal_reader = terminal_reader
        error.cleanup_errors = tuple(cleanup_errors)
        error.terminal_output = bytes(terminal_output)
        raise error from (operation_error or cleanup_errors[0])
    if operation_error is not None:
        operation_error.terminal_output = bytes(terminal_output)
        raise operation_error
    if terminal_errors:
        terminal_errors[0].terminal_output = bytes(terminal_output)
        raise terminal_errors[0]
    result = subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)
    result.terminal_output = bytes(terminal_output)
    if check:
        result.check_returncode()
    return result


def wait_for(probe, what, timeout=10, interval=0.02):
    """Poll probe() until it returns something truthy, which is returned; fail the test after timeout seconds."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(interval)
    raise AssertionError(f'timed out after {timeout}s waiting for {what}')


def running_child_pid(prefix):
    """Return the supervised server's pid once the child receipt says it is running."""
    try:
        document = json.loads((prefix / '.frayd-child.json').read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None
    return document['pid'] if document.get('state') == 'running' else None


def shared_running_child_pid(*roots):
    """Return the server's pid once every root's child receipt says running with that same pid.

    The helper writes the prefix receipt before the data receipt, so a missing or partly written second one reads as not yet.
    """
    pids = {running_child_pid(root) for root in roots}
    return pids.pop() if len(pids) == 1 and None not in pids else None


def snapshot(root):
    """Names, modes and bytes (or symlink targets) of everything under root."""
    return [(str(p.relative_to(root)), p.lstat().st_mode, os.readlink(p) if p.is_symlink() else p.read_bytes() if p.is_file() else None) for p in sorted(root.rglob('*'))]


def package(output, target=TARGET, server=SERVER, helper=HELPER, runner=None, **options):
    if runner is None:
        runner = run_bounded
    return runner([sys.executable, str(PACKAGE), str(output), target, str(server), str(helper)], **options)


def setUpModule():
    """Fail instead of skipping when full qualification is requested but an input is missing."""
    if os.environ.get('FRAYD_NATIVE_REQUIRE_QUALIFICATION') == '1':
        required = ['FRAYD_NATIVE_HELPER', 'FRAYD_NATIVE_ARCHIVE', 'FRAYD_NATIVE_BUILD_SOURCE']
        missing = [name for name in required if not os.environ.get(name)]
        if missing:
            raise RuntimeError('Native qualification prerequisites missing: ' + ', '.join(missing))


class RunBounded(unittest.TestCase):
    # The grandchild sleeps argv[3] seconds, takes the lease, then marks itself ready and holds the lease.
    GRANDCHILD = ("import fcntl,pathlib,sys,time; time.sleep(float(sys.argv[3])); f=open(sys.argv[1],'w'); "
                  "fcntl.flock(f,fcntl.LOCK_EX); pathlib.Path(sys.argv[2]).touch(); time.sleep(60)")
    DRIVER = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',*sys.argv[1:]]); time.sleep(60)"

    def run_after_handshake(self, startup_delay, ready_timeout):
        """Call run_bounded with a 1 s timeout that starts only once the grandchild holds its lease.

        The handshake has its own ready_timeout, so a slow interpreter start cannot consume the timeout under
        test. Returns the process group and a probe that reports whether the lease is still held.
        """
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        lease, ready = Path(directory.name) / 'lease', Path(directory.name) / 'ready'
        groups = []
        communicate = subprocess.Popen.communicate

        def lease_held():
            with lease.open('a') as handle:
                try:
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    return True
                fcntl.flock(handle, fcntl.LOCK_UN)
                return False

        def communicate_after_handshake(process, *arguments, **options):
            groups.append(process.pid)
            wait_for(lambda: ready.exists() and lease_held(), 'the grandchild to hold its lease', ready_timeout)
            return communicate(process, *arguments, **options)

        command = [sys.executable, '-c', self.DRIVER, self.GRANDCHILD, str(lease), str(ready), str(startup_delay)]
        try:
            with mock.patch.object(subprocess.Popen, 'communicate', communicate_after_handshake):
                run_bounded(command, timeout=1)
        finally:
            self.group = groups[0]
            self.lease_held = lease_held

    def assert_group_gone(self):
        def gone():
            try:
                os.killpg(self.group, 0)
            except ProcessLookupError:
                return True
            return False
        wait_for(gone, 'the killed process group to exit', timeout=5)
        self.assertFalse(self.lease_held(), 'a descendant still holds the lease')

    def test_terminal_pressure_is_drained_and_preserved(self):
        master, slave = pty.openpty()
        try:
            attributes = termios.tcgetattr(slave)
            attributes[3] &= ~termios.TOSTOP
            termios.tcsetattr(slave, termios.TCSANOW, attributes)
            command = [sys.executable, '-c', "import sys; sys.stdout.buffer.write(b'x'*262144); sys.stdout.flush(); sys.stderr.write('done\\n')"]
            result = run_bounded(command, timeout=3, stdout=slave, stderr=subprocess.PIPE,
                                 terminal_master=master,
                                 preexec_fn=lambda: fcntl.ioctl(slave, termios.TIOCSCTTY, 0))
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.terminal_output, b'x' * 262144)
            self.assertEqual(result.stderr, b'done\n')
            self.assertTrue(os.get_blocking(master))
        finally:
            os.close(slave)
            os.close(master)

    def test_terminal_capture_overflow_is_an_error_after_reaping(self):
        master, slave = pty.openpty()
        try:
            with self.assertRaisesRegex(RuntimeError, '1 MiB capture limit') as failure:
                run_bounded([sys.executable, '-c', "import sys; sys.stdout.buffer.write(b'x'*2097152)"],
                            timeout=3, stdout=slave, stderr=subprocess.PIPE, terminal_master=master)
            self.assertEqual(failure.exception.terminal_output, b'x' * 1048576)
            self.assertFalse(any(thread.name == 'frayd-pty-reader' for thread in threading.enumerate()))
        finally:
            os.close(slave)
            os.close(master)

    def test_small_terminal_turns_use_readiness_without_poll_delay(self):
        master, slave = pty.openpty()
        read = os.read
        pending_yield = False
        self.terminal_read_counts = {'data_turns': 0, 'would_block_turns': 0, 'bytes_read': 0}

        def small_read(descriptor, size):
            nonlocal pending_yield
            if threading.current_thread().name != 'frayd-pty-reader':
                return read(descriptor, size)
            if pending_yield:
                pending_yield = False
                self.terminal_read_counts['would_block_turns'] += 1
                raise BlockingIOError(errno.EAGAIN, 'controlled PTY scheduling boundary')
            chunk = read(descriptor, min(size, 1024))
            if chunk:
                pending_yield = True
                self.terminal_read_counts['data_turns'] += 1
                self.terminal_read_counts['bytes_read'] += len(chunk)
            return chunk

        try:
            with (
                mock.patch.object(os, 'read', side_effect=small_read),
                self.assertRaisesRegex(RuntimeError, '1 MiB capture limit') as failure,
            ):
                run_bounded([sys.executable, '-c', "import sys; sys.stdout.buffer.write(b'x'*2097152)"],
                            timeout=3, stdout=slave, stderr=subprocess.PIPE, terminal_master=master)
            self.assertEqual(failure.exception.terminal_output, b'x' * 1048576)
            self.assertEqual(self.terminal_read_counts['bytes_read'], 2097152)
            self.assertGreaterEqual(self.terminal_read_counts['data_turns'], 2048)
            self.assertGreaterEqual(self.terminal_read_counts['would_block_turns'],
                                    self.terminal_read_counts['data_turns'] - 1)
        finally:
            os.close(slave)
            os.close(master)

    def test_terminal_timeout_restores_each_borrowed_mode(self):
        for blocking in [True, False]:
            master, slave = pty.openpty()
            try:
                os.set_blocking(master, blocking)
                with self.subTest(blocking=blocking), self.assertRaises(subprocess.TimeoutExpired):
                    run_bounded([sys.executable, '-c', 'import time; time.sleep(60)'], timeout=0.05,
                                stdout=slave, stderr=subprocess.PIPE, terminal_master=master)
                self.assertEqual(os.get_blocking(master), blocking)
            finally:
                os.close(slave)
                os.close(master)

    def test_uncertain_join_cannot_read_a_reused_borrowed_descriptor(self):
        master, slave = pty.openpty()
        master_open = True
        entered = threading.Event()
        release = threading.Event()
        read = os.read
        join = threading.Thread.join
        descriptors = []
        reader = None

        def paused_read(descriptor, size):
            if threading.current_thread().name == 'frayd-pty-reader':
                descriptors.append(descriptor)
                entered.set()
                if not release.wait(timeout=2):
                    raise RuntimeError('controlled reader was not released')
            return read(descriptor, size)

        def skip_reader_join(thread, timeout=None):
            if thread.name != 'frayd-pty-reader':
                return join(thread, timeout=timeout)
            self.assertTrue(entered.wait(timeout=1))
            return None

        try:
            with (
                mock.patch.object(os, 'read', side_effect=paused_read),
                mock.patch.object(threading.Thread, 'join', new=skip_reader_join),
                self.assertRaises(RunBoundedCleanupUncertain),
            ):
                run_bounded([sys.executable, '-c', 'pass'], timeout=3,
                            stdout=slave, stderr=subprocess.PIPE, terminal_master=master)
            reader = next(thread for thread in threading.enumerate() if thread.name == 'frayd-pty-reader')
            os.close(master)
            master_open = False
            with tempfile.TemporaryDirectory(prefix='frayd-fd-reuse-', dir=TEMP_ROOT) as directory:
                sentinel = Path(directory) / 'unrelated'
                sentinel.write_bytes(b'unrelated descriptor must not be read')
                replacement = os.open(sentinel, os.O_RDONLY)
                if replacement != master:
                    os.dup2(replacement, master)
                    os.close(replacement)
                master_open = True
                os.set_blocking(master, False)
                release.set()
                join(reader, timeout=2)
                self.assertFalse(reader.is_alive())
                self.assertNotEqual(descriptors[0], master)
                self.assertEqual(os.lseek(master, 0, os.SEEK_CUR), 0)
                self.assertFalse(os.get_blocking(master))
                with self.assertRaises(OSError) as closed:
                    os.fstat(descriptors[0])
                self.assertEqual(closed.exception.errno, errno.EBADF)
        finally:
            release.set()
            if reader is not None:
                join(reader, timeout=2)
            os.close(slave)
            if master_open:
                os.close(master)

    def test_cancelled_reader_admission_reclaims_copy_before_late_target(self):
        for enter_late in [False, True]:
            master, slave = pty.openpty()
            copies = []
            targets = []
            duplicate = os.dup
            reader = mock.Mock()
            reader.start.side_effect = KeyboardInterrupt('controlled reader admission cancellation')

            def track_duplicate(descriptor, duplicate=duplicate, copies=copies):
                copy = duplicate(descriptor)
                copies.append(copy)
                return copy

            def deferred_reader(*, targets=targets, reader=reader, **options):
                targets.append(options['target'])
                return reader

            try:
                with (
                    self.subTest(enter_late=enter_late),
                    mock.patch.object(os, 'dup', side_effect=track_duplicate),
                    mock.patch.object(threading, 'Thread', side_effect=deferred_reader),
                    self.assertRaises(KeyboardInterrupt),
                ):
                    run_bounded([sys.executable, '-c', 'import time; time.sleep(60)'], timeout=3,
                                stdout=slave, stderr=subprocess.PIPE, terminal_master=master)
                self.assertEqual(len(copies), 1)
                self.assertTrue(os.get_blocking(master))
                with self.assertRaises(OSError) as closed:
                    os.fstat(copies[0])
                self.assertEqual(closed.exception.errno, errno.EBADF)
                reader.is_alive.assert_not_called()
                reader.join.assert_not_called()
                if enter_late:
                    with tempfile.TemporaryDirectory(prefix='frayd-late-reader-', dir=TEMP_ROOT) as directory:
                        sentinel = Path(directory) / 'unrelated'
                        sentinel.write_bytes(b'late target must not access this descriptor')
                        replacement = os.open(sentinel, os.O_RDONLY)
                        if replacement != copies[0]:
                            os.dup2(replacement, copies[0])
                            os.close(replacement)
                        os.set_blocking(copies[0], False)
                        try:
                            with (
                                mock.patch.object(os, 'read', wraps=os.read) as read,
                                mock.patch.object(os, 'close', wraps=os.close) as close,
                                mock.patch.object(os, 'set_blocking', wraps=os.set_blocking) as mode,
                            ):
                                targets[0]()
                            read.assert_not_called()
                            close.assert_not_called()
                            mode.assert_not_called()
                            os.fstat(copies[0])
                            self.assertEqual(os.lseek(copies[0], 0, os.SEEK_CUR), 0)
                            self.assertFalse(os.get_blocking(copies[0]))
                        finally:
                            os.close(copies[0])
            finally:
                # A failing pre-fix control has no actual OS reader thread.
                for descriptor in copies:
                    try:
                        os.set_blocking(descriptor, True)
                    except OSError as error:
                        if error.errno != errno.EBADF:
                            raise
                    else:
                        os.close(descriptor)
                os.close(slave)
                os.close(master)

    def test_known_reap_never_signals_and_never_implicitly_waits(self):
        process = mock.Mock(returncode=0, stdin=None, stdout=None, stderr=None)
        process.communicate.side_effect = subprocess.TimeoutExpired('reaped fixture', 1)
        with (
            mock.patch.object(subprocess, 'Popen', return_value=process),
            mock.patch.object(os, 'killpg') as kill,
            self.assertRaises(subprocess.TimeoutExpired),
        ):
            run_bounded(['reaped fixture'], timeout=1)
        kill.assert_not_called()
        process.wait.assert_not_called()
        self.assertNotIn('__exit__', [call[0] for call in process.mock_calls])

    def test_unknown_wait_custody_retains_cleanup_without_signalling(self):
        process = mock.Mock(returncode=None, stdin=None, stdout=None, stderr=None)
        process.communicate.side_effect = subprocess.TimeoutExpired('unknown fixture', 1)
        with (
            mock.patch.object(subprocess, 'Popen', return_value=process),
            mock.patch.object(os, 'waitid', side_effect=ChildProcessError('custody lost')),
            mock.patch.object(os, 'killpg') as kill,
            self.assertRaises(RunBoundedCleanupUncertain) as failure,
        ):
            run_bounded(['unknown fixture'], timeout=1)
        kill.assert_not_called()
        self.assertIsInstance(failure.exception.cleanup_errors[0], ChildProcessError)
        self.assertIs(failure.exception.process, process)
        self.assertEqual(process.wait.call_count, 1)
        self.assertLessEqual(process.wait.call_args.kwargs['timeout'], 5)

    def test_nondefault_child_policy_refuses_before_spawn(self):
        with (
            mock.patch.object(signal, 'getsignal', return_value=signal.SIG_IGN),
            mock.patch.object(subprocess, 'Popen') as spawn,
            self.assertRaisesRegex(RuntimeError, 'default SIGCHLD custody'),
        ):
            run_bounded(['must not spawn'])
        spawn.assert_not_called()
        with (
            mock.patch.object(os, 'waitid', None),
            mock.patch.object(subprocess, 'Popen') as spawn,
            self.assertRaisesRegex(RuntimeError, 'WNOWAIT child custody'),
        ):
            run_bounded(['must not spawn'])
        spawn.assert_not_called()
        for timeout in [None, 0, float('inf')]:
            with (
                mock.patch.object(subprocess, 'Popen') as spawn,
                self.assertRaisesRegex(ValueError, 'positive and finite'),
            ):
                run_bounded(['must not spawn'], timeout=timeout)
            spawn.assert_not_called()

    def test_cleanup_deadline_failure_keeps_the_owned_process_handle(self):
        children = []
        popen = subprocess.Popen

        def spawn(*args, **options):
            process = popen(*args, **options)
            children.append(process)
            return process

        try:
            with (
                mock.patch.object(subprocess, 'Popen', side_effect=spawn),
                mock.patch.object(popen, 'wait', side_effect=subprocess.TimeoutExpired('cleanup fixture', 5)),
                self.assertRaises(RunBoundedCleanupUncertain) as failure,
            ):
                run_bounded([sys.executable, '-c', 'import time; time.sleep(60)'],
                            timeout=0.05, capture_output=True)
            self.assertIs(failure.exception.process, children[0])
            self.assertIsInstance(failure.exception.__cause__, subprocess.TimeoutExpired)
        finally:
            for process in children:
                process.wait(timeout=5)

    def test_timeout_kills_descendants_even_after_a_slow_start(self):
        for startup_delay in (0, 3.2):
            with self.subTest(startup_delay=startup_delay):
                with self.assertRaises(subprocess.TimeoutExpired):
                    self.run_after_handshake(startup_delay, ready_timeout=30)
                self.assert_group_gone()

    def test_grandchild_that_never_gets_ready_fails_at_its_deadline_and_is_killed(self):
        started = time.monotonic()
        with self.assertRaisesRegex(AssertionError, 'the grandchild to hold its lease'):
            self.run_after_handshake(startup_delay=60, ready_timeout=1)
        self.assertLess(time.monotonic() - started, 5)
        self.assert_group_gone()


KEY = r'(\s*(?:- )?\s*){}:(?:\s+(.*?))?(?:\s+#.*)?$'
# Rosetta passes an x86_64 uname check; only sysctl.proc_translated=1 reveals translation.
ROSETTA_CHECK = 'test "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" != 1'
# An --exact cargo filter that matches nothing exits 0; its teed log must show exactly one passed test.
EXACT_TEE = '| tee "$RUNNER_TEMP/frayd-retained-job-test.log"'
EXACT_RESULT = 'grep -F \'test result: ok. 1 passed; 0 failed;\' "$RUNNER_TEMP/frayd-retained-job-test.log"'


def entries(lines, key):
    """Yield (column, inline value, body) for every `key:` line; body is the deeper-indented lines after it."""
    for index, line in enumerate(lines):
        match = re.match(KEY.format(key), line.rstrip())
        if match:
            column, body = len(match[1]), []
            for later in lines[index + 1:]:
                if later.strip() and not later.lstrip().startswith('#'):
                    if len(later) - len(later.lstrip()) <= column:
                        break
                    body.append(later)
            yield column, match[2] or '', body


def children(body):
    """Direct `name: first-token` children of an entry body, found by its shallowest indentation."""
    indent = min((len(line) - len(line.lstrip()) for line in body), default=0)
    return {m[1]: m[2] for line in body if (m := re.match(r'\s*([\w-]+):\s*(\S*)', line)) and len(line) - len(line.lstrip()) == indent}


def workflow_policy_violations(text):
    """Check known policy invariants in this repository's block-style macOS workflow."""
    lines, problems = text.splitlines(), []
    triggers = [(inline, children(body)) for column, inline, body in entries(lines, 'on') if column == 0]
    if len(triggers) != 1 or triggers[0][0] or set(triggers[0][1]) != {'workflow_dispatch'}:
        problems.append(f'triggers must be exactly workflow_dispatch, found {triggers}')
    scopes = [(column, inline, children(body)) for column, inline, body in entries(lines, 'permissions')]
    problems += [f'permission scope can write: {inline or scope}' for _, inline, scope in scopes
                 if inline or any(level not in ('read', 'none') for level in scope.values())]
    if [scope for column, _, scope in scopes if column == 0] != [{'contents': 'read'}]:
        problems.append(f'top-level permissions must be exactly contents: read, found {scopes}')
    include = '\n'.join(line for _, _, body in entries(lines, 'include') for line in body)
    pairs = set(re.findall(r'runner:\s*(\S+)(?:(?!\n\s*- ).)*?target:\s*(\S+)', include, re.DOTALL))
    required = {('macos-15', 'aarch64-apple-darwin'), ('macos-15-intel', 'x86_64-apple-darwin')}
    if not required <= pairs:
        problems.append(f'matrix must include runner/target pairs {sorted(required)}, found {sorted(pairs)}')
    for _, uses, _ in entries(lines, 'uses'):
        action, _, reference = uses.partition('@')
        if re.search(r'upload|download|release|publish|deploy|attest', action, re.IGNORECASE):
            problems.append(f'publishing or artifact action: {uses}')
        if not re.fullmatch(r'[0-9a-f]{40}', reference):
            problems.append(f'action not pinned to a commit: {uses}')
    scripts = [line for _, inline, body in entries(lines, 'run') for line in (body if re.fullmatch(r'[|>][+-]?', inline) else [inline])]
    cargo = [line.strip() for line in scripts if re.search(r'(?<![\w./-])cargo\s', line)]
    if any(re.search(r'\blipo\b|brew install', line) for line in scripts):
        problems.append('universal binaries and brew installs are not part of the native qualification')
    if ROSETTA_CHECK not in [line.strip() for line in scripts]:
        problems.append(f'no step rejects Rosetta translation with exactly: {ROSETTA_CHECK}')
    exact = [line.strip() for line in cargo if '--exact' in line]
    if exact and (EXACT_RESULT not in [line.strip() for line in scripts] or not all(line.endswith(EXACT_TEE) for line in exact)):
        problems.append('a cargo --exact filter must tee its log and assert exactly one passed test')
    problems += [f'cargo without --locked: {line}' for line in cargo if '--locked' not in line]
    if not any('--bin frayd-native' in line for line in cargo):
        problems.append('no cargo invocation builds --bin frayd-native')
    return problems


UNINSTALL_SHAPE = [
    r'set -euo pipefail',
    r'FRAYD_BUNDLE=\$\(cd -- "\$\(dirname -- "\$0"\)" && pwd -P\)',
    r'if \[\[ ! -x "\$FRAYD_BUNDLE/frayd-native" \]\]; then',
    r"printf '%s\\n' '[^']*' >&2",
    r'exit 2',
    r'fi',
    r'exec "\$FRAYD_BUNDLE/frayd-native" uninstall "\$@"',
]


def uninstall_policy_violations(text):
    """List every statement in the legacy uninstall shim beyond its helper-presence guard and the final exec."""
    statements = [line.strip() for line in text.splitlines() if line.strip() and not line.strip().startswith('#')]
    problems = [f'statement outside the guard-and-exec shape: {statement}' for statement in statements
                if not any(re.fullmatch(pattern, statement) for pattern in UNINSTALL_SHAPE)]
    if len(statements) != len(UNINSTALL_SHAPE) or not re.fullmatch(UNINSTALL_SHAPE[-1], statements[-1]):
        problems.append('the shim must be the guard followed by exactly one final exec of the helper')
    return problems


class NativeSourceContract(unittest.TestCase):
    release_workflow = ROOT / '.github/workflows/release-mac.yml'
    pinned = '3d3c42e5aac5ba805825da76410c181273ba90b1'

    def test_manual_nonpublishing_workflow(self):
        """The macOS workflow is manual, read-only, non-publishing, pinned and locked."""
        self.assertEqual(workflow_policy_violations(self.release_workflow.read_text()), [])

    def test_workflow_policy_detects_each_violation(self):
        text = self.release_workflow.read_text()
        publishing = f'      - uses: actions/upload-artifact@{self.pinned}\n        with:\n          name: bundle\n'
        mutations = {
            'tag trigger': ('on:\n  workflow_dispatch:\n', 'on:\n  push:\n    tags:\n      - v1\n  workflow_dispatch:\n', 'triggers'),
            'write permission': ('contents: read', 'contents: write', 'permission'),
            'upload action': ('      - name: Nonsecret package', publishing + '      - name: Nonsecret package', 'publishing or artifact'),
            'floating action': (f'actions/checkout@{self.pinned}', 'actions/checkout@v4', 'not pinned'),
            'unlocked cargo': ('cargo +1.99.1 clippy --locked', 'cargo +1.99.1 clippy', 'cargo without --locked'),
            'missing Intel runner': ('macos-15-intel', 'macos-14', 'matrix must include'),
            'missing ARM target': ('target: aarch64-apple-darwin', 'target: aarch64-unknown-linux-gnu', 'matrix must include'),
            'universal binary': ('uname -sm', 'lipo -create a b', 'universal binaries'),
            'flow-style trigger': ('on:\n  workflow_dispatch:\n', 'on: [push, workflow_dispatch]\n', 'triggers'),
            'inline write-all': ('permissions:\n  contents: read', 'permissions: write-all', 'permission'),
            'job-level write': ('    timeout-minutes: 35\n', '    timeout-minutes: 35\n    permissions:\n      contents: write\n', 'permission scope can write'),
            'Rosetta check removed': ('          ' + ROSETTA_CHECK + '\n', '', 'Rosetta'),
            'Rosetta check commented out': ('          ' + ROSETTA_CHECK, '          # ' + ROSETTA_CHECK, 'Rosetta'),
            'Rosetta check neutralized': (ROSETTA_CHECK, ROSETTA_CHECK + ' || true', 'Rosetta'),
            'zero-test exact filter': ('          ' + EXACT_RESULT + '\n', '', 'exactly one passed test'),
            'exact filter log not kept': (' ' + EXACT_TEE, '', 'exactly one passed test'),
            'any passed count accepted': ("ok. 1 passed; 0 failed;'", "ok.'", 'exactly one passed test'),
        }
        for name, (old, new, expected) in mutations.items():
            with self.subTest(name):
                self.assertIn(old, text)
                problems = workflow_policy_violations(text.replace(old, new, 1))
                self.assertTrue(any(expected in problem for problem in problems), problems)

    def test_uninstall_shim_only_execs_the_helper(self):
        """The legacy shim has no process, service or filesystem deletion of its own."""
        script = ROOT / 'scripts/uninstall.sh'
        text = script.read_text()
        self.assertEqual(uninstall_policy_violations(text), [])
        for name, extra in {'pkill': 'pkill -f frayd', 'rm': 'rm -rf "$HOME/.frayd"', 'systemctl': 'systemctl --user stop frayd',
                            'launchctl': 'launchctl unload x'}.items():
            with self.subTest(name):
                mutated = text.replace('exec "$FRAYD_BUNDLE', extra + '\nexec "$FRAYD_BUNDLE', 1)
                self.assertIn(extra, mutated)
                self.assertTrue(uninstall_policy_violations(mutated))
        with self.subTest('extra exec'):
            self.assertTrue(uninstall_policy_violations(text + 'exec rm -rf /\n'))

    def test_uninstall_shim_forwards_arguments_and_status_or_refuses_without_the_helper(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            shim = bundle / 'uninstall.sh'
            shim.write_text((ROOT / 'scripts/uninstall.sh').read_text())
            shim.chmod(0o755)
            refused = run_bounded([str(shim), '--yes'], capture_output=True, text=True)
            self.assertEqual(refused.returncode, 2)
            self.assertIn('extracted native bundle', refused.stderr)
            helper = bundle / 'frayd-native'
            helper.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > "{bundle}/arguments"\nexit 7\n')
            helper.chmod(0o755)
            forwarded = run_bounded([str(shim), '--prefix', 'some prefix', '--yes'], capture_output=True, text=True)
            self.assertEqual(forwarded.returncode, 7)
            self.assertEqual((bundle / 'arguments').read_text().splitlines(), ['uninstall', '--prefix', 'some prefix', '--yes'])

    def test_runtime_discovery_requires_both_inputs(self):
        """A lone archive or build source skips the runtime class; with both, archive validation is reached."""
        spec = importlib.util.spec_from_file_location('native_bundle_probe', __file__)
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / 'archive.tar.gz'
            archive.write_bytes(b'synthetic archive without a checksum file')
            for with_archive, with_build_source in [(False, False), (True, False), (False, True), (True, True)]:
                with self.subTest(archive=with_archive, build_source=with_build_source), mock.patch.dict(os.environ):
                    os.environ.pop('FRAYD_NATIVE_ARCHIVE', None)
                    os.environ.pop('FRAYD_NATIVE_BUILD_SOURCE', None)
                    if with_archive:
                        os.environ['FRAYD_NATIVE_ARCHIVE'] = str(archive)
                    if with_build_source:
                        os.environ['FRAYD_NATIVE_BUILD_SOURCE'] = directory
                    probe = importlib.util.module_from_spec(spec)
                    spec.loader.exec_module(probe)
                    result = unittest.TestResult()
                    unittest.defaultTestLoader.loadTestsFromTestCase(probe.NativeRuntime).run(result)
                    if with_archive and with_build_source:
                        self.assertEqual((len(result.skipped), result.failures), (0, []))
                        self.assertEqual(len(result.errors), 1, result.errors)
                        self.assertRegex(result.errors[0][1], r'FileNotFoundError: .*archive\.tar\.gz\.sha256')
                    else:
                        self.assertEqual((len(result.skipped), result.errors, result.failures), (1, [], []))

    def test_required_qualification_fails_instead_of_skipping(self):
        """With FRAYD_NATIVE_REQUIRE_QUALIFICATION=1, missing inputs fail the module before any class skip,
        and a root run fails the one scenario that otherwise skips for root."""
        with mock.patch.dict(os.environ, {'FRAYD_NATIVE_REQUIRE_QUALIFICATION': '1'}):
            for name in ['FRAYD_NATIVE_HELPER', 'FRAYD_NATIVE_ARCHIVE', 'FRAYD_NATIVE_BUILD_SOURCE']:
                os.environ.pop(name, None)
            with self.assertRaisesRegex(RuntimeError, 'missing: FRAYD_NATIVE_HELPER, FRAYD_NATIVE_ARCHIVE, FRAYD_NATIVE_BUILD_SOURCE'):
                setUpModule()
        root_only = 'test_final_removal_failure_after_confirmation_restores_retry_metadata'
        for required, outcome in [('1', 'failures'), (None, 'skipped')]:
            with self.subTest(required=required), mock.patch.dict(os.environ), mock.patch('os.geteuid', return_value=0), \
                    mock.patch.object(NativeOwnership, 'setUp', lambda test: None):
                os.environ.pop('FRAYD_NATIVE_REQUIRE_QUALIFICATION', None)
                if required:
                    os.environ['FRAYD_NATIVE_REQUIRE_QUALIFICATION'] = required
                result = unittest.TestResult()
                NativeOwnership(root_only).run(result)
                counts = {kind: len(getattr(result, kind)) for kind in ['failures', 'errors', 'skipped']}
                self.assertEqual(counts, {'failures': 0, 'errors': 0, 'skipped': 0, outcome: 1}, result.failures + result.errors)


class NativeOwnership(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not os.environ.get('FRAYD_NATIVE_HELPER'):
            raise unittest.SkipTest('Supply a freshly built FRAYD_NATIVE_HELPER explicitly; generic Python discovery does not install/build Rust')
        if not HELPER.is_file():
            raise RuntimeError('Explicit native helper is unavailable')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='frayd native ', dir=TEMP_ROOT)
        self.preserve_fixture = False
        self.addCleanup(self.cleanup_fixture)
        self.base = Path(self.temp.name)
        self.bundle = self.base / 'extracted bundle'
        self.bundle.mkdir(mode=0o700)
        shutil.copy2(HELPER, self.bundle / 'frayd-native')
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nprintf "%s\\n" "$FRAYD_BIND" "$FRAYD_DB" "$FRAYD_CLIENT_DIR"\n')
        (self.bundle / 'frayd-server').chmod(0o755)
        (self.bundle / 'client').mkdir()
        (self.bundle / 'client/index.html').write_text('fixture')
        for name, source in [('install.sh', 'scripts/native/install.sh'), ('start.sh', 'scripts/native/start.sh'), ('uninstall.sh', 'scripts/uninstall.sh')]:
            shutil.copy2(ROOT / source, self.bundle / name)
        (self.bundle / 'README.md').write_text('fixture bundle')
        self.manifest()
        self.prefix = self.base / 'installed app'
        self.data = self.base / 'private data'
        for name in ['.cargo', '.rustup', 'media library', 'original checkout/.git']:
            (self.base / name).mkdir(parents=True)
        for name in ['.cargo/sentinel', '.rustup/sentinel', 'media library/sentinel', 'original checkout/.env', 'original checkout/config.json']:
            (self.base / name).write_bytes(b'NONSECRET protected fixture sentinel')

    def cleanup_fixture(self):
        if self.preserve_fixture:
            self.temp._finalizer.detach()
            raise AssertionError(f'uncertain native fixture retained at {self.base}')
        self.temp.cleanup()

    def manifest(self):
        entries = []
        for p in sorted(self.bundle.rglob('*')):
            if p.name == 'manifest.json':
                continue
            entries.append({'path': p.relative_to(self.bundle).as_posix(), 'kind': 'dir' if p.is_dir() else 'file', 'mode': stat.S_IMODE(p.stat().st_mode), 'size': p.stat().st_size if p.is_file() else 0, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else ''})
        (self.bundle / 'manifest.json').write_text(json.dumps({'schema': 1, 'version': '0.1.1', 'source': 'a' * 40, 'target': 'fixture', 'toolchain': 'fixture', 'entries': entries}))

    def run_fixture(self, arguments, **options):
        try:
            return run_bounded(arguments, **options)
        except (RunBoundedCleanupUncertain, subprocess.TimeoutExpired):
            # Reaping a helper does not establish exit of its separate server group.
            self.preserve_fixture = True
            raise

    def run_helper(self, command, *args, code=0, answer='', environment=None, timeout=60):
        result = self.run_fixture([str(HELPER), command, '--prefix', str(self.prefix), *args], input=answer, text=True, capture_output=True, env=environment, timeout=timeout)
        self.assertEqual(result.returncode, code, result.stderr)
        return result

    def install(self, *args, **kw):
        return self.run_helper('install', '--bundle', str(self.bundle), '--data-dir', str(self.data), *args, **kw)

    def snapshot(self):
        return snapshot(self.base)

    def prompt(self, command, *args):
        """Start a helper command, wait at its consent prompt and return the process and the prompt bytes."""
        process = subprocess.Popen([str(HELPER), command, '--prefix', str(self.prefix), *args],
                                   stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
        self.addCleanup(self.stop_helper, process)
        os.set_blocking(process.stderr.fileno(), False)
        prompt = bytearray()

        def prompted():
            prompt.extend(process.stderr.read() or b'')
            return prompt.endswith(b'[y/N] ') or process.poll() is not None
        wait_for(prompted, f'{command} consent prompt')
        self.assertTrue(prompt.endswith(b'[y/N] '), prompt)
        os.set_blocking(process.stderr.fileno(), True)
        return process, bytes(prompt)

    def stop_helper(self, process):
        """Ask the unreaped supervisor to stop its own group; never guess a child PGID."""
        if process.poll() is None:
            process.send_signal(signal.SIGTERM)
        try:
            process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            self.preserve_fixture = True
            raise AssertionError(f'cleanup uncertain; native fixture retained at {self.base}') from None

    def test_start_timeout_retains_separately_grouped_server_fixture(self):
        stop = self.base / 'timeout-server-stop'
        ready = self.base / 'timeout-server-ready'
        lease = self.base / 'timeout-server-lease'
        done = self.base / 'timeout-server-done'
        (self.bundle / 'frayd-server').write_text(
            '#!' + sys.executable + '\nimport fcntl,json,os,pathlib,time\n'
            'handle=open(' + repr(str(lease)) + ', "w")\n'
            'fcntl.flock(handle,fcntl.LOCK_EX)\n'
            'pathlib.Path(' + repr(str(ready)) + ').write_text(json.dumps([os.getpid(),os.getpgrp()]))\n'
            'stop=pathlib.Path(' + repr(str(stop)) + ')\n'
            'for _ in range(1500):\n if stop.exists(): break\n time.sleep(.02)\n'
            'fcntl.flock(handle,fcntl.LOCK_UN)\n'
            'pathlib.Path(' + repr(str(done)) + ').touch()\n')
        self.manifest()
        self.install('--yes')
        communicate = subprocess.Popen.communicate
        helper_pids = []

        def lease_held():
            with lease.open('a') as handle:
                try:
                    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    return True
                fcntl.flock(handle, fcntl.LOCK_UN)
                return False

        def communicate_after_ready(process, *args, **options):
            helper_pids.append(process.pid)
            wait_for(lambda: ready.exists() and shared_running_child_pid(self.prefix, self.data),
                     'separately grouped server readiness')
            # Isolate the communication timeout from interpreter/helper startup.
            options['timeout'] = 0.05
            return communicate(process, *args, **options)

        try:
            with mock.patch.object(subprocess.Popen, 'communicate', communicate_after_ready):
                with self.assertRaises(subprocess.TimeoutExpired):
                    self.run_helper('start', timeout=20)
            child, group = json.loads(ready.read_text())
            self.assertEqual(shared_running_child_pid(self.prefix, self.data), child)
            self.assertEqual(group, child)
            self.assertNotEqual(group, helper_pids[0])
            self.assertTrue(lease_held(), 'server must survive helper-group cleanup')
            self.assertTrue(self.preserve_fixture, 'timeout must retain the live server fixture')
            before = self.snapshot()
            with self.assertRaisesRegex(AssertionError, 'uncertain native fixture retained'):
                self.cleanup_fixture()
            self.assertFalse(self.temp._finalizer.alive, 'retained fixture must outlive its finalizer')
            self.assertEqual(self.snapshot(), before, 'cleanup must retain receipts and live process data')
            self.assertTrue(lease_held())
        finally:
            # This fixture owns a stop-file protocol, not custody of the orphan PID.
            stop.touch()
            try:
                wait_for(lambda: done.exists() and not lease_held(), 'orphan fixture cooperative exit')
            except BaseException:
                self.preserve_fixture = True
                raise
            else:
                self.preserve_fixture = False
                self.temp.cleanup()

    def test_direct_install_cleanup_uncertainty_retains_fixture(self):
        self.prefix.mkdir()
        (self.prefix / '.frayd-install.json').write_text('NONSECRET fixture receipt')
        before = self.snapshot()
        failure = RunBoundedCleanupUncertain('injected direct-install cleanup failure')
        try:
            with mock.patch.object(self, 'install'), mock.patch(__name__ + '.run_bounded', side_effect=failure):
                with self.assertRaises(RunBoundedCleanupUncertain) as raised:
                    self.test_permissive_umask_and_unknown_existing_data()
            self.assertIs(raised.exception, failure)
            self.assertTrue(self.preserve_fixture)
            with self.assertRaisesRegex(AssertionError, 'uncertain native fixture retained'):
                self.cleanup_fixture()
            self.assertFalse(self.temp._finalizer.alive, 'retained fixture must outlive its finalizer')
            self.assertEqual(self.snapshot(), before)
        finally:
            # The injected runner never spawns; this test retains no live child.
            self.preserve_fixture = False
            self.temp.cleanup()

    def test_package_complete_manifest_and_architecture(self):
        output = self.base / 'package output'
        result = package(output, runner=self.run_fixture, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = next(output.glob('*.tar.gz'))
        self.assertEqual(archive.with_suffix(archive.suffix + '.sha256').read_text().split()[0], hashlib.sha256(archive.read_bytes()).hexdigest())
        payload = next(path for path in output.iterdir() if path.is_dir())
        manifest = json.loads((payload / 'manifest.json').read_text())
        self.assertTrue(manifest['toolchain'].startswith('rustc '))
        if os.environ.get('FRAYD_NATIVE_TOOLCHAIN_FILE'):
            self.assertEqual(manifest['toolchain'], Path(os.environ['FRAYD_NATIVE_TOOLCHAIN_FILE']).read_text().strip())
        expected = {'frayd-native', 'frayd-server', 'install.sh', 'start.sh', 'uninstall.sh', 'README.md', 'client/vendor/plyr/plyr.min.js', 'build-info.json'}
        self.assertTrue(expected.issubset({item['path'] for item in manifest['entries']}))
        self.assertFalse(any('.env' in item['path'] for item in manifest['entries']))
        self.assertEqual(manifest['target'], TARGET)
        identity = json.loads(self.run_fixture([str(SERVER), '--build-info'], check=True, capture_output=True, text=True).stdout)
        self.assertEqual(json.loads((payload / 'build-info.json').read_text()), identity)
        self.assertEqual(identity['commit'], manifest['source'])
        self.bundle = payload
        self.install('--yes')
        self.run_helper('uninstall', '--yes')
        fake_output = self.base / 'helper is not server'
        rejected = package(fake_output, runner=self.run_fixture, server=HELPER, capture_output=True)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertFalse(fake_output.exists())
        wrong_target = ('x86_64' if ARCHITECTURE == 'aarch64' else 'aarch64') + TARGET[len(ARCHITECTURE):]
        before = self.snapshot()
        result = package(self.base / 'wrong architecture output', runner=self.run_fixture, target=wrong_target, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.snapshot(), before)

    def test_package_refuses_server_as_helper_before_execution_or_output(self):
        # A copied or hardlinked server has the right architecture and server identity.
        server = self.base / 'server'
        shutil.copy2(SERVER, server)
        copied = self.base / 'copied server'
        linked = self.base / 'hardlinked server'
        shutil.copy2(SERVER, copied)
        os.link(server, linked)
        spec = importlib.util.spec_from_file_location('native_packager', PACKAGE)
        packager = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(packager)
        for helper in [server, copied, linked]:
            output = self.base / 'rejected duplicate'
            with self.subTest(helper=helper), mock.patch.object(sys, 'argv', [str(PACKAGE), str(output), TARGET, str(server), str(helper)]), \
                    mock.patch.object(packager.subprocess, 'check_output', side_effect=AssertionError('binary or git executed before duplicate rejection')):
                with self.assertRaisesRegex(SystemExit, 'Server and helper binaries must differ'):
                    packager.main()
            self.assertFalse(output.exists())
            rejected = package(output, runner=self.run_fixture, server=server, helper=helper, capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn('Server and helper binaries must differ', rejected.stderr)
            self.assertFalse(output.exists())

    def test_package_rejects_invalid_helper_identity_without_output(self):
        identity = json.loads(self.run_fixture([str(SERVER), '--build-info'], check=True, capture_output=True, text=True).stdout)
        valid = dict(identity, role='frayd-native')
        invalid = [
            ('wrong role', dict(valid, role='frayd-server')),
            ('missing role', identity),
            ('version mismatch', dict(valid, version='999.0.0')),
            ('commit mismatch', dict(valid, commit='0' * 40)),
            ('unknown commit', dict(valid, commit=None)),
            ('dirty mismatch', dict(valid, dirty=not identity['dirty'])),
            ('malformed dirty', dict(valid, dirty=1)),
            ('tag distance mismatch', dict(valid, commits_since_tag=(identity['commits_since_tag'] or 0) + 1)),
            ('malformed count', dict(valid, commits_since_tag=True)),
            ('missing count', {key: value for key, value in valid.items() if key != 'commits_since_tag'}),
            ('array', []),
            ('malformed JSON', None),
        ]
        spec = importlib.util.spec_from_file_location('native_packager', PACKAGE)
        packager = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(packager)
        helper = self.base / 'identity fixture'
        for name, document in invalid:
            text = 'not JSON' if document is None else json.dumps(document)
            helper.write_text('#!/bin/sh\nprintf %s ' + shlex.quote(text) + '\n')
            helper.chmod(0o755)
            output = self.base / ('rejected identity ' + name)
            with self.subTest(name=name), mock.patch.object(sys, 'argv', [str(PACKAGE), str(output), TARGET, str(SERVER), str(helper)]), \
                    mock.patch.object(packager, 'check_binary'):
                # Only bypass architecture for the shell identity fixture; execute its real CLI.
                with self.assertRaisesRegex(SystemExit, 'Helper binary'):
                    packager.main()
            self.assertFalse(output.exists())

    def test_helper_build_info_is_read_only_and_strict(self):
        before = self.snapshot()
        environment = dict(os.environ, FRAYD_DB=str(self.base / 'must not create.db'), FRAYD_LIBRARY=str(self.base / 'must not create library'))
        result = self.run_fixture([str(HELPER), '--build-info'], cwd=self.base, env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        identity = json.loads(result.stdout)
        self.assertEqual(identity.pop('role'), 'frayd-native')
        server_identity = json.loads(self.run_fixture([str(SERVER), '--build-info'], check=True, capture_output=True, text=True).stdout)
        self.assertEqual(identity, server_identity)
        rejected = self.run_fixture([str(HELPER), '--build-info', '--prefix', str(self.prefix)], cwd=self.base, env=environment, capture_output=True)
        self.assertEqual(rejected.returncode, 2)
        self.assertEqual(self.snapshot(), before)

    def test_tar_preserved_modes_under_restrictive_umask(self):
        output = self.base / 'umask package'
        package(output, runner=self.run_fixture, check=True, capture_output=True)
        archive = next(output.glob('*.tar.gz'))
        original = next(path for path in output.iterdir() if path.is_dir())
        self.assertIn('tar -xpzf', (original / 'README.md').read_text())
        plain = self.base / 'plain extraction'
        preserved = self.base / 'preserved extraction'
        plain.mkdir(mode=0o700)
        preserved.mkdir(mode=0o700)
        self.run_fixture(['tar', '-xzf', str(archive), '-C', str(plain)], umask=0o077, check=True, capture_output=True)
        self.run_fixture(['tar', '-xpzf', str(archive), '-C', str(preserved)], umask=0o077, check=True, capture_output=True)
        plain_bundle = next(plain.iterdir())
        preserved_bundle = next(preserved.iterdir())
        self.assertNotEqual(stat.S_IMODE((plain_bundle / 'client/index.html').stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE((preserved_bundle / 'client/index.html').stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE((preserved_bundle / 'install.sh').stat().st_mode), 0o755)
        before = self.snapshot()
        refused = self.run_fixture([str(plain_bundle / 'install.sh'), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], capture_output=True)
        self.assertEqual(refused.returncode, 2)
        self.assertEqual(self.snapshot(), before)
        self.run_fixture([str(preserved_bundle / 'install.sh'), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], check=True, capture_output=True)
        self.run_fixture([str(preserved_bundle / 'uninstall.sh'), '--prefix', str(self.prefix), '--yes'], check=True, capture_output=True)
        self.assertFalse(self.prefix.exists())
        self.assertTrue((self.data / '.frayd-data.json').is_file())

    def test_decline_zero_effects(self):
        before = self.snapshot()
        self.install(answer='n\n')
        self.assertEqual(self.snapshot(), before)
        self.install('--yes')
        before = self.snapshot()
        self.run_helper('uninstall', answer='n\n')
        self.assertEqual(self.snapshot(), before)

    def test_server_input_is_eof_even_when_supervisor_has_input(self):
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nif read -r value; then echo consumed-input; else echo input-eof; fi\n')
        self.manifest()
        self.install('--yes')
        result = self.run_helper('start', answer='supervisor-only-input\n')
        self.assertEqual(result.stdout.strip(), 'input-eof')

    def test_tostop_output_tty_refused_before_child_receipts(self):
        self.install('--yes')
        for stream in ['stdout', 'stderr']:
            for control in ['blocked', 'redirected', 'allowed']:
                with self.subTest(stream=stream, control=control):
                    # Darwin revokes a controlling PTY when its session leader
                    # exits. Each subprocess needs its own terminal and session.
                    master, slave = pty.openpty()
                    try:
                        attributes = termios.tcgetattr(slave)
                        if control == 'allowed':
                            attributes[3] &= ~termios.TOSTOP
                        else:
                            attributes[3] |= termios.TOSTOP
                        termios.tcsetattr(slave, termios.TCSANOW, attributes)
                        options = {'stdout': subprocess.PIPE, 'stderr': subprocess.PIPE}
                        if control != 'redirected':
                            options[stream] = slave
                        result = self.run_fixture([str(HELPER), 'start', '--prefix', str(self.prefix)],
                                                  stdin=subprocess.DEVNULL, timeout=10, terminal_master=master,
                                                  preexec_fn=lambda terminal=slave: fcntl.ioctl(terminal, termios.TIOCSCTTY, 0), **options)
                        if control == 'blocked':
                            self.assertEqual(result.returncode, 2)
                            diagnostic = result.stderr if stream == 'stdout' else result.terminal_output
                            self.assertIn(b'TOSTOP', diagnostic)
                            self.assertFalse((self.prefix / '.frayd-child.json').exists())
                            self.assertFalse((self.data / '.frayd-child.json').exists())
                        else:
                            self.assertEqual(result.returncode, 0, result.stderr)
                            if control == 'allowed' and stream == 'stdout':
                                self.assertEqual(result.terminal_output.splitlines(),
                                                 [b'127.0.0.1', str(self.data / 'frayd.db').encode(), str(self.prefix / 'client').encode()])
                    finally:
                        os.close(slave)
                        os.close(master)
                if self.preserve_fixture:
                    return

    def test_install_start_remove_preserves_data_and_reinstall(self):
        self.install('--yes')
        result = self.run_helper('start', '--port', '28484')
        self.assertEqual(result.stdout.splitlines(), ['127.0.0.1', str(self.data / 'frayd.db'), str(self.prefix / 'client')])
        sentinel = self.data / 'library.db'
        sentinel.write_bytes(b'keep')
        self.run_helper('uninstall', '--yes')
        self.assertFalse(self.prefix.exists())
        self.assertEqual(sentinel.read_bytes(), b'keep')
        self.install('--yes')
        self.run_helper('uninstall', '--yes')

    def test_removal_without_data_authority_preserves_all_remaining_data(self):
        for state in ['moved directory', 'deleted directory', 'missing marker', 'malformed marker']:
            with self.subTest(state=state):
                self.prefix = self.base / ('installed ' + state)
                self.data = self.base / ('data ' + state)
                self.install('--yes')
                (self.data / 'frayd.db').write_bytes(b'NONSECRET preserved database')
                marker = self.data / '.frayd-data.json'
                if state == 'moved directory':
                    self.data.rename(self.base / 'moved data')
                elif state == 'deleted directory':
                    marker.unlink()
                    (self.data / 'frayd.db').unlink()
                    self.data.rmdir()
                elif state == 'missing marker':
                    marker.unlink()
                else:
                    marker.write_bytes(b'NONSECRET malformed marker')
                before = self.snapshot()
                self.run_helper('start', code=2)
                self.assertEqual(self.snapshot(), before)
                self.run_helper('uninstall', '--yes')
                self.assertFalse(self.prefix.exists())
                remaining = [item for item in before if not (item[0] == self.prefix.name or item[0].startswith(self.prefix.name + '/'))]
                self.assertEqual(self.snapshot(), remaining)
                self.assertEqual(self.data.exists(), state in ['missing marker', 'malformed marker'])

    def test_missing_data_marker_does_not_weaken_removal_guards(self):
        for guard in ['modified payload', 'unknown payload', 'uncertain child', 'data alias overlap']:
            with self.subTest(guard=guard):
                self.prefix = self.base / ('installed ' + guard)
                self.data = self.base / ('data ' + guard)
                self.install('--yes')
                marker = self.data / '.frayd-data.json'
                marker.unlink()
                if guard == 'modified payload':
                    target = self.prefix / 'client/index.html'
                    original = target.read_bytes()
                    target.write_bytes(b'changed')
                elif guard == 'unknown payload':
                    target = self.prefix / 'unknown'
                    target.write_bytes(b'unknown')
                elif guard == 'uncertain child':
                    target = self.prefix / '.frayd-child.json'
                    target.write_bytes(b'uncertain')
                else:
                    self.data.rmdir()
                    self.data.symlink_to(self.prefix, target_is_directory=True)
                before = self.snapshot()
                self.run_helper('start', code=2)
                self.run_helper('uninstall', '--yes', code=2)
                self.assertEqual(self.snapshot(), before)
                if guard == 'modified payload':
                    target.write_bytes(original)
                elif guard == 'data alias overlap':
                    self.data.unlink()
                else:
                    target.unlink()
                self.run_helper('uninstall', '--yes')

    def test_missing_absolute_library_keeps_installed_lifecycle_available(self):
        self.install('--yes')
        sentinel = self.data / 'frayd.db'
        sentinel.write_bytes(b'NONSECRET preserved offline fixture database')
        external = self.base / 'external media parent'
        external.mkdir()
        alias = self.base / 'external media alias'
        alias.symlink_to(external, target_is_directory=True)
        for parent in [self.base, alias]:
            missing = parent / 'offline media' / 'nested root'
            environment = os.environ.copy()
            environment['FRAYD_LIBRARY'] = str(missing)
            before = self.snapshot()
            result = self.run_helper('start', environment=environment)
            self.assertEqual(result.stdout.splitlines(), ['127.0.0.1', str(self.data / 'frayd.db'), str(self.prefix / 'client')])
            self.assertEqual(self.snapshot(), before)
            self.assertFalse(missing.exists())
        self.run_helper('uninstall', '--yes', environment=environment)
        self.assertFalse(self.prefix.exists())
        self.assertEqual(sentinel.read_bytes(), b'NONSECRET preserved offline fixture database')
        self.assertFalse(missing.exists())

    def test_prospective_missing_library_overlap_refuses_without_writes(self):
        for parent in [self.prefix, self.data, self.bundle]:
            with self.subTest(operation='install', parent=parent.name):
                missing = parent / 'offline media' / 'nested root'
                environment = os.environ.copy()
                environment['FRAYD_LIBRARY'] = str(missing)
                before = self.snapshot()
                self.install('--yes', code=2, environment=environment)
                self.assertEqual(self.snapshot(), before)
                self.assertFalse(missing.exists())
        self.install('--yes')
        alias = self.base / 'data alias'
        alias.symlink_to(self.data, target_is_directory=True)
        dangling = self.base / 'dangling media alias'
        dangling.symlink_to(self.base / 'unavailable target', target_is_directory=True)
        not_directory = self.base / 'not a directory'
        not_directory.write_bytes(b'NONSECRET immutable fixture file')
        for parent in [self.prefix, self.data, alias, dangling, not_directory]:
            with self.subTest(operation='installed', parent=parent.name):
                missing = parent / 'offline media' / 'nested root'
                environment = os.environ.copy()
                environment['FRAYD_LIBRARY'] = str(missing)
                before = self.snapshot()
                self.run_helper('start', code=2, environment=environment)
                self.run_helper('uninstall', '--yes', code=2, environment=environment)
                self.assertEqual(self.snapshot(), before)
                self.assertFalse(missing.exists())

    def test_incomplete_bundle_refuses_before_writes(self):
        (self.bundle / 'start.sh').unlink()
        self.manifest()
        before = self.snapshot()
        self.install('--yes', code=2)
        self.assertEqual(self.snapshot(), before)

    def test_corrupt_or_extra_bundle_zero_effects(self):
        for mutation in ['checksum', 'extra', 'traversal', 'symlink']:
            with self.subTest(mutation=mutation):
                index = self.bundle / 'client/index.html'
                original = index.read_bytes()
                manifest = (self.bundle / 'manifest.json').read_bytes()
                if mutation == 'checksum':
                    index.write_text('changed')
                elif mutation == 'extra':
                    (self.bundle / 'surprise').write_text('unknown')
                elif mutation == 'symlink':
                    index.unlink()
                    index.symlink_to('/etc/passwd')
                else:
                    doc = json.loads(manifest)
                    doc['entries'][0]['path'] = '../escape'
                    (self.bundle / 'manifest.json').write_text(json.dumps(doc))
                before = self.snapshot()
                self.install('--yes', code=2)
                self.assertEqual(self.snapshot(), before)
                if index.is_symlink():
                    index.unlink()
                index.write_bytes(original)
                (self.bundle / 'surprise').unlink(missing_ok=True)
                (self.bundle / 'manifest.json').write_bytes(manifest)

    def test_permissive_umask_and_unknown_existing_data(self):
        self.data.mkdir(mode=0o700)
        (self.data / '.env').write_bytes(b'NONSECRET fixture configuration')
        (self.data / 'frayd.db').write_bytes(b'NONSECRET fixture database')
        before = self.snapshot()
        self.install('--yes', code=2)
        self.assertEqual(self.snapshot(), before)
        (self.data / '.env').unlink()
        (self.data / 'frayd.db').unlink()
        self.data.rmdir()
        result = self.run_fixture([str(HELPER), 'install', '--bundle', str(self.bundle), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], umask=0, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(stat.S_IMODE(self.prefix.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(self.data.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((self.prefix / '.frayd-install.json').stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE((self.data / '.frayd-data.json').stat().st_mode), 0o600)

    def test_unknown_modified_and_extra_prefix_refused(self):
        self.prefix.mkdir()
        (self.prefix / '.env').write_text('preserve')
        before = self.snapshot()
        self.install('--yes', code=2)
        self.run_helper('uninstall', '--yes', code=2)
        self.assertEqual(self.snapshot(), before)
        (self.prefix / '.env').unlink()
        self.prefix.rmdir()
        self.install('--yes')
        for name in ['client/index.html', 'unowned']:
            path = self.prefix / name
            original = path.read_bytes() if path.exists() else None
            path.write_text('preserve')
            before = self.snapshot()
            self.run_helper('uninstall', '--yes', code=2)
            self.assertEqual(self.snapshot(), before)
            if original is None:
                path.unlink()
            else:
                path.write_bytes(original)

    def test_filesystem_failure_preserves_receipt_for_retry(self):
        self.install('--yes')
        self.base.chmod(0o500)
        self.addCleanup(self.base.chmod, 0o700)
        before = self.snapshot()
        self.run_helper('uninstall', '--yes', code=2)
        self.assertTrue((self.prefix / '.frayd-install.json').is_file(), 'removal failure must retain authority for retry')
        self.assertEqual(self.snapshot(), before)
        self.base.chmod(0o700)
        self.run_helper('uninstall', '--yes')
        self.assertFalse(self.prefix.exists())

    def test_final_removal_failure_after_confirmation_restores_retry_metadata(self):
        if os.geteuid() == 0:
            reason = 'root bypasses the parent mode that makes the final prefix removal fail'
            if os.environ.get('FRAYD_NATIVE_REQUIRE_QUALIFICATION') == '1':
                self.fail(reason + '; required qualification must run unprivileged')
            self.skipTest(reason)
        self.install('--yes')
        receipt = (self.prefix / '.frayd-install.json').read_bytes()
        # The parent-mode preflight passes; the parent becomes read-only while the helper waits for consent.
        process = subprocess.Popen(
            [str(HELPER), 'uninstall', '--prefix', str(self.prefix)],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
        try:
            os.set_blocking(process.stderr.fileno(), False)
            prompt = bytearray()

            def prompted():
                prompt.extend(process.stderr.read() or b'')
                return prompt.endswith(b'[y/N] ') or process.poll() is not None
            wait_for(prompted, 'uninstall confirmation prompt')
            self.assertTrue(prompt.endswith(b'[y/N] '), prompt)
            os.set_blocking(process.stderr.fileno(), True)
            self.base.chmod(0o500)
            self.addCleanup(self.base.chmod, 0o700)
            _, stderr = process.communicate(b'y\n', timeout=10)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
        self.assertEqual(process.returncode, 2, stderr)
        self.assertIn(b'final removal failed', stderr)
        self.assertEqual(sorted(p.name for p in self.prefix.iterdir()), ['.frayd-install.json', '.frayd-lock'])
        self.assertEqual((self.prefix / '.frayd-install.json').read_bytes(), receipt)
        self.assertEqual((self.prefix / '.frayd-lock').read_bytes(), b'')
        for name in ['.frayd-install.json', '.frayd-lock']:
            self.assertEqual(stat.S_IMODE((self.prefix / name).stat().st_mode), 0o600, name)
        self.base.chmod(0o700)
        self.run_helper('uninstall', '--yes')
        self.assertFalse(self.prefix.exists())
        self.assertTrue((self.data / '.frayd-data.json').is_file())

    def test_partial_removal_retry_and_data_redirect_refusal(self):
        self.install('--yes')
        redirect = self.data / 'thumbs'
        redirect.symlink_to(self.base, target_is_directory=True)
        before = self.snapshot()
        self.run_helper('start', code=2)
        self.assertEqual(self.snapshot(), before)
        redirect.unlink()
        # Simulate a prior interrupted exact-file removal; receipt remains authority.
        (self.prefix / 'client/index.html').unlink()
        self.run_helper('uninstall', '--yes')
        self.assertFalse(self.prefix.exists())
        self.assertTrue((self.data / '.frayd-data.json').exists())

    def test_install_cancellation_rolls_back_only_owned_staging_paths(self):
        # A large sparse payload gives an observable copying window without a production test hook.
        payload = self.bundle / 'client/slow-copy.bin'
        with payload.open('wb') as file:
            file.truncate(64 * 1024 * 1024)
        self.manifest()
        for cancellation in [signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT]:
            for unknown in [False, True]:
                with self.subTest(cancellation=cancellation, unknown=unknown):
                    process = subprocess.Popen([str(HELPER), 'install', '--bundle', str(self.bundle),
                        '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'],
                        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
                    try:
                        def copying():
                            for stage in self.base.glob('.frayd-stage-*'):
                                try:
                                    copied = (stage / 'client/slow-copy.bin').stat().st_size
                                except FileNotFoundError:
                                    continue
                                if 0 < copied < payload.stat().st_size:
                                    return stage
                            return None
                        stage = wait_for(copying, 'partially copied install payload (fixture handshake)', timeout=30, interval=0.0005)
                        os.kill(process.pid, signal.SIGSTOP)
                        os.waitpid(process.pid, os.WUNTRACED)
                        self.assertIsNone(process.poll(), 'fixture setup: installer must still be copying')
                        copied = (stage / 'client/slow-copy.bin').stat().st_size
                        self.assertTrue(0 < copied < payload.stat().st_size, 'fixture setup: pause missed the partial-copy window')
                        sentinel = stage / 'unknown-preserve'
                        if unknown:
                            sentinel.write_bytes(b'preserve unowned staging entry')
                        os.kill(process.pid, cancellation)
                        os.kill(process.pid, signal.SIGCONT)
                        _, stderr = process.communicate(timeout=30)
                        self.assertEqual(process.returncode, 2, stderr)
                        self.assertIn(b'installation cancelled', stderr)
                        self.assertFalse(self.prefix.exists(), 'owned empty reservation must be removed')
                        if unknown:
                            self.assertEqual(sentinel.read_bytes(), b'preserve unowned staging entry')
                            self.assertIn(str(stage).encode(), stderr)
                            sentinel.unlink()
                            stage.rmdir()
                        else:
                            self.assertFalse(stage.exists())
                    finally:
                        if process.poll() is None:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait(timeout=10)
                        process.stderr.close()

    def test_inherited_ignored_sigchld_refuses_before_child_records_or_spawn(self):
        started = self.base / 'must-not-start'
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\ntouch ' + shlex.quote(str(started)) + '\n')
        self.manifest()
        self.install('--yes')
        before = self.snapshot()
        process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True,
            preexec_fn=lambda: signal.signal(signal.SIGCHLD, signal.SIG_IGN))
        _, stderr = process.communicate(timeout=10)
        self.assertEqual(process.returncode, 2, stderr)
        self.assertIn(b'unsupported SIGCHLD disposition', stderr)
        self.assertEqual(self.snapshot(), before)
        self.assertFalse(started.exists())
        for root in [self.prefix, self.data]:
            self.assertFalse((root / '.frayd-child.json').exists())

    def test_changed_child_group_retains_receipts_without_signalling_shared_group(self):
        stop = self.base / 'escaped-stop'
        ready = self.base / 'escaped-ready'
        done = self.base / 'escaped-done'
        server = self.bundle / 'frayd-server'
        server.write_text('#!' + sys.executable + '\nimport os,pathlib,time\n'
            'os.setpgid(0, os.getppid())\npathlib.Path(' + repr(str(ready)) + ').touch()\n'
            'stop=pathlib.Path(' + repr(str(stop)) + ')\n'
            'for _ in range(1000):\n if stop.exists(): break\n time.sleep(.02)\n'
            'pathlib.Path(' + repr(str(done)) + ').touch()\n')
        self.manifest()
        self.install('--yes')
        process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
        primary_error = None
        try:
            def child_receipts():
                child = shared_running_child_pid(self.prefix, self.data)
                if child is None and process.poll() is not None:
                    stdout, stderr = process.communicate(timeout=2)
                    self.fail(f'helper exited before child receipts: exit={process.returncode}; '
                              f'stdout={stdout!r}; stderr={stderr!r}')
                return child
            child = wait_for(child_receipts, 'child receipts')
            wait_for(ready.exists, 'fixture child group change')
            self.assertEqual(os.getpgid(child), process.pid)
            process.send_signal(signal.SIGTERM)
            self.assertEqual(process.wait(timeout=10), 2)
            for root in [self.prefix, self.data]:
                self.assertTrue((root / '.frayd-child.json').exists(), 'uncertain cleanup must retain both receipts')
            self.run_helper('start', code=2)
            self.run_helper('uninstall', '--yes', code=2)
            self.assertFalse(done.exists(), 'refusal must not signal the shared group')
        except BaseException as error:
            primary_error = error
            raise
        finally:
            # Always request cooperative stop; never signal the escaped PID/group.
            try:
                stop.touch()
                if process.poll() is None:
                    process.send_signal(signal.SIGTERM)
                    process.wait(timeout=10)
                if ready.exists():
                    wait_for(done.exists, 'escaped fixture self-exit', timeout=25)
                elif any((root / '.frayd-child.json').exists() for root in [self.prefix, self.data]):
                    self.fail('escaped fixture readiness unknown; retain fixture for inspection')
                stdout, stderr = process.communicate(timeout=10)
                if primary_error is None:
                    self.assertIn(b'owned child group identity uncertain', stderr,
                                  f'stdout={stdout!r}; stderr={stderr!r}')
            except BaseException as cleanup_error:
                self.preserve_fixture = True
                if primary_error is None:
                    raise
                primary_error.add_note(f'cleanup uncertain; fixture retained at {self.base}: {cleanup_error}')
            process.stderr.close()

    def test_owned_server_group_stops_descendants_and_preserves_foreign_group(self):
        foreign = subprocess.Popen(['sleep', '600'], start_new_session=True)
        self.addCleanup(foreign.wait, 10)
        self.addCleanup(foreign.kill)
        for stop_mode in ['normal', 'helper-term', 'helper-hup', 'group-term', 'group-hup', 'helper-term-ignore']:
            with self.subTest(stop_mode=stop_mode):
                stop = self.base / 'worker-stop'
                pidfile = self.base / 'worker-pid'
                worker = self.base / 'worker.py'
                release = self.base / 'server-release'
                done = self.base / 'worker-done'
                ready = self.base / 'worker-ready'
                lease = self.base / 'worker-lease'
                worker.write_text('import fcntl,pathlib,signal,time\n'
                    + ('signal.signal(signal.SIGTERM, signal.SIG_IGN)\n' if stop_mode.endswith('ignore') else '')
                    + 'f=open(' + repr(str(lease)) + ', \'w\');fcntl.flock(f, fcntl.LOCK_EX)\n'
                    + 'pathlib.Path(' + repr(str(ready)) + ').touch()\n'
                    + 'p=pathlib.Path(' + repr(str(stop)) + ')\nfor _ in range(1000):\n if p.exists(): break\n time.sleep(.02)\npathlib.Path(' + repr(str(done)) + ').touch()\n')

                def lease_held():
                    with lease.open('a') as handle:
                        try:
                            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        except BlockingIOError:
                            return True
                        fcntl.flock(handle, fcntl.LOCK_UN)
                        return False
                (self.bundle / 'frayd-server').write_text(
                    '#!/bin/sh\n' + shlex.quote(sys.executable) + ' ' + shlex.quote(str(worker)) + ' &\n'
                    'echo $! > ' + shlex.quote(str(pidfile)) + '\n'
                    + ('while [ ! -e ' + shlex.quote(str(release)) + ' ]; do sleep .02; done\nexit 0\n' if stop_mode == 'normal' else 'wait\n'))
                self.manifest()
                self.install('--yes')
                process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)],
                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
                try:
                    child = wait_for(lambda: shared_running_child_pid(self.prefix, self.data), 'owned group receipts')
                    self.assertEqual(os.getpgid(child), child, 'server must lead a dedicated group')
                    self.assertNotEqual(child, os.getpgid(process.pid))
                    wait_for(lambda: pidfile.exists() and pidfile.read_text().strip() and ready.exists() and lease_held(), 'descendant readiness and owned lease')
                    worker_pid = int(pidfile.read_text())
                    self.assertEqual(os.getpgid(worker_pid), child)
                    if stop_mode == 'normal':
                        release.touch()
                    else:
                        cancellation = signal.SIGHUP if stop_mode.endswith('hup') else signal.SIGTERM
                        if stop_mode.startswith('group'):
                            os.killpg(process.pid, cancellation)
                        else:
                            os.kill(process.pid, cancellation)
                    self.assertEqual(process.wait(timeout=15), 0)
                    # Assert before receipt checks or ANY cooperative cleanup stop.
                    self.assertFalse(lease_held(), 'owned descendant lease must be released before receipt cleanup')
                    with self.assertRaises(ProcessLookupError, msg='owned descendant must be absent before cleanup stop'):
                        os.kill(worker_pid, 0)
                    self.assertFalse((self.prefix / '.frayd-child.json').exists())
                    self.assertFalse((self.data / '.frayd-child.json').exists())
                    self.assertIsNone(foreign.poll(), 'foreign group must remain alive')
                    self.run_helper('uninstall', '--yes')
                finally:
                    # The worker has its own fixture stop oracle, so failure cleanup
                    # never guesses ownership of a possibly reaped server PID/PGID.
                    stop.touch()
                    release.touch()
                    if process.poll() is None:
                        process.send_signal(signal.SIGTERM)
                    process.wait(timeout=15)
                    process.stderr.close()
                    if pidfile.exists() and pidfile.read_text().strip():
                        worker_pid = int(pidfile.read_text())
                        def worker_exited():
                            if done.exists():
                                return True
                            try:
                                os.kill(worker_pid, 0)
                            except ProcessLookupError:
                                return True
                            return False
                        wait_for(worker_exited, 'fixture worker stop without PID signals', timeout=25)
                    pidfile.unlink(missing_ok=True)
                    stop.unlink(missing_ok=True)
                    release.unlink(missing_ok=True)
                    done.unlink(missing_ok=True)
                    ready.unlink(missing_ok=True)
                    lease.unlink(missing_ok=True)

    def test_foreground_interrupt_cleans_child_and_allows_removal(self):
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nulimit -c 0\nexec sleep 60\n')
        self.manifest()
        for target in ['group', 'helper', 'child', 'both-ready']:
            for interrupt in [signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT]:
                with self.subTest(target=target, interrupt=interrupt):
                    self.install('--yes')
                    process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, start_new_session=True)
                    try:
                        marker = self.prefix / '.frayd-child.json'
                        child = wait_for(lambda: running_child_pid(self.prefix), 'fixture child start')
                        if target == 'group':
                            os.killpg(process.pid, interrupt)
                        elif target == 'helper':
                            os.kill(process.pid, interrupt)
                        elif target == 'child':
                            os.kill(child, interrupt)
                        else:
                            # Queue a terminal signal while the helper cannot run,
                            # and let the child exit before resuming its select.
                            os.kill(process.pid, signal.SIGSTOP)
                            os.waitpid(process.pid, os.WUNTRACED)
                            os.kill(child, interrupt)
                            time.sleep(0.05)
                            os.kill(process.pid, interrupt)
                            os.kill(process.pid, signal.SIGCONT)
                        _, stderr = process.communicate(timeout=10)
                        self.assertFalse(marker.exists(), f'normal interruption must clear uncertainty after reaping; helper_exit={process.returncode}; stderr={stderr!r}')
                        with self.assertRaises(ProcessLookupError):
                            os.kill(child, 0)
                        data_before = [(p.relative_to(self.data), p.read_bytes()) for p in self.data.rglob('*') if p.is_file()]
                        self.run_helper('uninstall', '--yes')
                        self.assertFalse(self.prefix.exists())
                        self.assertEqual([(p.relative_to(self.data), p.read_bytes()) for p in self.data.rglob('*') if p.is_file()], data_before)
                        self.assertEqual(process.returncode, 0, stderr)
                        self.assertNotIn('server exited with', stderr)
                    finally:
                        if process.poll() is None:
                            self.stop_helper(process)
                        process.stderr.close()

    def test_unexpected_child_exit_remains_an_error(self):
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nexit 7\n')
        self.manifest()
        self.install('--yes')
        result = self.run_helper('start', code=2)
        self.assertIn('server exited with', result.stderr)
        self.assertFalse((self.prefix / '.frayd-child.json').exists())
        self.run_helper('uninstall', '--yes')

    def test_orphan_receipt_and_active_installation_refusal(self):
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nexec sleep 60\n')
        self.manifest()
        self.install('--yes')
        process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            marker = self.prefix / '.frayd-child.json'
            child = wait_for(lambda: shared_running_child_pid(self.prefix, self.data), 'prefix and data child receipts')
            before = self.snapshot()
            self.run_helper('uninstall', '--yes', code=2)
            self.run_helper('start', code=2)
            self.assertEqual(self.snapshot(), before)
            os.kill(child, signal.SIGKILL)
            self.assertEqual(process.wait(timeout=10), 2)
            self.assertFalse(marker.exists())
            marker.write_text('{"state":"unknown"}')
            before = self.snapshot()
            self.run_helper('uninstall', '--yes', code=2)
            self.run_helper('start', code=2)
            self.assertEqual(self.snapshot(), before)
        finally:
            if process.poll() is None:
                self.stop_helper(process)

    def test_invalid_targets_and_flags_zero_effects(self):
        before = self.snapshot()
        for target in ['relative', '/', '/usr/frayd', str(ROOT / 'native-test-prefix'), str(self.base) + '/./unsafe', str(self.base) + '//unsafe', str(self.base / '.cargo/new'), str(self.base / '.rustup/new'), str(self.base / 'original checkout/new')]:
            result = self.run_fixture([str(HELPER), 'install', '--bundle', str(self.bundle), '--prefix', target, '--data-dir', str(self.data), '--yes'], capture_output=True)
            self.assertEqual(result.returncode, 2, result.stderr)
        self.run_helper('uninstall', '--purge-library', code=2)
        self.assertEqual(self.snapshot(), before)
        link = self.base / 'link'
        link.symlink_to(self.base, target_is_directory=True)
        self.prefix = link / 'app'
        before = self.snapshot()
        self.install('--yes', code=2)
        self.assertEqual(self.snapshot(), before)

    def test_shared_writable_ancestor_refused_without_writes(self):
        self.addCleanup(self.base.chmod, 0o700)
        for mode in [0o775, 0o777]:
            with self.subTest(mode=oct(mode)):
                self.base.chmod(mode)
                before = self.snapshot()
                result = self.install('--yes', code=2)
                self.assertIn('group/other-writable non-sticky target ancestor refused', result.stderr)
                self.assertEqual(self.snapshot(), before)
        self.base.chmod(0o700)
        self.install('--yes')

    def test_data_directory_appearing_during_consent_is_not_adopted(self):
        process, _ = self.prompt('install', '--bundle', str(self.bundle), '--data-dir', str(self.data))
        self.data.mkdir(mode=0o700)
        sentinel = self.data / 'unowned'
        sentinel.write_bytes(b'preserve')
        _, stderr = process.communicate(b'y\n', timeout=10)
        self.assertEqual(process.returncode, 2, stderr)
        self.assertEqual(sentinel.read_bytes(), b'preserve')
        self.assertFalse((self.data / '.frayd-data.json').exists())
        self.assertFalse(self.prefix.exists())

    def test_uninstall_prompt_identifies_the_locked_installation(self):
        self.install('--yes')
        before = self.snapshot()
        process, prompt = self.prompt('uninstall')
        expected = f'Remove unchanged owned installation version 0.1.1 at {self.prefix}; keep all data at {self.data}? [y/N] '
        self.assertTrue(prompt.endswith(expected.encode()), prompt)
        busy = self.run_helper('uninstall', '--yes', code=2)
        self.assertIn('installation active or lifecycle busy', busy.stderr)
        process.communicate(b'n\n', timeout=10)
        self.assertEqual(process.returncode, 0)
        self.assertEqual(self.snapshot(), before)

    def test_shared_data_refuses_a_second_prefix_while_the_first_runs_and_after_its_supervisor_crashes(self):
        # The fixture server exits by itself once cleanup creates the stop file; no cleanup signals its pid.
        stop, exited = self.base / 'stop fixture server', self.base / 'fixture server exited'
        (self.bundle / 'frayd-server').write_text(
            f'#!/bin/sh\ni=0\nwhile [ ! -e {shlex.quote(str(stop))} ] && [ "$i" -lt 600 ]; do sleep 0.1; i=$((i + 1)); done\n'
            f': > {shlex.quote(str(exited))}\n')
        self.manifest()
        self.install('--yes')
        first, self.prefix = self.prefix, self.base / 'second prefix'
        self.install('--yes')
        supervisor = subprocess.Popen([str(HELPER), 'start', '--prefix', str(first)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        self.addCleanup(self.stop_helper, supervisor)
        child = wait_for(lambda: shared_running_child_pid(first, self.data), 'first prefix and data child receipts')
        self.addCleanup(wait_for, exited.exists, 'fixture server exit after its stop file')
        self.addCleanup(stop.touch)
        before = self.snapshot()
        refused = self.run_helper('start', code=2, timeout=10)
        self.assertIn('data directory active or lifecycle busy', refused.stderr)
        self.assertEqual(self.snapshot(), before)
        os.kill(supervisor.pid, signal.SIGKILL)  # Unreaped, so the pid is still the supervisor's.
        supervisor.wait(timeout=10)
        before = self.snapshot()
        refused = self.run_helper('start', code=2, timeout=10)
        self.assertIn(f'active or uncertain data child receipt at {self.data / ".frayd-child.json"}', refused.stderr)
        self.assertEqual(self.snapshot(), before)
        os.kill(child, 0)  # The orphaned first server still runs: the refused start neither stops nor adopts it.


@unittest.skipUnless(
    os.environ.get('FRAYD_NATIVE_ARCHIVE') and os.environ.get('FRAYD_NATIVE_BUILD_SOURCE'),
    'actual server qualification requires FRAYD_NATIVE_ARCHIVE and FRAYD_NATIVE_BUILD_SOURCE',
)
class NativeRuntime(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='frayd runtime ', dir=TEMP_ROOT)
        self.preserve_fixture = False
        self.addCleanup(self.cleanup_fixture)
        self.base = Path(self.temp.name)
        archive = Path(os.environ['FRAYD_NATIVE_ARCHIVE'])
        checksum = archive.with_suffix(archive.suffix + '.sha256').read_text().split()[0]
        self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), checksum)
        extracted = self.base / 'relocated extracted bundle'
        extracted.mkdir(mode=0o700)
        with tarfile.open(archive) as tar:
            members = tar.getmembers()
            names = [member.name for member in members]
            self.assertEqual(len(names), len(set(names)), 'duplicate archive member')
            for member in members:
                self.assertTrue(member.isfile() or member.isdir())
                self.assertFalse(Path(member.name).is_absolute())
                self.assertNotIn('..', Path(member.name).parts)
            bundle_name = archive.name.removesuffix('.tar.gz')
            manifest_name = bundle_name + '/manifest.json'
            manifest_member = tar.getmember(manifest_name)
            self.assertTrue(manifest_member.isfile())
            with tar.extractfile(manifest_member) as manifest_file:
                manifest = json.load(manifest_file)
            allowed = {bundle_name, manifest_name}
            allowed.update(bundle_name + '/' + entry['path'] for entry in manifest['entries'])
            self.assertEqual(set(names), allowed, 'archive members must match the manifest exactly')
        # Match the documented user workflow: preserve manifest modes even under umask 077.
        self.run_fixture(['tar', '-xpzf', str(archive), '-C', str(extracted)],
                    umask=0o077, check=True, capture_output=True)
        bundles = list(extracted.iterdir())
        self.assertEqual(len(bundles), 1)
        self.bundle = bundles[0]
        self.prefix = self.base / 'installed application'
        self.data = self.base / 'application data'
        self.library = self.base / 'library fixture'
        self.library.mkdir(mode=0o700)
        self.run_fixture([str(self.bundle / 'install.sh'), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], check=True, capture_output=True)
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            self.port = probe.getsockname()[1]
        self.environment = os.environ.copy()
        for name in ['FRAYD_LIBRARY', 'FRAYD_CERT', 'FRAYD_KEY', 'FRAYD_UPNP']:
            self.environment.pop(name, None)
        self.environment['FRAYD_BIND'] = 'invalid inherited value'
        # Paths deliberately poison inherited defaults: helper must override all four.
        for name in ['FRAYD_DB', 'FRAYD_THUMBS', 'FRAYD_TRANSCODES', 'FRAYD_CLIENT_DIR']:
            self.environment[name] = str(self.base / 'MUST NOT EXIST')
        self.process = None
        self.addCleanup(self.kill_server)

    cleanup_fixture = NativeOwnership.cleanup_fixture
    run_fixture = NativeOwnership.run_fixture

    def kill_server(self):
        if self.process is not None and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.preserve_fixture = True
                raise AssertionError(f'cleanup uncertain; runtime fixture retained at {self.base}') from None
        if self.process is not None and any((root / '.frayd-child.json').exists() for root in [self.prefix, self.data]):
            self.preserve_fixture = True
            raise AssertionError(f'child receipts remain; runtime fixture retained at {self.base}')

    def request(self, path, payload=None, token=None, expected=200):
        headers = {'Content-Type': 'application/json'}
        if token:
            headers['Authorization'] = 'Bearer ' + token
        body = None if payload is None else json.dumps(payload).encode()
        req = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=body, headers=headers)
        with urllib.request.urlopen(req, timeout=3) as response:
            self.assertEqual(response.status, expected)
            return response.read()

    def request_json(self, *args, **kw):
        return json.loads(self.request(*args, **kw))

    def start(self):
        self.process = subprocess.Popen([str(self.prefix / 'start.sh'), '--prefix', str(self.prefix), '--port', str(self.port)], env=self.environment, cwd=self.base, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)

        def healthy():
            self.assertIsNone(self.process.poll(), 'supervisor exited before readiness')
            try:
                return self.request_json('/api/health') == {'status': 'ok'}
            except OSError:
                return False

        wait_for(healthy, 'native HTTP readiness', timeout=20)

    def stop(self, group_interrupt=False):
        """Stop the server by group interrupt or by terminating its child; require a clean exit and a cleared receipt."""
        receipt = self.prefix / '.frayd-child.json'
        if group_interrupt:
            os.killpg(self.process.pid, signal.SIGINT)
        else:
            child = json.loads(receipt.read_text())['pid']
            parent = self.run_fixture(['ps', '-o', 'ppid=', '-p', str(child)], text=True, capture_output=True, check=True, timeout=10).stdout.strip()
            self.assertEqual(int(parent), self.process.pid)
            os.kill(child, signal.SIGTERM)
        self.assertEqual(self.process.wait(timeout=10), 0)
        self.assertFalse(receipt.exists())
        self.process = None

    def test_relocated_real_server_reader_and_persistence(self):
        build_source = Path(os.environ['FRAYD_NATIVE_BUILD_SOURCE'])
        self.assertTrue(build_source.is_absolute())
        self.assertFalse(any(build_source.iterdir()) if build_source.exists() else False, 'compile-time source must be absent or empty in an isolated namespace')
        self.assertFalse((build_source.parent / 'client/index.html').exists(), 'compile-time client assets must also be unavailable')
        # Every HTTP asset is compared with this extracted manifest, not source files.
        manifest = json.loads((self.bundle / 'manifest.json').read_text())
        self.assertTrue(manifest['toolchain'].startswith('rustc '))
        if os.environ.get('FRAYD_NATIVE_TOOLCHAIN_FILE'):
            self.assertEqual(manifest['toolchain'], Path(os.environ['FRAYD_NATIVE_TOOLCHAIN_FILE']).read_text().strip())
        if os.environ.get('FRAYD_NATIVE_EXPECT_ARCH'):
            self.assertEqual(os.uname().machine, os.environ['FRAYD_NATIVE_EXPECT_ARCH'])
        vendor_files = [p for p in manifest['entries'] if p['kind'] == 'file' and p['path'].startswith('client/vendor/')]
        for asset in vendor_files:
            self.assertTrue((self.prefix / asset['path']).is_file())
        self.start()
        self.assertEqual(self.request('/'), (self.prefix / 'client/index.html').read_bytes())
        vendor = vendor_files[0]
        self.assertEqual(hashlib.sha256(self.request('/' + vendor['path'][len('client/'):])).hexdigest(), vendor['sha256'])
        self.assertEqual(self.request_json('/api/setup'), {'needed': True})
        credentials = {'email': 'native-fixture@example.invalid', 'password': secrets.token_urlsafe(32)}
        owner = self.request_json('/api/setup', credentials)
        self.assertTrue(owner['is_admin'])
        token = owner['token']
        self.request('/api/admin/settings', {'library_paths': [str(self.library)]}, token, expected=204)
        image = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=')
        with zipfile.ZipFile(self.library / 'native-reader.cbz', 'w') as comic:
            comic.writestr('001.png', image)
        self.request('/api/scan', {}, token, expected=202)
        wait_for(lambda: not self.request_json('/api/scan/status')['scanning'], 'library scan to finish', timeout=20)
        media_id = next(item['id'] for item in self.request_json('/api/library') if item['kind'] == 'comic')
        self.assertEqual(self.request_json(f'/api/comic/{media_id}/pages'), ['001.png'])
        self.assertEqual(self.request(f'/api/comic/{media_id}/page/0'), image)
        self.request('/api/progress', {'media_id': media_id, 'position': 0.5}, token, expected=204)
        # Active lifecycle refusal must leave every installed byte unchanged.
        before = snapshot(self.prefix)
        refused = self.run_fixture([str(self.bundle / 'uninstall.sh'), '--prefix', str(self.prefix), '--yes'], capture_output=True)
        self.assertEqual(refused.returncode, 2)
        self.assertEqual(snapshot(self.prefix), before)
        self.stop()
        self.start()
        self.assertEqual(self.request_json('/api/setup'), {'needed': False})
        login = self.request_json('/api/auth/login', credentials)
        self.assertEqual(self.request_json(f'/api/progress/{media_id}', token=login['token'])['position'], 0.5)
        self.assertEqual(self.request(f'/api/comic/{media_id}/page/0'), image)
        self.assertFalse((self.base / 'MUST NOT EXIST').exists())
        self.stop(group_interrupt=True)
        # An unavailable explicit root must leave account/metadata reads
        # healthy without granting permission to recreate source media.
        missing_library = self.base / 'unavailable source mount' / 'media'
        self.environment['FRAYD_LIBRARY'] = str(missing_library)
        self.start()
        self.assertEqual(self.request_json('/api/health'), {'status': 'ok'})
        offline_status = self.request_json('/api/scan/status')
        self.assertTrue(offline_status['library_configured'])
        self.assertEqual(offline_status['library_count'], 1)
        offline_login = self.request_json('/api/auth/login', credentials)
        self.assertEqual(self.request_json(f'/api/progress/{media_id}', token=offline_login['token'])['position'], 0.5)
        offline_metadata = self.request_json('/api/library')
        self.assertTrue(any(item['id'] == media_id and item['kind'] == 'comic' and item['title'] == 'native-reader' for item in offline_metadata))
        self.assertFalse(missing_library.exists())
        self.assertFalse(missing_library.parent.exists())
        self.stop(group_interrupt=True)
        del self.environment['FRAYD_LIBRARY']
        self.start()
        self.assertEqual(self.request(f'/api/comic/{media_id}/page/0'), image)
        self.assertEqual(self.request_json(f'/api/progress/{media_id}', token=offline_login['token'])['position'], 0.5)
        self.stop(group_interrupt=True)
        preserved = snapshot(self.data)
        self.run_fixture([str(self.bundle / 'uninstall.sh'), '--prefix', str(self.prefix), '--yes'], check=True, capture_output=True)
        self.assertFalse(self.prefix.exists())
        self.assertEqual(snapshot(self.data), preserved, 'application data changed during removal')


if __name__ == '__main__':
    unittest.main()
