"""Failure-boundary regressions for the disposable Docker integration runner."""
import contextlib
import http.client
import importlib.util
import io
import os
import subprocess
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('integration', Path(__file__).parents[1] / 'scripts/integration.py')
integration = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(integration)


class DockerFailureTests(unittest.TestCase):
    """Keep diagnostics useful and preserve the first nonzero failure."""

    def test_docker_reports_bytes_and_text_stderr_without_stdout_or_input(self):
        """Both subprocess modes print stderr and re-raise the original status."""
        for stderr in (b'build failed\n', 'build failed\n'):
            with self.subTest(stderr_type=type(stderr).__name__):
                error = subprocess.CalledProcessError(17, ['docker', 'build'], output=b'private output', stderr=stderr)
                output = io.StringIO()
                with (
                    patch.object(integration.subprocess, 'run', side_effect=error),
                    contextlib.redirect_stderr(output),
                    self.assertRaises(subprocess.CalledProcessError) as caught,
                ):
                    integration.docker('build', input=b'private input')
                self.assertIs(caught.exception, error)
                self.assertEqual(caught.exception.returncode, 17)
                self.assertEqual(output.getvalue(), 'build failed\n')

    def test_real_subprocess_exposes_diagnostic_and_nonzero_status(self):
        """A fake Docker executable proves stderr reaches the caller without mocks."""
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder) / 'docker'
            executable.write_text(f'#!{sys.executable}\nimport sys\n'
                                  'sys.stderr.write("synthetic Docker failure\\n")\n'
                                  'sys.stdout.write("private stdout")\nsys.exit(23)\n')
            executable.chmod(0o755)
            output = io.StringIO()
            with (
                patch.dict(os.environ, {'PATH': folder}),
                contextlib.redirect_stderr(output),
                self.assertRaises(subprocess.CalledProcessError) as caught,
            ):
                integration.docker('synthetic-command', input=b'private stdin')
            self.assertEqual(caught.exception.returncode, 23)
            self.assertEqual(output.getvalue(), 'synthetic Docker failure\n')

    def test_cleanup_preserves_original_failure(self):
        """A cleanup failure cannot replace the original nonzero Docker status."""
        original = subprocess.CalledProcessError(17, ['docker', 'build'])
        failures = (subprocess.CalledProcessError(19, ['docker', 'compose', 'down']),
                    OSError('synthetic Docker unavailable'))
        for cleanup in failures:
            with (
                self.subTest(cleanup_type=type(cleanup).__name__),
                contextlib.redirect_stderr(io.StringIO()),
                patch.object(integration, 'docker', side_effect=cleanup),
                self.assertRaises(subprocess.CalledProcessError) as caught,
                integration.cleanup_project(integration.docker),
            ):
                raise original
            self.assertIs(caught.exception, original)

    def test_cleanup_failure_is_fatal_after_success(self):
        """Resource cleanup is required even when every HTTP assertion passes."""
        cleanup = subprocess.CalledProcessError(19, ['docker', 'compose', 'down'])
        with (
            patch.object(integration, 'docker', side_effect=cleanup) as compose,
            self.assertRaises(subprocess.CalledProcessError) as caught,
            integration.cleanup_project(compose),
        ):
            pass
        compose.assert_called_once_with('down', '--volumes', '--remove-orphans')
        self.assertIs(caught.exception, cleanup)


class CleanupInventoryTests(unittest.TestCase):
    """Exercise the Docker listing contract for stopped cleanup leftovers."""

    def test_stopped_labelled_container_prevents_cleanup_success(self):
        """Stopped containers appear only when Docker is asked for all states."""
        def list_resources(*args):
            """Model Docker's default running-only view of one stopped container."""
            output = b'stopped-fixture' if args[0] == 'container' and '-a' in args else b''
            return subprocess.CompletedProcess(args, 0, stdout=output)

        with (
            patch.object(integration, 'docker', side_effect=list_resources),
            self.assertRaises(AssertionError) as caught,
        ):
            integration.assert_project_removed('frayd-cleanup-fixture')
        self.assertEqual(caught.exception.args[0],
                         ('container', 'frayd-cleanup-fixture', b'stopped-fixture'))


class ReadinessTests(unittest.TestCase):
    """Keep transient network recovery bounded without hiding terminal failures."""

    @staticmethod
    def transient_errors():
        """Build raw and urllib-wrapped errors emitted by the HTTP request stack."""
        return (OSError('socket reset'), ConnectionResetError('peer reset'),
                TimeoutError('socket timed out'), http.client.RemoteDisconnected('peer disconnected'),
                urllib.error.URLError('connection refused'),
                urllib.error.HTTPError('/api/health', 503, 'unavailable', None, None),
                AssertionError('health status was 503 instead of 200'))

    def test_transient_failure_retries_then_accepts_success(self):
        """Every supported transport/status failure can recover on the next probe."""
        for error in self.transient_errors():
            with (
                self.subTest(error_type=type(error).__name__),
                patch.object(integration, 'request', side_effect=[error, b'{"status":"ok"}']) as request,
                patch.object(integration.time, 'monotonic', side_effect=[0, 0, 0.5]),
                patch.object(integration.time, 'sleep'),
            ):
                integration.ready()
                self.assertEqual(request.call_count, 2)

    def test_deadline_exhaustion_reports_the_last_failure(self):
        """A persistent failure terminates at the 20-second deadline with its cause."""
        for error in self.transient_errors():
            with self.subTest(error_type=type(error).__name__):
                with (
                    patch.object(integration, 'request', side_effect=error) as request,
                    patch.object(integration.time, 'monotonic', side_effect=[10, 10, 29.5, 30]),
                    patch.object(integration.time, 'sleep'),
                    self.assertRaises(AssertionError) as caught,
                ):
                    integration.ready()
                self.assertEqual(str(caught.exception), f'QA server did not become ready: {error}')
                self.assertEqual(request.call_count, 2)


if __name__ == '__main__':
    unittest.main()
