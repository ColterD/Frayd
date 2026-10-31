"""Exercise qualification entry points with Python assertions disabled."""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENTRY_POINTS = ('scripts/check-build-context.py', 'scripts/integration.py', 'tests/test_dns_recovery.py')


class QualificationModeTests(unittest.TestCase):
    def invoke(self, arguments, optimize=None):
        """Run the real interpreter without inherited optimization or access to Docker."""
        environment = os.environ.copy()
        environment.pop('PYTHONOPTIMIZE', None)
        if optimize is not None:
            environment['PYTHONOPTIMIZE'] = optimize
        with tempfile.TemporaryDirectory(prefix='frayd-no-tools-') as directory:
            environment['PATH'] = directory
            return subprocess.run([sys.executable, *arguments], cwd=ROOT, env=environment,
                                  capture_output=True, text=True, timeout=10)

    def test_optimized_cli_rejected_before_qualification(self):
        for entry_point in ENTRY_POINTS:
            for flags, optimize in ((['-O'], None), ([], '1')):
                with self.subTest(entry_point=entry_point, flags=flags, optimize=optimize):
                    result = self.invoke([*flags, entry_point, '--help'], optimize)
                    self.assertNotEqual(result.returncode, 0, result.stdout)
                    self.assertIn('unsupported optimized Python execution', result.stderr)
                    self.assertNotIn('PASS:', result.stdout)
                    self.assertNotIn('Traceback', result.stderr)

    def test_ordinary_cli_help_and_context_import(self):
        for entry_point in ENTRY_POINTS[1:]:
            with self.subTest(entry_point=entry_point):
                result = self.invoke([entry_point, '--help'])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('usage:', result.stdout)
        # The context probe has no help parser; importing it must remain effect-free.
        result = self.invoke(['-c', "import runpy, sys; sys.path.insert(0, 'scripts'); "
                              "runpy.run_path('scripts/check-build-context.py'); print('import ready')"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), 'import ready')
