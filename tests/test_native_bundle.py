"""Native ownership contract. Runtime qualification requires FRAYD_NATIVE_BUNDLE."""
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
import subprocess
import tarfile
import tempfile
import time
import unittest
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
HELPER = Path(os.environ.get('FRAYD_NATIVE_HELPER', ROOT / 'target/native-qualification/debug/frayd-native'))
TEMP_ROOT = Path(os.environ.get('FRAYD_NATIVE_TEST_TEMP_ROOT', '/var/tmp')).resolve()


def setUpModule():
    if os.environ.get('FRAYD_NATIVE_REQUIRE_QUALIFICATION') == '1':
        required = ['FRAYD_NATIVE_HELPER', 'FRAYD_NATIVE_ARCHIVE', 'FRAYD_NATIVE_BUILD_SOURCE']
        missing = [name for name in required if not os.environ.get(name)]
        if missing:
            raise RuntimeError('Native qualification prerequisites missing: ' + ', '.join(missing))


class NativeSourceContract(unittest.TestCase):
    def test_no_legacy_destructive_paths(self):
        script = (ROOT / 'scripts/uninstall.sh').read_text()
        for forbidden in ['pkill', 'rm -rf', 'systemctl', 'launchctl', 'PURGE_LIB', 'PURGE_DEPS']:
            self.assertNotIn(forbidden, script)

    def test_manual_nonpublishing_workflow(self):
        workflow = (ROOT / '.github/workflows/release-mac.yml').read_text()
        for forbidden in ['contents: write', 'softprops/', 'lipo ', 'tags:', '@v4', 'brew install']:
            self.assertNotIn(forbidden, workflow)
        for required in ['macos-15-intel', 'aarch64-apple-darwin', '--locked', '--bin frayd-native']:
            self.assertIn(required, workflow)

    def test_install_page_has_no_opaque_redirect(self):
        page = (ROOT / 'client/install/index.html').read_text()
        self.assertNotIn('location.href = liveBase', page)
        self.assertNotIn("r.type === 'opaque' || r.ok", page)


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

    def run_helper(self, command, *args, code=0, answer=''):
        result = subprocess.run([str(HELPER), command, '--prefix', str(self.prefix), *args], input=answer, text=True, capture_output=True)
        self.assertEqual(result.returncode, code, result.stderr)
        return result

    def install(self, *args, **kw):
        return self.run_helper('install', '--bundle', str(self.bundle), '--data-dir', str(self.data), *args, **kw)

    def snapshot(self):
        return [(str(p.relative_to(self.base)), p.lstat().st_mode, os.readlink(p) if p.is_symlink() else p.read_bytes() if p.is_file() else None) for p in sorted(self.base.rglob('*'))]

    def test_package_complete_manifest_and_architecture(self):
        architecture = 'aarch64' if os.uname().machine in ['arm64', 'aarch64'] else 'x86_64'
        target = architecture + ('-apple-darwin' if os.uname().sysname == 'Darwin' else '-unknown-linux-gnu')
        output = self.base / 'package output'
        # Two real Rust executables exercise packaging, without pretending the
        # helper used as a payload fixture is an actual Frayd HTTP server.
        result = subprocess.run([str(ROOT / 'scripts/native/package.sh'), str(output), target, str(HELPER), str(HELPER)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = next(output.glob('*.tar.gz'))
        self.assertEqual(archive.with_suffix(archive.suffix + '.sha256').read_text().split()[0], hashlib.sha256(archive.read_bytes()).hexdigest())
        payload = next(path for path in output.iterdir() if path.is_dir())
        manifest = json.loads((payload / 'manifest.json').read_text())
        self.assertTrue(manifest['toolchain'].startswith('rustc '))
        if os.environ.get('FRAYD_NATIVE_TOOLCHAIN_FILE'):
            self.assertEqual(manifest['toolchain'], Path(os.environ['FRAYD_NATIVE_TOOLCHAIN_FILE']).read_text().strip())
        expected = {'frayd-native', 'frayd-server', 'install.sh', 'start.sh', 'uninstall.sh', 'README.md', 'client/vendor/plyr/plyr.min.js'}
        self.assertTrue(expected.issubset({item['path'] for item in manifest['entries']}))
        self.assertFalse(any('.env' in item['path'] for item in manifest['entries']))
        self.assertEqual(manifest['target'], target)
        original_bundle = self.bundle
        self.bundle = payload
        self.install('--yes')
        self.run_helper('uninstall', '--yes')
        self.bundle = original_bundle
        wrong_output = self.base / 'wrong architecture output'
        wrong_target = ('x86_64' if architecture == 'aarch64' else 'aarch64') + target[len(architecture):]
        before = self.snapshot()
        result = subprocess.run([str(ROOT / 'scripts/native/package.sh'), str(wrong_output), wrong_target, str(HELPER), str(HELPER)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.snapshot(), before)

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
        result = subprocess.run([str(HELPER), 'install', '--bundle', str(self.bundle), '--prefix', str(self.prefix), '--data-dir', str(self.data), '--yes'], umask=0, capture_output=True)
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
        for interrupt in [signal.SIGINT, signal.SIGTERM]:
            with self.subTest(interrupt=interrupt):
                self.install('--yes')
                process = subprocess.Popen([str(HELPER), 'start', '--prefix', str(self.prefix)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
                try:
                    marker = self.prefix / '.frayd-child.json'
                    deadline = time.monotonic() + 10
                    while time.monotonic() < deadline:
                        if marker.exists() and json.loads(marker.read_text()).get('state') == 'running':
                            break
                        time.sleep(0.02)
                    else:
                        self.fail('fixture child did not start')
                    os.killpg(process.pid, interrupt)
                    process.wait(timeout=10)
                    self.assertFalse(marker.exists(), 'normal foreground interruption must remove orphan uncertainty after reaping its owned child')
                    self.run_helper('uninstall', '--yes')
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=10)

    def test_orphan_receipt_and_active_installation_refusal(self):
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
                    document = json.loads(marker.read_text())
                    if document.get('state') == 'running':
                        child = document['pid']
                        break
                time.sleep(0.02)
            self.assertIsNotNone(child)
            before = self.snapshot()
            self.run_helper('uninstall', '--yes', code=2)
            self.run_helper('start', code=2)
            self.assertEqual(self.snapshot(), before)
            os.kill(child, signal.SIGTERM)
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
            result = subprocess.run([str(HELPER), 'install', '--bundle', str(self.bundle), '--prefix', target, '--data-dir', str(self.data), '--yes'], capture_output=True)
            self.assertEqual(result.returncode, 2, result.stderr)
        self.run_helper('uninstall', '--purge-library', code=2)
        self.assertEqual(self.snapshot(), before)
        link = self.base / 'link'
        link.symlink_to(self.base, target_is_directory=True)
        self.prefix = link / 'app'
        before = self.snapshot()
        self.install('--yes', code=2)
        self.assertEqual(self.snapshot(), before)


@unittest.skipUnless(os.environ.get('FRAYD_NATIVE_ARCHIVE'), 'actual server qualification requires FRAYD_NATIVE_ARCHIVE and FRAYD_NATIVE_BUILD_SOURCE')
class NativeRuntime(unittest.TestCase):
    def test_relocated_real_server_reader_and_persistence(self):
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
            subprocess.run([str(bundle / 'install.sh'), '--prefix', str(prefix), '--data-dir', str(data), '--yes'], check=True, capture_output=True)
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
            environment = os.environ.copy()
            for name in ['FRAYD_LIBRARY', 'FRAYD_CERT', 'FRAYD_KEY']:
                environment.pop(name, None)
            environment['FRAYD_UPNP'] = '0'
            environment['FRAYD_BIND'] = 'invalid inherited value'
            # Paths deliberately poison inherited defaults: helper must override all four.
            for name in ['FRAYD_DB', 'FRAYD_THUMBS', 'FRAYD_TRANSCODES', 'FRAYD_CLIENT_DIR']:
                environment[name] = str(base / 'MUST NOT EXIST')
            process = None

            def request(path, payload=None, token=None, expected=200):
                headers = {'Content-Type': 'application/json'}
                if token:
                    headers['Authorization'] = 'Bearer ' + token
                body = None if payload is None else json.dumps(payload).encode()
                req = urllib.request.Request(url + path, data=body, headers=headers)
                with urllib.request.urlopen(req, timeout=3) as response:
                    self.assertEqual(response.status, expected)
                    return response.read()

            def start():
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
                nonlocal process
                if process is None or process.poll() is not None:
                    return
                if group_interrupt:
                    os.killpg(process.pid, signal.SIGINT)
                    self.assertIn(process.wait(timeout=10), [0, 2])
                    self.assertFalse((prefix / '.frayd-child.json').exists())
                    process = None
                    return
                child = json.loads((prefix / '.frayd-child.json').read_text())['pid']
                parent = subprocess.check_output(['ps', '-o', 'ppid=', '-p', str(child)], text=True).strip()
                self.assertEqual(int(parent), process.pid)
                os.kill(child, signal.SIGTERM)
                self.assertEqual(process.wait(timeout=10), 2)
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
                refused = subprocess.run([str(bundle / 'uninstall.sh'), '--prefix', str(prefix), '--yes'], capture_output=True)
                self.assertEqual(refused.returncode, 2)
                self.assertEqual(before, [(str(p.relative_to(prefix)), p.stat().st_mode, p.read_bytes() if p.is_file() else None) for p in sorted(prefix.rglob('*'))])
                stop()
                start()
                self.assertEqual(json.loads(request('/api/setup')), {'needed': False})
                login = json.loads(request('/api/auth/login', credentials))
                self.assertEqual(json.loads(request(f'/api/progress/{media_id}', token=login['token']))['position'], 0.5)
                self.assertEqual(request(f'/api/comic/{media_id}/page/0'), image)
                self.assertFalse((base / 'MUST NOT EXIST').exists())
                stop(group_interrupt=True)
                preserved = {str(p.relative_to(data)): p.read_bytes() for p in data.rglob('*') if p.is_file()}
                subprocess.run([str(bundle / 'uninstall.sh'), '--prefix', str(prefix), '--yes'], check=True, capture_output=True)
                self.assertFalse(prefix.exists())
                self.assertTrue(preserved == {str(p.relative_to(data)): p.read_bytes() for p in data.rglob('*') if p.is_file()}, 'application data changed during removal')
                print(json.dumps({'scenario': 'actual-native-relocated-reader-persistence-removal', 'status': 'passed', 'platform': os.uname().sysname, 'architecture': os.uname().machine, 'archive_sha256': checksum, 'base_source_commit': manifest['source'], 'version': manifest['version'], 'source_files_unavailable': True, 'media_codecs': 'not-qualified'}))
            finally:
                if process is not None and process.poll() is None:
                    # This process group was created by this exact fixture only.
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=10)


if __name__ == '__main__':
    unittest.main()
