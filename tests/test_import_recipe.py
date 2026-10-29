"""Run the documented import preflight; destination Docker calls are inert."""
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import unittest


def gnu_tool(name):
    """Resolve a verified GNU find or tar binary, accepting prefixed names when present."""
    expected_version = 'find (GNU findutils)' if name == 'find' else 'tar (GNU tar)'
    for command in ('g' + name, name):
        executable = shutil.which(command)
        if executable is None:
            continue
        try:
            version = subprocess.run([executable, '--version'], text=True,
                                     capture_output=True, timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            continue
        if version.returncode == 0 and version.stdout.startswith(expected_version):
            return executable
    return None


class ImportRecipeTests(unittest.TestCase):
    def setUp(self):
        """Build a source with spaces and inert destination wrappers for the published recipe."""
        self.fixture = tempfile.TemporaryDirectory(prefix='frayd-import-recipe-')
        self.addCleanup(self.fixture.cleanup)
        self.root = Path(self.fixture.name).resolve()
        self.source = self.root / 'source with spaces'
        self.source.mkdir()
        (self.source / 'payload').write_text('exact fixture bytes\n')
        (self.root / 'final-link').symlink_to(self.source, target_is_directory=True)
        (self.root / 'ancestor').symlink_to(self.root, target_is_directory=True)
        self.markers = self.root / 'markers'
        tools = self.root / 'tools'
        tools.mkdir()
        self.bash = shutil.which('bash')
        self.assertIsNotNone(self.bash)
        for name in ('find', 'tar', 'docker'):
            executable = (gnu_tool(name) if name != 'docker' else shutil.which('cat'))
            if name != 'docker' and executable is None:
                self.skipTest(f'GNU {name} required for documented import recipe')
            self.assertIsNotNone(executable)
            action = ('exec ' + shlex.quote(executable) + ' "$@"'
                      if name != 'docker' else 'exec ' + shlex.quote(executable) + ' > /dev/null')
            wrapper = tools / name
            wrapper.write_text('#!/bin/sh\nprintf "%s\\n" ' + name + ' >> "$IMPORT_MARKERS"\n' + action + '\n')
            wrapper.chmod(0o700)
        self.environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
                                IMPORT_MARKERS=str(self.markers))
        operations = Path(__file__).resolve().parents[1] / 'docs' / 'operations.md'
        blocks = re.findall(r'```sh\n(.*?)\n```', operations.read_text(), re.S)
        recipes = [block for block in blocks if '  SOURCE_DIR=/absolute/path/to/your/media\n' in block]
        self.assertEqual(len(recipes), 1, 'Expected one documented import recipe')
        self.recipe = recipes[0]

    def run_recipe(self, source):
        """Run the documented shell recipe with a chosen source and collect tool invocations."""
        self.markers.unlink(missing_ok=True)
        recipe = self.recipe.replace('SOURCE_DIR=/absolute/path/to/your/media',
                                     'SOURCE_DIR=' + shlex.quote(str(source)))
        result = subprocess.run([self.bash, '-c', recipe], input='', text=True,
                                capture_output=True, env=self.environment, timeout=5)
        markers = self.markers.read_text().splitlines() if self.markers.exists() else []
        return result, markers

    def test_symlink_components_rejected_before_tools(self):
        """Reject symlinks in final or ancestor path components before any tool runs."""
        paths = ['final-link', 'final-link/', 'final-link/.',
                 'final-link/../source with spaces', 'ancestor/source with spaces',
                 'ancestor/source with spaces/', 'ancestor/./source with spaces',
                 'ancestor/../' + self.root.name + '/source with spaces',
                 'source with spaces/../final-link/']
        for path in paths:
            with self.subTest(path=path):
                result, markers = self.run_recipe(str(self.root) + '/' + path)
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertIn('Source path must not contain symlinks', result.stderr)
                self.assertEqual(markers, [], 'Refusal must precede find, tar and destination calls')
        self.assertEqual((self.source / 'payload').read_text(), 'exact fixture bytes\n')

    def test_absolute_source_with_spaces_reaches_synthetic_destination(self):
        """Accept an absolute source containing spaces and invoke the expected four tools."""
        result, markers = self.run_recipe(self.source)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(markers.count('find'), 1)
        self.assertEqual(markers.count('tar'), 1)
        self.assertEqual(markers.count('docker'), 2)
        self.assertEqual(len(markers), 4)
        self.assertEqual((self.source / 'payload').read_text(), 'exact fixture bytes\n')


if __name__ == '__main__':
    unittest.main()
