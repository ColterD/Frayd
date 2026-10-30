"""Private qualification probe: preserve an old deadline failure, then test the PTY consumer.

Popen/preexec admission is deliberately reported separately and has no OS-bound
claim. The old case never signals a PID: it drains only its own PTY after the
original deadline and gives that same unreaped process one cleanup budget.
"""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import termios
import time


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--test-source', type=Path, default=Path('tests/test_native_bundle.py'))
    parser.add_argument('--expect-old-timeout', action='store_true')
    args = parser.parse_args()
    source = args.test_source.resolve()
    spec = importlib.util.spec_from_file_location('native_pty_probe_source', source)
    native = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(native)
    native.NativeOwnership.setUpClass()
    archive = Path(os.environ['FRAYD_NATIVE_ARCHIVE'])
    report = {
        'fixture_sha256': digest(source), 'helper_sha256': digest(native.HELPER),
        'archive_sha256': digest(archive), 'expect_old_timeout': args.expect_old_timeout,
        'admission_limit': 'Popen/preexec construction is outside the post-spawn deadline',
    }
    case = native.NativeOwnership()
    case.setUp()
    status = 1
    try:
        case.install('--yes')
        expected = [b'127.0.0.1', str(case.data / 'frayd.db').encode(), str(case.prefix / 'client').encode()]
        for control in ['old', 'new']:
            master, slave = native.pty.openpty()
            process = None
            try:
                attributes = termios.tcgetattr(slave)
                attributes[3] &= ~termios.TOSTOP
                termios.tcsetattr(slave, termios.TCSANOW, attributes)
                command = [str(native.HELPER), 'start', '--prefix', str(case.prefix)]
                options = dict(stdin=subprocess.DEVNULL, stdout=slave, stderr=subprocess.PIPE,
                               preexec_fn=lambda: fcntl.ioctl(slave, termios.TIOCSCTTY, 0))
                print(f'PTY_PROBE {control}: entering Popen/preexec admission', flush=True)
                if control == 'old':
                    process = subprocess.Popen(command, start_new_session=True, **options)
                    print('PTY_PROBE old: admitted; original communicate deadline 10 seconds', flush=True)
                    started = time.monotonic()
                    record = {'original_timeout': False}
                    report['old'] = record
                    try:
                        _, stderr = process.communicate(timeout=10)
                    except subprocess.TimeoutExpired:
                        record['original_timeout'] = True
                        record['original_result'] = 'RED_TIMEOUT_RETAINED'
                        record['original_seconds'] = time.monotonic() - started
                        print('PTY_PROBE old: original timeout recorded RED; now drain owned PTY only', flush=True)
                        cleanup_deadline = time.monotonic() + 5
                        os.set_blocking(master, False)
                        # This known fixture emits three short lines long before
                        # its deadline. One bounded read releases that queued output.
                        terminal_output = os.read(master, 65536)
                        _, stderr = process.communicate(timeout=max(0, cleanup_deadline - time.monotonic()))
                        record['cleanup_seconds'] = 5 - max(0, cleanup_deadline - time.monotonic())
                    else:
                        record['original_result'] = 'COMPLETED_WITHOUT_READER'
                        record['original_seconds'] = time.monotonic() - started
                        os.set_blocking(master, False)
                        terminal_output = os.read(master, 65536)
                    record.update(reaped_exit=process.returncode,
                                  terminal_bytes=len(terminal_output), terminal_output=terminal_output.decode(),
                                  stderr=stderr.decode())
                    report['old'] = record
                else:
                    started = time.monotonic()
                    result = native.run_bounded(command, timeout=10, terminal_master=master, **options)
                    terminal_output = result.terminal_output
                    report['new'] = dict(exit=result.returncode, seconds=time.monotonic() - started,
                                         terminal_bytes=len(terminal_output), terminal_output=terminal_output.decode(),
                                         stderr=result.stderr.decode())
                    assert result.returncode == 0
                    assert report['new']['seconds'] < 10
                assert terminal_output.splitlines() == expected
                assert not (case.prefix / '.frayd-child.json').exists()
                assert not (case.data / '.frayd-child.json').exists()
                print(f'PTY_PROBE {control}: reaped; real terminal bytes captured; child receipts absent', flush=True)
            except native.RunBoundedCleanupUncertain:
                case.preserve_fixture = True
                raise
            finally:
                if process is not None:
                    if process.returncode is None:
                        case.preserve_fixture = True
                    for stream in (process.stdin, process.stdout, process.stderr):
                        if stream is not None:
                            stream.close()
                os.close(slave)
                os.close(master)
        assert report['old']['reaped_exit'] == 0
        if args.expect_old_timeout:
            assert report['old']['original_timeout'], 'Mac negative oracle was not reproduced'
        assert digest(native.HELPER) == report['helper_sha256']
        assert digest(archive) == report['archive_sha256']
        case.run_helper('uninstall', '--yes')
        report['status'] = 'PASS'
        status = 0
    except BaseException as error:
        report['status'] = 'HELD' if case.preserve_fixture else 'FAIL'
        report['error'] = f'{type(error).__name__}: {error}'
        if isinstance(error, native.RunBoundedCleanupUncertain):
            report['cleanup_errors'] = [type(item).__name__ for item in error.cleanup_errors]
    finally:
        report['fixture_preserved'] = case.preserve_fixture
        if case.preserve_fixture:
            report['fixture_path'] = str(case.base)
            case.temp._finalizer.detach()
        else:
            case.cleanup_fixture()
        print(json.dumps(report, indent=2), flush=True)
    return status


if __name__ == '__main__':
    raise SystemExit(main())
