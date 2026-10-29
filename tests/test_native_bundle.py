"""Native ownership contract. Runtime qualification requires FRAYD_NATIVE_ARCHIVE and FRAYD_NATIVE_BUILD_SOURCE."""
import base64
import hashlib
import json
import os
from pathlib import Path
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
TEMP_ROOT = Path(os.environ.get('FRAYD_NATIVE_TEST_TEMP_ROOT', '/var/tmp')).resolve()


def run_bounded(arguments, *, timeout=60, **options):
    """Bound a finite fixture command and reap its private group on interruption."""
    input_data = options.pop('input', None)
    capture = options.pop('capture_output', False)
    check = options.pop('check', False)
    if input_data is not None:
        options['stdin'] = subprocess.PIPE
    if capture:
        options['stdout'] = subprocess.PIPE
        options['stderr'] = subprocess.PIPE
    process = subprocess.Popen(arguments, start_new_session=True, shell=False, **options)
    try:
        stdout, stderr = process.communicate(input_data, timeout=timeout)
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        raise
    result = subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)
    if check:
        result.check_returncode()
    return result


def setUpModule():
    """Require explicit native build inputs only when full qualification is requested."""
    if os.environ.get('FRAYD_NATIVE_REQUIRE_QUALIFICATION') == '1':
        required = ['FRAYD_NATIVE_HELPER', 'FRAYD_NATIVE_ARCHIVE', 'FRAYD_NATIVE_BUILD_SOURCE']
        missing = [name for name in required if not os.environ.get(name)]
        if missing:
            raise RuntimeError('Native qualification prerequisites missing: ' + ', '.join(missing))


class NativeSourceContract(unittest.TestCase):
    def test_no_legacy_destructive_paths(self):
        """Keep the legacy uninstall shim free of broad process and filesystem deletion."""
        script = (ROOT / 'scripts/uninstall.sh').read_text()
        for forbidden in ['pkill', 'rm -rf', 'systemctl', 'launchctl', 'PURGE_LIB', 'PURGE_DEPS']:
            self.assertNotIn(forbidden, script)

    def test_manual_nonpublishing_workflow(self):
        """Require manual macOS packaging without publication permissions or floating actions."""
        workflow = (ROOT / '.github/workflows/release-mac.yml').read_text()
        for forbidden in ['contents: write', 'softprops/', 'lipo ', 'tags:', '@v4', 'brew install']:
            self.assertNotIn(forbidden, workflow)
        for required in ['macos-15-intel', 'aarch64-apple-darwin', '--locked', '--bin frayd-native']:
            self.assertIn(required, workflow)

    def test_runtime_discovery_requires_both_inputs(self):
        """Partial runtime inputs skip discovery; complete inputs reach archive validation."""
        with tempfile.TemporaryDirectory(prefix='frayd discovery ', dir=TEMP_ROOT) as directory:
            archive = Path(directory) / 'fixture.tar.gz'
            build_source = Path(directory) / 'absent-source/server'
            for have_archive, have_source in [(False, False), (True, False), (False, True), (True, True)]:
                with self.subTest(archive=have_archive, build_source=have_source):
                    environment = os.environ.copy()
                    for name in ['FRAYD_NATIVE_ARCHIVE', 'FRAYD_NATIVE_BUILD_SOURCE', 'FRAYD_NATIVE_REQUIRE_QUALIFICATION']:
                        environment.pop(name, None)
                    if have_archive:
                        environment['FRAYD_NATIVE_ARCHIVE'] = str(archive)
                    if have_source:
                        environment['FRAYD_NATIVE_BUILD_SOURCE'] = str(build_source)
                    result = run_bounded(
                        [sys.executable, '-m', 'unittest', 'discover', '-s', str(ROOT / 'tests'),
                         '-p', 'test_native_bundle.py', '-k', 'NativeRuntime', '-v'],
                        env=environment, capture_output=True, text=True,
                    )
                    self.assertIn('Ran 1 test', result.stderr)
                    self.assertNotIn('KeyError', result.stderr)
                    if have_archive and have_source:
                        # Complete inputs must admit the real case, which rejects
                        # this deliberately absent synthetic archive checksum.
                        self.assertEqual(result.returncode, 1, result.stderr)
                        self.assertIn('FileNotFoundError', result.stderr)
                        self.assertIn('fixture.tar.gz.sha256', result.stderr)
                        self.assertNotIn('skipped', result.stderr)
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertIn('OK (skipped=1)', result.stderr)
                        self.assertIn('requires FRAYD_NATIVE_ARCHIVE and FRAYD_NATIVE_BUILD_SOURCE', result.stderr)


class NativeOwnership(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        """Skip ownership cases unless a freshly built helper path is supplied."""
        if not os.environ.get('FRAYD_NATIVE_HELPER'):
            raise unittest.SkipTest('Supply a freshly built FRAYD_NATIVE_HELPER explicitly; generic Python discovery does not install/build Rust')
        if not HELPER.is_file():
            raise RuntimeError('Explicit native helper is unavailable')

    def setUp(self):
        """Create a private bundle, install/data paths and protected sentinels for each case."""
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
        """Write a manifest covering each bundle entry with mode, size and content digest."""
        entries = []
        for p in sorted(self.bundle.rglob('*')):
            if p.name == 'manifest.json':
                continue
            entries.append({'path': p.relative_to(self.bundle).as_posix(), 'kind': 'dir' if p.is_dir() else 'file', 'mode': stat.S_IMODE(p.stat().st_mode), 'size': p.stat().st_size if p.is_file() else 0, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else ''})
        (self.bundle / 'manifest.json').write_text(json.dumps({'schema': 1, 'version': '0.1.1', 'source': 'a' * 40, 'target': 'fixture', 'toolchain': 'fixture', 'entries': entries}))

    def run_helper(self, command, *args, code=0, answer='', environment=None):
        """Invoke the native helper with this fixture prefix and assert its exit status."""
        result = run_bounded([str(HELPER), command, '--prefix', str(self.prefix), *args], input=answer, text=True, capture_output=True, env=environment)
        self.assertEqual(result.returncode, code, result.stderr)
        return result

    def install(self, *args, **kw):
        """Install the fixture bundle into its private prefix and data directory."""
        return self.run_helper('install', '--bundle', str(self.bundle), '--data-dir', str(self.data), *args, **kw)

    def snapshot(self):
        """Capture names, modes and bytes or symlink targets across the private fixture."""
        return [(str(p.relative_to(self.base)), p.lstat().st_mode, os.readlink(p) if p.is_symlink() else p.read_bytes() if p.is_file() else None) for p in sorted(self.base.rglob('*'))]

    def test_timeout_reaps_owned_descendants(self):
        """On timeout, reap a descendant proven to hold a file lease."""
        import fcntl
        lease = self.base / 'timeout child lease'
        ready = self.base / 'timeout child ready'
        group_record = self.base / 'timeout owned group'
        child = "import fcntl,pathlib,sys,time; f=open(sys.argv[1],'w'); fcntl.flock(f,fcntl.LOCK_EX); time.sleep(2); pathlib.Path(sys.argv[2]).write_text('ready'); time.sleep(60)"
        driver = "import os,pathlib,subprocess,sys,time; pathlib.Path(sys.argv[1]).write_text(str(os.getpgrp())); subprocess.Popen([sys.executable,'-c',sys.argv[2],sys.argv[3],sys.argv[4]]); time.sleep(60)"
        original_communicate = subprocess.Popen.communicate
        awaiting_readiness = True

        def communicate_after_ready(process, input_data=None, timeout=None):
            """Wait for the descendant lease before allowing the bounded runner to time out."""
            nonlocal awaiting_readiness
            if awaiting_readiness:
                # Patch only this test's first communicate call. run_bounded has
                # already recorded its process, so readiness failure still enters
                # its normal owned-group cleanup without changing the runner API.
                awaiting_readiness = False
                deadline = time.monotonic() + 10
                while not ready.exists():
                    self.assertIsNone(process.poll(), 'fixture exited before readiness')
                    self.assertLess(time.monotonic(), deadline, 'fixture readiness timed out')
                    time.sleep(0.02)
                with lease.open('r+') as handle:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return original_communicate(process, input_data, timeout=timeout)

        try:
            with mock.patch.object(subprocess.Popen, 'communicate', communicate_after_ready):
                with self.assertRaises(subprocess.TimeoutExpired):
                    run_bounded([sys.executable, '-c', driver, str(group_record), child, str(lease), str(ready)], timeout=1, capture_output=True, text=True)
            self.assertFalse(awaiting_readiness)
            self.assertTrue(ready.exists(), 'descendant was ready before the cleanup timer')
            with lease.open('r+') as handle:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        finally:
            if group_record.exists():
                try:
                    os.killpg(int(group_record.read_text()), signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_package_complete_manifest_and_architecture(self):
        """Package actual server identity and reject a mismatched target without fixture writes."""
        architecture = 'aarch64' if os.uname().machine in ['arm64', 'aarch64'] else 'x86_64'
        target = architecture + ('-apple-darwin' if os.uname().sysname == 'Darwin' else '-unknown-linux-gnu')
        output = self.base / 'package output'
        result = run_bounded([str(ROOT / 'scripts/native/package.sh'), str(output), target, str(SERVER), str(HELPER)], capture_output=True, text=True)
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
        self.assertEqual(manifest['target'], target)
        identity = json.loads(run_bounded([str(SERVER), '--build-info'], check=True, capture_output=True, text=True).stdout)
        self.assertEqual(json.loads((payload / 'build-info.json').read_text()), identity)
        self.assertEqual(identity['commit'], manifest['source'])
        original_bundle = self.bundle
        self.bundle = payload
        self.install('--yes')
        self.run_helper('uninstall', '--yes')
        self.bundle = original_bundle
        fake_output = self.base / 'helper is not server'
        rejected = run_bounded([str(ROOT / 'scripts/native/package.sh'), str(fake_output), target, str(HELPER), str(HELPER)], capture_output=True)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertFalse(fake_output.exists())
        wrong_output = self.base / 'wrong architecture output'
        wrong_target = ('x86_64' if architecture == 'aarch64' else 'aarch64') + target[len(architecture):]
        before = self.snapshot()
        result = run_bounded([str(ROOT / 'scripts/native/package.sh'), str(wrong_output), wrong_target, str(SERVER), str(HELPER)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.snapshot(), before)

    def test_tar_preserved_modes_under_restrictive_umask(self):
        """Require preserved archive modes for installation under a restrictive umask."""
        architecture = 'aarch64' if os.uname().machine in ['arm64', 'aarch64'] else 'x86_64'
        target = architecture + ('-apple-darwin' if os.uname().sysname == 'Darwin' else '-unknown-linux-gnu')
        output = self.base / 'umask package'
        run_bounded([str(ROOT / 'scripts/native/package.sh'), str(output), target, str(SERVER), str(HELPER)], check=True, capture_output=True)
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
        """Declining install or uninstall leaves the fixture byte-for-byte unchanged."""
        before = self.snapshot()
        self.install(answer='n\n')
        self.assertEqual(self.snapshot(), before)
        self.install('--yes')
        before = self.snapshot()
        self.run_helper('uninstall', answer='n\n')
        self.assertEqual(self.snapshot(), before)

    def test_install_start_remove_preserves_data_and_reinstall(self):
        """Starting uses private data; uninstall preserves it and permits reinstall."""
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
        """Missing data authority permits prefix removal while retaining remaining data."""
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
        """A missing data marker cannot bypass modified payload or overlap guards."""
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
        """An unavailable absolute library allows lifecycle actions without creating roots."""
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
        """Refuse missing library paths that may overlap owned or aliased locations."""
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
        """An incomplete bundle is refused before any installation write."""
        (self.bundle / 'start.sh').unlink()
        self.manifest()
        before = self.snapshot()
        self.install('--yes', code=2)
        self.assertEqual(self.snapshot(), before)

    def test_corrupt_or_extra_bundle_zero_effects(self):
        """Checksum, extra-entry, traversal and symlink corruption leave no effects."""
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
        """Unknown existing data is refused and permissive umask still yields private modes."""
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
        """Existing unknown or modified prefix content blocks install and removal."""
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
        """A filesystem removal failure keeps the receipt available for a later retry."""
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

    def test_partial_removal_retry_and_data_redirect_refusal(self):
        """Reject redirected data; tolerate a previously removed owned file on retry."""
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
        """Helper, child and simultaneous foreground INT/TERM exit cleanly after reaping."""
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nexec sleep 60\n')
        self.manifest()
        for target in ['group', 'helper', 'child', 'both-ready']:
            for interrupt in [signal.SIGINT, signal.SIGTERM]:
                with self.subTest(target=target, interrupt=interrupt):
                    self.install('--yes')
                    process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, start_new_session=True)
                    try:
                        marker = self.prefix / '.frayd-child.json'
                        deadline = time.monotonic() + 10
                        while time.monotonic() < deadline:
                            if marker.exists():
                                try:
                                    document = json.loads(marker.read_text())
                                except json.JSONDecodeError:
                                    continue
                                if document.get('state') == 'running':
                                    child = document['pid']
                                    break
                            time.sleep(0.02)
                        else:
                            self.fail('fixture child did not start')
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
        """Ordinary failures must remain errors rather than being accepted as interrupts."""
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nexit 7\n')
        self.manifest()
        self.install('--yes')
        result = self.run_helper('start', code=2)
        self.assertIn('server exited with', result.stderr)
        self.assertFalse((self.prefix / '.frayd-child.json').exists())
        self.run_helper('uninstall', '--yes')

    def test_orphan_receipt_and_active_installation_refusal(self):
        """Active or uncertain child receipts block start and uninstall without writes."""
        (self.bundle / 'frayd-server').write_text('#!/bin/sh\nexec sleep 60\n')
        self.manifest()
        self.install('--yes')
        process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            deadline = time.monotonic() + 10
            marker = self.prefix / '.frayd-child.json'
            child = None
            while time.monotonic() < deadline:
                if marker.exists():
                    try:
                        document = json.loads(marker.read_text())
                    except (json.JSONDecodeError, FileNotFoundError):
                        time.sleep(0.02)
                        continue
                    if document.get('state') == 'running':
                        child = document['pid']
                        break
                time.sleep(0.02)
            self.assertIsNotNone(child)
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
        """Invalid target paths and destructive flags leave all fixture paths unchanged."""
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
    def test_relocated_real_server_reader_and_persistence(self):
        """Relocated archive serves real HTTP assets and retains reader state across restart."""
        build_source = Path(os.environ['FRAYD_NATIVE_BUILD_SOURCE'])
        self.assertTrue(build_source.is_absolute())
        self.assertFalse(any(build_source.iterdir()) if build_source.exists() else False, 'compile-time source must be absent or empty in an isolated namespace')
        self.assertFalse((build_source.parent / 'client/index.html').exists(), 'compile-time client assets must also be unavailable')
        with tempfile.TemporaryDirectory(prefix='frayd runtime ', dir=TEMP_ROOT) as directory:
            base = Path(directory)
            extracted = base / 'relocated extracted bundle'
            extracted.mkdir(mode=0o700)
            archive = Path(os.environ['FRAYD_NATIVE_ARCHIVE'])
            checksum = archive.with_suffix(archive.suffix + '.sha256').read_text().split()[0]
            self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), checksum)
            with tarfile.open(archive) as tar:
                for member in tar.getmembers():
                    self.assertTrue(member.isfile() or member.isdir())
                    self.assertFalse(Path(member.name).is_absolute())
                    self.assertNotIn('..', Path(member.name).parts)
                tar.extractall(extracted, filter='data')
            bundles = list(extracted.iterdir())
            self.assertEqual(len(bundles), 1)
            bundle = bundles[0]
            prefix = base / 'installed application'
            data = base / 'application data'
            library = base / 'library fixture'
            library.mkdir(mode=0o700)
            run_bounded([str(bundle / 'install.sh'), '--prefix', str(prefix), '--data-dir', str(data), '--yes'], check=True, capture_output=True)
            # Every HTTP asset is compared with this extracted manifest, not source files.
            manifest = json.loads((bundle / 'manifest.json').read_text())
            self.assertTrue(manifest['toolchain'].startswith('rustc '))
            if os.environ.get('FRAYD_NATIVE_TOOLCHAIN_FILE'):
                self.assertEqual(manifest['toolchain'], Path(os.environ['FRAYD_NATIVE_TOOLCHAIN_FILE']).read_text().strip())
            expected_architecture = os.environ.get('FRAYD_NATIVE_EXPECT_ARCH')
            if expected_architecture:
                self.assertEqual(os.uname().machine, expected_architecture)
            for asset in (p for p in manifest['entries'] if p['kind'] == 'file' and p['path'].startswith('client/vendor/')):
                self.assertTrue((prefix / asset['path']).is_file())
            with socket.socket() as probe:
                probe.bind(('127.0.0.1', 0))
                port = probe.getsockname()[1]
            url = f'http://127.0.0.1:{port}'
            tools = base / 'private tools'
            tools.mkdir(mode=0o700)
            upnp_record = base / 'router mapping requests'
            upnp_recorder = tools / 'upnpc'
            upnp_recorder.write_text('#!/bin/sh\nprintf \'requested\\n\' >> "$FRAYD_NATIVE_UPNP_RECORD"\n')
            upnp_recorder.chmod(0o700)
            environment = os.environ.copy()
            environment['PATH'] = str(tools) + os.pathsep + environment.get('PATH', '')
            environment['FRAYD_NATIVE_UPNP_RECORD'] = str(upnp_record)
            # Prove the recorder works; it performs no router or network operation.
            run_bounded([str(upnp_recorder)], env=environment, check=True)
            self.assertEqual(upnp_record.read_text(), 'requested\n')
            upnp_record.unlink()
            for name in ['FRAYD_LIBRARY', 'FRAYD_CERT', 'FRAYD_KEY']:
                environment.pop(name, None)
            environment['FRAYD_UPNP'] = '0'
            environment['FRAYD_BIND'] = 'invalid inherited value'
            # Paths deliberately poison inherited defaults: helper must override all four.
            for name in ['FRAYD_DB', 'FRAYD_THUMBS', 'FRAYD_TRANSCODES', 'FRAYD_CLIENT_DIR']:
                environment[name] = str(base / 'MUST NOT EXIST')
            process = None

            def request(path, payload=None, token=None, expected=200):
                """Issue a loopback JSON request and assert its expected HTTP status."""
                headers = {'Content-Type': 'application/json'}
                if token:
                    headers['Authorization'] = 'Bearer ' + token
                body = None if payload is None else json.dumps(payload).encode()
                req = urllib.request.Request(url + path, data=body, headers=headers)
                with urllib.request.urlopen(req, timeout=3) as response:
                    self.assertEqual(response.status, expected)
                    return response.read()

            def start():
                """Start the relocated server and wait up to twenty seconds for health."""
                nonlocal process
                process = subprocess.Popen([str(prefix / 'start.sh'), '--prefix', str(prefix), '--port', str(port)], env=environment, cwd=base, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline:
                    self.assertIsNone(process.poll(), 'supervisor exited before readiness')
                    try:
                        if json.loads(request('/api/health')) == {'status': 'ok'}:
                            return
                    except OSError:
                        time.sleep(0.1)
                self.fail('native HTTP readiness timed out')

            def stop(group_interrupt=False):
                """Stop the server by group interrupt or child signal and clear its receipt."""
                nonlocal process
                if process is None or process.poll() is not None:
                    return
                if group_interrupt:
                    os.killpg(process.pid, signal.SIGINT)
                    self.assertEqual(process.wait(timeout=10), 0)
                    self.assertFalse((prefix / '.frayd-child.json').exists())
                    process = None
                    return
                child = json.loads((prefix / '.frayd-child.json').read_text())['pid']
                parent = run_bounded(['ps', '-o', 'ppid=', '-p', str(child)], text=True, capture_output=True, check=True, timeout=10).stdout.strip()
                self.assertEqual(int(parent), process.pid)
                os.kill(child, signal.SIGTERM)
                self.assertEqual(process.wait(timeout=10), 0)
                self.assertFalse((prefix / '.frayd-child.json').exists())
                process = None

            try:
                start()
                self.assertEqual(request('/'), (prefix / 'client/index.html').read_bytes())
                vendor = next(p for p in manifest['entries'] if p['kind'] == 'file' and p['path'].startswith('client/vendor/'))
                self.assertEqual(hashlib.sha256(request('/' + vendor['path'][len('client/'):])).hexdigest(), vendor['sha256'])
                self.assertEqual(json.loads(request('/api/setup')), {'needed': True})
                credentials = {'email': 'native-fixture@example.invalid', 'password': secrets.token_urlsafe(32)}
                owner = json.loads(request('/api/setup', credentials))
                self.assertTrue(owner['is_admin'])
                token = owner['token']
                request('/api/admin/settings', {'library_paths': [str(library)]}, token, expected=204)
                image = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=')
                with zipfile.ZipFile(library / 'native-reader.cbz', 'w') as comic:
                    comic.writestr('001.png', image)
                request('/api/scan', {}, token, expected=202)
                deadline = time.monotonic() + 20
                while json.loads(request('/api/scan/status'))['scanning']:
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(0.1)
                media_id = next(item['id'] for item in json.loads(request('/api/library')) if item['kind'] == 'comic')
                self.assertEqual(json.loads(request(f'/api/comic/{media_id}/pages')), ['001.png'])
                self.assertEqual(request(f'/api/comic/{media_id}/page/0'), image)
                request('/api/progress', {'media_id': media_id, 'position': 0.5}, token, expected=204)
                # Active lifecycle refusal must leave every installed byte unchanged.
                before = [(str(p.relative_to(prefix)), p.stat().st_mode, p.read_bytes() if p.is_file() else None) for p in sorted(prefix.rglob('*'))]
                refused = run_bounded([str(bundle / 'uninstall.sh'), '--prefix', str(prefix), '--yes'], capture_output=True)
                self.assertEqual(refused.returncode, 2)
                self.assertEqual(before, [(str(p.relative_to(prefix)), p.stat().st_mode, p.read_bytes() if p.is_file() else None) for p in sorted(prefix.rglob('*'))])
                request('/api/admin/settings', {'upnp': True}, token, expected=204)
                stop()
                environment['FRAYD_UPNP'] = '1'
                start()
                # Both inherited and persisted opt-ins must be inert on loopback.
                deadline = time.monotonic() + 0.5
                while time.monotonic() < deadline:
                    self.assertFalse(upnp_record.exists(), 'loopback server requested router mapping')
                    time.sleep(0.05)
                self.assertEqual(json.loads(request('/api/setup')), {'needed': False})
                login = json.loads(request('/api/auth/login', credentials))
                self.assertEqual(json.loads(request(f'/api/progress/{media_id}', token=login['token']))['position'], 0.5)
                self.assertEqual(request(f'/api/comic/{media_id}/page/0'), image)
                self.assertFalse((base / 'MUST NOT EXIST').exists())
                request('/api/admin/settings', {'upnp': False}, login['token'], expected=204)
                environment['FRAYD_UPNP'] = '0'
                stop(group_interrupt=True)
                # An unavailable explicit root must leave account/metadata reads
                # healthy without granting permission to recreate source media.
                missing_library = base / 'unavailable source mount' / 'media'
                environment['FRAYD_LIBRARY'] = str(missing_library)
                start()
                self.assertEqual(json.loads(request('/api/health')), {'status': 'ok'})
                offline_status = json.loads(request('/api/scan/status'))
                self.assertTrue(offline_status['library_configured'])
                self.assertEqual(offline_status['library_count'], 1)
                offline_login = json.loads(request('/api/auth/login', credentials))
                self.assertEqual(json.loads(request(f'/api/progress/{media_id}', token=offline_login['token']))['position'], 0.5)
                offline_metadata = json.loads(request('/api/library'))
                self.assertTrue(any(item['id'] == media_id and item['kind'] == 'comic' and item['title'] == 'native-reader' for item in offline_metadata))
                self.assertFalse(missing_library.exists())
                self.assertFalse(missing_library.parent.exists())
                stop(group_interrupt=True)
                environment.pop('FRAYD_LIBRARY')
                start()
                self.assertEqual(request(f'/api/comic/{media_id}/page/0'), image)
                self.assertEqual(json.loads(request(f'/api/progress/{media_id}', token=offline_login['token']))['position'], 0.5)
                stop(group_interrupt=True)
                preserved = {str(p.relative_to(data)): p.read_bytes() for p in data.rglob('*') if p.is_file()}
                run_bounded([str(bundle / 'uninstall.sh'), '--prefix', str(prefix), '--yes'], check=True, capture_output=True)
                self.assertFalse(prefix.exists())
                self.assertTrue(preserved == {str(p.relative_to(data)): p.read_bytes() for p in data.rglob('*') if p.is_file()}, 'application data changed during removal')
                print(json.dumps({'scenario': 'actual-native-relocated-reader-persistence-removal', 'status': 'passed', 'platform': os.uname().sysname, 'architecture': os.uname().machine, 'archive_sha256': checksum, 'base_source_commit': manifest['source'], 'version': manifest['version'], 'source_files_unavailable': True, 'unavailable_absolute_library_reads': 'passed', 'loopback_upnp_requests': 0, 'media_codecs': 'not-qualified'}))
            finally:
                if process is not None and process.poll() is None:
                    # This process group was created by this exact fixture only.
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=10)


if __name__ == '__main__':
    unittest.main()
