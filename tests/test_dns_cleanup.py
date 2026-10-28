"""Real subprocess failure boundaries for the disposable DNS fixture cleanup."""
import contextlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('dns_fixture', Path(__file__).with_name('test_dns_recovery.py'))
fixture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fixture)


@contextlib.contextmanager
def fake_docker(mode):
    """Execute a fake Docker that records attempts and hangs or fails inventory."""
    with tempfile.TemporaryDirectory() as directory:
        trace = Path(directory) / 'trace.jsonl'
        executable = Path(directory) / 'docker'
        executable.write_text(
            f'#!{sys.executable}\nimport json, sys, time\n'
            f'with open({str(trace)!r}, "a") as out: out.write(json.dumps(sys.argv[1:]) + "\\n")\n'
            f'mode = {mode!r}\n'
            'args = sys.argv[1:]\n'
            'if mode == "hang" and (args[0] == "logs" or args[1:2] == ["ls"]): time.sleep(30)\n'
            'if args[1:2] == ["ls"]:\n'
            '    if mode == "error": sys.exit(29)\n'
            '    if mode == "leftover": print("stopped-fixture")\n')
        executable.chmod(0o755)
        with patch.dict(os.environ, {'PATH': directory}):
            yield trace


class DNSCleanupTests(unittest.TestCase):
    """Preserve scenario failures while proving bounded, exhaustive cleanup attempts."""

    def assert_original_failure_survives(self, mode):
        """Drive the actual fixture finally block after a distinct scenario error."""
        original = AssertionError('original DNS scenario failure')

        def fail_scenario(*args):
            """Fail setup distinctly from any legacy inventory command exception."""
            if args[:2] == ('network', 'create'):
                raise original
            raise OSError('cleanup inventory unavailable')

        output = io.StringIO()
        with (
            fake_docker(mode) as trace,
            patch.object(fixture, 'docker', side_effect=fail_scenario),
            patch.object(fixture, 'CLEANUP_TIMEOUT', 0.2, create=True),
            contextlib.redirect_stdout(output),
            contextlib.redirect_stderr(output),
        ):
            started = time.monotonic()
            with self.assertRaises(AssertionError) as caught:
                fixture.run_fixture(SimpleNamespace())
            elapsed = time.monotonic() - started
            commands = [json.loads(line) for line in trace.read_text().splitlines()]
        self.assertIs(caught.exception, original)
        self.assertLess(elapsed, 4)
        self.assertEqual(len(commands), 10)
        self.assertEqual(sum(args[:2] == ['rm', '-f'] for args in commands), 4)
        self.assertEqual(sum(args[:2] == ['network', 'rm'] for args in commands), 1)
        self.assertEqual(sum(args[1:2] == ['ls'] for args in commands), 3)
        self.assertNotIn('PASS: task-owned', output.getvalue())

    def test_inventory_error_preserves_original_failure(self):
        """Nonzero Docker inventory cannot mask the actual scenario assertion."""
        self.assert_original_failure_survives('error')

    def test_hanging_diagnostics_and_inventory_are_bounded(self):
        """Hung subprocesses time out and every removal and inventory still runs."""
        self.assert_original_failure_survives('hang')

    def test_cleanup_failure_is_fatal_after_success(self):
        """Unavailable inventory or stopped leftovers fail an otherwise successful run."""
        for mode in ('error', 'leftover'):
            with (
                self.subTest(mode=mode),
                fake_docker(mode),
                contextlib.redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()),
                self.assertRaises(AssertionError),
            ):
                fixture.cleanup_fixture('frayd-dns-cleanup-test', ['app', 'dns', 'proxy', 'reserve'], False)


if __name__ == '__main__':
    unittest.main()
