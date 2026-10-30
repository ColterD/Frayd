"""Native bundle contract. Helper scenarios need FRAYD_NATIVE_HELPER; the real-server scenario also needs FRAYD_NATIVE_ARCHIVE and FRAYD_NATIVE_BUILD_SOURCE."""
import base64
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import stat
import sys
import subprocess
import tarfile
import tempfile
import time
import unittest
from unittest import mock
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
HELPER = Path(os.environ.get('FRAYD_NATIVE_HELPER', ROOT / 'target/native-qualification/debug/frayd-native'))
SERVER = HELPER.with_name('frayd-server')
PACKAGE = ROOT / 'scripts/native/package.py'
TEMP_ROOT = Path('/var/tmp').resolve()
ARCHITECTURE = 'aarch64' if os.uname().machine in ['arm64', 'aarch64'] else 'x86_64'
TARGET = ARCHITECTURE + ('-apple-darwin' if os.uname().sysname == 'Darwin' else '-unknown-linux-gnu')


def run_bounded(arguments, *, timeout=60, check=False, input=None, capture_output=False, **options):
    """Run a finite fixture command in a private session and kill the whole process group on timeout or interruption.

    subprocess.run(timeout=...) kills only the direct child, which would leave a hung helper's server running.
    """
    if capture_output:
        options['stdout'] = options['stderr'] = subprocess.PIPE
    if input is not None:
        options['stdin'] = subprocess.PIPE
    with subprocess.Popen(arguments, start_new_session=True, **options) as process:
        try:
            stdout, stderr = process.communicate(input, timeout=timeout)
        except BaseException:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            raise
    result = subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)
    if check:
        result.check_returncode()
    return result


def wait_for(probe, what, timeout=10):
    """Poll probe() until it returns something truthy, which is returned; fail the test after timeout seconds."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError(f'timed out after {timeout}s waiting for {what}')


def running_child_pid(prefix):
    """Return the supervised server's pid once the child receipt says it is running."""
    try:
        document = json.loads((prefix / '.frayd-child.json').read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None
    return document['pid'] if document.get('state') == 'running' else None


def snapshot(root):
    """Names, modes and bytes (or symlink targets) of everything under root."""
    return [(str(p.relative_to(root)), p.lstat().st_mode, os.readlink(p) if p.is_symlink() else p.read_bytes() if p.is_file() else None) for p in sorted(root.rglob('*'))]


def package(output, target=TARGET, server=SERVER, helper=HELPER, **options):
    return run_bounded([sys.executable, str(PACKAGE), str(output), target, str(server), str(helper)], **options)


def setUpModule():
    """Fail instead of skipping when full qualification is requested but an input is missing."""
    if os.environ.get('FRAYD_NATIVE_REQUIRE_QUALIFICATION') == '1':
        required = ['FRAYD_NATIVE_HELPER', 'FRAYD_NATIVE_ARCHIVE', 'FRAYD_NATIVE_BUILD_SOURCE']
        missing = [name for name in required if not os.environ.get(name)]
        if missing:
            raise RuntimeError('Native qualification prerequisites missing: ' + ', '.join(missing))


class RunBounded(unittest.TestCase):
    def test_timeout_kills_descendants(self):
        grandchild = "import fcntl,pathlib,sys,time; f=open(sys.argv[1],'w'); fcntl.flock(f,fcntl.LOCK_EX); pathlib.Path(sys.argv[2]).touch(); time.sleep(20)"
        driver = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',*sys.argv[1:]]); time.sleep(20)"
        with tempfile.TemporaryDirectory() as directory:
            lease = Path(directory) / 'lease'
            ready = Path(directory) / 'ready'
            with self.assertRaises(subprocess.TimeoutExpired):
                run_bounded([sys.executable, '-c', driver, grandchild, str(lease), str(ready)], timeout=3)
            self.assertTrue(ready.exists(), 'grandchild never took its lease')

            def lease_released():
                with lease.open('a') as handle:
                    try:
                        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError:
                        return False
                    return True

            wait_for(lease_released, 'the killed grandchild to release its lease', timeout=5)


KEY = r'(\s*(?:- )?\s*){}:(?:\s+(.*?))?(?:\s+#.*)?$'


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
    """List every way the macOS workflow stops being manual, read-only, non-publishing and locked."""
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


class NativeOwnership(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not os.environ.get('FRAYD_NATIVE_HELPER'):
            raise unittest.SkipTest('Supply a freshly built FRAYD_NATIVE_HELPER explicitly; generic Python discovery does not install/build Rust')
        if not HELPER.is_file():
            raise RuntimeError('Explicit native helper is unavailable')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='frayd native ', dir=TEMP_ROOT)
        self.addCleanup(self.temp.cleanup)
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

    def manifest(self):
        entries = []
        for p in sorted(self.bundle.rglob('*')):
            if p.name == 'manifest.json':
                continue
            entries.append({'path': p.relative_to(self.bundle).as_posix(), 'kind': 'dir' if p.is_dir() else 'file', 'mode': stat.S_IMODE(p.stat().st_mode), 'size': p.stat().st_size if p.is_file() else 0, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else ''})
        (self.bundle / 'manifest.json').write_text(json.dumps({'schema': 1, 'version': '0.1.1', 'source': 'a' * 40, 'target': 'fixture', 'toolchain': 'fixture', 'entries': entries}))

    def run_helper(self, command, *args, code=0, answer='', environment=None):
        result = run_bounded([str(HELPER), command, '--prefix', str(self.prefix), *args], input=answer, text=True, capture_output=True, env=environment)
        self.assertEqual(result.returncode, code, result.stderr)
        return result

    def install(self, *args, **kw):
        return self.run_helper('install', '--bundle', str(self.bundle), '--data-dir', str(self.data), *args, **kw)

    def snapshot(self):
        return snapshot(self.base)

    def test_package_complete_manifest_and_architecture(self):
        output = self.base / 'package output'
        result = package(output, capture_output=True, text=True)
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
        identity = json.loads(run_bounded([str(SERVER), '--build-info'], check=True, capture_output=True, text=True).stdout)
        self.assertEqual(json.loads((payload / 'build-info.json').read_text()), identity)
        self.assertEqual(identity['commit'], manifest['source'])
        self.bundle = payload
        self.install('--yes')
        self.run_helper('uninstall', '--yes')
        fake_output = self.base / 'helper is not server'
        rejected = package(fake_output, server=HELPER, capture_output=True)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertFalse(fake_output.exists())
        wrong_target = ('x86_64' if ARCHITECTURE == 'aarch64' else 'aarch64') + TARGET[len(ARCHITECTURE):]
        before = self.snapshot()
        result = package(self.base / 'wrong architecture output', target=wrong_target, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.snapshot(), before)

    def test_tar_preserved_modes_under_restrictive_umask(self):
        output = self.base / 'umask package'
        package(output, check=True, capture_output=True)
        archive = next(output.glob('*.tar.gz'))
        original = next(path for path in output.iterdir() if path.is_dir())
        self.assertIn('tar -xpzf', (original / 'README.md').read_text())
        plain = self.base / 'plain extraction'
        preserved = self.base / 'preserved extraction'
        plain.mkdir(mode=0o700)
        preserved.mkdir(mode=0o700)
        run_bounded(['tar', '-xzf', str(archive), '-C', str(plain)], umask=0o077, check=True, capture_output=True)
        run_bounded(['tar', '-xpzf', str(archive), '-C', str(preserved)], umask=0o077, check=True, capture_output=True)
        plain_bundle = next(plain.iterdir())
        preserved_bundle = next(preserved.iterdir())
        self.assertNotEqual(stat.S_IMODE((plain_bundle / 'client/index.html').stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE((preserved_bundle / 'client/index.html').stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE((preserved_bundle / 'install.sh').stat().st_mode), 0o755)
        before = self.snapshot()
        refused = run_bounded([str(plain_bundle / 'install.sh'), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], capture_output=True)
        self.assertEqual(refused.returncode, 2)
        self.assertEqual(self.snapshot(), before)
        run_bounded([str(preserved_bundle / 'install.sh'), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], check=True, capture_output=True)
        run_bounded([str(preserved_bundle / 'uninstall.sh'), '--prefix', str(self.prefix), '--yes'], check=True, capture_output=True)
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
        result = run_bounded([str(HELPER), 'install', '--bundle', str(self.bundle), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], umask=0, capture_output=True)
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
            self.skipTest('root bypasses the parent mode that makes the final prefix removal fail')
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

    def test_foreground_interrupt_cleans_child_and_allows_removal(self):
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nexec sleep 60\n')
        self.manifest()
        for target in ['group', 'helper', 'child', 'both-ready']:
            for interrupt in [signal.SIGINT, signal.SIGTERM]:
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
                        self.assertFalse(marker.exists(), 'normal interruption must clear uncertainty after reaping')
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
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait(timeout=10)
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
            child = wait_for(lambda: running_child_pid(self.prefix), 'fixture child start')
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
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)

    def test_invalid_targets_and_flags_zero_effects(self):
        before = self.snapshot()
        for target in ['relative', '/', '/usr/frayd', str(ROOT / 'native-test-prefix'), str(self.base) + '/./unsafe', str(self.base) + '//unsafe', str(self.base / '.cargo/new'), str(self.base / '.rustup/new'), str(self.base / 'original checkout/new')]:
            result = run_bounded([str(HELPER), 'install', '--bundle', str(self.bundle), '--prefix', target, '--data-dir', str(self.data), '--yes'], capture_output=True)
            self.assertEqual(result.returncode, 2, result.stderr)
        self.run_helper('uninstall', '--purge-library', code=2)
        self.assertEqual(self.snapshot(), before)
        link = self.base / 'link'
        link.symlink_to(self.base, target_is_directory=True)
        self.prefix = link / 'app'
        before = self.snapshot()
        self.install('--yes', code=2)
        self.assertEqual(self.snapshot(), before)


@unittest.skipUnless(
    os.environ.get('FRAYD_NATIVE_ARCHIVE') and os.environ.get('FRAYD_NATIVE_BUILD_SOURCE'),
    'actual server qualification requires FRAYD_NATIVE_ARCHIVE and FRAYD_NATIVE_BUILD_SOURCE',
)
class NativeRuntime(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='frayd runtime ', dir=TEMP_ROOT)
        self.addCleanup(temp.cleanup)
        self.base = Path(temp.name)
        archive = Path(os.environ['FRAYD_NATIVE_ARCHIVE'])
        checksum = archive.with_suffix(archive.suffix + '.sha256').read_text().split()[0]
        self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), checksum)
        extracted = self.base / 'relocated extracted bundle'
        extracted.mkdir(mode=0o700)
        with tarfile.open(archive) as tar:
            for member in tar.getmembers():
                self.assertTrue(member.isfile() or member.isdir())
                self.assertFalse(Path(member.name).is_absolute())
                self.assertNotIn('..', Path(member.name).parts)
            tar.extractall(extracted, filter='data')
        bundles = list(extracted.iterdir())
        self.assertEqual(len(bundles), 1)
        self.bundle = bundles[0]
        self.prefix = self.base / 'installed application'
        self.data = self.base / 'application data'
        self.library = self.base / 'library fixture'
        self.library.mkdir(mode=0o700)
        run_bounded([str(self.bundle / 'install.sh'), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], check=True, capture_output=True)
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

    def kill_server(self):
        if self.process is not None and self.process.poll() is None:
            # This process group was created by this fixture only.
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=10)

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
            parent = run_bounded(['ps', '-o', 'ppid=', '-p', str(child)], text=True, capture_output=True, check=True, timeout=10).stdout.strip()
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
        refused = run_bounded([str(self.bundle / 'uninstall.sh'), '--prefix', str(self.prefix), '--yes'], capture_output=True)
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
        run_bounded([str(self.bundle / 'uninstall.sh'), '--prefix', str(self.prefix), '--yes'], check=True, capture_output=True)
        self.assertFalse(self.prefix.exists())
        self.assertEqual(snapshot(self.data), preserved, 'application data changed during removal')


if __name__ == '__main__':
    unittest.main()
