"""Failure-boundary regressions for the disposable Docker integration runner."""
import contextlib
import importlib.util
import io
import os
import subprocess
import sys
import tempfile
import unittest
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


if __name__ == '__main__':
    unittest.main()
