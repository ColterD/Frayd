"""Exercise the macOS workflow's actual source-copy commands without a compiler or runner."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(shutil.which('bash'), 'requires the workflow shell')
class IsolatedBuildFixture(unittest.TestCase):
    def test_mac_source_copy_keeps_exact_compile_fixture_and_no_test_neighbors(self):
        source = (ROOT / '.github/workflows/release-mac.yml').read_text()
        step = source.split('- name: Exact Rust toolchain and isolated source', 1)[1].split('- name:', 1)[0]
        commands = [line.strip() for line in step.splitlines() if line.strip().startswith(('mkdir ', 'cp '))]
        self.assertTrue(commands, 'No source-preparation commands found')
        with tempfile.TemporaryDirectory(prefix='frayd-copy-fixture-') as temporary:
            root = Path(temporary)
            checkout = root / 'checkout'
            runner = root / 'runner'
            (checkout / 'server/src').mkdir(parents=True)
            (checkout / 'tests/fixtures/private').mkdir(parents=True)
            runner.mkdir()
            (checkout / 'server/src/main.rs').write_text('synthetic Rust source')
            fixture = b'project-generated test fixture sentinel'
            (checkout / 'tests/fixtures/native-resume.mp4').write_bytes(fixture)
            for name in ('tests/other.py', 'tests/fixtures/neighbor.mp4', 'tests/fixtures/.env',
                         'tests/fixtures/private/secret.txt'):
                (checkout / name).write_text('NONSECRET_EXCLUDED_TEST_NEIGHBOR')
            result = subprocess.run(['bash', '-eu', '-c', '\n'.join(commands)], cwd=checkout,
                                    env={'PATH': str(Path(shutil.which('bash')).parent) + ':/usr/bin:/bin',
                                         'RUNNER_TEMP': str(runner)}, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            copied = runner / 'frayd-build-source'
            included = copied / 'server/src/../../tests/fixtures/native-resume.mp4'
            self.assertTrue(included.is_file(), 'Rust include_bytes! target is absent from isolated source')
            self.assertEqual(included.read_bytes(), fixture)
            self.assertEqual(sorted(path.relative_to(copied).as_posix() for path in copied.rglob('*') if path.is_file()),
                             ['server/src/main.rs', 'tests/fixtures/native-resume.mp4'])


if __name__ == '__main__':
    unittest.main()
