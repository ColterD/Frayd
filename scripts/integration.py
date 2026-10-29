"""Disposable HTTP user-path QA; credentials stay only in process memory."""
import argparse
import base64
import hashlib
import io
import json
import math
import os
import secrets
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import zipfile
from contextlib import closing, contextmanager
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
base = ''

def docker_timeout(args):
    """Budget finite operations used by HTTP QA and the importing context probe."""
    if not args:
        raise ValueError('Missing Docker operation')
    command = args[0]
    if command == 'compose':
        index = 1
        while index < len(args) and args[index] in ('-p', '-f'):
            index += 2
        command = args[index] if index < len(args) else ''
        if command in ('config', 'ps', 'port'):
            return 'metadata', 20
        if command == 'up':
            # Existing --wait-timeout90 plus bounded CLI/setup overhead; no builds.
            return 'compose-up', 120
        if command == 'down':
            return 'cleanup', 60
        if command in ('restart', 'stop', 'rm'):
            return 'lifecycle', 60
    elif command in ('inspect', 'logs'):
        return 'metadata', 20
    elif command == 'build':
        # check-build-context builds a local scratch/COPY probe; no network base pull.
        return 'context-build', 600
    elif command == 'create':
        return 'container-create', 30
    elif command == 'export':
        return 'container-export', 60
    elif command == 'rm' or (command == 'image' and len(args) > 1 and args[1] == 'rm'):
        return 'cleanup', 60
    elif command in ('container', 'network', 'volume') and len(args) > 1 and args[1] in ('ls', 'inspect'):
        return 'metadata', 20
    elif command == 'exec':
        index = 1
        while index < len(args) and args[index] == '-i':
            index += 1
        executable = args[index + 1] if index + 1 < len(args) else ''
        if executable in ('ffmpeg', '/usr/bin/ffmpeg'):
            # Owned two-second synthetic encodes; intentionally longer than control calls.
            return 'media-conversion', 120
        if executable:
            return 'exec', 30
    raise ValueError('Unsupported Docker operation; assign an explicit operation budget')


def run_docker(args, operation, timeout, **options):
    """Own CLI/plugin descendants and never expose private timeout argv or output."""
    input_data = options.pop('input', None)
    process = subprocess.Popen(['docker', *args], cwd=ROOT, start_new_session=True,
                               shell=False, stdin=subprocess.PIPE if input_data is not None else None,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, **options)
    try:
        stdout, stderr = process.communicate(input_data, timeout=timeout)
    except BaseException as error:
        # The CLI may already have exited while its Compose plugin still owns pipes.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            # Drain only in memory: inherited pipes close when killed plugin children exit.
            process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            error.add_note('Docker local process cleanup exceeded10s')
        finally:
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream is not None:
                    stream.close()
        if isinstance(error, subprocess.TimeoutExpired):
            raise subprocess.TimeoutExpired(['docker', operation], timeout) from None
        raise
    result = subprocess.CompletedProcess(['docker', operation], process.returncode, stdout, stderr)
    result.check_returncode()
    return result


def docker(*args, **kwargs):
    """Run an explicitly budgeted operation; only ordinary failures expose stderr."""
    operation, budget = docker_timeout(args)
    timeout = kwargs.pop('timeout', budget)
    if not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('Docker operation timeout must be finite and positive')
    try:
        return run_docker(args, operation, timeout, **kwargs)
    except subprocess.CalledProcessError as error:
        diagnostic = error.stderr or b''
        if isinstance(diagnostic, bytes):
            diagnostic = diagnostic.decode('utf-8', errors='replace')
        if diagnostic:
            print(diagnostic, file=sys.stderr, end='' if diagnostic.endswith('\n') else '\n')
        raise


@contextmanager
def cleanup_project(compose):
    """Remove disposable resources without masking the original scenario failure."""
    try:
        yield
    finally:
        failing = sys.exc_info()[0] is not None
        try:
            compose('down', '--volumes', '--remove-orphans')
        except Exception:
            if not failing:
                raise
            print('QA cleanup failed; original scenario failure retained', file=sys.stderr)


def request(path, data=None, token=None, headers=None, expected=200, timeout=15):
    """Issue an HTTP request and assert its expected status."""
    fields = dict(headers or {})
    if token:
        fields['Authorization'] = 'Bearer ' + token
    if data is not None:
        fields['Content-Type'] = 'application/json'
    req = urllib.request.Request(base + path, data=None if data is None else json.dumps(data).encode(), headers=fields)
    try:
        response = urllib.request.urlopen(req, timeout=timeout)
    except urllib.error.HTTPError as error:
        with closing(error):
            assert error.code == expected, (path, error.code, expected)
        return b''
    with closing(response):
        assert response.status == expected, (path, response.status, expected)
        return response.read()


def curl_request(path, data=None, token=None, method='GET', expected=200):
    """Capture real HTTP headers with curl -i; credentials remain in stdin/memory."""
    config = [f'url = {json.dumps(base + path)}', f'request = {json.dumps(method)}']
    if token:
        config.append('header = ' + json.dumps('Authorization: Bearer ' + token))
    if data is not None:
        config.extend(['header = "Content-Type: application/json"',
                       'data = ' + json.dumps(json.dumps(data))])
    result = subprocess.run(['curl', '-i', '--silent', '--show-error', '--max-time', '15',
                             '--config', '-'], input='\n'.join(config).encode(),
                            capture_output=True, check=True)
    headers, body = result.stdout.split(b'\r\n\r\n', 1)
    status = int(headers.split(b'\r\n', 1)[0].split()[1])
    # Never print response bodies: setup and login bodies contain session capabilities.
    print(f'HTTP curl-i {method} {path}: {status} expected {expected}')
    assert status == expected, (method, path, status, expected)
    return body

def websocket_upgrade(token):
    """Verify creator identity and private host proof survive the recovered proxy handshake."""
    party = json.loads(request('/api/party', {'identity': 'qa-host'}, token))
    code = party['code']
    host_capability = party['host_capability']
    address = urlsplit(base)
    def upgrade(authenticated):
        """Keep credentials/proof in memory; print only public status observations."""
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        authorization = f'Authorization: Bearer {token}\r\n' if authenticated else ''
        handshake = (f'GET /api/party/{code}/ws?identity=qa-host HTTP/1.1\r\n'
                     f'Host: {address.netloc}\r\n'
                     'Upgrade: websocket\r\nConnection: Upgrade\r\n'
                     f'Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n'
                     f'{authorization}'
                     f'Sec-WebSocket-Protocol: frayd-party, frayd-host.{host_capability}\r\n\r\n')
        with socket.create_connection((address.hostname, address.port), timeout=2) as connection:
            connection.sendall(handshake.encode())
            response = b''
            while b'\r\n\r\n' not in response:
                chunk = connection.recv(4096)
                assert chunk and len(response) < 16384, 'incomplete WebSocket handshake'
                response += chunk
        return key, response

    _, rejected = upgrade(False)
    assert rejected.startswith(b'HTTP/1.1 403 '), rejected.split(b'\r\n', 1)[0]
    print('PASS: WebSocket host proof with a different device identity is refused403')
    key, response = upgrade(True)
    assert response.startswith(b'HTTP/1.1 101 '), response.split(b'\r\n', 1)[0]
    accept = base64.b64encode(hashlib.sha1(
        (key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode(), usedforsecurity=False).digest())
    headers = dict(line.split(b': ', 1) for line in response.split(b'\r\n\r\n', 1)[0].split(b'\r\n')[1:])
    assert next(value for name, value in headers.items() if name.lower() == b'sec-websocket-accept') == accept
    assert next(value for name, value in headers.items() if name.lower() == b'sec-websocket-protocol') == b'frayd-party'


def check_scan_read_paths(phase, media_id, token):
    """Assert bounded proxy reads preserve health, comic visibility and saved progress."""
    for path, auth in [('/api/health', None), ('/', None), ('/api/library', None),
                       (f'/api/progress/{media_id}', token)]:
        started = time.monotonic()
        result = request(path, token=auth, timeout=2)
        elapsed = time.monotonic() - started
        assert elapsed < 2.5, (phase, path, elapsed)
        if path == '/api/health':
            assert json.loads(result) == {'status': 'ok'}
        elif path == '/api/library':
            assert any(item['id'] == media_id and item['kind'] == 'comic'
                       for item in json.loads(result))
        elif path.startswith('/api/progress/'):
            assert json.loads(result)['position'] == 0.5
        print(f'PASS: {phase} {path} responsive with expected data ({elapsed:.3f}s)')


def ready():
    """Wait for the disposable HTTP endpoint to accept requests."""
    deadline = time.monotonic() + 20
    last_error = None
    while time.monotonic() < deadline:
        try:
            request('/api/health', timeout=2)
            return
        except (OSError, AssertionError) as error:
            last_error = error
            time.sleep(0.5)
    raise AssertionError(f'QA server did not become ready: {last_error}')

def dns_recovery_ready(proxy, app):
    """Preserve bounded DNS and upstream diagnostics before failed QA cleanup."""
    try:
        ready()
    except AssertionError:
        probes = [
            ('proxy identity', ('inspect', '--format', '{{.Id}} {{.State.StartedAt}}', proxy)),
            ('app network', ('inspect', '--format', '{{json .NetworkSettings.Networks}}', app)),
            ('direct app health', ('exec', app, 'curl', '--fail', '--silent', '--max-time', '2',
                                   'http://127.0.0.1:8484/api/health')),
            ('Docker DNS', ('exec', proxy, 'nslookup', 'app', '127.0.0.11')),
            ('proxy errors', ('logs', '--tail', '30', proxy)),
        ]
        for label, args in probes:
            try:
                result = docker(*args, timeout=5)
                print(f'DIAGNOSTIC {label}: {(result.stdout + result.stderr).decode(errors="replace")[:12000]}',
                      file=sys.stderr)
            except Exception as error:
                print(f'DIAGNOSTIC {label} unavailable: {type(error).__name__}', file=sys.stderr)
        raise


def wait_for_scan_idle():
    """Retry only scan-status transport failures within the disposable scan budget."""
    deadline = time.monotonic() + 30
    last_transport_error = None
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            status = json.loads(request('/api/scan/status', timeout=min(2, remaining)))
        except urllib.error.HTTPError:
            raise
        except OSError as error:
            last_transport_error = type(error).__name__
        else:
            last_transport_error = None
            if not status['scanning']:
                return
        pause = min(0.5, deadline - time.monotonic())
        if pause > 0:
            time.sleep(pause)
    if last_transport_error is not None:
        raise AssertionError(f'disposable scan did not finish within 30 seconds; '
                             f'last status transport error: {last_transport_error}') from None
    raise AssertionError('disposable scan did not finish within 30 seconds')


def legacy_fixture_snapshot(database):
    """Read only nonsecret media/progress columns from the disposable fixture DB."""
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        connection.execute('BEGIN')
        assert connection.execute('PRAGMA quick_check').fetchone()[0] == 'ok'
        media = [dict(row) for row in connection.execute(
            'SELECT id,kind,path,title,size_bytes,mtime_secs,transcode_path FROM media WHERE id IN (1,2) ORDER BY id')]
        progress = [dict(row) for row in connection.execute(
            'SELECT identity_id,media_id,position,completed,updated_at FROM progress WHERE identity_id=1 AND media_id=2')]
        return {'media': media, 'progress': progress}


def epub_fixture(chapters):
    """Build a deterministic EPUB whose manifest order matches supplied XHTML entries."""
    entries = {
        'mimetype': b'application/epub+zip',
        'META-INF/container.xml': b'<container><rootfiles><rootfile full-path="book.opf"/></rootfiles></container>',
        'book.opf': ('<package><manifest>' + ''.join(
            f'<item id="c{index}" href="{name}" media-type="application/xhtml+xml"/>'
            for index, (name, _) in enumerate(chapters)) + '</manifest><spine>' + ''.join(
            f'<itemref idref="c{index}"/>' for index in range(len(chapters))) + '</spine></package>').encode(),
        **dict(chapters),
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        for name, content in entries.items():
            info = zipfile.ZipInfo(name, date_time=(2000, 1, 1, 0, 0, 0))
            archive.writestr(info, content)
    return buffer.getvalue()


def epub_fixture_snapshot(database):
    """Read the synthetic ebook metadata and device progress in one read-only snapshot."""
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        connection.execute('BEGIN')
        media = dict(connection.execute(
            'SELECT id,kind,path,title,size_bytes,mtime_secs,meta_json FROM media WHERE id=3').fetchone())
        metadata = json.loads(media.pop('meta_json'))
        progress = dict(connection.execute(
            'SELECT identity_id,media_id,position,completed,updated_at FROM progress WHERE identity_id=1 AND media_id=3').fetchone())
        return {'media': media, 'metadata': metadata, 'progress': progress}


def comic_cache_failure_snapshot(database):
    """Read the faulted synthetic comic row, progress and trigger without writing."""
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        connection.execute('BEGIN')
        media = dict(connection.execute(
            'SELECT id,kind,path,title,size_bytes,mtime_secs,meta_json FROM media WHERE id=4').fetchone())
        progress = dict(connection.execute(
            'SELECT identity_id,media_id,position,completed,updated_at FROM progress WHERE identity_id=1 AND media_id=4').fetchone())
        trigger = connection.execute("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='qa_reject_comic_cache'").fetchone()[0]
        return {'media': media, 'progress': progress, 'trigger': trigger}


def assert_project_removed(project):
    """Reject any remaining task-owned containers, networks or volumes."""
    for resource in ('container', 'network', 'volume'):
        flags = ('-a',) if resource == 'container' else ()
        remaining = docker(resource, 'ls', '-q', *flags, '--filter', f'label=com.docker.compose.project={project}').stdout.strip()
        assert not remaining, (resource, project, remaining)


def upstream_contract_qualification(args):
    """Run fresh two-root/admin/queued-ingest HTTP paths on the caller-qualified image."""
    global base
    project = 'frayd-upstream-qa-' + secrets.token_hex(4)
    print(f'SCENARIO upstream-contract: fresh {project}; curl -i through isolated proxy; expected transactional roots/admin and queued indexing')
    config = json.loads(docker('compose', '-f', str(ROOT / 'compose.yaml'), 'config', '--format', 'json').stdout)
    config['name'] = project
    for key, network in config['networks'].items():
        network['name'] = project + '_' + key
    for key, volume in config['volumes'].items():
        volume['name'] = project + '_' + key
    for service in config['services'].values():
        service.pop('build', None)
        service['restart'] = 'no'
    config['services']['app']['image'] = args.app_image
    config['services']['proxy']['image'] = args.proxy_image
    config['services']['proxy']['ports'][0]['published'] = '0'
    with tempfile.TemporaryDirectory(prefix=project) as directory:
        folder = Path(directory)
        data, roots, tools = (folder / key for key in ('data', 'roots', 'tools'))
        for path in (data, roots, tools, roots / 'a', roots / 'b',
                     data / 'thumbs', data / 'transcodes', roots / 'a/Internet'):
            path.mkdir()
            path.chmod(0o2770)
        # This owned synthetic DB is shared with the image's actual uid10001.
        # Precreating it fixes both host SQL access and inherited WAL/SHM modes.
        (data / 'frayd.sqlite').touch(mode=0o660)
        (data / 'frayd.sqlite').chmod(0o660)
        image = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=')
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, 'w') as comic:
            comic.writestr('001.png', image)
        source_bytes = archive.getvalue()
        for path in (roots / 'a/first.cbz', roots / 'b/second.cbz', tools / 'comic.cbz'):
            path.write_bytes(source_bytes)
        (tools / 'yt-dlp').write_text('#!/bin/sh\nif [ "$1" = --version ]; then echo owned-fixture; exit 0; fi\n'
                                    'cp /qa/comic.cbz /roots/a/Internet/queued.cbz\n')
        (tools / 'ffprobe').write_text('#!/bin/sh\nif [ -e /data/hold-probe ]; then touch /data/probe-entered; '
                                     'while [ -e /data/hold-probe ]; do sleep 0.05; done; fi\nexec /usr/bin/ffprobe "$@"\n')
        for command in ('yt-dlp', 'ffprobe'):
            (tools / command).chmod(0o755)
        app = config['services']['app']
        # Only the private disposable fixture group is shared; keep actual app UID
        # and dropped capabilities while avoiding world-readable synthetic sessions.
        app['group_add'] = [str(os.getgid())]
        app['volumes'] = [{'type': 'bind', 'source': str(data), 'target': '/data'},
                          {'type': 'bind', 'source': str(roots), 'target': '/roots'},
                          {'type': 'bind', 'source': str(tools), 'target': '/qa', 'read_only': True}]
        app['environment'] = {**app.get('environment', {}), 'FRAYD_LIBRARY': '',
                              'PATH': '/qa:/usr/local/bin:/usr/bin:/bin'}

        def compose(*commands):
            """Run each Compose command against this fixture project and its in-memory config."""
            return docker('compose', '-p', project, '-f', '-', *commands, input=json.dumps(config).encode())

        def sql(statement, parameters=()):
            """Execute and commit a fixture SQL statement; return all result rows."""
            with closing(sqlite3.connect(data / 'frayd.sqlite', timeout=10)) as db:
                result = db.execute(statement, parameters).fetchall()
                db.commit()
                return result

        def settings():
            """Read the effective admin settings using the owner capability."""
            return json.loads(curl_request('/api/admin/settings', token=token))

        def library():
            """Read the visible library through the disposable proxy."""
            return json.loads(curl_request('/api/library', token=token))

        def post_settings(payload, expected=204):
            """Post an admin settings change and assert the expected HTTP status."""
            return curl_request('/api/admin/settings', payload, token, 'POST', expected)

        with cleanup_project(compose):
            compose('up', '-d', '--wait', '--wait-timeout', '90')
            base = 'http://' + compose('port', 'proxy', '8080').stdout.decode().strip()
            ready()
            assert json.loads(curl_request('/api/health')) == {'status': 'ok'}
            owner = json.loads(curl_request('/api/setup', {'email': 'upstream-qa@example.invalid',
                                                         'password': secrets.token_urlsafe(32)}, method='POST'))
            token = owner['token']
            assert settings()['effective_library_paths'] == []
            assert json.loads(curl_request('/api/scan/status')) == {
                'scanning': False, 'scan_error': None, 'library_configured': False, 'library_count': 0, 'ingesting': None}
            curl_request('/api/scan', token=token, method='POST', expected=501)
            post_settings({'library_paths': ['/roots/a', '/roots/b']})
            curl_request('/api/scan', token=token, method='POST', expected=202)
            wait_for_scan_idle()
            assert json.loads(curl_request('/api/scan/status')) == {
                'scanning': False, 'scan_error': None, 'library_configured': True, 'library_count': 2, 'ingesting': None}
            items = library()
            ids = dict(sql('SELECT path,id FROM media'))
            assert set(ids) == {'/roots/a/first.cbz', '/roots/b/second.cbz'}, ids
            assert {item['id'] for item in items} == set(ids.values())
            media_id = ids['/roots/a/first.cbz']
            curl_request('/api/progress', {'media_id': media_id, 'position': 0.5}, token, 'POST', 204)
            curl_request('/api/annotations', {'media_id': media_id, 'kind': 'bookmark', 'spine_idx': 0,
                                             'cfi': 'qa-root-preservation'}, token, 'POST', 201)
            before_settings = settings()
            post_settings({'library_paths': ['/roots/a', '/missing-owned-qa'], 'site_name': 'must-not-commit'}, 400)
            assert settings() == before_settings
            sql("CREATE TRIGGER qa_settings_failure BEFORE INSERT ON settings WHEN NEW.key='qa_failure' BEGIN SELECT RAISE(ABORT,'owned QA'); END")
            post_settings({'library_paths': ['/roots/b'], 'qa_failure': True}, 500)
            assert settings() == before_settings
            sql('DROP TRIGGER qa_settings_failure')
            post_settings({'library_paths': ['/roots/b']})
            compose('restart', 'app')
            ready()
            wait_for_scan_idle()
            assert dict(sql('SELECT path,id FROM media')) == ids
            assert {item['id'] for item in library()} == set(ids.values())
            assert json.loads(curl_request(f'/api/progress/{media_id}', token=token))['position'] == 0.5
            assert len(json.loads(curl_request(f'/api/annotations/{media_id}', token=token))) == 1
            assert curl_request(f'/api/stream/{media_id}') == source_bytes
            assert settings()['effective_library_paths'] == ['/roots/b']
            post_settings({'library_path': '/roots/a'})
            post_settings({'library_paths': []})
            compose('restart', 'app')
            ready()
            wait_for_scan_idle()
            assert settings()['effective_library_paths'] == []
            assert sql("SELECT key FROM settings WHERE key='library_path'") == []
            print('PASS upstream-roots: two-root IDs, root removal, restart, progress, annotations, original bytes, legacy clear, missing-path400 and atomic DB500')

            app['environment']['FRAYD_LIBRARY'] = '/roots/b'
            compose('up', '-d', '--no-deps', 'app')
            ready()
            wait_for_scan_idle()
            assert settings()['library_locked'] is True
            assert settings()['library_paths'] == []
            assert settings()['effective_library_paths'] == ['/roots/b']
            post_settings({'library_paths': ['/roots/a']}, 409)
            app['environment']['FRAYD_LIBRARY'] = ''
            compose('up', '-d', '--no-deps', 'app')
            ready()
            wait_for_scan_idle()
            post_settings({'library_paths': ['/roots/a', '/roots/b']})

            # An actual scan blocks in our owned probe while the fake downloader exits.
            # Indexing must queue behind that scan, never advertise completion early.
            (data / 'hold-probe').touch()
            (roots / 'b/block.mp4').write_bytes(b'owned invalid probe input')
            curl_request('/api/scan', token=token, method='POST', expected=202)
            deadline = time.monotonic() + 10
            while not (data / 'probe-entered').exists():
                assert time.monotonic() < deadline, 'scan never reached owned blocking probe'
                time.sleep(0.05)
            curl_request('/api/ingest', {'url': 'https://example.invalid/owned-queue'}, token, 'POST', 202)
            deadline = time.monotonic() + 10
            while True:
                status = json.loads(curl_request('/api/scan/status'))
                if status['ingesting'] == 'indexing':
                    break
                assert time.monotonic() < deadline, status
                time.sleep(0.05)
            assert status['scanning'] is True
            curl_request('/api/ingest', {'url': 'https://example.invalid/owned-queue'}, token, 'POST', 409)
            curl_request('/api/scan', token=token, method='POST', expected=409)
            post_settings({'library_paths': []}, 409)
            curl_request(f'/api/media/{media_id}', token=token, method='DELETE', expected=409)
            (data / 'hold-probe').unlink()
            deadline = time.monotonic() + 30
            while True:
                status = json.loads(curl_request('/api/scan/status'))
                if status['ingesting'] in ('done', 'failed'):
                    break
                assert time.monotonic() < deadline, status
                time.sleep(0.1)
            assert status['ingesting'] == 'done', status
            queued_id = sql("SELECT id FROM media WHERE path='/roots/a/Internet/queued.cbz'")[0][0]
            assert queued_id in {item['id'] for item in library()}
            wait_for_scan_idle()
            print('PASS upstream-ingest: actual scan admission, fake owned downloader, queued indexing, busy409 and indexed-before-done; no network download')

            curl_request('/api/media/999999', token=token, method='DELETE', expected=404)
            curl_request(f'/api/media/{media_id}?identity=qa-nonadmin', method='DELETE', expected=403)
            curl_request(f'/api/media/{media_id}', token=token, method='DELETE', expected=409)
            own_id = sql("SELECT id FROM identities WHERE key='upstream-qa@example.invalid'")[0][0]
            target = 10001
            sql("INSERT INTO identities(id,kind,key) VALUES(?,'account','qa-target'),(10002,'account','qa-unrelated')", (target,))
            sql('INSERT INTO user_perms(identity_id,is_admin) VALUES(?,0),(10002,0)', (target,))
            sql('INSERT INTO progress(identity_id,media_id,position) VALUES(?,?,2),(10002,?,3)', (target, media_id, media_id))
            sql("INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi) VALUES(?,?,'bookmark',0,'target'),(10002,?,'bookmark',0,'other')", (target, media_id, media_id))
            sql('INSERT INTO sessions(token,identity_id,expires_at) VALUES(?,?,unixepoch()+1000)', (secrets.token_hex(32), target))
            curl_request('/api/admin/perms', {'identity_id': target, 'allowed_kinds': ['episode']}, token, 'POST', 204)
            curl_request('/api/admin/perms', {'identity_id': target, 'allowed_kinds': None}, token, 'POST', 204)
            assert sql('SELECT allowed_kinds FROM user_perms WHERE identity_id=?', (target,)) == [(None,)]
            curl_request(f'/api/admin/users/{own_id}', token=token, method='DELETE', expected=409)
            sql("CREATE TRIGGER qa_user_failure BEFORE DELETE ON identities WHEN OLD.id=10001 BEGIN SELECT RAISE(ABORT,'owned QA'); END")
            before = [sql(f'SELECT COUNT(*) FROM {table} WHERE identity_id=?', (target,)) for table in ('sessions', 'user_perms', 'progress', 'annotations')]
            curl_request(f'/api/admin/users/{target}', token=token, method='DELETE', expected=500)
            assert before == [sql(f'SELECT COUNT(*) FROM {table} WHERE identity_id=?', (target,)) for table in ('sessions', 'user_perms', 'progress', 'annotations')]
            sql('DROP TRIGGER qa_user_failure')
            curl_request(f'/api/admin/users/{target}', token=token, method='DELETE', expected=204)
            assert sql('SELECT id FROM identities WHERE id=?', (target,)) == []
            assert sql('SELECT position FROM progress WHERE identity_id=10002') == [(3.0,)]
            assert sql('SELECT cfi FROM annotations WHERE identity_id=10002') == [('other',)]
            sql('DELETE FROM annotations WHERE media_id=?', (media_id,))
            sql('DELETE FROM progress WHERE media_id=?', (media_id,))
            thumbnail = data / 'thumbs' / f'{media_id}.jpg'
            thumbnail_bytes = thumbnail.read_bytes()
            sql("CREATE TRIGGER qa_media_failure BEFORE DELETE ON media BEGIN SELECT RAISE(ABORT,'owned QA'); END")
            curl_request(f'/api/media/{media_id}', token=token, method='DELETE', expected=500)
            assert sql('SELECT path FROM media WHERE id=?', (media_id,)) == [('/roots/a/first.cbz',)]
            assert (roots / 'a/first.cbz').read_bytes() == source_bytes
            assert thumbnail.read_bytes() == thumbnail_bytes
            sql('DROP TRIGGER qa_media_failure')
            curl_request(f'/api/media/{media_id}', token=token, method='DELETE', expected=204)
            assert sql('SELECT id FROM media WHERE id=?', (media_id,)) == []
            for source in (roots / 'a/first.cbz', roots / 'b/second.cbz'):
                assert source.read_bytes() == source_bytes
            print('PASS upstream-admin: explicit-null permission clear, self409, user DB500 rollback and target-only cleanup, media refs/busy409/admin403/missing404/DB500/success204; original sources preserved')
            # An already configured removable/NAS root becoming unavailable must not
            # destroy HTTP/admin access or turn the stored snapshot into an empty list.
            # New writes still use the strict missing-path400 oracle above.
            # Normalize the expected rediscovery of the source-preserving deletion
            # before taking the separate unavailable-root snapshot.
            curl_request('/api/scan', token=token, method='POST', expected=202)
            wait_for_scan_idle()
            offline_id = ids['/roots/b/second.cbz']
            curl_request('/api/progress', {'media_id': offline_id, 'position': 0.75}, token, 'POST', 204)
            curl_request('/api/annotations', {'media_id': offline_id, 'kind': 'bookmark', 'spine_idx': 0,
                                             'cfi': 'qa-offline-root'}, token, 'POST', 201)
            before_offline = sql('SELECT id,path,title FROM media ORDER BY id')
            offline = roots / 'b-offline'
            (roots / 'b').rename(offline)
            try:
                compose('restart', 'app')
                ready()
                wait_for_scan_idle()
                assert json.loads(curl_request('/api/health')) == {'status': 'ok'}
                assert settings()['effective_library_paths'] == ['/roots/a', '/roots/b']
                raw_status = curl_request('/api/scan/status')
                status = json.loads(raw_status)
                assert status['library_configured'] is True and status['library_count'] == 2
                assert status['scanning'] is False and status['scan_error'] == 'Library root unavailable'
                assert all(path not in raw_status.decode() for path in ('/roots/a', '/roots/b')), 'scan status exposed a configured root path'
                assert sql('SELECT id,path,title FROM media ORDER BY id') == before_offline
                assert offline_id in {item['id'] for item in library()}
                assert json.loads(curl_request(f'/api/progress/{offline_id}', token=token))['position'] == 0.75
                assert len(json.loads(curl_request(f'/api/annotations/{offline_id}', token=token))) == 1
                assert curl_request(f'/api/stream/{queued_id}') == source_bytes
                assert not (roots / 'b').exists(), 'startup silently recreated unavailable root'
                assert (offline / 'second.cbz').read_bytes() == source_bytes
                before_offline_write = settings()
                post_settings({'library_paths': ['/roots/a', '/roots/b'], 'site_name': 'must-not-commit'}, 400)
                assert settings() == before_offline_write
            finally:
                offline.rename(roots / 'b')
            curl_request('/api/scan', token=token, method='POST', expected=202)
            wait_for_scan_idle()
            assert json.loads(curl_request('/api/scan/status'))['scan_error'] is None
            compose('restart', 'app')
            ready()
            wait_for_scan_idle()
            assert json.loads(curl_request('/api/scan/status'))['scan_error'] is None
            assert curl_request(f'/api/stream/{offline_id}') == source_bytes
            assert json.loads(curl_request(f'/api/progress/{offline_id}', token=token))['position'] == 0.75
            assert sql('SELECT id,path,title FROM media ORDER BY id') == before_offline
            print('PASS upstream-startup: unavailable stored root preserves healthy HTTP/admin/configured count, rows/progress/annotations/bytes; no root creation; return/rescan/restart recovers')
    assert_project_removed(project)
    print(f'PASS upstream cleanup: {project} containers/networks/volumes removed')


def main():
    """Exercise isolated container HTTP, persistence, DNS and read-only media paths."""
    global base
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app-image', default='frayd-qa:local')
    parser.add_argument('--proxy-image', default='frayd-proxy-qa:local')
    parser.add_argument('--upstream-only', action='store_true', help='Run only the fresh multi-root/admin/queued-ingest contract; default retains all historical scenarios too')
    args = parser.parse_args()
    if args.upstream_only:
        upstream_contract_qualification(args)
        return
    project = 'frayd-qa-' + secrets.token_hex(4)
    print(f'QA project: {project}')
    config = json.loads(docker('compose', '-f', str(ROOT / 'compose.yaml'), 'config', '--format', 'json').stdout)
    config['name'] = project
    config['services']['app']['image'] = args.app_image
    config['services']['proxy']['image'] = args.proxy_image
    for key, network in config['networks'].items():
        network['name'] = project + '_' + key
    for key, volume in config['volumes'].items():
        volume['name'] = project + '_' + key
    for service in config['services'].values():
        service.pop('build', None)
    config['services']['proxy']['ports'][0]['published'] = '0'
    config_data = json.dumps(config).encode()

    def compose(*args):
        """Run Compose against the task-owned in-memory configuration."""
        return docker('compose', '-p', project, '-f', '-', *args, input=config_data)

    with cleanup_project(compose):
        compose('up', '-d', '--wait', '--wait-timeout', '90')
        address = compose('port', 'proxy', '8080').stdout.decode().strip()
        assert address.startswith('127.0.0.1:'), address
        base = 'http://' + address
        name = compose('ps', '-q', 'app').stdout.decode().strip()
        runtime = json.loads(docker('inspect', name).stdout)[0]
        assert runtime['Config']['User'] == '10001:10001'
        assert runtime['HostConfig']['ReadonlyRootfs'] is True
        assert runtime['HostConfig']['PortBindings'] == {}
        assert runtime['HostConfig']['Memory'] == 2 * 1024**3
        network = next(iter(runtime['NetworkSettings']['Networks']))
        assert json.loads(docker('network', 'inspect', network).stdout)[0]['Internal'] is True
        print('PASS: nonroot, read-only root, bounded memory, unpublished app, internal app network')
        ready()
        assert json.loads(request('/api/setup')) == {'needed': True}
        assert b'Frayd' in request('/')
        assert request('/js/app.js')
        request('/.env.local', expected=404)
        docker('exec', name, 'sh', '-c', 'test ! -e /app/client/.env.local && test ! -d /app/client/install && test -w /data/transcodes')
        credentials = {'email': 'qa@example.invalid', 'password': secrets.token_urlsafe(32)}
        owner = json.loads(request('/api/setup', credentials))
        token = owner['token']
        assert owner['is_admin'] is True
        request('/api/setup', credentials, expected=409)
        assert json.loads(request('/api/auth/me', token=token))['is_admin'] is True
        print('PASS: first-run page, owner setup, duplicate-setup rejection, authenticated session')
        image = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=')
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, 'w') as comic:
            comic.writestr('001.png', image)
        fixture = archive.getvalue()
        docker('exec', '-i', name, 'sh', '-c', 'cat > /data/library/qa-comic.cbz', input=fixture)
        request('/api/scan', {}, token, expected=202)
        wait_for_scan_idle()
        items = json.loads(request('/api/library'))
        assert len(items) == 1 and items[0]['kind'] == 'comic', items
        media_id = items[0]['id']
        pages = json.loads(request(f'/api/comic/{media_id}/pages'))
        assert len(pages) == 1
        assert request(f'/api/comic/{media_id}/page/0') == image
        assert request(f'/api/stream/{media_id}') == fixture
        assert request(f'/api/stream/{media_id}', headers={'Range': 'bytes=5-'}, expected=206) == fixture[5:]
        for byte_range, status, content_range, expected_body in [
            ('bytes=5-9', 206, f'bytes 5-9/{len(fixture)}', fixture[5:10]),
            ('bytes=-5', 206, f'bytes {len(fixture)-5}-{len(fixture)-1}/{len(fixture)}', fixture[-5:]),
            (f'bytes={len(fixture)-3}-99999', 206, f'bytes {len(fixture)-3}-{len(fixture)-1}/{len(fixture)}', fixture[-3:]),
            (f'bytes={len(fixture)}-', 416, f'bytes */{len(fixture)}', b''),
            ('bytes=bad', 416, f'bytes */{len(fixture)}', b''),
            ('bytes=0-1,4-5', 200, None, fixture),
        ]:
            range_request = urllib.request.Request(base + f'/api/stream/{media_id}', headers={'Range': byte_range})
            try:
                range_response = urllib.request.urlopen(range_request, timeout=15)
            except urllib.error.HTTPError as error:
                range_response = error
            with range_response:
                assert range_response.status == status, (byte_range, range_response.status)
                assert range_response.headers.get('Content-Range') == content_range, byte_range
                assert range_response.headers.get('Accept-Ranges') == 'bytes', byte_range
                assert range_response.headers.get('Content-Type') == 'application/vnd.comicbook+zip', byte_range
                assert range_response.read(len(fixture) + 1) == expected_body, byte_range
                if status != 416:
                    assert int(range_response.headers['Content-Length']) == len(expected_body), byte_range
        print('PASS: bounded/open/suffix/clamped byte payloads, unsatisfied/malformed416, multipart complete200 and exact range headers')
        request('/api/progress', {'media_id': media_id, 'position': 0.5}, token, expected=204)
        assert json.loads(request(f'/api/progress/{media_id}', token=token))['position'] == 0.5
        print('PASS: synthetic comic scan, page bytes, full stream, range stream, saved progress')
        proxy_name = compose('ps', '-q', 'proxy').stdout.decode().strip()
        proxy_before = json.loads(docker('inspect', proxy_name).stdout)[0]
        old_ip = runtime['NetworkSettings']['Networks'][network]['IPAddress']
        compose('stop', 'app')
        compose('rm', '-f', 'app')
        config['services']['reserve'] = {
            'image': args.proxy_image, 'entrypoint': ['sleep', '120'],
            'networks': {'isolated': {}},
            'read_only': True, 'cap_drop': ['ALL'],
        }
        config_data = json.dumps(config).encode()
        compose('up', '-d', '--no-deps', 'reserve')
        compose('up', '-d', '--no-deps', 'app')
        name = compose('ps', '-q', 'app').stdout.decode().strip()
        new_runtime = json.loads(docker('inspect', name).stdout)[0]
        new_ip = new_runtime['NetworkSettings']['Networks'][network]['IPAddress']
        assert new_ip != old_ip, (old_ip, new_ip)
        print(f"PROBE: app IP {old_ip} -> {new_ip}; proxy {proxy_before['Id']} StartedAt={proxy_before['State']['StartedAt']}")
        started = time.monotonic()
        dns_recovery_ready(proxy_name, name)
        recovered = time.monotonic() - started
        assert recovered < 25, recovered
        proxy_after = json.loads(docker('inspect', proxy_name).stdout)[0]
        assert proxy_before['Id'] == proxy_after['Id']
        assert proxy_before['State']['StartedAt'] == proxy_after['State']['StartedAt']
        assert request(f'/api/stream/{media_id}?qa=dns') == fixture
        assert request(f'/api/stream/{media_id}?qa=dns', headers={'Range': 'bytes=5-'}, expected=206) == fixture[5:]
        websocket_upgrade(token)
        print(f"PASS: proxy container {proxy_after['Id']} StartedAt={proxy_after['State']['StartedAt']} unchanged")
        print(f'PASS: app IP changed {old_ip} -> {new_ip}; unchanged proxy recovered in {recovered:.2f}s; query/range bytes and WebSocket upgrade preserved')
        compose('rm', '-s', '-f', 'reserve')
        del config['services']['reserve']
        config_data = json.dumps(config).encode()
        compose('down')
        compose('up', '-d', '--wait', '--wait-timeout', '90')
        base = 'http://' + compose('port', 'proxy', '8080').stdout.decode().strip()
        ready()
        assert json.loads(request('/api/setup')) == {'needed': False}
        login = json.loads(request('/api/auth/login', credentials))
        token = login['token']
        assert json.loads(request(f'/api/progress/{media_id}', token=token))['position'] == 0.5
        request('/api/auth/logout', {}, token, expected=204)
        request('/api/auth/me', token=token, expected=401)
        print('PASS: container recreation preserves setup and progress; password login; logout revokes session')
        name = compose('ps', '-q', 'app').stdout.decode().strip()
        with tempfile.TemporaryDirectory(prefix=project + '-') as directory:
            media = Path(directory)
            media.chmod(0o755)
            (media / 'qa-comic.cbz').write_bytes(fixture)
            docker('exec', name, 'ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
                   'color=c=black:s=16x16:r=10:d=0.5', '-f', 'lavfi', '-i',
                   'sine=frequency=440:sample_rate=48000:duration=0.5', '-c:v', 'libx264',
                   '-pix_fmt', 'yuv420p', '-c:a', 'ac3', '-shortest', '/tmp/qa-video.mkv')
            video = docker('exec', name, 'cat', '/tmp/qa-video.mkv').stdout
            (media / 'qa-video.mkv').write_bytes(video)
            source_hash = hashlib.sha256(video).hexdigest()
            source_names = {path.name for path in media.iterdir()}
            wrapper = media / 'qa-ffprobe'
            wrapper.write_text('#!/bin/sh\ntouch /tmp/qa-ffprobe-entered\n'
                               'while [ ! -e /tmp/qa-ffprobe-release ]; do sleep 0.1; done\n'
                               'exec /usr/bin/ffprobe "$@"\n')
            wrapper.chmod(0o755)
            config['services']['app']['volumes'].append({
                'type': 'bind', 'source': str(wrapper), 'target': '/usr/local/bin/ffprobe',
                'read_only': True, 'bind': {'create_host_path': False},
            })
            config['services']['app']['volumes'].append({
                'type': 'bind', 'source': directory, 'target': '/data/library',
                'read_only': True, 'bind': {'create_host_path': False},
            })
            token = json.loads(request('/api/auth/login', credentials))['token']
            assert json.loads(request('/api/scan/status', timeout=2))['scanning'] is False
            check_scan_read_paths('BEFORE blocked scan', media_id, token)
            config_data = json.dumps(config).encode()
            compose('up', '-d', '--wait', '--wait-timeout', '90')
            base = 'http://' + compose('port', 'proxy', '8080').stdout.decode().strip()
            name = compose('ps', '-q', 'app').stdout.decode().strip()
            ready()
            assert request(f'/api/stream/{media_id}') == fixture
            docker('exec', name, 'sh', '-c', 'test ! -w /data/library/qa-comic.cbz')
            print('PASS: documented read-only media bind mount serves exact fixture bytes')
            # Recreated apps automatically scan; the wrapper must block that startup scan.
            assert json.loads(request('/api/scan/status', timeout=2))['scanning'] is True
            request('/api/scan', {}, token, expected=409)
            for _ in range(50):
                entered = docker('exec', name, 'sh', '-c',
                                 'if test -e /tmp/qa-ffprobe-entered; then echo entered; fi').stdout
                if entered.strip() == b'entered':
                    break
                time.sleep(0.1)
            else:
                raise AssertionError('scan did not enter blocked ffprobe')
            assert json.loads(request('/api/scan/status', timeout=2))['scanning'] is True
            check_scan_read_paths('DURING blocked scan', media_id, token)
            docker('exec', name, 'touch', '/tmp/qa-ffprobe-release')
            wait_for_scan_idle()
            check_scan_read_paths('AFTER released scan', media_id, token)
            movies = [item for item in json.loads(request('/api/library'))
                      if item['title'] == 'qa-video' and item['kind'] == 'movie']
            assert len(movies) == 1, movies
            print('PASS: released ffprobe completes scan against read-only media')
            movie_id = movies[0]['id']
            for _ in range(60):
                outputs = docker('exec', name, 'find', '/data/transcodes', '-type', 'f', '-name', 'audio.mp4').stdout.decode().splitlines()
                if outputs:
                    break
                time.sleep(0.5)
            else:
                raise AssertionError('real ffmpeg conversion did not produce cached output')
            assert len(outputs) == 1, outputs
            output = Path(outputs[0])
            assert output.parent.parent == Path('/data/transcodes') / str(movie_id), output
            uuid.UUID(output.parent.name)
            codecs = json.loads(docker('exec', name, '/usr/bin/ffprobe', '-v', 'error',
                                       '-show_entries', 'stream=codec_name', '-of', 'json', str(output)).stdout)
            assert {stream['codec_name'] for stream in codecs['streams']} == {'h264', 'aac'}, codecs
            converted = docker('exec', name, 'cat', str(output)).stdout
            assert converted and converted != video
            for _ in range(40):
                if request(f'/api/stream/{movie_id}') == converted:
                    break
                time.sleep(0.1)
            else:
                raise AssertionError('completed conversion path was not published to HTTP streaming')
            assert hashlib.sha256((media / 'qa-video.mkv').read_bytes()).hexdigest() == source_hash
            assert {path.name for path in media.iterdir()} == source_names | {'qa-ffprobe'}
            compose('restart', 'app')
            ready()
            assert request(f'/api/stream/{movie_id}') == converted
            print(f'PASS: actual H264/AC3 -> H264/AAC conversion at {output}; source hash/names unchanged; exact converted HTTP bytes persist after app restart')
            docker('exec', name, 'touch', '/tmp/qa-ffprobe-release')
            wait_for_scan_idle()
            request('/api/progress', {'media_id': movie_id, 'position': 0.25}, token, expected=204)
            assert json.loads(request(f'/api/progress/{movie_id}', token=token))['position'] == 0.25
            old_outputs = set(docker('exec', name, 'find', '/data/transcodes', '-type', 'f', '-name', 'audio.mp4').stdout.decode().splitlines())
            docker('exec', name, 'ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
                   'color=c=blue:s=32x32:r=10:d=1.5', '-f', 'lavfi', '-i',
                   'sine=frequency=880:sample_rate=48000:duration=1.5', '-c:v', 'libx264',
                   '-pix_fmt', 'yuv420p', '-c:a', 'ac3', '-shortest', '/tmp/qa-replacement.mkv')
            replacement_video = docker('exec', name, 'cat', '/tmp/qa-replacement.mkv').stdout
            assert replacement_video != video and len(replacement_video) != len(video)
            source = media / 'qa-video.mkv'
            old_stat = source.stat()
            source.write_bytes(replacement_video)
            os.utime(source, ns=(old_stat.st_atime_ns, old_stat.st_mtime_ns + 5_000_000_000))
            replacement_hash = hashlib.sha256(replacement_video).hexdigest()
            assert source.stat().st_size != old_stat.st_size
            assert source.stat().st_mtime_ns > old_stat.st_mtime_ns
            print(f'PROBE: replaced only synthetic source for media {movie_id}; size {old_stat.st_size}->{source.stat().st_size}; mtime_ns {old_stat.st_mtime_ns}->{source.stat().st_mtime_ns}; previous cache {output}')
            request('/api/scan', {}, token, expected=202)
            wait_for_scan_idle()
            assert request(f'/api/stream/{movie_id}', timeout=2) != converted, (
                f'rescan served stale pre-replacement converted bytes for media {movie_id}')
            assert json.loads(request(f'/api/progress/{movie_id}', token=token))['position'] == 0.25
            for _ in range(60):
                current_outputs = set(docker('exec', name, 'find', '/data/transcodes', '-type', 'f', '-name', 'audio.mp4').stdout.decode().splitlines())
                new_outputs = current_outputs - old_outputs
                if new_outputs:
                    break
                time.sleep(0.5)
            else:
                raise AssertionError('replacement source did not produce a new conversion UUID')
            assert len(new_outputs) == 1, new_outputs
            replacement_output = Path(new_outputs.pop())
            assert replacement_output != output
            assert replacement_output.parent.parent == Path('/data/transcodes') / str(movie_id)
            assert replacement_output.parent.name != output.parent.name
            uuid.UUID(replacement_output.parent.name)
            replacement_codecs = json.loads(docker('exec', name, '/usr/bin/ffprobe', '-v', 'error',
                '-show_entries', 'stream=codec_name', '-of', 'json', str(replacement_output)).stdout)
            assert {stream['codec_name'] for stream in replacement_codecs['streams']} == {'h264', 'aac'}
            replacement_converted = docker('exec', name, 'cat', str(replacement_output)).stdout
            assert replacement_converted and replacement_converted not in (converted, replacement_video)
            for _ in range(40):
                if request(f'/api/stream/{movie_id}', timeout=2) == replacement_converted:
                    break
                time.sleep(0.1)
            else:
                raise AssertionError('replacement conversion was not published to HTTP streaming')
            assert hashlib.sha256(source.read_bytes()).hexdigest() == replacement_hash
            assert {path.name for path in media.iterdir()} == source_names | {'qa-ffprobe'}
            docker('exec', name, 'sh', '-c', 'test ! -w /data/library/qa-video.mkv')
            replacement_items = [item for item in json.loads(request('/api/library')) if item['title'] == 'qa-video']
            assert len(replacement_items) == 1 and replacement_items[0]['id'] == movie_id
            print(f'PASS: source replacement uses new H264/AAC cache {replacement_output}; old converted bytes rejected; source hash/names and media ID/progress preserved')
            compose('restart', 'app')
            ready()
            docker('exec', name, 'touch', '/tmp/qa-ffprobe-release')
            wait_for_scan_idle()
            assert request(f'/api/stream/{movie_id}') == replacement_converted
            assert json.loads(request(f'/api/progress/{movie_id}', token=token))['position'] == 0.25
            assert hashlib.sha256(source.read_bytes()).hexdigest() == replacement_hash
            assert {path.name for path in media.iterdir()} == source_names | {'qa-ffprobe'}
            assert any(item['id'] == movie_id for item in json.loads(request('/api/library')))
            print('PASS: replacement converted HTTP bytes, media ID and progress persist after app restart')
            compose('down', '--volumes')

        with tempfile.TemporaryDirectory(prefix=project + '-legacy-') as directory:
            data = Path(directory)
            data.chmod(0o777)
            library = data / 'library'
            library.mkdir(mode=0o755)
            for cache in ['thumbs', 'transcodes']:
                (data / cache).mkdir()
                (data / cache).chmod(0o777)
            legacy_source = library / 'Film.mkv'
            genuine_source = library / 'Film.browser.mp4'
            legacy_source.write_bytes(video)
            genuine_source.write_bytes(converted)
            old_chapters = [('old-one.xhtml', b'<html><body>OLD ONE</body></html>'),
                            ('old-two.xhtml', b'<html><body>OLD TWO</body></html>'),
                            ('removed.xhtml', b'<html><body>REMOVED CHAPTER</body></html>')]
            new_chapters = [('new-one.xhtml', b'<html><body>NEW ONE ' + b'expanded ' * 80 + b'</body></html>'),
                            ('new-two.xhtml', b'<html><body>NEW TWO</body></html>')]
            old_epub = epub_fixture(old_chapters)
            new_epub = epub_fixture(new_chapters)
            assert old_epub != new_epub and len(old_epub) != len(new_epub)
            ebook_source = library / 'qa-book.epub'
            ebook_source.write_bytes(old_epub)
            comic_source = library / 'qa-cache-failure.cbz'
            comic_source.write_bytes(fixture)
            comic_metadata = {'synopsis': 'Keep comic synopsis', 'custom': {'edition': 4}}
            ebook_metadata = {'synopsis': 'Keep EPUB synopsis', 'custom': {'edition': 7},
                             'pages': ['unrelated-preserved-field']}
            assert len(converted) != len(replacement_converted)
            timestamp = int(time.time()) - 10
            for path in [legacy_source, genuine_source, ebook_source, comic_source]:
                os.utime(path, (timestamp, timestamp))
            database = data / 'frayd.sqlite'
            with closing(sqlite3.connect(database)) as connection:
                connection.executescript((ROOT / 'server/schema.sql').read_text())
                connection.executescript((ROOT / 'server/schema_annotations.sql').read_text())
                connection.execute("INSERT INTO media(id,kind,path,title,size_bytes,mtime_secs,transcode_path) VALUES(1,'movie',?,'Legacy film',?,?,?)",
                    ('/data/library/Film.mkv', len(video), timestamp, '/data/library/Film.browser.mp4'))
                connection.execute("INSERT INTO media(id,kind,path,title,size_bytes,mtime_secs) VALUES(2,'movie',?,'Genuine browser source',?,?)",
                    ('/data/library/Film.browser.mp4', len(converted), timestamp))
                connection.execute("INSERT INTO identities(id,kind,key) VALUES(1,'device','qa-legacy-device')")
                connection.execute('INSERT INTO progress(identity_id,media_id,position,updated_at) VALUES(1,2,42,12345)')
                connection.execute("INSERT INTO media(id,kind,path,title,size_bytes,mtime_secs,meta_json) VALUES(3,'ebook',?,'Preserved EPUB title',?,?,?)",
                    ('/data/library/qa-book.epub', len(old_epub), timestamp, json.dumps(ebook_metadata)))
                connection.execute('INSERT INTO progress(identity_id,media_id,position,updated_at) VALUES(1,3,1,12346)')
                connection.execute("INSERT INTO media(id,kind,path,title,size_bytes,mtime_secs,meta_json) VALUES(4,'comic',?,'Preserved uncached comic',?,?,?)",
                    ('/data/library/qa-cache-failure.cbz', len(fixture), timestamp, json.dumps(comic_metadata)))
                connection.execute('INSERT INTO progress(identity_id,media_id,position,updated_at) VALUES(1,4,0,12347)')
                connection.execute("""CREATE TRIGGER qa_reject_comic_cache BEFORE UPDATE OF meta_json ON media
                    WHEN OLD.id=4 AND json_extract(NEW.meta_json,'$.pages') IS NOT NULL
                    BEGIN SELECT RAISE(ABORT,'qa comic cache publication refused'); END""")
                connection.commit()
                try:
                    connection.execute("UPDATE media SET meta_json=json_set(meta_json,'$.pages',json('[\"001.png\"]')) WHERE id=4")
                except sqlite3.IntegrityError as error:
                    assert 'qa comic cache publication refused' in str(error)
                else:
                    raise AssertionError('comic cache write-failure trigger did not reject its probe')
                finally:
                    connection.rollback()
            database.chmod(0o666)
            before_legacy = legacy_fixture_snapshot(database)
            legacy_hash = hashlib.sha256(video).hexdigest()
            config['services']['app']['volumes'] = [
                {'type': 'bind', 'source': str(data), 'target': '/data',
                 'bind': {'create_host_path': False}},
                {'type': 'bind', 'source': str(library), 'target': '/data/library',
                 'read_only': True, 'bind': {'create_host_path': False}},
            ]
            config_data = json.dumps(config).encode()
            with cleanup_project(compose):
                compose('up', '-d', '--wait', '--wait-timeout', '90')
                base = 'http://' + compose('port', 'proxy', '8080').stdout.decode().strip()
                name = compose('ps', '-q', 'app').stdout.decode().strip()
                ready()
                wait_for_scan_idle()
                assert legacy_fixture_snapshot(database) == before_legacy
                assert request('/api/stream/2') == converted
                assert json.loads(request('/api/progress/2?identity=qa-legacy-device'))['position'] == 42
                genuine_source.write_bytes(replacement_converted)
                os.utime(genuine_source, (timestamp + 5, timestamp + 5))
                genuine_hash = hashlib.sha256(replacement_converted).hexdigest()
                print(f'PROBE: genuine legacy-referenced media2 Film.browser.mp4 source size {len(converted)}->{len(replacement_converted)}, mtime {timestamp}->{timestamp + 5}; media1 legacy pointer retained')
                request('/api/scan', {}, expected=202)
                wait_for_scan_idle()
                after_legacy = legacy_fixture_snapshot(database)
                assert len(after_legacy['media']) == 2
                assert after_legacy['media'][0] == before_legacy['media'][0]
                assert after_legacy['progress'] == before_legacy['progress']
                assert hashlib.sha256(legacy_source.read_bytes()).hexdigest() == legacy_hash
                assert hashlib.sha256(genuine_source.read_bytes()).hexdigest() == genuine_hash
                assert {path.name for path in library.iterdir()} == {'Film.mkv', 'Film.browser.mp4', 'qa-book.epub', 'qa-cache-failure.cbz'}
                assert request('/api/stream/2') == replacement_converted
                assert json.loads(request('/api/progress/2?identity=qa-legacy-device'))['position'] == 42
                genuine_row = after_legacy['media'][1]
                assert genuine_row['size_bytes'] == len(replacement_converted), (
                    f"legacy-referenced genuine media2 stored size {genuine_row['size_bytes']} != actual {len(replacement_converted)} after scan")
                expected_genuine = {**before_legacy['media'][1], 'size_bytes': len(replacement_converted), 'mtime_secs': timestamp + 5}
                assert genuine_row == expected_genuine
                docker('exec', name, 'sh', '-c', 'test ! -w /data/library/Film.browser.mp4')
                print(f"PASS: legacy-referenced genuine media2 DB size {before_legacy['media'][1]['size_bytes']}->{genuine_row['size_bytes']}, mtime refreshed; both IDs/titles, progress42, source hashes/names and legacy reference preserved")
                compose('restart', 'app')
                ready()
                wait_for_scan_idle()
                assert legacy_fixture_snapshot(database) == after_legacy
                assert request('/api/stream/2') == replacement_converted
                assert json.loads(request('/api/progress/2?identity=qa-legacy-device'))['position'] == 42
                assert hashlib.sha256(legacy_source.read_bytes()).hexdigest() == legacy_hash
                assert hashlib.sha256(genuine_source.read_bytes()).hexdigest() == genuine_hash
                print('PASS: genuine legacy-reference metadata, device progress and exact source bytes persist after restart')
                old_spine = [name for name, _ in old_chapters]
                new_spine = [name for name, _ in new_chapters]
                assert json.loads(request('/api/epub/3/spine')) == old_spine
                assert request('/api/epub/3/chunk/0') == old_chapters[0][1]
                assert request('/api/epub/3/chunk/2') == old_chapters[2][1]
                warm_ebook = epub_fixture_snapshot(database)
                assert warm_ebook['metadata'] == {**ebook_metadata, 'spine': old_spine}
                assert warm_ebook['media']['id'] == 3
                assert warm_ebook['media']['title'] == 'Preserved EPUB title'
                assert warm_ebook['progress']['position'] == 1
                request('/api/scan', {}, expected=202)
                wait_for_scan_idle()
                assert epub_fixture_snapshot(database) == warm_ebook
                ebook_source.write_bytes(new_epub)
                os.utime(ebook_source, (timestamp + 5, timestamp + 5))
                ebook_hash = hashlib.sha256(new_epub).hexdigest()
                print(f'PROBE: replaced EPUB media3 size {len(old_epub)}->{len(new_epub)}, mtime {timestamp}->{timestamp + 5}; warmed spine {old_spine} -> expected {new_spine}')
                request('/api/scan', {}, expected=202)
                wait_for_scan_idle()
                invalidated_ebook = epub_fixture_snapshot(database)
                expected_ebook = {
                    'media': {**warm_ebook['media'], 'size_bytes': len(new_epub), 'mtime_secs': timestamp + 5},
                    'metadata': ebook_metadata,
                    'progress': warm_ebook['progress'],
                }
                if invalidated_ebook['metadata'].get('spine') == old_spine:
                    request('/api/epub/3/chunk/0', expected=404)
                    print('PROBE: stale SQLite spine retained and new EPUB chapter0 HTTP404 reproduced')
                assert 'spine' not in invalidated_ebook['metadata'], 'EPUB rescan retained stale cached spine'
                assert invalidated_ebook == expected_ebook
                assert json.loads(request('/api/epub/3/spine')) == new_spine
                assert request('/api/epub/3/chunk/0') == new_chapters[0][1]
                assert request('/api/epub/3/chunk/1') == new_chapters[1][1]
                request('/api/epub/3/chunk/2', expected=404)
                refreshed_ebook = epub_fixture_snapshot(database)
                assert refreshed_ebook == {**expected_ebook, 'metadata': {**ebook_metadata, 'spine': new_spine}}
                assert json.loads(request('/api/progress/3?identity=qa-legacy-device')) == {'position': 1.0, 'updated_at': 12346}
                assert hashlib.sha256(ebook_source.read_bytes()).hexdigest() == ebook_hash
                assert hashlib.sha256(legacy_source.read_bytes()).hexdigest() == legacy_hash
                assert hashlib.sha256(genuine_source.read_bytes()).hexdigest() == genuine_hash
                assert {path.name for path in library.iterdir()} == {'Film.mkv', 'Film.browser.mp4', 'qa-book.epub', 'qa-cache-failure.cbz'}
                print('PASS: EPUB media3 old3->new2 renamed chapters; SQL spine invalidated then refreshed, exact new XHTML0/1 and removed chapter2 HTTP404; ID/title/progress/custom/pages/source hashes preserved')
                compose('restart', 'app')
                ready()
                wait_for_scan_idle()
                assert epub_fixture_snapshot(database) == refreshed_ebook
                assert request('/api/epub/3/chunk/0') == new_chapters[0][1]
                request('/api/epub/3/chunk/2', expected=404)
                assert json.loads(request('/api/progress/3?identity=qa-legacy-device'))['position'] == 1
                assert hashlib.sha256(ebook_source.read_bytes()).hexdigest() == ebook_hash
                print('PASS: replaced EPUB metadata/cache, current chapters and device progress persist after restart')
                comic_before = comic_cache_failure_snapshot(database)
                assert json.loads(comic_before['media']['meta_json']) == comic_metadata
                assert comic_before['progress']['position'] == 0
                comic_hash = hashlib.sha256(fixture).hexdigest()
                assert hashlib.sha256(comic_source.read_bytes()).hexdigest() == comic_hash
                docker('exec', name, 'sh', '-c', 'test -r /data/library/qa-cache-failure.cbz && test ! -w /data/library/qa-cache-failure.cbz')
                print('PROBE: comic media4 has readable realCBZ, uncached metadata, progress0 and validated SQLiteABORT trigger scoped only to pages publication')
                assert json.loads(request('/api/comic/4/pages')) == ['001.png']
                assert request('/api/comic/4/page/0') == image
                request('/api/comic/4/page/1', expected=404)
                assert comic_cache_failure_snapshot(database) == comic_before
                assert json.loads(request('/api/progress/4?identity=qa-legacy-device')) == {'position': 0.0, 'updated_at': 12347}
                assert hashlib.sha256(comic_source.read_bytes()).hexdigest() == comic_hash
                assert {path.name for path in library.iterdir()} == {'Film.mkv', 'Film.browser.mp4', 'qa-book.epub', 'qa-cache-failure.cbz'}
                print('PASS: comic optional-cache SQLiteerror still returns pagesHTTP200 and exactPNGpage0HTTP200; outofrange404; cache/ID/title/meta/progress0/source unchanged')
                compose('restart', 'app')
                ready()
                wait_for_scan_idle()
                assert json.loads(request('/api/comic/4/pages')) == ['001.png']
                assert request('/api/comic/4/page/0') == image
                assert comic_cache_failure_snapshot(database) == comic_before
                assert hashlib.sha256(comic_source.read_bytes()).hexdigest() == comic_hash
                print('PASS: comic remains readable after restart while cache-publication trigger still rejects writes and metadata/progress stay unchanged')

    assert_project_removed(project)
    print(f'PASS: task-owned {project} containers, networks and volume removed')
    upstream_contract_qualification(args)


if __name__ == '__main__':
    main()
